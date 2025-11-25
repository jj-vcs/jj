// Copyright 2020-2023 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Template environment for `jj log`, `jj evolog` and similar.

use std::any::Any;
use std::cmp::Ordering;
use std::cmp::max;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use bstr::BString;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream::BoxStream;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::conflicts;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::copies::CopiesTreeDiffEntry;
use jj_lib::copies::CopiesTreeDiffEntryPath;
use jj_lib::copies::CopyRecords;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::extensions_map::ExtensionsMap;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::id_prefix::IdPrefixIndex;
use jj_lib::index::IndexResult;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::LocalRemoteRefTarget;
use jj_lib::op_store::OperationId;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo;
use jj_lib::repo::RepoLoader;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetContainingFn;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetModifier;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::signing::SigStatus;
use jj_lib::signing::SignError;
use jj_lib::signing::SignResult;
use jj_lib::signing::Verification;
use jj_lib::store::Store;
use jj_lib::trailer;
use jj_lib::trailer::Trailer;
use once_cell::unsync::OnceCell;
use pollster::FutureExt as _;
use serde::Serialize as _;

use crate::diff_util;
use crate::diff_util::DiffStatEntry;
use crate::diff_util::DiffStats;
use crate::formatter::Formatter;
use crate::operation_templater;
use crate::operation_templater::OperationTemplateBuildFnTable;
use crate::operation_templater::OperationTemplateEnvironment;
use crate::operation_templater::OperationTemplatePropertyKind;
use crate::operation_templater::OperationTemplatePropertyVar;
use crate::revset_util;
use crate::template_builder;
use crate::template_builder::BuildContext;
use crate::template_builder::CoreTemplateBuildFnTable;
use crate::template_builder::CoreTemplatePropertyKind;
use crate::template_builder::CoreTemplatePropertyVar;
use crate::template_builder::TemplateBuildMethodFnMap;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::expect_stringify_expression;
use crate::template_builder::merge_fn_map;
use crate::template_parser;
use crate::template_parser::ExpressionNode;
use crate::template_parser::FunctionCallNode;
use crate::template_parser::TemplateDiagnostics;
use crate::template_parser::TemplateParseError;
use crate::template_parser::TemplateParseResult;
use crate::templater;
use crate::templater::BoxedSerializeProperty;
use crate::templater::BoxedTemplateProperty;
use crate::templater::ListTemplate;
use crate::templater::PlainTextFormattedProperty;
use crate::templater::SizeHint;
use crate::templater::Template;
use crate::templater::TemplateFormatter;
use crate::templater::TemplatePropertyError;
use crate::templater::TemplatePropertyExt as _;

pub trait CommitTemplateLanguageExtension {
    fn build_fn_table<'repo>(&self) -> CommitTemplateBuildFnTable<'repo>;

    fn build_cache_extensions(&self, extensions: &mut ExtensionsMap);
}

/// Template environment for `jj log` and `jj evolog`.
pub struct CommitTemplateLanguage<'repo> {
    repo: &'repo dyn Repo,
    path_converter: &'repo RepoPathUiConverter,
    workspace_name: WorkspaceNameBuf,
    // RevsetParseContext doesn't borrow a repo, but we'll need 'repo lifetime
    // anyway to capture it to evaluate dynamically-constructed user expression
    // such as `revset("ancestors(" ++ commit_id ++ ")")`.
    // TODO: Maybe refactor context structs? RepoPathUiConverter and
    // WorkspaceName are contained in RevsetParseContext for example.
    revset_parse_context: RevsetParseContext<'repo>,
    id_prefix_context: &'repo IdPrefixContext,
    immutable_expression: Arc<UserRevsetExpression>,
    conflict_marker_style: ConflictMarkerStyle,
    build_fn_table: CommitTemplateBuildFnTable<'repo>,
    keyword_cache: CommitKeywordCache<'repo>,
    cache_extensions: ExtensionsMap,
}

impl<'repo> CommitTemplateLanguage<'repo> {
    /// Sets up environment where commit template will be transformed to
    /// evaluation tree.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        repo: &'repo dyn Repo,
        path_converter: &'repo RepoPathUiConverter,
        workspace_name: &WorkspaceName,
        revset_parse_context: RevsetParseContext<'repo>,
        id_prefix_context: &'repo IdPrefixContext,
        immutable_expression: Arc<UserRevsetExpression>,
        conflict_marker_style: ConflictMarkerStyle,
        extensions: &[impl AsRef<dyn CommitTemplateLanguageExtension>],
    ) -> Self {
        let mut build_fn_table = CommitTemplateBuildFnTable::builtin();
        let mut cache_extensions = ExtensionsMap::empty();

        for extension in extensions {
            build_fn_table.merge(extension.as_ref().build_fn_table());
            extension
                .as_ref()
                .build_cache_extensions(&mut cache_extensions);
        }

        CommitTemplateLanguage {
            repo,
            path_converter,
            workspace_name: workspace_name.to_owned(),
            revset_parse_context,
            id_prefix_context,
            immutable_expression,
            conflict_marker_style,
            build_fn_table,
            keyword_cache: CommitKeywordCache::default(),
            cache_extensions,
        }
    }
}

impl<'repo> TemplateLanguage<'repo> for CommitTemplateLanguage<'repo> {
    type Property = CommitTemplatePropertyKind<'repo>;

    fn settings(&self) -> &UserSettings {
        self.repo.base_repo().settings()
    }

    fn build_function(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        let table = &self.build_fn_table.core;
        table.build_function(self, diagnostics, build_ctx, function)
    }

    fn build_method(
        &self,
        diagnostics: &mut TemplateDiagnostics,
        build_ctx: &BuildContext<Self::Property>,
        property: Self::Property,
        function: &FunctionCallNode,
    ) -> TemplateParseResult<Self::Property> {
        let type_name = property.type_name();
        match property {
            CommitTemplatePropertyKind::Core(property) => {
                let table = &self.build_fn_table.core;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::Operation(property) => {
                let table = &self.build_fn_table.operation;
                table.build_method(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::Commit(property) => {
                let table = &self.build_fn_table.commit_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CommitOpt(property) => {
                let type_name = "Commit";
                let table = &self.build_fn_table.commit_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::CommitList(property) => {
                let table = &self.build_fn_table.commit_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CommitEvolutionEntry(property) => {
                let table = &self.build_fn_table.commit_evolution_entry_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CommitRef(property) => {
                let table = &self.build_fn_table.commit_ref_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CommitRefOpt(property) => {
                let type_name = "CommitRef";
                let table = &self.build_fn_table.commit_ref_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::CommitRefList(property) => {
                let table = &self.build_fn_table.commit_ref_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::WorkspaceRef(property) => {
                let table = &self.build_fn_table.workspace_ref_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::WorkspaceRefOpt(property) => {
                let type_name = "WorkspaceRef";
                let table = &self.build_fn_table.workspace_ref_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::WorkspaceRefList(property) => {
                let table = &self.build_fn_table.workspace_ref_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::RefSymbol(property) => {
                let table = &self.build_fn_table.core.string_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.map(|RefSymbolBuf(s)| s).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::RefSymbolOpt(property) => {
                let type_name = "RefSymbol";
                let table = &self.build_fn_table.core.string_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property
                    .try_unwrap(type_name)
                    .map(|RefSymbolBuf(s)| s)
                    .into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::RepoPath(property) => {
                let table = &self.build_fn_table.repo_path_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::RepoPathOpt(property) => {
                let type_name = "RepoPath";
                let table = &self.build_fn_table.repo_path_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::ChangeId(property) => {
                let table = &self.build_fn_table.change_id_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CommitId(property) => {
                let table = &self.build_fn_table.commit_id_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::ShortestIdPrefix(property) => {
                let table = &self.build_fn_table.shortest_id_prefix_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TreeDiff(property) => {
                let table = &self.build_fn_table.tree_diff_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TreeDiffEntry(property) => {
                let table = &self.build_fn_table.tree_diff_entry_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TreeDiffEntryList(property) => {
                let table = &self.build_fn_table.tree_diff_entry_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TreeEntry(property) => {
                let table = &self.build_fn_table.tree_entry_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TreeEntryList(property) => {
                let table = &self.build_fn_table.tree_entry_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::DiffStats(property) => {
                let table = &self.build_fn_table.diff_stats_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                // Strip off formatting parameters which are needed only for the
                // default template output.
                let property = property.map(|formatted| formatted.stats).into_dyn();
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::DiffStatEntry(property) => {
                let table = &self.build_fn_table.diff_stat_entry_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::DiffStatEntryList(property) => {
                let table = &self.build_fn_table.diff_stat_entry_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::CryptographicSignatureOpt(property) => {
                let type_name = "CryptographicSignature";
                let table = &self.build_fn_table.cryptographic_signature_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                let inner_property = property.try_unwrap(type_name).into_dyn();
                build(self, diagnostics, build_ctx, inner_property, function)
            }
            CommitTemplatePropertyKind::AnnotationLine(property) => {
                let type_name = "AnnotationLine";
                let table = &self.build_fn_table.annotation_line_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::Trailer(property) => {
                let table = &self.build_fn_table.trailer_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
            CommitTemplatePropertyKind::TrailerList(property) => {
                let table = &self.build_fn_table.trailer_list_methods;
                let build = template_parser::lookup_method(type_name, table, function)?;
                build(self, diagnostics, build_ctx, property, function)
            }
        }
    }
}

// If we need to add multiple languages that support Commit types, this can be
// turned into a trait which extends TemplateLanguage.
impl<'repo> CommitTemplateLanguage<'repo> {
    pub fn repo(&self) -> &'repo dyn Repo {
        self.repo
    }

    pub fn workspace_name(&self) -> &WorkspaceName {
        &self.workspace_name
    }

    pub fn keyword_cache(&self) -> &CommitKeywordCache<'repo> {
        &self.keyword_cache
    }

    pub fn cache_extension<T: Any>(&self) -> Option<&T> {
        self.cache_extensions.get::<T>()
    }
}

impl OperationTemplateEnvironment for CommitTemplateLanguage<'_> {
    fn repo_loader(&self) -> &RepoLoader {
        self.repo.base_repo().loader()
    }

    fn current_op_id(&self) -> Option<&OperationId> {
        // TODO: Maybe return None if the repo is a MutableRepo?
        Some(self.repo.base_repo().op_id())
    }
}

pub enum CommitTemplatePropertyKind<'repo> {
    Core(CoreTemplatePropertyKind<'repo>),
    Operation(OperationTemplatePropertyKind<'repo>),
    Commit(BoxedTemplateProperty<'repo, Commit>),
    CommitOpt(BoxedTemplateProperty<'repo, Option<Commit>>),
    CommitList(BoxedTemplateProperty<'repo, Vec<Commit>>),
    CommitEvolutionEntry(BoxedTemplateProperty<'repo, CommitEvolutionEntry>),
    CommitRef(BoxedTemplateProperty<'repo, Rc<CommitRef>>),
    CommitRefOpt(BoxedTemplateProperty<'repo, Option<Rc<CommitRef>>>),
    CommitRefList(BoxedTemplateProperty<'repo, Vec<Rc<CommitRef>>>),
    WorkspaceRef(BoxedTemplateProperty<'repo, WorkspaceRef>),
    WorkspaceRefOpt(BoxedTemplateProperty<'repo, Option<WorkspaceRef>>),
    WorkspaceRefList(BoxedTemplateProperty<'repo, Vec<WorkspaceRef>>),
    RefSymbol(BoxedTemplateProperty<'repo, RefSymbolBuf>),
    RefSymbolOpt(BoxedTemplateProperty<'repo, Option<RefSymbolBuf>>),
    RepoPath(BoxedTemplateProperty<'repo, RepoPathBuf>),
    RepoPathOpt(BoxedTemplateProperty<'repo, Option<RepoPathBuf>>),
    ChangeId(BoxedTemplateProperty<'repo, ChangeId>),
    CommitId(BoxedTemplateProperty<'repo, CommitId>),
    ShortestIdPrefix(BoxedTemplateProperty<'repo, ShortestIdPrefix>),
    TreeDiff(BoxedTemplateProperty<'repo, TreeDiff>),
    TreeDiffEntry(BoxedTemplateProperty<'repo, TreeDiffEntry>),
    TreeDiffEntryList(BoxedTemplateProperty<'repo, Vec<TreeDiffEntry>>),
    TreeEntry(BoxedTemplateProperty<'repo, TreeEntry>),
    TreeEntryList(BoxedTemplateProperty<'repo, Vec<TreeEntry>>),
    DiffStats(BoxedTemplateProperty<'repo, DiffStatsFormatted<'repo>>),
    DiffStatEntry(BoxedTemplateProperty<'repo, DiffStatEntry>),
    DiffStatEntryList(BoxedTemplateProperty<'repo, Vec<DiffStatEntry>>),
    CryptographicSignatureOpt(BoxedTemplateProperty<'repo, Option<CryptographicSignature>>),
    AnnotationLine(BoxedTemplateProperty<'repo, AnnotationLine>),
    Trailer(BoxedTemplateProperty<'repo, Trailer>),
    TrailerList(BoxedTemplateProperty<'repo, Vec<Trailer>>),
}

template_builder::impl_core_property_wrappers!(<'repo> CommitTemplatePropertyKind<'repo> => Core);
operation_templater::impl_operation_property_wrappers!(<'repo> CommitTemplatePropertyKind<'repo> => Operation);
template_builder::impl_property_wrappers!(<'repo> CommitTemplatePropertyKind<'repo> {
    Commit(Commit),
    CommitOpt(Option<Commit>),
    CommitList(Vec<Commit>),
    CommitEvolutionEntry(CommitEvolutionEntry),
    CommitRef(Rc<CommitRef>),
    CommitRefOpt(Option<Rc<CommitRef>>),
    CommitRefList(Vec<Rc<CommitRef>>),
    WorkspaceRef(WorkspaceRef),
    WorkspaceRefOpt(Option<WorkspaceRef>),
    WorkspaceRefList(Vec<WorkspaceRef>),
    RefSymbol(RefSymbolBuf),
    RefSymbolOpt(Option<RefSymbolBuf>),
    RepoPath(RepoPathBuf),
    RepoPathOpt(Option<RepoPathBuf>),
    ChangeId(ChangeId),
    CommitId(CommitId),
    ShortestIdPrefix(ShortestIdPrefix),
    TreeDiff(TreeDiff),
    TreeDiffEntry(TreeDiffEntry),
    TreeDiffEntryList(Vec<TreeDiffEntry>),
    TreeEntry(TreeEntry),
    TreeEntryList(Vec<TreeEntry>),
    DiffStats(DiffStatsFormatted<'repo>),
    DiffStatEntry(DiffStatEntry),
    DiffStatEntryList(Vec<DiffStatEntry>),
    CryptographicSignatureOpt(Option<CryptographicSignature>),
    AnnotationLine(AnnotationLine),
    Trailer(Trailer),
    TrailerList(Vec<Trailer>),
});

impl<'repo> CoreTemplatePropertyVar<'repo> for CommitTemplatePropertyKind<'repo> {
    fn wrap_template(template: Box<dyn Template + 'repo>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_template(template))
    }

    fn wrap_list_template(template: Box<dyn ListTemplate + 'repo>) -> Self {
        Self::Core(CoreTemplatePropertyKind::wrap_list_template(template))
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Core(property) => property.type_name(),
            Self::Operation(property) => property.type_name(),
            Self::Commit(_) => "Commit",
            Self::CommitOpt(_) => "Option<Commit>",
            Self::CommitList(_) => "List<Commit>",
            Self::CommitEvolutionEntry(_) => "CommitEvolutionEntry",
            Self::CommitRef(_) => "CommitRef",
            Self::CommitRefOpt(_) => "Option<CommitRef>",
            Self::CommitRefList(_) => "List<CommitRef>",
            Self::WorkspaceRef(_) => "WorkspaceRef",
            Self::WorkspaceRefOpt(_) => "Option<WorkspaceRef>",
            Self::WorkspaceRefList(_) => "List<WorkspaceRef>",
            Self::RefSymbol(_) => "RefSymbol",
            Self::RefSymbolOpt(_) => "Option<RefSymbol>",
            Self::RepoPath(_) => "RepoPath",
            Self::RepoPathOpt(_) => "Option<RepoPath>",
            Self::ChangeId(_) => "ChangeId",
            Self::CommitId(_) => "CommitId",
            Self::ShortestIdPrefix(_) => "ShortestIdPrefix",
            Self::TreeDiff(_) => "TreeDiff",
            Self::TreeDiffEntry(_) => "TreeDiffEntry",
            Self::TreeDiffEntryList(_) => "List<TreeDiffEntry>",
            Self::TreeEntry(_) => "TreeEntry",
            Self::TreeEntryList(_) => "List<TreeEntry>",
            Self::DiffStats(_) => "DiffStats",
            Self::DiffStatEntry(_) => "DiffStatEntry",
            Self::DiffStatEntryList(_) => "List<DiffStatEntry>",
            Self::CryptographicSignatureOpt(_) => "Option<CryptographicSignature>",
            Self::AnnotationLine(_) => "AnnotationLine",
            Self::Trailer(_) => "Trailer",
            Self::TrailerList(_) => "List<Trailer>",
        }
    }

    fn try_into_boolean(self) -> Option<BoxedTemplateProperty<'repo, bool>> {
        match self {
            Self::Core(property) => property.try_into_boolean(),
            Self::Operation(property) => property.try_into_boolean(),
            Self::Commit(_) => None,
            Self::CommitOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::CommitList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::CommitEvolutionEntry(_) => None,
            Self::CommitRef(_) => None,
            Self::CommitRefOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::CommitRefList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::WorkspaceRef(_) => None,
            Self::WorkspaceRefOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::WorkspaceRefList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::RefSymbol(_) => None,
            Self::RefSymbolOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::RepoPath(_) => None,
            Self::RepoPathOpt(property) => Some(property.map(|opt| opt.is_some()).into_dyn()),
            Self::ChangeId(_) => None,
            Self::CommitId(_) => None,
            Self::ShortestIdPrefix(_) => None,
            // TODO: boolean cast could be implemented, but explicit
            // diff.empty() method might be better.
            Self::TreeDiff(_) => None,
            Self::TreeDiffEntry(_) => None,
            Self::TreeDiffEntryList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::TreeEntry(_) => None,
            Self::TreeEntryList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::DiffStats(_) => None,
            Self::DiffStatEntry(_) => None,
            Self::DiffStatEntryList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
            Self::CryptographicSignatureOpt(property) => {
                Some(property.map(|sig| sig.is_some()).into_dyn())
            }
            Self::AnnotationLine(_) => None,
            Self::Trailer(_) => None,
            Self::TrailerList(property) => Some(property.map(|l| !l.is_empty()).into_dyn()),
        }
    }

    fn try_into_integer(self) -> Option<BoxedTemplateProperty<'repo, i64>> {
        match self {
            Self::Core(property) => property.try_into_integer(),
            Self::Operation(property) => property.try_into_integer(),
            _ => None,
        }
    }

    fn try_into_stringify(self) -> Option<BoxedTemplateProperty<'repo, String>> {
        match self {
            Self::Core(property) => property.try_into_stringify(),
            Self::Operation(property) => property.try_into_stringify(),
            Self::RefSymbol(property) => Some(property.map(|RefSymbolBuf(s)| s).into_dyn()),
            Self::RefSymbolOpt(property) => Some(
                property
                    .map(|opt| opt.map_or_else(String::new, |RefSymbolBuf(s)| s))
                    .into_dyn(),
            ),
            _ => {
                let template = self.try_into_template()?;
                Some(PlainTextFormattedProperty::new(template).into_dyn())
            }
        }
    }

    fn try_into_serialize(self) -> Option<BoxedSerializeProperty<'repo>> {
        match self {
            Self::Core(property) => property.try_into_serialize(),
            Self::Operation(property) => property.try_into_serialize(),
            Self::Commit(property) => Some(property.into_serialize()),
            Self::CommitOpt(property) => Some(property.into_serialize()),
            Self::CommitList(property) => Some(property.into_serialize()),
            Self::CommitEvolutionEntry(property) => Some(property.into_serialize()),
            Self::CommitRef(property) => Some(property.into_serialize()),
            Self::CommitRefOpt(property) => Some(property.into_serialize()),
            Self::CommitRefList(property) => Some(property.into_serialize()),
            Self::WorkspaceRef(property) => Some(property.into_serialize()),
            Self::WorkspaceRefOpt(property) => Some(property.into_serialize()),
            Self::WorkspaceRefList(property) => Some(property.into_serialize()),
            Self::RefSymbol(property) => Some(property.into_serialize()),
            Self::RefSymbolOpt(property) => Some(property.into_serialize()),
            Self::RepoPath(property) => Some(property.into_serialize()),
            Self::RepoPathOpt(property) => Some(property.into_serialize()),
            Self::ChangeId(property) => Some(property.into_serialize()),
            Self::CommitId(property) => Some(property.into_serialize()),
            Self::ShortestIdPrefix(property) => Some(property.into_serialize()),
            Self::TreeDiff(_) => None,
            Self::TreeDiffEntry(_) => None,
            Self::TreeDiffEntryList(_) => None,
            Self::TreeEntry(_) => None,
            Self::TreeEntryList(_) => None,
            Self::DiffStats(_) => None,
            Self::DiffStatEntry(_) => None,
            Self::DiffStatEntryList(_) => None,
            Self::CryptographicSignatureOpt(_) => None,
            Self::AnnotationLine(_) => None,
            Self::Trailer(_) => None,
            Self::TrailerList(_) => None,
        }
    }

    fn try_into_template(self) -> Option<Box<dyn Template + 'repo>> {
        match self {
            Self::Core(property) => property.try_into_template(),
            Self::Operation(property) => property.try_into_template(),
            Self::Commit(_) => None,
            Self::CommitOpt(_) => None,
            Self::CommitList(_) => None,
            Self::CommitEvolutionEntry(_) => None,
            Self::CommitRef(property) => Some(property.into_template()),
            Self::CommitRefOpt(property) => Some(property.into_template()),
            Self::CommitRefList(property) => Some(property.into_template()),
            Self::WorkspaceRef(property) => Some(property.into_template()),
            Self::WorkspaceRefOpt(property) => Some(property.into_template()),
            Self::WorkspaceRefList(property) => Some(property.into_template()),
            Self::RefSymbol(property) => Some(property.into_template()),
            Self::RefSymbolOpt(property) => Some(property.into_template()),
            Self::RepoPath(property) => Some(property.into_template()),
            Self::RepoPathOpt(property) => Some(property.into_template()),
            Self::ChangeId(property) => Some(property.into_template()),
            Self::CommitId(property) => Some(property.into_template()),
            Self::ShortestIdPrefix(property) => Some(property.into_template()),
            Self::TreeDiff(_) => None,
            Self::TreeDiffEntry(_) => None,
            Self::TreeDiffEntryList(_) => None,
            Self::TreeEntry(_) => None,
            Self::TreeEntryList(_) => None,
            Self::DiffStats(property) => Some(property.into_template()),
            Self::DiffStatEntry(_) => None,
            Self::DiffStatEntryList(_) => None,
            Self::CryptographicSignatureOpt(_) => None,
            Self::AnnotationLine(_) => None,
            Self::Trailer(property) => Some(property.into_template()),
            Self::TrailerList(property) => Some(property.into_template()),
        }
    }

    fn try_into_eq(self, other: Self) -> Option<BoxedTemplateProperty<'repo, bool>> {
        type Core<'repo> = CoreTemplatePropertyKind<'repo>;
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_eq(rhs),
            (Self::Core(lhs), Self::Operation(rhs)) => rhs.try_into_eq_core(lhs),
            (Self::Core(Core::String(lhs)), Self::RefSymbol(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| RefSymbolBuf(l) == r).into_dyn())
            }
            (Self::Core(Core::String(lhs)), Self::RefSymbolOpt(rhs)) => Some(
                (lhs, rhs)
                    .map(|(l, r)| Some(RefSymbolBuf(l)) == r)
                    .into_dyn(),
            ),
            (Self::Operation(lhs), Self::Core(rhs)) => lhs.try_into_eq_core(rhs),
            (Self::Operation(lhs), Self::Operation(rhs)) => lhs.try_into_eq(rhs),
            (Self::RefSymbol(lhs), Self::Core(Core::String(rhs))) => {
                Some((lhs, rhs).map(|(l, r)| l == RefSymbolBuf(r)).into_dyn())
            }
            (Self::RefSymbol(lhs), Self::RefSymbol(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::RefSymbol(lhs), Self::RefSymbolOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| Some(l) == r).into_dyn())
            }
            (Self::RefSymbolOpt(lhs), Self::Core(Core::String(rhs))) => Some(
                (lhs, rhs)
                    .map(|(l, r)| l == Some(RefSymbolBuf(r)))
                    .into_dyn(),
            ),
            (Self::RefSymbolOpt(lhs), Self::RefSymbol(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == Some(r)).into_dyn())
            }
            (Self::RefSymbolOpt(lhs), Self::RefSymbolOpt(rhs)) => {
                Some((lhs, rhs).map(|(l, r)| l == r).into_dyn())
            }
            (Self::Core(_), _) => None,
            (Self::Operation(_), _) => None,
            (Self::Commit(_), _) => None,
            (Self::CommitOpt(_), _) => None,
            (Self::CommitList(_), _) => None,
            (Self::CommitEvolutionEntry(_), _) => None,
            (Self::CommitRef(_), _) => None,
            (Self::CommitRefOpt(_), _) => None,
            (Self::CommitRefList(_), _) => None,
            (Self::WorkspaceRef(_), _) => None,
            (Self::WorkspaceRefOpt(_), _) => None,
            (Self::WorkspaceRefList(_), _) => None,
            (Self::RefSymbol(_), _) => None,
            (Self::RefSymbolOpt(_), _) => None,
            (Self::RepoPath(_), _) => None,
            (Self::RepoPathOpt(_), _) => None,
            (Self::ChangeId(_), _) => None,
            (Self::CommitId(_), _) => None,
            (Self::ShortestIdPrefix(_), _) => None,
            (Self::TreeDiff(_), _) => None,
            (Self::TreeDiffEntry(_), _) => None,
            (Self::TreeDiffEntryList(_), _) => None,
            (Self::TreeEntry(_), _) => None,
            (Self::TreeEntryList(_), _) => None,
            (Self::DiffStats(_), _) => None,
            (Self::DiffStatEntry(_), _) => None,
            (Self::DiffStatEntryList(_), _) => None,
            (Self::CryptographicSignatureOpt(_), _) => None,
            (Self::AnnotationLine(_), _) => None,
            (Self::Trailer(_), _) => None,
            (Self::TrailerList(_), _) => None,
        }
    }

    fn try_into_cmp(self, other: Self) -> Option<BoxedTemplateProperty<'repo, Ordering>> {
        match (self, other) {
            (Self::Core(lhs), Self::Core(rhs)) => lhs.try_into_cmp(rhs),
            (Self::Core(lhs), Self::Operation(rhs)) => rhs
                .try_into_cmp_core(lhs)
                .map(|property| property.map(Ordering::reverse).into_dyn()),
            (Self::Operation(lhs), Self::Core(rhs)) => lhs.try_into_cmp_core(rhs),
            (Self::Operation(lhs), Self::Operation(rhs)) => lhs.try_into_cmp(rhs),
            (Self::Core(_), _) => None,
            (Self::Operation(_), _) => None,
            (Self::Commit(_), _) => None,
            (Self::CommitOpt(_), _) => None,
            (Self::CommitList(_), _) => None,
            (Self::CommitEvolutionEntry(_), _) => None,
            (Self::CommitRef(_), _) => None,
            (Self::CommitRefOpt(_), _) => None,
            (Self::CommitRefList(_), _) => None,
            (Self::WorkspaceRef(_), _) => None,
            (Self::WorkspaceRefOpt(_), _) => None,
            (Self::WorkspaceRefList(_), _) => None,
            (Self::RefSymbol(_), _) => None,
            (Self::RefSymbolOpt(_), _) => None,
            (Self::RepoPath(_), _) => None,
            (Self::RepoPathOpt(_), _) => None,
            (Self::ChangeId(_), _) => None,
            (Self::CommitId(_), _) => None,
            (Self::ShortestIdPrefix(_), _) => None,
            (Self::TreeDiff(_), _) => None,
            (Self::TreeDiffEntry(_), _) => None,
            (Self::TreeDiffEntryList(_), _) => None,
            (Self::TreeEntry(_), _) => None,
            (Self::TreeEntryList(_), _) => None,
            (Self::DiffStats(_), _) => None,
            (Self::DiffStatEntry(_), _) => None,
            (Self::DiffStatEntryList(_), _) => None,
            (Self::CryptographicSignatureOpt(_), _) => None,
            (Self::AnnotationLine(_), _) => None,
            (Self::Trailer(_), _) => None,
            (Self::TrailerList(_), _) => None,
        }
    }
}

impl<'repo> OperationTemplatePropertyVar<'repo> for CommitTemplatePropertyKind<'repo> {}

/// Table of functions that translate method call node of self type `T`.
pub type CommitTemplateBuildMethodFnMap<'repo, T> =
    TemplateBuildMethodFnMap<'repo, CommitTemplateLanguage<'repo>, T>;

/// Symbol table of methods available in the commit template.
pub struct CommitTemplateBuildFnTable<'repo> {
    pub core: CoreTemplateBuildFnTable<'repo, CommitTemplateLanguage<'repo>>,
    pub operation: OperationTemplateBuildFnTable<'repo, CommitTemplateLanguage<'repo>>,
    pub commit_methods: CommitTemplateBuildMethodFnMap<'repo, Commit>,
    pub commit_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<Commit>>,
    pub commit_evolution_entry_methods: CommitTemplateBuildMethodFnMap<'repo, CommitEvolutionEntry>,
    pub commit_ref_methods: CommitTemplateBuildMethodFnMap<'repo, Rc<CommitRef>>,
    pub commit_ref_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<Rc<CommitRef>>>,
    pub workspace_ref_methods: CommitTemplateBuildMethodFnMap<'repo, WorkspaceRef>,
    pub workspace_ref_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<WorkspaceRef>>,
    pub repo_path_methods: CommitTemplateBuildMethodFnMap<'repo, RepoPathBuf>,
    pub change_id_methods: CommitTemplateBuildMethodFnMap<'repo, ChangeId>,
    pub commit_id_methods: CommitTemplateBuildMethodFnMap<'repo, CommitId>,
    pub shortest_id_prefix_methods: CommitTemplateBuildMethodFnMap<'repo, ShortestIdPrefix>,
    pub tree_diff_methods: CommitTemplateBuildMethodFnMap<'repo, TreeDiff>,
    pub tree_diff_entry_methods: CommitTemplateBuildMethodFnMap<'repo, TreeDiffEntry>,
    pub tree_diff_entry_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<TreeDiffEntry>>,
    pub tree_entry_methods: CommitTemplateBuildMethodFnMap<'repo, TreeEntry>,
    pub tree_entry_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<TreeEntry>>,
    pub diff_stats_methods: CommitTemplateBuildMethodFnMap<'repo, DiffStats>,
    pub diff_stat_entry_methods: CommitTemplateBuildMethodFnMap<'repo, DiffStatEntry>,
    pub diff_stat_entry_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<DiffStatEntry>>,
    pub cryptographic_signature_methods:
        CommitTemplateBuildMethodFnMap<'repo, CryptographicSignature>,
    pub annotation_line_methods: CommitTemplateBuildMethodFnMap<'repo, AnnotationLine>,
    pub trailer_methods: CommitTemplateBuildMethodFnMap<'repo, Trailer>,
    pub trailer_list_methods: CommitTemplateBuildMethodFnMap<'repo, Vec<Trailer>>,
}

impl CommitTemplateBuildFnTable<'_> {
    pub fn empty() -> Self {
        Self {
            core: CoreTemplateBuildFnTable::empty(),
            operation: OperationTemplateBuildFnTable::empty(),
            commit_methods: HashMap::new(),
            commit_list_methods: HashMap::new(),
            commit_evolution_entry_methods: HashMap::new(),
            commit_ref_methods: HashMap::new(),
            commit_ref_list_methods: HashMap::new(),
            workspace_ref_methods: HashMap::new(),
            workspace_ref_list_methods: HashMap::new(),
            repo_path_methods: HashMap::new(),
            change_id_methods: HashMap::new(),
            commit_id_methods: HashMap::new(),
            shortest_id_prefix_methods: HashMap::new(),
            tree_diff_methods: HashMap::new(),
            tree_diff_entry_methods: HashMap::new(),
            tree_diff_entry_list_methods: HashMap::new(),
            tree_entry_methods: HashMap::new(),
            tree_entry_list_methods: HashMap::new(),
            diff_stats_methods: HashMap::new(),
            diff_stat_entry_methods: HashMap::new(),
            diff_stat_entry_list_methods: HashMap::new(),
            cryptographic_signature_methods: HashMap::new(),
            annotation_line_methods: HashMap::new(),
            trailer_methods: HashMap::new(),
            trailer_list_methods: HashMap::new(),
        }
    }

    fn merge(&mut self, other: Self) {
        let Self {
            core,
            operation,
            commit_methods,
            commit_list_methods,
            commit_evolution_entry_methods,
            commit_ref_methods,
            commit_ref_list_methods,
            workspace_ref_methods,
            workspace_ref_list_methods,
            repo_path_methods,
            change_id_methods,
            commit_id_methods,
            shortest_id_prefix_methods,
            tree_diff_methods,
            tree_diff_entry_methods,
            tree_diff_entry_list_methods,
            tree_entry_methods,
            tree_entry_list_methods,
            diff_stats_methods,
            diff_stat_entry_methods,
            diff_stat_entry_list_methods,
            cryptographic_signature_methods,
            annotation_line_methods,
            trailer_methods,
            trailer_list_methods,
        } = other;

        self.core.merge(core);
        self.operation.merge(operation);
        merge_fn_map(&mut self.commit_methods, commit_methods);
        merge_fn_map(&mut self.commit_list_methods, commit_list_methods);
        merge_fn_map(
            &mut self.commit_evolution_entry_methods,
            commit_evolution_entry_methods,
        );
        merge_fn_map(&mut self.commit_ref_methods, commit_ref_methods);
        merge_fn_map(&mut self.commit_ref_list_methods, commit_ref_list_methods);
        merge_fn_map(&mut self.workspace_ref_methods, workspace_ref_methods);
        merge_fn_map(
            &mut self.workspace_ref_list_methods,
            workspace_ref_list_methods,
        );
        merge_fn_map(&mut self.repo_path_methods, repo_path_methods);
        merge_fn_map(&mut self.change_id_methods, change_id_methods);
        merge_fn_map(&mut self.commit_id_methods, commit_id_methods);
        merge_fn_map(
            &mut self.shortest_id_prefix_methods,
            shortest_id_prefix_methods,
        );
        merge_fn_map(&mut self.tree_diff_methods, tree_diff_methods);
        merge_fn_map(&mut self.tree_diff_entry_methods, tree_diff_entry_methods);
        merge_fn_map(
            &mut self.tree_diff_entry_list_methods,
            tree_diff_entry_list_methods,
        );
        merge_fn_map(&mut self.tree_entry_methods, tree_entry_methods);
        merge_fn_map(&mut self.tree_entry_list_methods, tree_entry_list_methods);
        merge_fn_map(&mut self.diff_stats_methods, diff_stats_methods);
        merge_fn_map(&mut self.diff_stat_entry_methods, diff_stat_entry_methods);
        merge_fn_map(
            &mut self.diff_stat_entry_list_methods,
            diff_stat_entry_list_methods,
        );
        merge_fn_map(
            &mut self.cryptographic_signature_methods,
            cryptographic_signature_methods,
        );
        merge_fn_map(&mut self.annotation_line_methods, annotation_line_methods);
        merge_fn_map(&mut self.trailer_methods, trailer_methods);
        merge_fn_map(&mut self.trailer_list_methods, trailer_list_methods);
    }

    /// Creates new symbol table containing the builtin methods.
    fn builtin() -> Self {
        Self {
            core: CoreTemplateBuildFnTable::builtin(),
            operation: OperationTemplateBuildFnTable::builtin(),
            commit_methods: builtin_commit_methods(),
            commit_list_methods: template_builder::builtin_unformattable_list_methods(),
            commit_evolution_entry_methods: builtin_commit_evolution_entry_methods(),
            commit_ref_methods: builtin_commit_ref_methods(),
            commit_ref_list_methods: template_builder::builtin_formattable_list_methods(),
            workspace_ref_methods: builtin_workspace_ref_methods(),
            workspace_ref_list_methods: template_builder::builtin_formattable_list_methods(),
            repo_path_methods: builtin_repo_path_methods(),
            change_id_methods: builtin_change_id_methods(),
            commit_id_methods: builtin_commit_id_methods(),
            shortest_id_prefix_methods: builtin_shortest_id_prefix_methods(),
            tree_diff_methods: builtin_tree_diff_methods(),
            tree_diff_entry_methods: builtin_tree_diff_entry_methods(),
            tree_diff_entry_list_methods: template_builder::builtin_unformattable_list_methods(),
            tree_entry_methods: builtin_tree_entry_methods(),
            tree_entry_list_methods: template_builder::builtin_unformattable_list_methods(),
            diff_stats_methods: builtin_diff_stats_methods(),
            diff_stat_entry_methods: builtin_diff_stat_entry_methods(),
            diff_stat_entry_list_methods: template_builder::builtin_unformattable_list_methods(),
            cryptographic_signature_methods: builtin_cryptographic_signature_methods(),
            annotation_line_methods: builtin_annotation_line_methods(),
            trailer_methods: builtin_trailer_methods(),
            trailer_list_methods: builtin_trailer_list_methods(),
        }
    }
}

#[derive(Default)]
pub struct CommitKeywordCache<'repo> {
    // Build index lazily, and Rc to get away from &self lifetime.
    bookmarks_index: OnceCell<Rc<CommitRefsIndex>>,
    tags_index: OnceCell<Rc<CommitRefsIndex>>,
    git_refs_index: OnceCell<Rc<CommitRefsIndex>>,
    is_immutable_fn: OnceCell<Rc<RevsetContainingFn<'repo>>>,
}

impl<'repo> CommitKeywordCache<'repo> {
    pub fn bookmarks_index(&self, repo: &dyn Repo) -> &Rc<CommitRefsIndex> {
        self.bookmarks_index
            .get_or_init(|| Rc::new(build_local_remote_refs_index(repo.view().bookmarks())))
    }

    pub fn tags_index(&self, repo: &dyn Repo) -> &Rc<CommitRefsIndex> {
        self.tags_index
            .get_or_init(|| Rc::new(build_local_remote_refs_index(repo.view().tags())))
    }

    pub fn git_refs_index(&self, repo: &dyn Repo) -> &Rc<CommitRefsIndex> {
        self.git_refs_index
            .get_or_init(|| Rc::new(build_commit_refs_index(repo.view().git_refs())))
    }

    pub fn is_immutable_fn(
        &self,
        language: &CommitTemplateLanguage<'repo>,
        span: pest::Span<'_>,
    ) -> TemplateParseResult<&Rc<RevsetContainingFn<'repo>>> {
        // Alternatively, a negated (i.e. visible mutable) set could be computed.
        // It's usually smaller than the immutable set. The revset engine can also
        // optimize "::<recent_heads>" query to use bitset-based implementation.
        self.is_immutable_fn.get_or_try_init(|| {
            let expression = &language.immutable_expression;
            let revset = evaluate_revset_expression(language, span, expression)?;
            Ok(revset.containing_fn().into())
        })
    }
}

fn builtin_commit_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, Commit> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<Commit>::new();
    map.insert(
        "description",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.description().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "trailers",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property
                .map(|commit| trailer::parse_description_trailers(commit.description()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "change_id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.change_id().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "commit_id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.id().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "parents",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|commit| {
                let commits: Vec<_> = commit.parents().try_collect()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "author",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.author().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "committer",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.committer().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "mine",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let user_email = language.revset_parse_context.user_email.to_owned();
            let out_property = self_property.map(move |commit| commit.author().email == user_email);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "signature",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(CryptographicSignature::new);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "working_copies",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.map(|commit| extract_working_copies(repo, &commit));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "current_working_copy",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let name = language.workspace_name.clone();
            let out_property = self_property
                .map(move |commit| Some(commit.id()) == repo.view().get_wc_commit_id(&name));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "bookmarks",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language
                .keyword_cache
                .bookmarks_index(language.repo)
                .clone();
            let out_property =
                self_property.map(move |commit| collect_distinct_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "local_bookmarks",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language
                .keyword_cache
                .bookmarks_index(language.repo)
                .clone();
            let out_property =
                self_property.map(move |commit| collect_local_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "remote_bookmarks",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language
                .keyword_cache
                .bookmarks_index(language.repo)
                .clone();
            let out_property =
                self_property.map(move |commit| collect_remote_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_distinct_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "local_tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_local_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "remote_tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_remote_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "git_refs",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.git_refs_index(language.repo).clone();
            let out_property = self_property.map(move |commit| index.get(commit.id()).to_vec());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "git_head",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.map(|commit| {
                let target = repo.view().git_head();
                target.added_ids().contains(commit.id())
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "divergent",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| {
                // The given commit could be hidden in e.g. `jj evolog`.
                let maybe_entries = repo.resolve_change_id(commit.change_id())?;
                let divergent = maybe_entries.map_or(0, |entries| entries.len()) > 1;
                Ok(divergent)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "hidden",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| Ok(commit.is_hidden(repo)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "immutable",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let is_immutable = language
                .keyword_cache
                .is_immutable_fn(language, function.name_span)?
                .clone();
            let out_property = self_property.and_then(move |commit| Ok(is_immutable(commit.id())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "contained_in",
        |language, diagnostics, _build_ctx, self_property, function| {
            let [revset_node] = function.expect_exact_arguments()?;

            let is_contained =
                template_parser::catch_aliases(diagnostics, revset_node, |diagnostics, node| {
                    let text = template_parser::expect_string_literal(node)?;
                    let revset = evaluate_user_revset(language, diagnostics, node.span, text)?;
                    Ok(revset.containing_fn())
                })?;

            let out_property = self_property.and_then(move |commit| Ok(is_contained(commit.id())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.has_conflict());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "empty",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| Ok(commit.is_empty(repo)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "diff",
        |language, diagnostics, _build_ctx, self_property, function| {
            let ([], [files_node]) = function.expect_arguments()?;
            let files = if let Some(node) = files_node {
                expect_fileset_literal(diagnostics, node, language.path_converter)?
            } else {
                // TODO: defaults to CLI path arguments?
                // https://github.com/jj-vcs/jj/issues/2933#issuecomment-1925870731
                FilesetExpression::all()
            };
            let repo = language.repo;
            let matcher: Rc<dyn Matcher> = files.to_matcher().into();
            let out_property = self_property
                .and_then(move |commit| Ok(TreeDiff::from_commit(repo, &commit, matcher.clone())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "files",
        |language, diagnostics, _build_ctx, self_property, function| {
            let ([], [files_node]) = function.expect_arguments()?;
            let files = if let Some(node) = files_node {
                expect_fileset_literal(diagnostics, node, language.path_converter)?
            } else {
                // TODO: defaults to CLI path arguments?
                // https://github.com/jj-vcs/jj/issues/2933#issuecomment-1925870731
                FilesetExpression::all()
            };
            let matcher = files.to_matcher();
            let out_property = self_property.and_then(move |commit| {
                let tree = commit.tree();
                let entries: Vec<_> = tree
                    .entries_matching(&*matcher)
                    .map(|(path, value)| value.map(|value| (path, value)))
                    .map_ok(|(path, value)| TreeEntry { path, value })
                    .try_collect()?;
                Ok(entries)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "root",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.map(|commit| commit.id() == repo.store().root_commit_id());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn extract_working_copies(repo: &dyn Repo, commit: &Commit) -> Vec<WorkspaceRef> {
    if repo.view().wc_commit_ids().len() <= 1 {
        // No non-default working copies, return empty list.
        return vec![];
    }

    repo.view()
        .wc_commit_ids()
        .iter()
        .filter(|(_, wc_commit_id)| *wc_commit_id == commit.id())
        .map(|(name, _)| WorkspaceRef::new(name.to_owned(), commit.to_owned()))
        .collect()
}

fn expect_fileset_literal(
    diagnostics: &mut TemplateDiagnostics,
    node: &ExpressionNode,
    path_converter: &RepoPathUiConverter,
) -> Result<FilesetExpression, TemplateParseError> {
    template_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let text = template_parser::expect_string_literal(node)?;
        let mut inner_diagnostics = FilesetDiagnostics::new();
        let expression =
            fileset::parse(&mut inner_diagnostics, text, path_converter).map_err(|err| {
                TemplateParseError::expression("In fileset expression", node.span).with_source(err)
            })?;
        diagnostics.extend_with(inner_diagnostics, |diag| {
            TemplateParseError::expression("In fileset expression", node.span).with_source(diag)
        });
        Ok(expression)
    })
}

fn evaluate_revset_expression<'repo>(
    language: &CommitTemplateLanguage<'repo>,
    span: pest::Span<'_>,
    expression: &UserRevsetExpression,
) -> Result<Box<dyn Revset + 'repo>, TemplateParseError> {
    let make_error = || TemplateParseError::expression("Failed to evaluate revset", span);
    let repo = language.repo;
    let symbol_resolver = revset_util::default_symbol_resolver(
        repo,
        language.revset_parse_context.extensions.symbol_resolvers(),
        language.id_prefix_context,
    );
    let revset = expression
        .resolve_user_expression(repo, &symbol_resolver)
        .map_err(|err| make_error().with_source(err))?
        .evaluate(repo)
        .map_err(|err| make_error().with_source(err))?;
    Ok(revset)
}

fn evaluate_user_revset<'repo>(
    language: &CommitTemplateLanguage<'repo>,
    diagnostics: &mut TemplateDiagnostics,
    span: pest::Span<'_>,
    revset: &str,
) -> Result<Box<dyn Revset + 'repo>, TemplateParseError> {
    let mut inner_diagnostics = RevsetDiagnostics::new();
    let (expression, modifier) = revset::parse_with_modifier(
        &mut inner_diagnostics,
        revset,
        &language.revset_parse_context,
    )
    .map_err(|err| TemplateParseError::expression("In revset expression", span).with_source(err))?;
    diagnostics.extend_with(inner_diagnostics, |diag| {
        TemplateParseError::expression("In revset expression", span).with_source(diag)
    });
    let (None | Some(RevsetModifier::All)) = modifier;

    evaluate_revset_expression(language, span, &expression)
}

fn builtin_commit_evolution_entry_methods<'repo>()
-> CommitTemplateBuildMethodFnMap<'repo, CommitEvolutionEntry> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<CommitEvolutionEntry>::new();
    map.insert(
        "commit",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.commit);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "operation",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.operation);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    // TODO: add predecessors() -> Vec<Commit>?
    map
}

/// Bookmark or tag name with metadata.
#[derive(Debug, serde::Serialize)]
pub struct CommitRef {
    // Not using Ref/GitRef/RemoteName types here because it would be overly
    // complex to generalize the name type as T: RefName|GitRefName.
    /// Local name.
    name: RefSymbolBuf,
    /// Remote name if this is a remote or Git-tracking ref.
    #[serde(skip_serializing_if = "Option::is_none")] // local ref shouldn't have this field
    remote: Option<RefSymbolBuf>,
    /// Target commit ids.
    target: RefTarget,
    /// Local ref metadata which tracks this remote ref.
    #[serde(rename = "tracking_target")]
    #[serde(skip_serializing_if = "Option::is_none")] // local ref shouldn't have this field
    #[serde(serialize_with = "serialize_tracking_target")]
    tracking_ref: Option<TrackingRef>,
    /// Local ref is synchronized with all tracking remotes, or tracking remote
    /// ref is synchronized with the local.
    #[serde(skip)] // internal state used mainly for Template impl
    synced: bool,
}

#[derive(Debug)]
struct TrackingRef {
    /// Local ref target which tracks the other remote ref.
    target: RefTarget,
    /// Number of commits ahead of the tracking `target`.
    ahead_count: OnceCell<SizeHint>,
    /// Number of commits behind of the tracking `target`.
    behind_count: OnceCell<SizeHint>,
}

impl CommitRef {
    // CommitRef is wrapped by Rc<T> to make it cheaply cloned and share
    // lazy-evaluation results across clones.

    /// Creates local ref representation which might track some of the
    /// `remote_refs`.
    pub fn local<'a>(
        name: impl Into<String>,
        target: RefTarget,
        remote_refs: impl IntoIterator<Item = &'a RemoteRef>,
    ) -> Rc<Self> {
        let synced = remote_refs
            .into_iter()
            .all(|remote_ref| !remote_ref.is_tracked() || remote_ref.target == target);
        Rc::new(Self {
            name: RefSymbolBuf(name.into()),
            remote: None,
            target,
            tracking_ref: None,
            synced,
        })
    }

    /// Creates local ref representation which doesn't track any remote refs.
    pub fn local_only(name: impl Into<String>, target: RefTarget) -> Rc<Self> {
        Self::local(name, target, [])
    }

    /// Creates remote ref representation which might be tracked by a local ref
    /// pointing to the `local_target`.
    pub fn remote(
        name: impl Into<String>,
        remote_name: impl Into<String>,
        remote_ref: RemoteRef,
        local_target: &RefTarget,
    ) -> Rc<Self> {
        let synced = remote_ref.is_tracked() && remote_ref.target == *local_target;
        let tracking_ref = remote_ref.is_tracked().then(|| {
            let count = if synced {
                OnceCell::from((0, Some(0))) // fast path for synced remotes
            } else {
                OnceCell::new()
            };
            TrackingRef {
                target: local_target.clone(),
                ahead_count: count.clone(),
                behind_count: count,
            }
        });
        Rc::new(Self {
            name: RefSymbolBuf(name.into()),
            remote: Some(RefSymbolBuf(remote_name.into())),
            target: remote_ref.target,
            tracking_ref,
            synced,
        })
    }

    /// Creates remote ref representation which isn't tracked by a local ref.
    pub fn remote_only(
        name: impl Into<String>,
        remote_name: impl Into<String>,
        target: RefTarget,
    ) -> Rc<Self> {
        Rc::new(Self {
            name: RefSymbolBuf(name.into()),
            remote: Some(RefSymbolBuf(remote_name.into())),
            target,
            tracking_ref: None,
            synced: false, // has no local counterpart
        })
    }

    /// Local name.
    pub fn name(&self) -> &str {
        self.name.as_ref()
    }

    /// Remote name if this is a remote or Git-tracking ref.
    pub fn remote_name(&self) -> Option<&str> {
        self.remote.as_ref().map(AsRef::as_ref)
    }

    /// Target commit ids.
    pub fn target(&self) -> &RefTarget {
        &self.target
    }

    /// Returns true if this is a local ref.
    pub fn is_local(&self) -> bool {
        self.remote.is_none()
    }

    /// Returns true if this is a remote ref.
    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    /// Returns true if this ref points to no commit.
    pub fn is_absent(&self) -> bool {
        self.target.is_absent()
    }

    /// Returns true if this ref points to any commit.
    pub fn is_present(&self) -> bool {
        self.target.is_present()
    }

    /// Whether the ref target has conflicts.
    pub fn has_conflict(&self) -> bool {
        self.target.has_conflict()
    }

    /// Returns true if this ref is tracked by a local ref. The local ref might
    /// have been deleted (but not pushed yet.)
    pub fn is_tracked(&self) -> bool {
        self.tracking_ref.is_some()
    }

    /// Returns true if this ref is tracked by a local ref, and if the local ref
    /// is present.
    pub fn is_tracking_present(&self) -> bool {
        self.tracking_ref
            .as_ref()
            .is_some_and(|tracking| tracking.target.is_present())
    }

    /// Number of commits ahead of the tracking local ref.
    fn tracking_ahead_count(&self, repo: &dyn Repo) -> Result<SizeHint, TemplatePropertyError> {
        let Some(tracking) = &self.tracking_ref else {
            return Err(TemplatePropertyError("Not a tracked remote ref".into()));
        };
        tracking
            .ahead_count
            .get_or_try_init(|| {
                let self_ids = self.target.added_ids().cloned().collect_vec();
                let other_ids = tracking.target.added_ids().cloned().collect_vec();
                Ok(revset::walk_revs(repo, &self_ids, &other_ids)?.count_estimate()?)
            })
            .copied()
    }

    /// Number of commits behind of the tracking local ref.
    fn tracking_behind_count(&self, repo: &dyn Repo) -> Result<SizeHint, TemplatePropertyError> {
        let Some(tracking) = &self.tracking_ref else {
            return Err(TemplatePropertyError("Not a tracked remote ref".into()));
        };
        tracking
            .behind_count
            .get_or_try_init(|| {
                let self_ids = self.target.added_ids().cloned().collect_vec();
                let other_ids = tracking.target.added_ids().cloned().collect_vec();
                Ok(revset::walk_revs(repo, &other_ids, &self_ids)?.count_estimate()?)
            })
            .copied()
    }
}

// If wrapping with Rc<T> becomes common, add generic impl for Rc<T>.
impl Template for Rc<CommitRef> {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter.labeled("name"), "{}", self.name)?;
        if let Some(remote) = &self.remote {
            write!(formatter, "@")?;
            write!(formatter.labeled("remote"), "{remote}")?;
        }
        // Don't show both conflict and unsynced sigils as conflicted ref wouldn't
        // be pushed.
        if self.has_conflict() {
            write!(formatter, "??")?;
        } else if self.is_local() && !self.synced {
            write!(formatter, "*")?;
        }
        Ok(())
    }
}

impl Template for Vec<Rc<CommitRef>> {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        templater::format_joined(formatter, self, " ")
    }
}

/// Workspace name together with its working-copy commit for templating.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceRef {
    /// Workspace name as a symbol.
    name: WorkspaceNameBuf,
    /// Working-copy commit of this workspace.
    target: Commit,
}

impl WorkspaceRef {
    /// Creates a new workspace reference from the workspace name and commit.
    pub fn new(name: WorkspaceNameBuf, target: Commit) -> Self {
        Self { name, target }
    }

    /// Returns the workspace name symbol.
    pub fn name(&self) -> &WorkspaceName {
        self.name.as_ref()
    }

    /// Returns the working-copy commit of this workspace.
    pub fn target(&self) -> &Commit {
        &self.target
    }
}

impl Template for WorkspaceRef {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{}@", self.name.as_symbol())
    }
}

impl Template for Vec<WorkspaceRef> {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        templater::format_joined(formatter, self, " ")
    }
}

fn builtin_workspace_ref_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, WorkspaceRef> {
    let mut map = CommitTemplateBuildMethodFnMap::<WorkspaceRef>::new();
    map.insert(
        "name",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|ws_ref| RefSymbolBuf(ws_ref.name.into()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "target",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|ws_ref| ws_ref.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn serialize_tracking_target<S>(
    tracking_ref: &Option<TrackingRef>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let target = tracking_ref.as_ref().map(|tracking| &tracking.target);
    target.serialize(serializer)
}

fn builtin_commit_ref_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, Rc<CommitRef>> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<Rc<CommitRef>>::new();
    map.insert(
        "name",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.name.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "remote",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.remote.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "present",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_present());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.has_conflict());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "normal_target",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit_ref| {
                let maybe_id = commit_ref.target.as_normal();
                Ok(maybe_id.map(|id| repo.store().get_commit(id)).transpose()?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "removed_targets",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit_ref| {
                let ids = commit_ref.target.removed_ids();
                let commits: Vec<_> = ids.map(|id| repo.store().get_commit(id)).try_collect()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "added_targets",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit_ref| {
                let ids = commit_ref.target.added_ids();
                let commits: Vec<_> = ids.map(|id| repo.store().get_commit(id)).try_collect()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tracked",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_tracked());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tracking_present",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_tracking_present());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tracking_ahead_count",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.and_then(|commit_ref| commit_ref.tracking_ahead_count(repo));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "tracking_behind_count",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.and_then(|commit_ref| commit_ref.tracking_behind_count(repo));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "synced",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.synced);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

/// Cache for reverse lookup refs.
#[derive(Clone, Debug, Default)]
pub struct CommitRefsIndex {
    index: HashMap<CommitId, Vec<Rc<CommitRef>>>,
}

impl CommitRefsIndex {
    fn insert<'a>(&mut self, ids: impl IntoIterator<Item = &'a CommitId>, name: Rc<CommitRef>) {
        for id in ids {
            let commit_refs = self.index.entry(id.clone()).or_default();
            commit_refs.push(name.clone());
        }
    }

    pub fn get(&self, id: &CommitId) -> &[Rc<CommitRef>] {
        self.index.get(id).map_or(&[], |refs: &Vec<_>| refs)
    }
}

fn build_local_remote_refs_index<'a>(
    local_remote_refs: impl IntoIterator<Item = (&'a RefName, LocalRemoteRefTarget<'a>)>,
) -> CommitRefsIndex {
    let mut index = CommitRefsIndex::default();
    for (name, target) in local_remote_refs {
        let local_target = target.local_target;
        let remote_refs = target.remote_refs;
        if local_target.is_present() {
            let commit_ref = CommitRef::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            index.insert(local_target.added_ids(), commit_ref);
        }
        for &(remote_name, remote_ref) in &remote_refs {
            let commit_ref = CommitRef::remote(name, remote_name, remote_ref.clone(), local_target);
            index.insert(remote_ref.target.added_ids(), commit_ref);
        }
    }
    index
}

fn build_commit_refs_index<'a, K: Into<String>>(
    ref_pairs: impl IntoIterator<Item = (K, &'a RefTarget)>,
) -> CommitRefsIndex {
    let mut index = CommitRefsIndex::default();
    for (name, target) in ref_pairs {
        let commit_ref = CommitRef::local_only(name, target.clone());
        index.insert(target.added_ids(), commit_ref);
    }
    index
}

fn collect_distinct_refs(commit_refs: &[Rc<CommitRef>]) -> Vec<Rc<CommitRef>> {
    commit_refs
        .iter()
        .filter(|commit_ref| commit_ref.is_local() || !commit_ref.synced)
        .cloned()
        .collect()
}

fn collect_local_refs(commit_refs: &[Rc<CommitRef>]) -> Vec<Rc<CommitRef>> {
    commit_refs
        .iter()
        .filter(|commit_ref| commit_ref.is_local())
        .cloned()
        .collect()
}

fn collect_remote_refs(commit_refs: &[Rc<CommitRef>]) -> Vec<Rc<CommitRef>> {
    commit_refs
        .iter()
        .filter(|commit_ref| commit_ref.is_remote())
        .cloned()
        .collect()
}

/// Wrapper to render ref/remote name in revset syntax.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct RefSymbolBuf(String);

impl AsRef<str> for RefSymbolBuf {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Display for RefSymbolBuf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad(&revset::format_symbol(&self.0))
    }
}

impl Template for RefSymbolBuf {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{self}")
    }
}

impl Template for RepoPathBuf {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{}", self.as_internal_file_string())
    }
}

fn builtin_repo_path_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, RepoPathBuf> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<RepoPathBuf>::new();
    map.insert(
        "absolute",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            // We handle the absolute path here instead of in a wrapper in
            // `RepoPathUiConverter` because absolute paths only make sense for
            // filesystem paths. Other cases should fail here.
            let out_property = self_property.and_then(move |path| match path_converter {
                RepoPathUiConverter::Fs { cwd: _, base } => path
                    .to_fs_path(base)?
                    .into_os_string()
                    .into_string()
                    .map_err(|_| TemplatePropertyError("Invalid UTF-8 sequence in path".into())),
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "display",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let out_property = self_property.map(|path| path_converter.format_file_path(&path));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "parent",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|path| Some(path.parent()?.to_owned()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

trait ShortestIdPrefixLen {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize>;
}

impl ShortestIdPrefixLen for ChangeId {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize> {
        index.shortest_change_prefix_len(repo, self)
    }
}

impl Template for ChangeId {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{self}")
    }
}

fn builtin_change_id_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, ChangeId> {
    let mut map = builtin_commit_or_change_id_methods::<ChangeId>();
    map.insert(
        "normal_hex",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            // Note: this is _not_ the same as id.to_string(), which returns the
            // "reverse" hex (z-k), instead of the "forward" / normal hex
            // (0-9a-f) we want here.
            let out_property = self_property.map(|id| id.hex());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

impl ShortestIdPrefixLen for CommitId {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize> {
        index.shortest_commit_prefix_len(repo, self)
    }
}

impl Template for CommitId {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{self}")
    }
}

fn builtin_commit_id_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, CommitId> {
    let mut map = builtin_commit_or_change_id_methods::<CommitId>();
    // TODO: Remove in jj 0.36+
    map.insert(
        "normal_hex",
        |_language, diagnostics, _build_ctx, self_property, function| {
            diagnostics.add_warning(TemplateParseError::expression(
                "commit_id.normal_hex() is deprecated; use stringify(commit_id) instead",
                function.name_span,
            ));
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.hex());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_commit_or_change_id_methods<'repo, O>() -> CommitTemplateBuildMethodFnMap<'repo, O>
where
    O: Display + ShortestIdPrefixLen + 'repo,
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<O>::new();
    map.insert(
        "short",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [len_node]) = function.expect_arguments()?;
            let len_property = len_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let out_property = (self_property, len_property)
                .map(|(id, len)| format!("{id:.len$}", len = len.unwrap_or(12)));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "shortest",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [len_node]) = function.expect_arguments()?;
            let len_property = len_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let repo = language.repo;
            let index = match language.id_prefix_context.populate(repo) {
                Ok(index) => index,
                Err(err) => {
                    // Not an error because we can still produce somewhat
                    // reasonable output.
                    diagnostics.add_warning(
                        TemplateParseError::expression(
                            "Failed to load short-prefixes index",
                            function.name_span,
                        )
                        .with_source(err),
                    );
                    IdPrefixIndex::empty()
                }
            };
            // The length of the id printed will be the maximum of the minimum
            // `len` and the length of the shortest unique prefix.
            let out_property = (self_property, len_property).and_then(move |(id, len)| {
                let prefix_len = id.shortest_prefix_len(repo, &index)?;
                let mut hex = format!("{id:.len$}", len = max(prefix_len, len.unwrap_or(0)));
                let rest = hex.split_off(prefix_len);
                Ok(ShortestIdPrefix { prefix: hex, rest })
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ShortestIdPrefix {
    pub prefix: String,
    pub rest: String,
}

impl Template for ShortestIdPrefix {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter.labeled("prefix"), "{}", self.prefix)?;
        write!(formatter.labeled("rest"), "{}", self.rest)?;
        Ok(())
    }
}

impl ShortestIdPrefix {
    fn to_upper(&self) -> Self {
        Self {
            prefix: self.prefix.to_ascii_uppercase(),
            rest: self.rest.to_ascii_uppercase(),
        }
    }
    fn to_lower(&self) -> Self {
        Self {
            prefix: self.prefix.to_ascii_lowercase(),
            rest: self.rest.to_ascii_lowercase(),
        }
    }
}

fn builtin_shortest_id_prefix_methods<'repo>()
-> CommitTemplateBuildMethodFnMap<'repo, ShortestIdPrefix> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<ShortestIdPrefix>::new();
    map.insert(
        "prefix",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.prefix);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "rest",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.rest);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "upper",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.to_upper());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "lower",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.to_lower());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

/// Pair of trees to be diffed.
#[derive(Debug)]
pub struct TreeDiff {
    from_tree: MergedTree,
    to_tree: MergedTree,
    matcher: Rc<dyn Matcher>,
    copy_records: CopyRecords,
}

impl TreeDiff {
    fn from_commit(
        repo: &dyn Repo,
        commit: &Commit,
        matcher: Rc<dyn Matcher>,
    ) -> BackendResult<Self> {
        let mut copy_records = CopyRecords::default();
        for parent in commit.parent_ids() {
            let records =
                diff_util::get_copy_records(repo.store(), parent, commit.id(), &*matcher)?;
            copy_records.add_records(records)?;
        }
        Ok(Self {
            from_tree: commit.parent_tree(repo)?,
            to_tree: commit.tree(),
            matcher,
            copy_records,
        })
    }

    fn diff_stream(&self) -> BoxStream<'_, CopiesTreeDiffEntry> {
        self.from_tree
            .diff_stream_with_copies(&self.to_tree, &*self.matcher, &self.copy_records)
    }

    async fn collect_entries(&self) -> BackendResult<Vec<TreeDiffEntry>> {
        self.diff_stream()
            .map(TreeDiffEntry::from_backend_entry_with_copies)
            .try_collect()
            .await
    }

    fn into_formatted<F, E>(self, show: F) -> TreeDiffFormatted<F>
    where
        F: Fn(&mut dyn Formatter, &Store, BoxStream<CopiesTreeDiffEntry>) -> Result<(), E>,
        E: Into<TemplatePropertyError>,
    {
        TreeDiffFormatted { diff: self, show }
    }
}

/// Tree diff to be rendered by predefined function `F`.
struct TreeDiffFormatted<F> {
    diff: TreeDiff,
    show: F,
}

impl<F, E> Template for TreeDiffFormatted<F>
where
    F: Fn(&mut dyn Formatter, &Store, BoxStream<CopiesTreeDiffEntry>) -> Result<(), E>,
    E: Into<TemplatePropertyError>,
{
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        let show = &self.show;
        let store = self.diff.from_tree.store();
        let tree_diff = self.diff.diff_stream();
        show(formatter.as_mut(), store, tree_diff).or_else(|err| formatter.handle_error(err.into()))
    }
}

fn builtin_tree_diff_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, TreeDiff> {
    type P<'repo> = CommitTemplatePropertyKind<'repo>;
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<TreeDiff>::new();
    map.insert(
        "files",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            // TODO: cache and reuse diff entries within the current evaluation?
            let out_property =
                self_property.and_then(|diff| Ok(diff.collect_entries().block_on()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "color_words",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [context_node]) = function.expect_arguments()?;
            let context_property = context_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let path_converter = language.path_converter;
            let options = diff_util::ColorWordsDiffOptions::from_settings(language.settings())
                .map_err(|err| {
                    let message = "Failed to load diff settings";
                    TemplateParseError::expression(message, function.name_span).with_source(err)
                })?;
            let conflict_marker_style = language.conflict_marker_style;
            let template = (self_property, context_property)
                .map(move |(diff, context)| {
                    let mut options = options.clone();
                    if let Some(context) = context {
                        options.context = context;
                    }
                    diff.into_formatted(move |formatter, store, tree_diff| {
                        diff_util::show_color_words_diff(
                            formatter,
                            store,
                            tree_diff,
                            path_converter,
                            &options,
                            conflict_marker_style,
                        )
                        .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    map.insert(
        "git",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [context_node]) = function.expect_arguments()?;
            let context_property = context_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let options = diff_util::UnifiedDiffOptions::from_settings(language.settings())
                .map_err(|err| {
                    let message = "Failed to load diff settings";
                    TemplateParseError::expression(message, function.name_span).with_source(err)
                })?;
            let conflict_marker_style = language.conflict_marker_style;
            let template = (self_property, context_property)
                .map(move |(diff, context)| {
                    let mut options = options.clone();
                    if let Some(context) = context {
                        options.context = context;
                    }
                    diff.into_formatted(move |formatter, store, tree_diff| {
                        diff_util::show_git_diff(
                            formatter,
                            store,
                            tree_diff,
                            &options,
                            conflict_marker_style,
                        )
                        .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    map.insert(
        "stat",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [width_node]) = function.expect_arguments()?;
            let width_property = width_node
                .map(|node| {
                    template_builder::expect_usize_expression(
                        language,
                        diagnostics,
                        build_ctx,
                        node,
                    )
                })
                .transpose()?;
            let path_converter = language.path_converter;
            // No user configuration exists for diff stat.
            let options = diff_util::DiffStatOptions::default();
            let conflict_marker_style = language.conflict_marker_style;
            // TODO: cache and reuse stats within the current evaluation?
            let out_property = (self_property, width_property).and_then(move |(diff, width)| {
                let store = diff.from_tree.store();
                let tree_diff = diff.diff_stream();
                let stats = DiffStats::calculate(store, tree_diff, &options, conflict_marker_style)
                    .block_on()?;
                Ok(DiffStatsFormatted {
                    stats,
                    path_converter,
                    // TODO: fall back to current available width
                    width: width.unwrap_or(80),
                })
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "summary",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let template = self_property
                .map(move |diff| {
                    diff.into_formatted(move |formatter, _store, tree_diff| {
                        diff_util::show_diff_summary(formatter, tree_diff, path_converter)
                            .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    // TODO: add support for external tools
    map
}

/// [`MergedTree`] diff entry.
#[derive(Clone, Debug)]
pub struct TreeDiffEntry {
    pub path: CopiesTreeDiffEntryPath,
    pub values: Diff<MergedTreeValue>,
}

impl TreeDiffEntry {
    pub fn from_backend_entry_with_copies(entry: CopiesTreeDiffEntry) -> BackendResult<Self> {
        Ok(Self {
            path: entry.path,
            values: entry.values?,
        })
    }

    fn status_label(&self) -> &'static str {
        diff_util::diff_status(&self.path, &self.values).label()
    }

    fn into_source_entry(self) -> TreeEntry {
        TreeEntry {
            path: self.path.source.map_or(self.path.target, |(path, _)| path),
            value: self.values.before,
        }
    }

    fn into_target_entry(self) -> TreeEntry {
        TreeEntry {
            path: self.path.target,
            value: self.values.after,
        }
    }
}

fn builtin_tree_diff_entry_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, TreeDiffEntry>
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<TreeDiffEntry>::new();
    map.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "status",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status_label().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    // TODO: add status_code() or status_char()?
    map.insert(
        "source",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(TreeDiffEntry::into_source_entry);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "target",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(TreeDiffEntry::into_target_entry);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

/// [`MergedTree`] entry.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub path: RepoPathBuf,
    pub value: MergedTreeValue,
}

fn builtin_tree_entry_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, TreeEntry> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<TreeEntry>::new();
    map.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| !entry.value.is_resolved());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "file_type",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| describe_file_type(&entry.value).to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "executable",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| is_executable_file(&entry.value).unwrap_or_default());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn describe_file_type(value: &MergedTreeValue) -> &'static str {
    match value.as_resolved() {
        Some(Some(TreeValue::File { .. })) => "file",
        Some(Some(TreeValue::Symlink(_))) => "symlink",
        Some(Some(TreeValue::Tree(_))) => "tree",
        Some(Some(TreeValue::GitSubmodule(_))) => "git-submodule",
        Some(None) => "", // absent
        None => "conflict",
    }
}

fn is_executable_file(value: &MergedTreeValue) -> Option<bool> {
    let executable = value.to_executable_merge()?;
    conflicts::resolve_file_executable(&executable)
}

/// [`DiffStats`] with rendering parameters.
#[derive(Clone, Debug)]
pub struct DiffStatsFormatted<'a> {
    stats: DiffStats,
    path_converter: &'a RepoPathUiConverter,
    width: usize,
}

impl Template for DiffStatsFormatted<'_> {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        diff_util::show_diff_stats(
            formatter.as_mut(),
            &self.stats,
            self.path_converter,
            self.width,
        )
    }
}

fn builtin_diff_stats_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, DiffStats> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<DiffStats>::new();
    map.insert(
        "files",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|diff| Ok(diff.entries().to_vec()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "total_added",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|stats| Ok(i64::try_from(stats.count_total_added())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "total_removed",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|stats| Ok(i64::try_from(stats.count_total_removed())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_diff_stat_entry_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, DiffStatEntry>
{
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<DiffStatEntry>::new();
    map.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "status",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status.label().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "status_char",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status.char().to_string());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "lines_added",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|entry| {
                Ok(i64::try_from(
                    entry.added_removed.map_or(0, |(added, _)| added),
                )?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "lines_removed",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|entry| {
                Ok(i64::try_from(
                    entry.added_removed.map_or(0, |(_, removed)| removed),
                )?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "bytes_delta",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|entry| Ok(i64::try_from(entry.bytes_delta)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

#[derive(Debug)]
pub struct CryptographicSignature {
    commit: Commit,
}

impl CryptographicSignature {
    fn new(commit: Commit) -> Option<Self> {
        commit.is_signed().then_some(Self { commit })
    }

    fn verify(&self) -> SignResult<Verification> {
        self.commit
            .verification()
            .transpose()
            .expect("must have signature")
    }

    fn status(&self) -> SignResult<SigStatus> {
        self.verify().map(|verification| verification.status)
    }

    /// Defaults to empty string if key is not present.
    fn key(&self) -> SignResult<String> {
        self.verify()
            .map(|verification| verification.key.unwrap_or_default())
    }

    /// Defaults to empty string if display is not present.
    fn display(&self) -> SignResult<String> {
        self.verify()
            .map(|verification| verification.display.unwrap_or_default())
    }
}

fn builtin_cryptographic_signature_methods<'repo>()
-> CommitTemplateBuildMethodFnMap<'repo, CryptographicSignature> {
    // Not using maplit::hashmap!{} or custom declarative macro here because
    // code completion inside macro is quite restricted.
    let mut map = CommitTemplateBuildMethodFnMap::<CryptographicSignature>::new();
    map.insert(
        "status",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|sig| match sig.status() {
                Ok(status) => Ok(status.to_string()),
                Err(SignError::InvalidSignatureFormat) => Ok("invalid".to_string()),
                Err(err) => Err(err.into()),
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "key",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|sig| Ok(sig.key()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "display",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|sig| Ok(sig.display()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

#[derive(Debug, Clone)]
pub struct AnnotationLine {
    pub commit: Commit,
    pub content: BString,
    pub line_number: usize,
    pub original_line_number: usize,
    pub first_line_in_hunk: bool,
}

fn builtin_annotation_line_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, AnnotationLine>
{
    type P<'repo> = CommitTemplatePropertyKind<'repo>;
    let mut map = CommitTemplateBuildMethodFnMap::<AnnotationLine>::new();
    map.insert(
        "commit",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.commit);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "content",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.content);
            // TODO: Add Bytes or BString template type?
            Ok(P::wrap_template(out_property.into_template()))
        },
    );
    map.insert(
        "line_number",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|line| Ok(i64::try_from(line.line_number)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "original_line_number",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|line| Ok(i64::try_from(line.original_line_number)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "first_line_in_hunk",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.first_line_in_hunk);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

impl Template for Trailer {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{}: {}", self.key, self.value)
    }
}

impl Template for Vec<Trailer> {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        templater::format_joined(formatter, self, "\n")
    }
}

fn builtin_trailer_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, Trailer> {
    let mut map = CommitTemplateBuildMethodFnMap::<Trailer>::new();
    map.insert(
        "key",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|trailer| trailer.key);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map.insert(
        "value",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|trailer| trailer.value);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

fn builtin_trailer_list_methods<'repo>() -> CommitTemplateBuildMethodFnMap<'repo, Vec<Trailer>> {
    let mut map: CommitTemplateBuildMethodFnMap<Vec<Trailer>> =
        template_builder::builtin_formattable_list_methods();
    map.insert(
        "contains_key",
        |language, diagnostics, build_ctx, self_property, function| {
            let [key_node] = function.expect_exact_arguments()?;
            let key_property =
                expect_stringify_expression(language, diagnostics, build_ctx, key_node)?;
            let out_property = (self_property, key_property)
                .map(|(trailers, key)| trailers.iter().any(|t| t.key == key));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    map
}

#[cfg(test)]
mod tests {
    use std::path::Component;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;

    use jj_lib::config::ConfigLayer;
    use jj_lib::config::ConfigSource;
    use jj_lib::revset::RevsetAliasesMap;
    use jj_lib::revset::RevsetExpression;
    use jj_lib::revset::RevsetExtensions;
    use jj_lib::revset::RevsetWorkspaceContext;
    use testutils::TestRepoBackend;
    use testutils::TestWorkspace;
    use testutils::repo_path_buf;

    use super::*;
    use crate::template_parser::TemplateAliasesMap;
    use crate::templater::TemplateRenderer;
    use crate::templater::WrapTemplateProperty;

    // TemplateBuildFunctionFn defined for<'a>
    type BuildFunctionFn = for<'a> fn(
        &CommitTemplateLanguage<'a>,
        &mut TemplateDiagnostics,
        &BuildContext<CommitTemplatePropertyKind<'a>>,
        &FunctionCallNode,
    ) -> TemplateParseResult<CommitTemplatePropertyKind<'a>>;

    struct CommitTemplateTestEnv {
        test_workspace: TestWorkspace,
        path_converter: RepoPathUiConverter,
        revset_extensions: Arc<RevsetExtensions>,
        id_prefix_context: IdPrefixContext,
        revset_aliases_map: RevsetAliasesMap,
        template_aliases_map: TemplateAliasesMap,
        immutable_expression: Arc<UserRevsetExpression>,
        extra_functions: HashMap<&'static str, BuildFunctionFn>,
    }

    impl CommitTemplateTestEnv {
        fn init() -> Self {
            // Stabilize commit id of the initialized working copy
            let settings = stable_settings();
            let test_workspace =
                TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &settings);
            let path_converter = RepoPathUiConverter::Fs {
                cwd: test_workspace.workspace.workspace_root().to_owned(),
                base: test_workspace.workspace.workspace_root().to_owned(),
            };
            let revset_extensions = Arc::new(RevsetExtensions::new());
            let id_prefix_context = IdPrefixContext::new(revset_extensions.clone());
            Self {
                test_workspace,
                path_converter,
                revset_extensions,
                id_prefix_context,
                revset_aliases_map: RevsetAliasesMap::new(),
                template_aliases_map: TemplateAliasesMap::new(),
                immutable_expression: RevsetExpression::none(),
                extra_functions: HashMap::new(),
            }
        }

        fn set_base_and_cwd(&mut self, base: PathBuf, cwd: impl AsRef<Path>) {
            self.path_converter = RepoPathUiConverter::Fs {
                cwd: base.join(cwd),
                base,
            };
        }

        fn add_function(&mut self, name: &'static str, f: BuildFunctionFn) {
            self.extra_functions.insert(name, f);
        }

        fn new_language(&self) -> CommitTemplateLanguage<'_> {
            let revset_parse_context = RevsetParseContext {
                aliases_map: &self.revset_aliases_map,
                local_variables: HashMap::new(),
                user_email: "test.user@example.com",
                date_pattern_context: chrono::DateTime::UNIX_EPOCH.fixed_offset().into(),
                default_ignored_remote: None,
                use_glob_by_default: false,
                extensions: &self.revset_extensions,
                workspace: Some(RevsetWorkspaceContext {
                    path_converter: &self.path_converter,
                    workspace_name: self.test_workspace.workspace.workspace_name(),
                }),
            };
            let mut language = CommitTemplateLanguage::new(
                self.test_workspace.repo.as_ref(),
                &self.path_converter,
                self.test_workspace.workspace.workspace_name(),
                revset_parse_context,
                &self.id_prefix_context,
                self.immutable_expression.clone(),
                ConflictMarkerStyle::Diff,
                &[] as &[Box<dyn CommitTemplateLanguageExtension>],
            );
            // Not using .extend() to infer lifetime of f
            for (&name, &f) in &self.extra_functions {
                language.build_fn_table.core.functions.insert(name, f);
            }
            language
        }

        fn parse<'a, C>(&'a self, text: &str) -> TemplateParseResult<TemplateRenderer<'a, C>>
        where
            C: Clone + 'a,
            CommitTemplatePropertyKind<'a>: WrapTemplateProperty<'a, C>,
        {
            let language = self.new_language();
            let mut diagnostics = TemplateDiagnostics::new();
            template_builder::parse(
                &language,
                &mut diagnostics,
                text,
                &self.template_aliases_map,
            )
        }

        fn render_ok<'a, C>(&'a self, text: &str, context: &C) -> String
        where
            C: Clone + 'a,
            CommitTemplatePropertyKind<'a>: WrapTemplateProperty<'a, C>,
        {
            let template = self.parse(text).unwrap();
            let output = template.format_plain_text(context);
            String::from_utf8(output).unwrap()
        }
    }

    fn stable_settings() -> UserSettings {
        let mut config = testutils::base_user_config();
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        layer
            .set_value("debug.commit-timestamp", "2001-02-03T04:05:06+07:00")
            .unwrap();
        config.add_layer(layer);
        UserSettings::from_config(config).unwrap()
    }

    #[test]
    fn test_ref_symbol_type() {
        let mut env = CommitTemplateTestEnv::init();
        env.add_function("sym", |language, diagnostics, build_ctx, function| {
            let [value_node] = function.expect_exact_arguments()?;
            let value = expect_stringify_expression(language, diagnostics, build_ctx, value_node)?;
            let out_property = value.map(RefSymbolBuf);
            Ok(out_property.into_dyn_wrapped())
        });
        let sym = |s: &str| RefSymbolBuf(s.to_owned());

        // default formatting
        insta::assert_snapshot!(env.render_ok("self", &sym("")), @r#""""#);
        insta::assert_snapshot!(env.render_ok("self", &sym("foo")), @"foo");
        insta::assert_snapshot!(env.render_ok("self", &sym("foo bar")), @r#""foo bar""#);

        // comparison
        insta::assert_snapshot!(env.render_ok("self == 'foo'", &sym("foo")), @"true");
        insta::assert_snapshot!(env.render_ok("'bar' == self", &sym("foo")), @"false");
        insta::assert_snapshot!(env.render_ok("self == self", &sym("foo")), @"true");
        insta::assert_snapshot!(env.render_ok("self == sym('bar')", &sym("foo")), @"false");

        insta::assert_snapshot!(env.render_ok("self == 'bar'", &Some(sym("foo"))), @"false");
        insta::assert_snapshot!(env.render_ok("self == sym('foo')", &Some(sym("foo"))), @"true");
        insta::assert_snapshot!(env.render_ok("'foo' == self", &Some(sym("foo"))), @"true");
        insta::assert_snapshot!(env.render_ok("sym('bar') == self", &Some(sym("foo"))), @"false");
        insta::assert_snapshot!(env.render_ok("self == self", &Some(sym("foo"))), @"true");
        insta::assert_snapshot!(env.render_ok("self == ''", &None::<RefSymbolBuf>), @"false");
        insta::assert_snapshot!(env.render_ok("sym('') == self", &None::<RefSymbolBuf>), @"false");
        insta::assert_snapshot!(env.render_ok("self == self", &None::<RefSymbolBuf>), @"true");

        // string cast != formatting: it would be weird if function argument of
        // string type were quoted/escaped. (e.g. `"foo".contains(bookmark)`)
        insta::assert_snapshot!(env.render_ok("stringify(self)", &sym("a b")), @"a b");
        insta::assert_snapshot!(env.render_ok("stringify(self)", &Some(sym("a b"))), @"a b");
        insta::assert_snapshot!(env.render_ok("stringify(self)", &None::<RefSymbolBuf>), @"");

        // string methods
        insta::assert_snapshot!(env.render_ok("self.len()", &sym("a b")), @"3");

        // JSON
        insta::assert_snapshot!(env.render_ok("json(self)", &sym("foo bar")), @r#""foo bar""#);
    }

    #[test]
    fn test_repo_path_type() {
        let mut env = CommitTemplateTestEnv::init();
        let mut base = PathBuf::from(Component::RootDir.as_os_str());
        base.extend(["path", "to", "repo"]);
        env.set_base_and_cwd(base, "dir");

        // slash-separated by default
        insta::assert_snapshot!(
            env.render_ok("self", &repo_path_buf("dir/file")), @"dir/file");

        // .absolute() to convert to absolute path.
        if cfg!(windows) {
            insta::assert_snapshot!(
                env.render_ok("self.absolute()", &repo_path_buf("file")),
                @"\\path\\to\\repo\\file");
            insta::assert_snapshot!(
                env.render_ok("self.absolute()", &repo_path_buf("dir/file")),
                @"\\path\\to\\repo\\dir\\file");
        } else {
            insta::assert_snapshot!(
                env.render_ok("self.absolute()", &repo_path_buf("file")), @"/path/to/repo/file");
            insta::assert_snapshot!(
                env.render_ok("self.absolute()", &repo_path_buf("dir/file")),
                @"/path/to/repo/dir/file");
        }

        // .display() to convert to filesystem path
        insta::assert_snapshot!(
            env.render_ok("self.display()", &repo_path_buf("dir/file")), @"file");
        if cfg!(windows) {
            insta::assert_snapshot!(
                env.render_ok("self.display()", &repo_path_buf("file")), @"..\\file");
        } else {
            insta::assert_snapshot!(
                env.render_ok("self.display()", &repo_path_buf("file")), @"../file");
        }

        let template = "if(self.parent(), self.parent(), '<none>')";
        insta::assert_snapshot!(env.render_ok(template, &repo_path_buf("")), @"<none>");
        insta::assert_snapshot!(env.render_ok(template, &repo_path_buf("file")), @"");
        insta::assert_snapshot!(env.render_ok(template, &repo_path_buf("dir/file")), @"dir");

        // JSON
        insta::assert_snapshot!(
            env.render_ok("json(self)", &repo_path_buf("dir/file")), @r#""dir/file""#);
        insta::assert_snapshot!(
            env.render_ok("json(self)", &None::<RepoPathBuf>), @"null");
    }

    #[test]
    fn test_commit_id_type() {
        let env = CommitTemplateTestEnv::init();

        let id = CommitId::from_hex("08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.normal_hex()", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");

        insta::assert_snapshot!(env.render_ok("self.short()", &id), @"08a70ab33d71");
        insta::assert_snapshot!(env.render_ok("self.short(0)", &id), @"");
        insta::assert_snapshot!(env.render_ok("self.short(-0)", &id), @"");
        insta::assert_snapshot!(
            env.render_ok("self.short(100)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.short(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        insta::assert_snapshot!(env.render_ok("self.shortest()", &id), @"08");
        insta::assert_snapshot!(env.render_ok("self.shortest(0)", &id), @"08");
        insta::assert_snapshot!(env.render_ok("self.shortest(-0)", &id), @"08");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(100)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        // JSON
        insta::assert_snapshot!(
            env.render_ok("json(self)", &id), @r#""08a70ab33d7143b7130ed8594d8216ef688623c0""#);
    }

    #[test]
    fn test_change_id_type() {
        let env = CommitTemplateTestEnv::init();

        let id = ChangeId::from_hex("ffdaa62087a280bddc5e3d3ff933b8ae");
        insta::assert_snapshot!(
            env.render_ok("self", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
        insta::assert_snapshot!(
            env.render_ok("self.normal_hex()", &id), @"ffdaa62087a280bddc5e3d3ff933b8ae");

        insta::assert_snapshot!(env.render_ok("self.short()", &id), @"kkmpptxzrspx");
        insta::assert_snapshot!(env.render_ok("self.short(0)", &id), @"");
        insta::assert_snapshot!(env.render_ok("self.short(-0)", &id), @"");
        insta::assert_snapshot!(
            env.render_ok("self.short(100)", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
        insta::assert_snapshot!(
            env.render_ok("self.short(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        insta::assert_snapshot!(env.render_ok("self.shortest()", &id), @"k");
        insta::assert_snapshot!(env.render_ok("self.shortest(0)", &id), @"k");
        insta::assert_snapshot!(env.render_ok("self.shortest(-0)", &id), @"k");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(100)", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        // JSON
        insta::assert_snapshot!(
            env.render_ok("json(self)", &id), @r#""kkmpptxzrspxrzommnulwmwkkqwworpl""#);
    }

    #[test]
    fn test_shortest_id_prefix_type() {
        let env = CommitTemplateTestEnv::init();

        let id = ShortestIdPrefix {
            prefix: "012".to_owned(),
            rest: "3abcdef".to_owned(),
        };

        // JSON
        insta::assert_snapshot!(
            env.render_ok("json(self)", &id), @r#"{"prefix":"012","rest":"3abcdef"}"#);
    }
}
