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
use std::cmp::max;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::io;
use std::path::Path;
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
use jj_lib::backend::MergedTreeValue;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::copies::CopiesTreeDiffEntry;
use jj_lib::copies::CopiesTreeDiffEntryPath;
use jj_lib::copies::CopyRecords;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::extensions_map::ExtensionsMap;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::id_prefix::IdPrefixIndex;
use jj_lib::index::IndexResult;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
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
use jj_lib::revset;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetContainingFn;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetStreamExt as _;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::rewrite::rebase_to_dest_parent;
use jj_lib::settings::UserSettings;
use jj_lib::signing::SigStatus;
use jj_lib::signing::SignError;
use jj_lib::signing::SignResult;
use jj_lib::signing::Verification;
use jj_lib::store::Store;
use jj_lib::template_parser;
use jj_lib::template_parser::ExpressionNode;
use jj_lib::template_parser::TemplateDiagnostics;
use jj_lib::template_parser::TemplateParseError;
use jj_lib::template_parser::TemplateParseResult;
use jj_lib::trailer;
use jj_lib::trailer::Trailer;
use jj_lib::ui_path::RepoPathUiConverter;
use once_cell::unsync::OnceCell;
use pollster::FutureExt as _;
use serde::Serialize as _;

use crate::diff_util;
use crate::diff_util::DiffStatEntry;
use crate::diff_util::DiffStatOptions;
use crate::diff_util::DiffStats;
use crate::formatter::Formatter;
use crate::operation_templater;
use crate::operation_templater::OperationTemplateEnvironment;
use crate::revset_util;
use crate::revset_util::UserRevsetEvaluationError;
use crate::template_builder;
use crate::template_builder::TemplateLanguage;
use crate::template_builder::TemplatePropertyKind;
use crate::template_builder::TemplateSymbolTable;
use crate::template_builder::expect_stringify_expression;
use crate::template_builder::register_formattable_list_methods;
use crate::template_builder::register_unformattable_list_methods;
use crate::template_value::OpaqueTemplateValue;
use crate::template_value::TemplateTypeRegistry;
use crate::template_value::TemplateValue;
use crate::templater;
use crate::templater::BoxedTemplateProperty;
use crate::templater::Literal;
use crate::templater::NoTemplateBooleanCast;
use crate::templater::SizeHint;
use crate::templater::Template;
use crate::templater::TemplateFormatter;
use crate::templater::TemplatePropertyError;
use crate::templater::TemplatePropertyExt as _;

pub trait CommitTemplateLanguageExtension {
    fn symbol_table<'repo>(&self) -> TemplateSymbolTable<'repo, CommitTemplateLanguage<'repo>>;

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
    // TODO: Maybe refactor context structs in a way that
    // Revset/FilesetParseContext are constructed from a common resource?
    // RepoPathUiConverter and WorkspaceName are contained in RevsetParseContext
    // for example.
    revset_parse_context: RevsetParseContext<'repo>,
    id_prefix_context: &'repo IdPrefixContext,
    immutable_expression: Arc<UserRevsetExpression>,
    conflict_marker_style: ConflictMarkerStyle,
    type_registry: TemplateTypeRegistry,
    symbol_table: TemplateSymbolTable<'repo, Self>,
    keyword_cache: CommitKeywordCache,
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
        let mut type_registry = template_builder::new_core_type_registry();
        operation_templater::register_types(&mut type_registry);
        register_types(&mut type_registry);
        let mut symbol_table = TemplateSymbolTable::builtin();
        operation_templater::register_symbols(&mut symbol_table);
        register_symbols(&mut symbol_table);

        let mut cache_extensions = ExtensionsMap::empty();
        for extension in extensions {
            symbol_table.merge(extension.as_ref().symbol_table());
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
            type_registry,
            symbol_table,
            keyword_cache: CommitKeywordCache::default(),
            cache_extensions,
        }
    }

    fn conflict_materialize_options(&self) -> ConflictMaterializeOptions {
        ConflictMaterializeOptions {
            marker_style: self.conflict_marker_style,
            marker_len: None,
            merge: self.repo.store().merge_options().clone(),
        }
    }

    fn fileset_parse_context(&self) -> FilesetParseContext<'repo> {
        FilesetParseContext {
            aliases_map: self.revset_parse_context.fileset_aliases_map,
            path_converter: self.path_converter,
        }
    }
}

impl<'repo> TemplateLanguage<'repo> for CommitTemplateLanguage<'repo> {
    fn settings(&self) -> &UserSettings {
        self.repo.base_repo().settings()
    }

    fn current_dir(&self) -> &Path {
        let RepoPathUiConverter::Fs { cwd, base: _ } = self.path_converter;
        cwd
    }

    fn type_registry(&self) -> &TemplateTypeRegistry {
        &self.type_registry
    }

    fn symbol_table(&self) -> &TemplateSymbolTable<'repo, Self> {
        &self.symbol_table
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

    pub fn keyword_cache(&self) -> &CommitKeywordCache {
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

impl OpaqueTemplateValue for Commit {}
impl OpaqueTemplateValue for CommitEvolutionEntry {}
impl OpaqueTemplateValue for Rc<CommitRef> {}
impl OpaqueTemplateValue for WorkspaceRef {}
impl OpaqueTemplateValue for RefSymbolBuf {}
impl OpaqueTemplateValue for RepoPathBuf {}
impl OpaqueTemplateValue for ChangeId {}
impl OpaqueTemplateValue for CommitId {}
impl OpaqueTemplateValue for ShortestIdPrefix {}
impl OpaqueTemplateValue for TreeDiff {}
impl OpaqueTemplateValue for TreeDiffEntry {}
impl OpaqueTemplateValue for TreeEntry {}
impl OpaqueTemplateValue for TreeValue {}
impl OpaqueTemplateValue for DiffStatsFormatted {}
impl OpaqueTemplateValue for DiffStatEntry {}
impl OpaqueTemplateValue for CryptographicSignature {}
impl OpaqueTemplateValue for AnnotationLine {}
impl OpaqueTemplateValue for Trailer {}

impl NoTemplateBooleanCast for Commit {}
impl NoTemplateBooleanCast for CommitEvolutionEntry {}
impl NoTemplateBooleanCast for Rc<CommitRef> {}
impl NoTemplateBooleanCast for WorkspaceRef {}
impl NoTemplateBooleanCast for RefSymbolBuf {}
impl NoTemplateBooleanCast for RepoPathBuf {}
impl NoTemplateBooleanCast for ChangeId {}
impl NoTemplateBooleanCast for CommitId {}
impl NoTemplateBooleanCast for ShortestIdPrefix {}
impl NoTemplateBooleanCast for TreeDiff {}
impl NoTemplateBooleanCast for TreeDiffEntry {}
impl NoTemplateBooleanCast for TreeEntry {}
impl NoTemplateBooleanCast for TreeValue {}
impl NoTemplateBooleanCast for DiffStatsFormatted {}
impl NoTemplateBooleanCast for DiffStatEntry {}
impl NoTemplateBooleanCast for CryptographicSignature {}
impl NoTemplateBooleanCast for AnnotationLine {}
impl NoTemplateBooleanCast for Trailer {}

fn register_types(registry: &mut TemplateTypeRegistry) {
    registry.register::<Commit>("Commit").serializable();
    registry
        .register::<Option<Commit>>("Option<Commit>")
        .conditionable()
        .serializable();
    registry
        .register::<Vec<Commit>>("List<Commit>")
        .conditionable()
        .serializable();
    registry
        .register::<Vec<Option<Commit>>>("List<Option<Commit>>")
        .conditionable()
        .serializable();
    registry
        .register::<CommitEvolutionEntry>("CommitEvolutionEntry")
        .serializable();
    registry
        .register::<Rc<CommitRef>>("CommitRef")
        .formattable()
        .serializable();
    registry
        .register::<Option<Rc<CommitRef>>>("Option<CommitRef>")
        .conditionable()
        .formattable()
        .serializable();
    registry
        .register::<Vec<Rc<CommitRef>>>("List<CommitRef>")
        .conditionable()
        .formattable()
        .serializable();
    registry
        .register::<WorkspaceRef>("WorkspaceRef")
        .formattable()
        .serializable();
    registry
        .register::<Option<WorkspaceRef>>("Option<WorkspaceRef>")
        .conditionable()
        .formattable()
        .serializable();
    registry
        .register::<Vec<WorkspaceRef>>("List<WorkspaceRef>")
        .conditionable()
        .formattable()
        .serializable();
    registry
        .register::<RefSymbolBuf>("RefSymbol")
        .newtype_of::<String>()
        .formattable()
        .serializable()
        .equatable()
        .equatable_with::<String>();
    registry
        .register::<Option<RefSymbolBuf>>("Option<RefSymbol>")
        .conditionable()
        .formattable()
        .serializable()
        .equatable();
    registry
        .register::<RepoPathBuf>("RepoPath")
        .formattable()
        .serializable();
    registry
        .register::<Option<RepoPathBuf>>("Option<RepoPath>")
        .conditionable()
        .formattable()
        .serializable();
    registry
        .register::<ChangeId>("ChangeId")
        .formattable()
        .serializable();
    registry
        .register::<CommitId>("CommitId")
        .formattable()
        .serializable();
    registry
        .register::<ShortestIdPrefix>("ShortestIdPrefix")
        .formattable()
        .serializable();
    registry.register::<TreeDiff>("TreeDiff");
    registry.register::<TreeDiffEntry>("TreeDiffEntry");
    registry
        .register::<Option<TreeDiffEntry>>("Option<TreeDiffEntry>")
        .conditionable();
    registry
        .register::<Vec<TreeDiffEntry>>("List<TreeDiffEntry>")
        .conditionable();
    registry.register::<TreeEntry>("TreeEntry");
    registry
        .register::<Option<TreeEntry>>("Option<TreeEntry>")
        .conditionable();
    registry
        .register::<Vec<TreeEntry>>("List<TreeEntry>")
        .conditionable();
    registry.register::<TreeValue>("TreeValue").formattable();
    registry
        .register::<Option<TreeValue>>("Option<TreeValue>")
        .conditionable()
        .formattable();
    registry
        .register::<Vec<Option<TreeValue>>>("List<Option<TreeValue>>")
        .conditionable();
    registry
        .register::<DiffStatsFormatted>("DiffStats")
        .formattable();
    registry.register::<DiffStatEntry>("DiffStatEntry");
    registry
        .register::<Option<DiffStatEntry>>("Option<DiffStatEntry>")
        .conditionable();
    registry
        .register::<Vec<DiffStatEntry>>("List<DiffStatEntry>")
        .conditionable();
    registry.register::<CryptographicSignature>("CryptographicSignature");
    registry
        .register::<Option<CryptographicSignature>>("Option<CryptographicSignature>")
        .conditionable();
    registry.register::<AnnotationLine>("AnnotationLine");
    registry.register::<Trailer>("Trailer").formattable();
    registry
        .register::<Option<Trailer>>("Option<Trailer>")
        .conditionable()
        .formattable();
    registry
        .register::<Vec<Trailer>>("List<Trailer>")
        .conditionable()
        .formattable();
}

fn register_symbols(table: &mut CommitTemplateSymbolTable<'_>) {
    register_functions(table);
    register_commit_methods(table);
    register_unformattable_list_methods::<_, Commit>(table);
    register_unformattable_list_methods::<_, Option<Commit>>(table);
    register_commit_evolution_entry_methods(table);
    register_commit_ref_methods(table);
    register_formattable_list_methods::<_, Rc<CommitRef>>(table);
    register_workspace_ref_methods(table);
    register_formattable_list_methods::<_, WorkspaceRef>(table);
    register_repo_path_methods(table);
    register_change_id_methods(table);
    register_commit_id_methods(table);
    register_shortest_id_prefix_methods(table);
    register_tree_diff_methods(table);
    register_tree_diff_entry_methods(table);
    register_unformattable_list_methods::<_, TreeDiffEntry>(table);
    register_tree_entry_methods(table);
    register_unformattable_list_methods::<_, TreeEntry>(table);
    register_tree_value_methods(table);
    register_formattable_list_methods::<_, Option<TreeValue>>(table);
    register_diff_stats_methods(table);
    register_diff_stat_entry_methods(table);
    register_unformattable_list_methods::<_, DiffStatEntry>(table);
    register_cryptographic_signature_methods(table);
    register_annotation_line_methods(table);
    register_trailer_methods(table);
    register_trailer_list_methods(table);
}

type CommitTemplateSymbolTable<'repo> = TemplateSymbolTable<'repo, CommitTemplateLanguage<'repo>>;

#[derive(Default)]
pub struct CommitKeywordCache {
    // Build index lazily, and Rc to get away from &self lifetime.
    bookmarks_index: OnceCell<Rc<CommitRefsIndex>>,
    tags_index: OnceCell<Rc<CommitRefsIndex>>,
    git_refs_index: OnceCell<Rc<CommitRefsIndex>>,
    is_immutable_fn: OnceCell<Rc<RevsetContainingFn<'static>>>,
}

impl CommitKeywordCache {
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
        language: &CommitTemplateLanguage,
        span: pest::Span<'_>,
    ) -> TemplateParseResult<&Rc<RevsetContainingFn<'static>>> {
        // Alternatively, a negated (i.e. visible mutable) set could be computed.
        // It's usually smaller than the immutable set. The revset engine can also
        // optimize "::<recent_heads>" query to use bitset-based implementation.
        self.is_immutable_fn.get_or_try_init(|| {
            let expression = &language.immutable_expression;
            let revset = evaluate_revset_expression(
                language.repo,
                &language.revset_parse_context,
                language.id_prefix_context,
                expression,
            )
            .map_err(|err| {
                TemplateParseError::expression("Failed to evaluate revset", span).with_source(err)
            })?;
            Ok(revset.containing_fn().into())
        })
    }
}

/// Builtin functions for the commit template language.
fn register_functions(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut functions = table.register_functions();
    functions.insert("revset", |language, diagnostics, build_ctx, function| {
        let [revset_node] = function.expect_exact_arguments()?;
        let revset_property =
            expect_stringify_expression(language, diagnostics, build_ctx, revset_node)?;
        if let Ok(query) = revset_property.extract() {
            let commits: Vec<Commit> =
                template_parser::catch_aliases(diagnostics, revset_node, |diagnostics, node| {
                    let revset =
                        parse_evaluate_static_revset(language, diagnostics, &query, node.span)?;
                    revset
                        .stream()
                        .commits(language.repo.store())
                        .try_collect()
                        .block_on()
                        .map_err(|err| {
                            TemplateParseError::expression("Failed to evaluate revset", node.span)
                                .with_source(err)
                        })
                })?;
            Ok(Literal(commits).into_dyn_wrapped())
        } else {
            let repo = language.repo;
            let revset_parse_context = language.revset_parse_context.clone();
            let id_prefix_context = language.id_prefix_context;
            let out_property = revset_property.and_then(move |query| {
                let (expression, _) = parse_revset_expression(&revset_parse_context, &query)?;
                let revset = evaluate_revset_expression(
                    repo,
                    &revset_parse_context,
                    id_prefix_context,
                    &expression,
                )?;
                let commits: Vec<Commit> = revset
                    .stream()
                    .commits(repo.store())
                    .try_collect()
                    .block_on()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        }
    });
    functions.insert(
        "git_web_url",
        |language, diagnostics, build_ctx, function| {
            let ([], [remote_node]) = function.expect_named_arguments(&["remote"])?;
            let remote_property: BoxedTemplateProperty<String> = match remote_node {
                Some(node) => expect_stringify_expression(language, diagnostics, build_ctx, node)?,
                None => Box::new(Literal("origin".to_owned())),
            };
            #[cfg(feature = "git")]
            let out_property = {
                let repo = language.repo;
                remote_property.map(move |remote_name| {
                    crate::git_util::get_remote_web_url(repo.base_repo(), &remote_name)
                        .unwrap_or_default()
                })
            };
            #[cfg(not(feature = "git"))]
            let out_property = {
                drop(remote_property);
                Literal(String::new())
            };
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

fn register_commit_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<Commit>();
    methods.insert(
        "description",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.description().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "trailers",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property
                .map(|commit| trailer::parse_description_trailers(commit.description()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "change_id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.change_id().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "commit_id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.id().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "parents",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|commit| {
                let commits = commit.parents().block_on()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "author",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.author().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "committer",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.committer().clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "mine",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let user_email = language.revset_parse_context.user_email.to_owned();
            let out_property = self_property.map(move |commit| commit.author().email == user_email);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "signature",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(CryptographicSignature::new);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "working_copies",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.map(|commit| extract_working_copies(repo, &commit));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
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
    methods.insert(
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
    methods.insert(
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
    methods.insert(
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
    methods.insert(
        "tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_distinct_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "local_tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_local_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "remote_tags",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let index = language.keyword_cache.tags_index(language.repo).clone();
            let out_property =
                self_property.map(move |commit| collect_remote_refs(index.get(commit.id())));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "divergent",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| {
                // The given commit could be hidden in e.g. `jj evolog`.
                let maybe_targets = repo.resolve_change_id(commit.change_id()).block_on()?;
                let divergent = maybe_targets.is_some_and(|targets| targets.is_divergent());
                Ok(divergent)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "hidden",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| {
                //  A commit is hidden if its commit id is not in the change id index.
                let maybe_targets = repo.resolve_change_id(commit.change_id()).block_on()?;
                Ok(maybe_targets.is_none_or(|targets| !targets.has_visible(commit.id())))
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "change_offset",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit| {
                // The given commit could be hidden in e.g. `jj evolog`.
                let maybe_targets = repo.resolve_change_id(commit.change_id()).block_on()?;
                let offset = maybe_targets
                    .and_then(|targets| targets.find_offset(commit.id()))
                    .map(i64::try_from)
                    .transpose()?;
                Ok(offset)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "immutable",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let is_immutable = language
                .keyword_cache
                .is_immutable_fn(language, function.name_span)?
                .clone();
            let out_property =
                self_property.and_then(move |commit| Ok(is_immutable(commit.id()).block_on()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "contained_in",
        |language, diagnostics, build_ctx, self_property, function| {
            let [revset_node] = function.expect_exact_arguments()?;
            let revset_property =
                expect_stringify_expression(language, diagnostics, build_ctx, revset_node)?;
            if let Ok(query) = revset_property.extract() {
                let revset = template_parser::catch_aliases(
                    diagnostics,
                    revset_node,
                    |diagnostics, node| {
                        parse_evaluate_static_revset(language, diagnostics, &query, node.span)
                    },
                )?;
                let is_contained = revset.containing_fn();
                let out_property =
                    self_property.and_then(move |commit| Ok(is_contained(commit.id()).block_on()?));
                Ok(out_property.into_dyn_wrapped())
            } else {
                let repo = language.repo;
                let revset_parse_context = language.revset_parse_context.clone();
                let id_prefix_context = language.id_prefix_context;
                let out_property =
                    (self_property, revset_property).and_then(move |(commit, query)| {
                        let (expression, _) =
                            parse_revset_expression(&revset_parse_context, &query)?;
                        let revset = evaluate_revset_expression(
                            repo,
                            &revset_parse_context,
                            id_prefix_context,
                            &expression,
                        )?;
                        Ok(revset.containing_fn()(commit.id()).block_on()?)
                    });
                Ok(out_property.into_dyn_wrapped())
            }
        },
    );
    methods.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit| commit.has_conflict());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "empty",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.and_then(|commit| Ok(commit.is_empty(repo.index()).block_on()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "diff",
        |language, diagnostics, _build_ctx, self_property, function| {
            let ([], [files_node]) = function.expect_arguments()?;
            let files = if let Some(node) = files_node {
                expect_fileset_literal(diagnostics, node, &language.fileset_parse_context())?
            } else {
                // TODO: defaults to CLI path arguments?
                // https://github.com/jj-vcs/jj/issues/2933#issuecomment-1925870731
                FilesetExpression::all()
            };
            let repo = language.repo;
            let matcher: Rc<dyn Matcher> = files.to_matcher().into();
            let out_property = self_property.and_then(move |commit| {
                Ok(TreeDiff::from_commit(repo, &commit, matcher.clone()).block_on()?)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "files",
        |language, diagnostics, _build_ctx, self_property, function| {
            let ([], [files_node]) = function.expect_arguments()?;
            let files = if let Some(node) = files_node {
                expect_fileset_literal(diagnostics, node, &language.fileset_parse_context())?
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
                    .map(|(path, value)| value.map(|value| TreeEntry { path, value }))
                    .try_collect()
                    .block_on()?;
                Ok(entries)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "conflicted_files",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|commit| {
                let tree = commit.tree();
                let entries: Vec<_> = tree
                    .conflicts()
                    .map(|(path, value)| value.map(|value| TreeEntry { path, value }))
                    .try_collect()
                    .block_on()?;
                Ok(entries)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "root",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.map(|commit| commit.id() == repo.store().root_commit_id());
            Ok(out_property.into_dyn_wrapped())
        },
    );
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
    context: &FilesetParseContext,
) -> Result<FilesetExpression, TemplateParseError> {
    template_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        let text = template_parser::expect_string_literal(node)?;
        let mut inner_diagnostics = FilesetDiagnostics::new();
        let expression = fileset::parse(&mut inner_diagnostics, text, context).map_err(|err| {
            TemplateParseError::expression("In fileset expression", node.span).with_source(err)
        })?;
        diagnostics.extend_with(inner_diagnostics, |diag| {
            TemplateParseError::expression("In fileset expression", node.span).with_source(diag)
        });
        Ok(expression)
    })
}

fn parse_revset_expression(
    revset_parse_context: &RevsetParseContext,
    revset_str: &str,
) -> Result<(Arc<UserRevsetExpression>, RevsetDiagnostics), RevsetParseError> {
    let mut diagnostics = RevsetDiagnostics::new();
    let expression = revset::parse(&mut diagnostics, revset_str, revset_parse_context)?;
    Ok((expression, diagnostics))
}

fn evaluate_revset_expression(
    repo: &dyn Repo,
    revset_parse_context: &RevsetParseContext,
    id_prefix_context: &IdPrefixContext,
    expression: &UserRevsetExpression,
) -> Result<Box<dyn Revset>, UserRevsetEvaluationError> {
    let symbol_resolver = revset_util::default_symbol_resolver(
        repo,
        revset_parse_context.extensions.symbol_resolvers(),
        id_prefix_context,
    );
    let revset = expression
        .resolve_user_expression(repo, &symbol_resolver)
        .map_err(UserRevsetEvaluationError::Resolution)?
        .evaluate()
        .map_err(UserRevsetEvaluationError::Evaluation)?;
    Ok(revset)
}

fn parse_evaluate_static_revset(
    language: &CommitTemplateLanguage,
    diagnostics: &mut TemplateDiagnostics,
    query: &str,
    span: pest::Span<'_>,
) -> Result<Box<dyn Revset>, TemplateParseError> {
    let (expression, inner_diagnostics) =
        parse_revset_expression(&language.revset_parse_context, query).map_err(|err| {
            TemplateParseError::expression("In revset expression", span).with_source(err)
        })?;
    diagnostics.extend_with(inner_diagnostics, |diag| {
        TemplateParseError::expression("In revset expression", span).with_source(diag)
    });
    evaluate_revset_expression(
        language.repo,
        &language.revset_parse_context,
        language.id_prefix_context,
        &expression,
    )
    .map_err(|err| {
        TemplateParseError::expression("Failed to evaluate revset", span).with_source(err)
    })
}

fn register_commit_evolution_entry_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<CommitEvolutionEntry>();
    methods.insert(
        "commit",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.commit);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "operation",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.operation);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "predecessors",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|entry| {
                let commits = entry.predecessors().block_on()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "inter_diff",
        |language, diagnostics, _build_ctx, self_property, function| {
            let ([], [files_node]) = function.expect_arguments()?;
            let files = if let Some(node) = files_node {
                expect_fileset_literal(diagnostics, node, &language.fileset_parse_context())?
            } else {
                FilesetExpression::all()
            };
            let repo = language.repo;
            let matcher: Rc<dyn Matcher> = files.to_matcher().into();
            let out_property = self_property.and_then(move |entry| {
                let predecessors = entry.predecessors().block_on()?;
                let from_tree =
                    rebase_to_dest_parent(repo.index(), &predecessors, &entry.commit).block_on()?;
                let to_tree = entry.commit.tree();
                Ok(TreeDiff {
                    from_tree,
                    to_tree,
                    matcher: matcher.clone(),
                    copy_records: Rc::new(CopyRecords::default()), // TODO: copy tracking
                })
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
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

    /// Whether the ref target is resolved.
    pub fn is_resolved(&self) -> bool {
        self.target.is_resolved()
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
                let self_ids = self.target.present_adds().cloned().collect_vec();
                let other_ids = tracking.target.present_adds().cloned().collect_vec();
                Ok(revset::walk_revs(repo, &self_ids, &other_ids)?
                    .count_estimate()
                    .block_on()?)
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
                let self_ids = self.target.present_adds().cloned().collect_vec();
                let other_ids = tracking.target.present_adds().cloned().collect_vec();
                Ok(revset::walk_revs(repo, &other_ids, &self_ids)?
                    .count_estimate()
                    .block_on()?)
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
        if !self.is_resolved() {
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

fn register_workspace_ref_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<WorkspaceRef>();
    methods.insert(
        "name",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|ws_ref| RefSymbolBuf(ws_ref.name.into()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "current_workspace",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let current_workspace_name = language.workspace_name.clone();
            let out_property =
                self_property.map(move |ws_ref| ws_ref.name == current_workspace_name);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "target",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|ws_ref| ws_ref.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "root",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let workspace_store = language.repo.base_repo().loader().workspace_store();
            // Workspaces created before jj 0.38.0 may not have a recorded path. List
            // templates should also keep rendering if a recorded path is stale or
            // unavailable. Use `jj workspace root --name` for strict path diagnostics.
            let out_property = self_property
                .and_then(|ws_ref| Ok(workspace_store.get_workspace_path(&ws_ref.name)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

#[expect(clippy::ref_option)]
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

fn register_commit_ref_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<Rc<CommitRef>>();
    methods.insert(
        "name",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.name.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "remote",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.remote.clone());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "present",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_present());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| !commit_ref.is_resolved());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
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
    methods.insert(
        "removed_targets",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit_ref| {
                let ids = commit_ref.target.removes().map(Option::as_ref);
                let commits: Vec<_> = ids
                    .map(|id| id.map(|id| repo.store().get_commit(id)).transpose())
                    .try_collect()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "added_targets",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property = self_property.and_then(|commit_ref| {
                let ids = commit_ref.target.adds().map(Option::as_ref);
                let commits: Vec<_> = ids
                    .map(|id| id.map(|id| repo.store().get_commit(id)).transpose())
                    .try_collect()?;
                Ok(commits)
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "tracked",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_tracked());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "tracking_present",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.is_tracking_present());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "tracking_ahead_count",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.and_then(|commit_ref| commit_ref.tracking_ahead_count(repo));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "tracking_behind_count",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let repo = language.repo;
            let out_property =
                self_property.and_then(|commit_ref| commit_ref.tracking_behind_count(repo));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "synced",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|commit_ref| commit_ref.synced);
            Ok(out_property.into_dyn_wrapped())
        },
    );
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
            index.insert(local_target.present_adds(), commit_ref);
        }
        for &(remote_name, remote_ref) in &remote_refs {
            let commit_ref = CommitRef::remote(name, remote_name, remote_ref.clone(), local_target);
            index.insert(remote_ref.target.present_adds(), commit_ref);
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
        index.insert(target.present_adds(), commit_ref);
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

impl From<RefSymbolBuf> for String {
    fn from(value: RefSymbolBuf) -> Self {
        value.0
    }
}

impl PartialEq<String> for RefSymbolBuf {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl PartialEq<RefSymbolBuf> for String {
    fn eq(&self, other: &RefSymbolBuf) -> bool {
        self == &other.0
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

fn register_repo_path_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<RepoPathBuf>();
    methods.insert(
        "absolute",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            // We handle the absolute path here instead of in a wrapper in
            // `RepoPathUiConverter` because absolute paths only make sense for
            // filesystem paths. Other cases should fail here.
            let out_property = self_property.and_then(move |path| match path_converter {
                RepoPathUiConverter::Fs { cwd: _, base } => Ok(path.to_fs_path(base)?),
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "display",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let out_property = self_property.map(|path| path_converter.format_file_path(&path));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "parent",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|path| Some(path.parent()?.to_owned()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

trait ShortestIdPrefixLen {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize>;
}

impl ShortestIdPrefixLen for ChangeId {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize> {
        index.shortest_change_prefix_len(repo, self).block_on()
    }
}

impl Template for ChangeId {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{self}")
    }
}

fn register_change_id_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    register_commit_or_change_id_methods::<ChangeId>(table);
    let mut methods = table.register_methods::<ChangeId>();
    methods.insert(
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
}

impl ShortestIdPrefixLen for CommitId {
    fn shortest_prefix_len(&self, repo: &dyn Repo, index: &IdPrefixIndex) -> IndexResult<usize> {
        index.shortest_commit_prefix_len(repo, self).block_on()
    }
}

impl Template for CommitId {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{self}")
    }
}

fn register_commit_id_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    register_commit_or_change_id_methods::<CommitId>(table);
}

fn register_commit_or_change_id_methods<'repo, O>(table: &mut CommitTemplateSymbolTable<'repo>)
where
    O: Display + ShortestIdPrefixLen + TemplateValue + 'repo,
{
    let mut methods = table.register_methods::<O>();
    methods.insert(
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
            let out_property = (self_property, len_property).map(|(id, len)| {
                let mut id_str = id.to_string();
                id_str.truncate(len.unwrap_or(12));
                id_str
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
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
            let index = match language.id_prefix_context.populate(repo).block_on() {
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
                let mut hex = id.to_string();
                hex.truncate(max(prefix_len, len.unwrap_or(0)));
                let rest = hex.split_off(prefix_len);
                Ok(ShortestIdPrefix { prefix: hex, rest })
            });
            Ok(out_property.into_dyn_wrapped())
        },
    );
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

fn register_shortest_id_prefix_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<ShortestIdPrefix>();
    methods.insert(
        "prefix",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.prefix);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "rest",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.rest);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "upper",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.to_upper());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "lower",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|id| id.to_lower());
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

/// Pair of trees to be diffed.
#[derive(Clone, Debug)]
pub struct TreeDiff {
    from_tree: MergedTree,
    to_tree: MergedTree,
    matcher: Rc<dyn Matcher>,
    copy_records: Rc<CopyRecords>,
}

impl TreeDiff {
    async fn from_commit(
        repo: &dyn Repo,
        commit: &Commit,
        matcher: Rc<dyn Matcher>,
    ) -> BackendResult<Self> {
        let mut copy_records = CopyRecords::default();
        for parent in commit.parent_ids() {
            let records =
                diff_util::get_copy_records(repo.store(), parent, commit.id(), &*matcher).await?;
            copy_records.add_records(records);
        }
        Ok(Self {
            from_tree: commit.parent_tree(repo.index()).await?,
            to_tree: commit.tree(),
            matcher,
            copy_records: Rc::new(copy_records),
        })
    }

    fn diff_stream(&self) -> BoxStream<'_, CopiesTreeDiffEntry> {
        self.from_tree
            .diff_with_copies(&self.to_tree, &*self.matcher, &self.copy_records)
    }

    async fn collect_entries(&self) -> BackendResult<Vec<TreeDiffEntry>> {
        self.diff_stream()
            .map(TreeDiffEntry::from_backend_entry_with_copies)
            .try_collect()
            .await
    }

    fn into_formatted<F, E>(self, show: F) -> TreeDiffFormatted<F>
    where
        F: Fn(
            &mut dyn Formatter,
            &Store,
            BoxStream<CopiesTreeDiffEntry>,
            Diff<&ConflictLabels>,
        ) -> Result<(), E>,
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
    F: Fn(
        &mut dyn Formatter,
        &Store,
        BoxStream<CopiesTreeDiffEntry>,
        Diff<&ConflictLabels>,
    ) -> Result<(), E>,
    E: Into<TemplatePropertyError>,
{
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        let show = &self.show;
        let store = self.diff.from_tree.store();
        let tree_diff = self.diff.diff_stream();
        let conflict_labels = Diff::new(self.diff.from_tree.labels(), self.diff.to_tree.labels());
        show(formatter.as_mut(), store, tree_diff, conflict_labels)
            .or_else(|err| formatter.handle_error(err.into()))
    }
}

fn register_tree_diff_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<TreeDiff>();
    type P<'repo> = TemplatePropertyKind<'repo>;
    methods.insert(
        "files",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            // TODO: cache and reuse diff entries within the current evaluation?
            let out_property =
                self_property.and_then(|diff| Ok(diff.collect_entries().block_on()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
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
            let materialize_options = language.conflict_materialize_options();
            let template = (self_property, context_property)
                .map(move |(diff, context)| {
                    let mut options = options.clone();
                    if let Some(context) = context {
                        options.context = context;
                    }
                    let materialize_options = materialize_options.clone();
                    diff.into_formatted(move |formatter, store, tree_diff, conflict_labels| {
                        diff_util::show_color_words_diff(
                            formatter,
                            store,
                            tree_diff,
                            conflict_labels,
                            path_converter,
                            &options,
                            &materialize_options,
                        )
                        .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    methods.insert(
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
            let materialize_options = language.conflict_materialize_options();
            let template = (self_property, context_property)
                .map(move |(diff, context)| {
                    let mut options = options.clone();
                    if let Some(context) = context {
                        options.context = context;
                    }
                    let materialize_options = materialize_options.clone();
                    diff.into_formatted(move |formatter, store, trees, tree_diff| {
                        diff_util::show_git_diff(
                            formatter,
                            store,
                            trees,
                            tree_diff,
                            &options,
                            &materialize_options,
                        )
                        .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    methods.insert(
        "stat",
        |language, diagnostics, build_ctx, self_property, function| {
            let ([], [width_node, max_bar_width_node]) =
                function.expect_named_arguments(&["", "max_bar_width"])?;
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
            let max_bar_width_property = max_bar_width_node
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
            let options =
                diff_util::DiffStatOptions::from_settings(language.settings()).map_err(|err| {
                    let message = "Failed to load diff settings";
                    TemplateParseError::expression(message, function.name_span).with_source(err)
                })?;
            let materialize_options = language.conflict_materialize_options();
            // TODO: cache and reuse stats within the current evaluation?
            let out_property = (self_property, width_property, max_bar_width_property).and_then(
                move |(diff, width, max_bar_width)| {
                    let mut options = options.clone();
                    if let Some(max_bar_width) = max_bar_width {
                        options.max_bar_width = Some(max_bar_width);
                    }
                    let store = diff.from_tree.store();
                    let tree_diff = diff.diff_stream();
                    let stats =
                        DiffStats::calculate(store, tree_diff, &options, &materialize_options)
                            .block_on()?;
                    Ok(DiffStatsFormatted {
                        stats,
                        path_converter: path_converter.clone(),
                        // TODO: fall back to current available width
                        width: width.unwrap_or(80),
                        options,
                    })
                },
            );
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "summary",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let template = self_property
                .map(move |diff| {
                    diff.into_formatted(move |formatter, _store, tree_diff, _conflict_labels| {
                        diff_util::show_diff_summary(formatter, tree_diff, path_converter)
                            .block_on()
                    })
                })
                .into_template();
            Ok(P::wrap_template(template))
        },
    );
    // TODO: add support for external tools
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

    fn status(&self) -> diff_util::DiffEntryStatus {
        diff_util::diff_status(&self.path, &self.values)
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

fn format_diff_path(
    path: &CopiesTreeDiffEntryPath,
    path_converter: &RepoPathUiConverter,
) -> String {
    match path.to_diff() {
        Some(paths) => path_converter.format_copied_path(paths),
        None => path_converter.format_file_path(path.target()),
    }
}

fn register_tree_diff_entry_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<TreeDiffEntry>();
    methods.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "display_diff_path",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let out_property =
                self_property.map(move |entry| format_diff_path(&entry.path, path_converter));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "status",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status().label().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "status_char",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status().char().to_string());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "source",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(TreeDiffEntry::into_source_entry);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "target",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(TreeDiffEntry::into_target_entry);
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

/// [`MergedTree`] entry.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub path: RepoPathBuf,
    pub value: MergedTreeValue,
}

fn register_tree_entry_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<TreeEntry>();
    methods.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "normal_value",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.value.as_normal().cloned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "removed_values",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| entry.value.simplify().removes().cloned().collect_vec());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "added_values",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| entry.value.simplify().adds().cloned().collect_vec());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "conflict",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| !entry.value.is_resolved());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "conflict_side_count",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property
                .and_then(|entry| Ok(i64::try_from(entry.value.simplify().num_sides())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "file_type",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| describe_file_type(&entry.value).to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "executable",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|entry| is_executable_file(&entry.value).unwrap_or_default());
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

fn register_tree_value_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<TreeValue>();
    methods.insert(
        "object_id",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|tree_value| tree_value_object_id(&tree_value));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "file_type",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.map(|tree_value| tree_value_file_type(&tree_value).to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "executable",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|tree_value| tree_value_executable(&tree_value));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

impl Template for TreeValue {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        write!(formatter, "{}", tree_value_object_id(self))
    }
}

fn tree_value_object_id(value: &TreeValue) -> String {
    match value {
        TreeValue::File { id, .. } => id.to_string(),
        TreeValue::Symlink(id) => id.to_string(),
        TreeValue::Tree(id) => id.to_string(),
        TreeValue::GitSubmodule(id) => id.to_string(),
    }
}

fn tree_value_file_type(value: &TreeValue) -> &'static str {
    match value {
        TreeValue::File { .. } => "file",
        TreeValue::Symlink(_) => "symlink",
        TreeValue::Tree(_) => "tree",
        TreeValue::GitSubmodule(_) => "git-submodule",
    }
}

fn tree_value_executable(value: &TreeValue) -> bool {
    match value {
        TreeValue::File { executable, .. } => *executable,
        TreeValue::Symlink(_) | TreeValue::Tree(_) | TreeValue::GitSubmodule(_) => false,
    }
}

fn describe_file_type(value: &MergedTreeValue) -> &'static str {
    match value.as_resolved() {
        Some(Some(tree_value)) => tree_value_file_type(tree_value),
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
pub struct DiffStatsFormatted {
    stats: DiffStats,
    path_converter: RepoPathUiConverter,
    width: usize,
    options: DiffStatOptions,
}

impl Template for DiffStatsFormatted {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        diff_util::show_diff_stats(
            formatter.as_mut(),
            &self.stats,
            &self.path_converter,
            self.width,
            &self.options,
        )
    }
}

fn register_diff_stats_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<DiffStatsFormatted>();
    methods.insert(
        "files",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|fm| Ok(fm.stats.entries().to_vec()));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "total_added",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|fm| Ok(i64::try_from(fm.stats.count_total_added())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "total_removed",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|fm| Ok(i64::try_from(fm.stats.count_total_removed())?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

fn register_diff_stat_entry_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<DiffStatEntry>();
    methods.insert(
        "path",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.path.target);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "display_diff_path",
        |language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let path_converter = language.path_converter;
            let out_property =
                self_property.map(move |entry| format_diff_path(&entry.path, path_converter));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "status",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status.label().to_owned());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "status_char",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|entry| entry.status.char().to_string());
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
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
    methods.insert(
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
    methods.insert(
        "bytes_delta",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|entry| Ok(i64::try_from(entry.bytes_delta)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

#[derive(Clone, Debug)]
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

fn register_cryptographic_signature_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<CryptographicSignature>();
    methods.insert(
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
    methods.insert(
        "key",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|sig| Ok(sig.key()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "display",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|sig| Ok(sig.display()?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

#[derive(Debug, Clone)]
pub struct AnnotationLine {
    pub commit: Commit,
    pub content: BString,
    pub line_number: usize,
    pub original_line_number: usize,
    pub first_line_in_hunk: bool,
}

fn register_annotation_line_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<AnnotationLine>();
    methods.insert(
        "commit",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.commit);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "content",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.content);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "line_number",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.and_then(|line| Ok(i64::try_from(line.line_number)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "original_line_number",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property =
                self_property.and_then(|line| Ok(i64::try_from(line.original_line_number)?));
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "first_line_in_hunk",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|line| line.first_line_in_hunk);
            Ok(out_property.into_dyn_wrapped())
        },
    );
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

fn register_trailer_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    let mut methods = table.register_methods::<Trailer>();
    methods.insert(
        "key",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|trailer| trailer.key);
            Ok(out_property.into_dyn_wrapped())
        },
    );
    methods.insert(
        "value",
        |_language, _diagnostics, _build_ctx, self_property, function| {
            function.expect_no_arguments()?;
            let out_property = self_property.map(|trailer| trailer.value);
            Ok(out_property.into_dyn_wrapped())
        },
    );
}

fn register_trailer_list_methods(table: &mut CommitTemplateSymbolTable<'_>) {
    register_formattable_list_methods::<_, Trailer>(table);
    let mut methods = table.register_methods::<Vec<Trailer>>();
    methods.insert(
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
}

#[cfg(test)]
mod tests {
    use std::path::Component;
    use std::path::PathBuf;

    use jiff::tz::TimeZone;
    use jj_lib::backend::CopyId;
    use jj_lib::backend::FileId;
    use jj_lib::backend::SymlinkId;
    use jj_lib::backend::TreeId;
    use jj_lib::config::ConfigLayer;
    use jj_lib::config::ConfigSource;
    use jj_lib::fileset::FilesetAliasesMap;
    use jj_lib::revset::RevsetAliasesMap;
    use jj_lib::revset::RevsetExpression;
    use jj_lib::revset::RevsetExtensions;
    use jj_lib::revset::RevsetWorkspaceContext;
    use jj_lib::template_parser::FunctionCallNode;
    use jj_lib::template_parser::TemplateAliasesMap;
    use testutils::TestRepoBackend;
    use testutils::TestWorkspace;
    use testutils::repo_path_buf;

    use super::*;
    use crate::template_builder::BuildContext;
    use crate::templater::TemplateRenderer;

    // TemplateBuildFunctionFn defined for<'a>
    type BuildFunctionFn = for<'a> fn(
        &CommitTemplateLanguage<'a>,
        &mut TemplateDiagnostics,
        &BuildContext<'a, '_>,
        &FunctionCallNode,
    ) -> TemplateParseResult<TemplatePropertyKind<'a>>;

    struct CommitTemplateTestEnv {
        test_workspace: TestWorkspace,
        path_converter: RepoPathUiConverter,
        revset_extensions: Arc<RevsetExtensions>,
        id_prefix_context: IdPrefixContext,
        fileset_aliases_map: FilesetAliasesMap,
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
                fileset_aliases_map: FilesetAliasesMap::new(),
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
                current_time: jiff::Timestamp::UNIX_EPOCH.to_zoned(TimeZone::UTC),
                default_ignored_remote: None,
                fileset_aliases_map: &self.fileset_aliases_map,
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
            let mut functions = language.symbol_table.register_functions();
            for (&name, &f) in &self.extra_functions {
                functions.insert(name, f);
            }
            language
        }

        fn parse<'a, C>(&'a self, text: &str) -> TemplateParseResult<TemplateRenderer<'a, C>>
        where
            C: Clone + TemplateValue,
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

        #[track_caller]
        fn render_ok<C>(&self, text: &str, context: &C) -> BString
        where
            C: Clone + TemplateValue,
        {
            let template = self.parse(text).unwrap();
            template.format_plain_text(context).into()
        }
    }

    fn stable_settings() -> UserSettings {
        let mut config = testutils::base_user_config();
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        layer
            .set_value("debug.commit-timestamp", "2001-02-03T04:05:06+07:00")
            .unwrap();
        config.add_layer(layer);
        testutils::user_settings_from_config(config)
    }

    #[test]
    fn test_tree_entry_normal_value() {
        let env = CommitTemplateTestEnv::init();
        let file = TreeValue::File {
            id: FileId::from_hex("0123456789abcdef"),
            executable: false,
            copy_id: CopyId::from_hex("00"),
        };
        let mut entry = TreeEntry {
            path: repo_path_buf("entry"),
            value: MergedTreeValue::normal(file.clone()),
        };
        let template =
            r#"if(self.normal_value(), "present", "absent") ++ ":" ++ self.normal_value()"#;
        insta::assert_snapshot!(env.render_ok(template, &entry), @"present:0123456789abcdef");

        entry.value = MergedTreeValue::normal(TreeValue::GitSubmodule(CommitId::from_hex(
            "0123456789abcdef0123456789abcdef01234567",
        )));
        insta::assert_snapshot!(env.render_ok(template, &entry), @"present:0123456789abcdef0123456789abcdef01234567");
        entry.value = MergedTreeValue::absent();
        insta::assert_snapshot!(env.render_ok(template, &entry), @"absent:");

        entry.value = MergedTreeValue::from_removes_adds(
            [None],
            [
                Some(file.clone()),
                Some(TreeValue::Symlink(SymlinkId::from_hex("abcdef"))),
            ],
        );
        insta::assert_snapshot!(env.render_ok(template, &entry), @"absent:");

        // Do not simplify an unresolved value, even if its terms cancel out.
        entry.value = MergedTreeValue::from_removes_adds([None], [Some(file), None]);
        insta::assert_snapshot!(env.render_ok(template, &entry), @"absent:");

        assert!(env.parse::<TreeEntry>("self.normal_value(1)").is_err());
    }

    #[test]
    fn test_tree_entry_removed_added_values() {
        let env = CommitTemplateTestEnv::init();
        let file = TreeValue::File {
            id: FileId::from_hex("0123456789abcdef"),
            executable: false,
            copy_id: CopyId::from_hex("00"),
        };
        let entry = TreeEntry {
            path: repo_path_buf("entry"),
            value: MergedTreeValue::from_removes_adds(
                [None],
                [
                    Some(file),
                    Some(TreeValue::Symlink(SymlinkId::from_hex("abcdef"))),
                ],
            ),
        };
        let format_values = |method: &str| {
            let template = format!(
                r#"self.{method}().map(|tv| if(tv, tv.file_type() ++ ":" ++ tv.object_id(), "<absent>")).join(", ")"#
            );
            env.render_ok(&template, &entry)
        };
        insta::assert_snapshot!(format_values("removed_values"), @"<absent>");
        insta::assert_snapshot!(
            format_values("added_values"),
            @"file:0123456789abcdef, symlink:abcdef"
        );
    }

    #[test]
    fn test_tree_value_tree() {
        let tree_hex = "0000000000000000000000000000000000000000";
        let tree_id = TreeId::from_hex(tree_hex);
        let tree_val = TreeValue::Tree(tree_id);

        assert_eq!(tree_value_file_type(&tree_val), "tree");
        assert_eq!(tree_value_object_id(&tree_val), tree_hex);
        assert!(!tree_value_executable(&tree_val));
    }

    #[test]
    fn test_tree_value_git_submodule() {
        let commit_hex = "1111111111111111111111111111111111111111";
        let commit_id = CommitId::from_hex(commit_hex);
        let sub_val = TreeValue::GitSubmodule(commit_id);

        assert_eq!(tree_value_file_type(&sub_val), "git-submodule");
        assert_eq!(tree_value_object_id(&sub_val), commit_hex);
        assert!(!tree_value_executable(&sub_val));
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

        // coerce to integer
        insta::assert_snapshot!(env.parse::<RefSymbolBuf>("fill(self, '')").err().unwrap(), @"
         --> 1:6
          |
        1 | fill(self, '')
          |      ^--^
          |
          = Expected expression of type `Integer`, but actual type is `RefSymbol`
        ");

        // unknown method
        insta::assert_snapshot!(env.parse::<RefSymbolBuf>("self.foo()").err().unwrap(), @"
         --> 1:6
          |
        1 | self.foo()
          |      ^-^
          |
          = Method `foo` doesn't exist for type `RefSymbol`
        ");
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

        insta::assert_snapshot!(env.render_ok("self.short()", &id), @"08a70ab33d71");
        insta::assert_snapshot!(env.render_ok("self.short(0)", &id), @"");
        insta::assert_snapshot!(env.render_ok("self.short(-0)", &id), @"");
        insta::assert_snapshot!(
            env.render_ok("self.short(100)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.short(65536)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.short(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        insta::assert_snapshot!(env.render_ok("self.shortest()", &id), @"08");
        insta::assert_snapshot!(env.render_ok("self.shortest(0)", &id), @"08");
        insta::assert_snapshot!(env.render_ok("self.shortest(-0)", &id), @"08");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(100)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(65536)", &id), @"08a70ab33d7143b7130ed8594d8216ef688623c0");
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
            env.render_ok("self.short(65536)", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
        insta::assert_snapshot!(
            env.render_ok("self.short(-100)", &id),
            @"<Error: out of range integral type conversion attempted>");

        insta::assert_snapshot!(env.render_ok("self.shortest()", &id), @"k");
        insta::assert_snapshot!(env.render_ok("self.shortest(0)", &id), @"k");
        insta::assert_snapshot!(env.render_ok("self.shortest(-0)", &id), @"k");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(100)", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
        insta::assert_snapshot!(
            env.render_ok("self.shortest(65536)", &id), @"kkmpptxzrspxrzommnulwmwkkqwworpl");
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
