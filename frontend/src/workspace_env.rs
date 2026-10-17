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
use jj_lib::fileset;
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::fileset::FilesetParseError;
use jj_lib::gitignore::GitIgnoreError;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::lock::FileLock;
use jj_lib::matchers::Matcher;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo;
#[cfg(feature = "git")]
use jj_lib::repo_path::RepoPath;
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
use jj_lib::settings::HumanByteSize;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::workspace::Workspace;
use thiserror::Error;
use tracing::instrument;

use crate::revset_util::ImmutableHeadsError;
use crate::revset_util::default_ignored_remote_name;
use crate::runner::GitImportExportLock;
use crate::runner::RunnerError;

/// Returns the closest ancestor of `cwd` (or `cwd` itself) that contains a
/// `.jj` directory, or `cwd` if there's no such ancestor.
pub fn find_workspace_dir(cwd: &Path) -> &Path {
    cwd.ancestors()
        .find(|path| path.join(".jj").is_dir())
        .unwrap_or(cwd)
}

/// Error from loading a [`WorkspaceEnvironment`].
#[derive(Debug, Error)]
pub enum WorkspaceEnvironmentError {
    /// A config value couldn't be read.
    #[error(transparent)]
    Config(#[from] ConfigGetError),
    /// The `immutable_heads()` revset alias isn't defined. It's defined by
    /// [`crate::config_util::default_config_layers()`].
    #[error("The `revset-aliases.immutable_heads()` alias is not defined")]
    ImmutableHeadsNotDefined,
    /// The configured `immutable_heads()` couldn't be parsed.
    #[error("Invalid `revset-aliases.immutable_heads()`")]
    InvalidImmutableHeads(#[source] RevsetParseError),
    /// The configured `revsets.short-prefixes` couldn't be parsed.
    #[error("Invalid `revsets.short-prefixes`")]
    InvalidShortPrefixes(#[source] RevsetParseError),
    /// A fileset couldn't be parsed.
    #[error(transparent)]
    FilesetParse(#[from] FilesetParseError),
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
        .map_err(|err| match err {
            ImmutableHeadsError::NotDefined => WorkspaceEnvironmentError::ImmutableHeadsNotDefined,
            ImmutableHeadsError::Parse(err) => {
                WorkspaceEnvironmentError::InvalidImmutableHeads(err)
            }
        })
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

    /// Loads ignore patterns that apply in addition to the working copy's
    /// `.gitignore` files: Git's global excludes file (`core.excludesFile`) and,
    /// if the repo is backed by Git, its `info/exclude` file. `home_dir` is used
    /// for expanding `~` in `core.excludesFile`.
    #[cfg(not(feature = "git"))]
    pub fn base_ignores(
        &self,
        _store: &Store,
        _home_dir: Option<&Path>,
    ) -> Result<Arc<GitIgnoreFile>, GitIgnoreError> {
        Ok(GitIgnoreFile::empty())
    }

    /// Loads ignore patterns that apply in addition to the working copy's
    /// `.gitignore` files: Git's global excludes file (`core.excludesFile`) and,
    /// if the repo is backed by Git, its `info/exclude` file. `home_dir` is used
    /// for expanding `~` in `core.excludesFile`.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    pub fn base_ignores(
        &self,
        store: &Store,
        home_dir: Option<&Path>,
    ) -> Result<Arc<GitIgnoreFile>, GitIgnoreError> {
        let get_excludes_file_path = |config: &gix::config::File| -> Option<PathBuf> {
            // TODO: maybe use path() and interpolate(), which can process non-utf-8
            // path on Unix.
            if let Some(value) = config.string("core.excludesFile") {
                let path = str::from_utf8(&value)
                    .ok()
                    .map(|value| jj_lib::file_util::expand_home_path(value, home_dir))?;
                // The configured path is usually absolute, but if it's relative,
                // the "git" command would read the file at the work-tree directory.
                Some(self.workspace_root().join(path))
            } else {
                xdg_config_home().map(|x| x.join("git").join("ignore"))
            }
        };

        fn xdg_config_home() -> Option<PathBuf> {
            if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
                && !x.is_empty()
            {
                return Some(PathBuf::from(x));
            }
            etcetera::home_dir().ok().map(|home| home.join(".config"))
        }

        let mut git_ignores = GitIgnoreFile::empty();
        if let Ok(git_backend) = jj_lib::git::get_git_backend(store) {
            let git_repo = git_backend.git_repo();
            if let Some(excludes_file_path) = get_excludes_file_path(&git_repo.config_snapshot()) {
                git_ignores = git_ignores.chain_with_file(RepoPath::root(), excludes_file_path)?;
            }
            git_ignores = git_ignores.chain_with_file(
                RepoPath::root(),
                git_backend.git_repo_path().join("info").join("exclude"),
            )?;
        } else if let Ok(git_config) = gix::config::File::from_globals()
            && let Some(excludes_file_path) = get_excludes_file_path(&git_config)
        {
            git_ignores = git_ignores.chain_with_file(RepoPath::root(), excludes_file_path)?;
        }
        Ok(git_ignores)
    }

    /// Size limit for new files to be tracked by snapshots, from
    /// `snapshot.max-new-file-size`.
    pub fn max_new_file_size(&self) -> Result<u64, ConfigGetError> {
        let HumanByteSize(max_new_file_size) = self
            .settings
            .get_value_with("snapshot.max-new-file-size", TryInto::try_into)?;
        if max_new_file_size == 0 {
            Ok(u64::MAX)
        } else {
            Ok(max_new_file_size)
        }
    }

    /// Matcher for new files to track automatically, from
    /// `snapshot.auto-track`.
    pub fn auto_tracking_matcher(
        &self,
        diagnostics: &mut FilesetDiagnostics,
    ) -> Result<Box<dyn Matcher>, WorkspaceEnvironmentError> {
        let pattern = self.settings.get_string("snapshot.auto-track")?;
        let context = self.fileset_parse_context_for_config();
        let expression = fileset::parse(diagnostics, &pattern, &context)?;
        Ok(expression.to_matcher())
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
