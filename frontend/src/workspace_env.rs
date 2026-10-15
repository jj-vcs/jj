// Copyright 2026 The Jujutsu Authors
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

//! Configuration for working with revsets and filesets in a workspace.

use std::collections::HashMap;
use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use jiff::Timestamp;
use jiff::Zoned;
use jiff::tz::TimeZone;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::lock::FileLock;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetAliasesMap;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetExtensions;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetWorkspaceContext;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::workspace::Workspace;
use thiserror::Error;
use tracing::instrument;

use crate::revset_util::default_ignored_remote_name;
use crate::runner::GitImportExportLock;
use crate::runner::RunnerError;

/// Error from loading a [`WorkspaceEnvironment`].
#[derive(Debug, Error)]
pub enum WorkspaceEnvironmentError {
    /// A config value couldn't be read.
    #[error(transparent)]
    Config(#[from] ConfigGetError),
    /// The configured `immutable_heads()` couldn't be parsed.
    #[error("Invalid `revset-aliases.immutable_heads()`")]
    InvalidImmutableHeads(#[source] RevsetParseError),
    /// The configured `revsets.short-prefixes` couldn't be parsed.
    #[error("Invalid `revsets.short-prefixes`")]
    InvalidShortPrefixes(#[source] RevsetParseError),
    /// The Git repo at the workspace root couldn't be opened.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitRepoAtWorkdir(#[from] jj_lib::git_backend::GitRepoAtWorkdirError),
}

/// Non-fatal problems found while loading a [`WorkspaceEnvironment`].
#[derive(Debug)]
pub struct WorkspaceEnvironmentDiagnostics {
    /// Diagnostics from parsing `revset-aliases.immutable_heads()`.
    pub immutable_heads: RevsetDiagnostics,
    /// Diagnostics from parsing `revsets.short-prefixes`.
    pub short_prefixes: RevsetDiagnostics,
}

impl WorkspaceEnvironmentDiagnostics {
    /// Creates empty diagnostics.
    pub fn new() -> Self {
        Self {
            immutable_heads: RevsetDiagnostics::new(),
            short_prefixes: RevsetDiagnostics::new(),
        }
    }
}

impl Default for WorkspaceEnvironmentDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata and configuration loaded for a specific workspace.
///
/// This contains what's needed to parse and evaluate revsets and filesets in
/// the workspace, independent of how the user interacts with it.
#[derive(Clone)]
pub struct WorkspaceEnvironment {
    settings: UserSettings,
    revset_extensions: Arc<RevsetExtensions>,
    fileset_aliases_map: FilesetAliasesMap,
    pub(crate) revset_aliases_map: RevsetAliasesMap,
    default_ignored_remote: Option<&'static RemoteName>,
    path_converter: RepoPathUiConverter,
    pub(crate) working_copy_shared_with_git: bool,
    workspace_name: WorkspaceNameBuf,
    immutable_heads_expression: Arc<UserRevsetExpression>,
    short_prefixes_expression: Option<Arc<UserRevsetExpression>>,
    conflict_marker_style: ConflictMarkerStyle,
}

impl WorkspaceEnvironment {
    /// Loads environment for the given `workspace`. Relative paths in revsets
    /// and filesets are resolved relative to `cwd`.
    #[instrument(skip_all)]
    pub fn new(
        diagnostics: &mut WorkspaceEnvironmentDiagnostics,
        workspace: &Workspace,
        cwd: &Path,
        revset_extensions: Arc<RevsetExtensions>,
        fileset_aliases_map: FilesetAliasesMap,
        revset_aliases_map: RevsetAliasesMap,
    ) -> Result<Self, WorkspaceEnvironmentError> {
        let settings = workspace.settings();
        let default_ignored_remote = default_ignored_remote_name(workspace.repo_loader().store());
        let path_converter = RepoPathUiConverter::Fs {
            cwd: cwd.to_owned(),
            base: workspace.workspace_root().to_owned(),
        };
        #[cfg(feature = "git")]
        let working_copy_shared_with_git = crate::git_util::is_colocated_git_workspace(workspace)?;
        #[cfg(not(feature = "git"))]
        let working_copy_shared_with_git = false;
        let mut env = Self {
            settings: settings.clone(),
            revset_extensions,
            fileset_aliases_map,
            revset_aliases_map,
            default_ignored_remote,
            path_converter,
            working_copy_shared_with_git,
            workspace_name: workspace.workspace_name().to_owned(),
            immutable_heads_expression: RevsetExpression::root(),
            short_prefixes_expression: None,
            conflict_marker_style: settings.get("ui.conflict-marker-style")?,
        };
        env.reload_revset_expressions(diagnostics)?;
        Ok(env)
    }

    /// Converts between paths relative to the cwd and repo paths.
    pub fn path_converter(&self) -> &RepoPathUiConverter {
        &self.path_converter
    }

    /// The directory that relative paths are resolved against.
    pub fn cwd(&self) -> &Path {
        let RepoPathUiConverter::Fs { cwd, base: _ } = &self.path_converter;
        cwd
    }

    /// The root directory of the workspace.
    pub fn workspace_root(&self) -> &Path {
        let RepoPathUiConverter::Fs { cwd: _, base } = &self.path_converter;
        base
    }

    /// The name of the workspace.
    pub fn workspace_name(&self) -> &WorkspaceName {
        &self.workspace_name
    }

    /// Revset aliases, including any temporary overrides such as the `trunk()`
    /// fallback.
    pub fn revset_aliases_map(&self) -> &RevsetAliasesMap {
        &self.revset_aliases_map
    }

    /// Whether the working copy is shared with a Git repository.
    pub fn working_copy_shared_with_git(&self) -> bool {
        self.working_copy_shared_with_git
    }

    /// Settings for this workspace.
    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    /// Acquires a lock for Git import/export operations if the workspace is
    /// supposed to be colocated.
    pub fn lock_git_import_export(
        &self,
        workspace: &Workspace,
    ) -> Result<GitImportExportLock, RunnerError> {
        let lock = if self.working_copy_shared_with_git {
            let lock_path = workspace.repo_path().join("git_import_export.lock");
            Some(FileLock::lock(lock_path).map_err(RunnerError::GitImportExportLock)?)
        } else {
            None
        };
        Ok(GitImportExportLock { _lock: lock })
    }

    /// Parsing context for fileset expressions specified by command arguments.
    pub fn fileset_parse_context(&self) -> FilesetParseContext<'_> {
        FilesetParseContext {
            aliases_map: &self.fileset_aliases_map,
            path_converter: &self.path_converter,
        }
    }

    /// Parsing context for fileset expressions loaded from config files.
    pub fn fileset_parse_context_for_config(&self) -> FilesetParseContext<'_> {
        static ROOT_PATH_CONVERTER: RepoPathUiConverter = RepoPathUiConverter::Fs {
            cwd: PathBuf::new(),
            base: PathBuf::new(),
        };
        FilesetParseContext {
            aliases_map: &self.fileset_aliases_map,
            path_converter: &ROOT_PATH_CONVERTER,
        }
    }

    /// Parsing context for revset expressions.
    pub fn revset_parse_context(&self) -> RevsetParseContext<'_> {
        let workspace_context = RevsetWorkspaceContext {
            path_converter: &self.path_converter,
            workspace_name: &self.workspace_name,
        };
        let now = if let Some(timestamp) = self.settings.commit_timestamp() {
            Timestamp::from_millisecond(timestamp.timestamp.0)
                .unwrap()
                .to_zoned(TimeZone::system())
        } else {
            Zoned::now()
        };
        RevsetParseContext {
            aliases_map: &self.revset_aliases_map,
            local_variables: HashMap::new(),
            user_email: self.settings.user_email(),
            current_time: now,
            default_ignored_remote: self.default_ignored_remote,
            fileset_aliases_map: &self.fileset_aliases_map,
            extensions: &self.revset_extensions,
            workspace: Some(workspace_context),
        }
    }

    /// Creates fresh new context which manages cache of short commit/change ID
    /// prefixes. New context should be created per repo view (or operation.)
    pub fn new_id_prefix_context(&self) -> IdPrefixContext {
        let context = IdPrefixContext::new(self.revset_extensions.clone());
        match &self.short_prefixes_expression {
            None => context,
            Some(expression) => context.disambiguate_within(expression.clone()),
        }
    }

    /// Updates parsed revset expressions.
    pub(crate) fn reload_revset_expressions(
        &mut self,
        diagnostics: &mut WorkspaceEnvironmentDiagnostics,
    ) -> Result<(), WorkspaceEnvironmentError> {
        self.immutable_heads_expression =
            self.load_immutable_heads_expression(&mut diagnostics.immutable_heads)?;
        self.short_prefixes_expression =
            self.load_short_prefixes_expression(&mut diagnostics.short_prefixes)?;
        Ok(())
    }

    /// User-configured expression defining the immutable set.
    pub fn immutable_expression(&self) -> Arc<UserRevsetExpression> {
        // Negated ancestors expression `~::(<heads> | root())` is slightly
        // easier to optimize than negated union `~(::<heads> | root())`.
        self.immutable_heads_expression.ancestors()
    }

    /// User-configured expression defining the heads of the immutable set.
    pub fn immutable_heads_expression(&self) -> &Arc<UserRevsetExpression> {
        &self.immutable_heads_expression
    }

    /// User-configured conflict marker style for materializing conflicts
    pub fn conflict_marker_style(&self) -> ConflictMarkerStyle {
        self.conflict_marker_style
    }

    fn load_immutable_heads_expression(
        &self,
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<Arc<UserRevsetExpression>, WorkspaceEnvironmentError> {
        crate::revset_util::parse_immutable_heads_expression(
            diagnostics,
            &self.revset_parse_context(),
        )
        .map_err(WorkspaceEnvironmentError::InvalidImmutableHeads)
    }

    fn load_short_prefixes_expression(
        &self,
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<Option<Arc<UserRevsetExpression>>, WorkspaceEnvironmentError> {
        let revset_string = self
            .settings
            .get_string("revsets.short-prefixes")
            .optional()?
            .map_or_else(|| self.settings.get_string("revsets.log"), Ok)?;
        if revset_string.is_empty() {
            Ok(None)
        } else {
            let expression =
                revset::parse(diagnostics, &revset_string, &self.revset_parse_context())
                    .map_err(WorkspaceEnvironmentError::InvalidShortPrefixes)?;
            Ok(Some(expression))
        }
    }

    /// Resolves the effective `immutable()` expression to test against commits
    /// during a rewrite. If `ignore_immutable` is true, only the root commit is
    /// considered immutable.
    pub fn resolve_immutable_expression(
        &self,
        repo: &dyn Repo,
        ignore_immutable: bool,
    ) -> Result<Arc<ResolvedRevsetExpression>, RunnerError> {
        let immutable_expression = if ignore_immutable {
            UserRevsetExpression::root()
        } else {
            self.immutable_expression()
        };

        // Not using self.id_prefix_context() because the disambiguation data
        // must not be calculated and cached against arbitrary repo. It's also
        // unlikely that the immutable expression contains short hashes.
        let id_prefix_context = IdPrefixContext::new(self.revset_extensions.clone());
        let symbol_resolver = SymbolResolver::new(repo, self.revset_extensions.symbol_resolvers())
            .with_id_prefix_context(&id_prefix_context);
        let (expression, _other_repos) = immutable_expression
            .resolve_user_expression(repo, &symbol_resolver)
            .map_err(RunnerError::InvalidImmutableHeads)?
            .into_inner();
        Ok(expression)
    }
}
