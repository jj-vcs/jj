// Copyright 2022 The Jujutsu Authors
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

use std::collections::{HashSet, VecDeque};
use std::env::{self, ArgsOs, VarError};
use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::iter;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use clap::builder::{NonEmptyStringValueParser, TypedValueParser, ValueParserFactory};
use clap::{Arg, ArgAction, ArgMatches, Command, FromArgMatches};
use git2::{Oid, Repository};
use indexmap::IndexSet;
use itertools::Itertools;
use jujutsu_lib::backend::{BackendError, ChangeId, CommitId, ObjectId, TreeId};
use jujutsu_lib::commit::Commit;
use jujutsu_lib::git::{GitExportError, GitImportError};
use jujutsu_lib::gitignore::GitIgnoreFile;
use jujutsu_lib::hex_util::to_reverse_hex;
use jujutsu_lib::matchers::{EverythingMatcher, Matcher, PrefixMatcher, Visit};
use jujutsu_lib::op_heads_store::{self, OpHeadResolutionError, OpHeadsStore};
use jujutsu_lib::op_store::{OpStore, OpStoreError, OperationId, RefTarget, WorkspaceId};
use jujutsu_lib::operation::Operation;
use jujutsu_lib::repo::{
    CheckOutCommitError, EditCommitError, MutableRepo, ReadonlyRepo, Repo, RepoLoader, RepoRef,
    RewriteRootCommit, StoreFactories,
};
use jujutsu_lib::repo_path::{FsPathParseError, RepoPath};
use jujutsu_lib::revset::{
    Revset, RevsetAliasesMap, RevsetError, RevsetExpression, RevsetParseError,
    RevsetWorkspaceContext,
};
use jujutsu_lib::settings::UserSettings;
use jujutsu_lib::transaction::Transaction;
use jujutsu_lib::tree::{Tree, TreeMergeError};
use jujutsu_lib::working_copy::{
    CheckoutStats, LockedWorkingCopy, ResetError, SnapshotError, WorkingCopy,
};
use jujutsu_lib::workspace::{Workspace, WorkspaceInitError, WorkspaceLoadError, WorkspaceLoader};
use jujutsu_lib::{dag_walk, file_util, git, revset};
use thiserror::Error;
use tracing_subscriber::prelude::*;

use crate::config::{AnnotatedValue, CommandNameAndArgs, LayeredConfigs};
use crate::formatter::{Formatter, PlainTextFormatter};
use crate::merge_tools::{ConflictResolveError, DiffEditError};
use crate::template_parser::{self, TemplateAliasesMap, TemplateParseError};
use crate::templater::Template;
use crate::ui::{ColorChoice, Ui};

#[derive(Clone, Debug)]
pub enum CommandError {
    UserError {
        message: String,
        hint: Option<String>,
    },
    ConfigError(String),
    /// Invalid command line
    CliError(String),
    /// Invalid command line detected by clap
    ClapCliError(Arc<clap::Error>),
    BrokenPipe,
    InternalError(String),
}

pub fn user_error(message: impl Into<String>) -> CommandError {
    CommandError::UserError {
        message: message.into(),
        hint: None,
    }
}
pub fn user_error_with_hint(message: impl Into<String>, hint: impl Into<String>) -> CommandError {
    CommandError::UserError {
        message: message.into(),
        hint: Some(hint.into()),
    }
}

impl From<std::io::Error> for CommandError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            CommandError::BrokenPipe
        } else {
            // TODO: Record the error as a chained cause
            CommandError::InternalError(format!("I/O error: {err}"))
        }
    }
}

impl From<config::ConfigError> for CommandError {
    fn from(err: config::ConfigError) -> Self {
        CommandError::ConfigError(err.to_string())
    }
}

impl From<crate::config::ConfigError> for CommandError {
    fn from(err: crate::config::ConfigError) -> Self {
        CommandError::ConfigError(err.to_string())
    }
}

impl From<RewriteRootCommit> for CommandError {
    fn from(err: RewriteRootCommit) -> Self {
        CommandError::InternalError(format!("Attempted to rewrite the root commit: {err}"))
    }
}

impl From<EditCommitError> for CommandError {
    fn from(err: EditCommitError) -> Self {
        CommandError::InternalError(format!("Failed to edit a commit: {err}"))
    }
}

impl From<CheckOutCommitError> for CommandError {
    fn from(err: CheckOutCommitError) -> Self {
        CommandError::InternalError(format!("Failed to check out a commit: {err}"))
    }
}

impl From<BackendError> for CommandError {
    fn from(err: BackendError) -> Self {
        user_error(format!("Unexpected error from backend: {err}"))
    }
}

impl From<WorkspaceInitError> for CommandError {
    fn from(err: WorkspaceInitError) -> Self {
        match err {
            WorkspaceInitError::DestinationExists(_) => {
                user_error("The target repo already exists")
            }
            WorkspaceInitError::NonUnicodePath => {
                user_error("The target repo path contains non-unicode characters")
            }
            WorkspaceInitError::CheckOutCommit(err) => CommandError::InternalError(format!(
                "Failed to check out the initial commit: {err}"
            )),
            WorkspaceInitError::Path(err) => {
                CommandError::InternalError(format!("Failed to access the repository: {err}"))
            }
        }
    }
}

impl From<OpHeadResolutionError<CommandError>> for CommandError {
    fn from(err: OpHeadResolutionError<CommandError>) -> Self {
        match err {
            OpHeadResolutionError::NoHeads => CommandError::InternalError(
                "Corrupt repository: there are no operations".to_string(),
            ),
            OpHeadResolutionError::Err(e) => e,
        }
    }
}

impl From<SnapshotError> for CommandError {
    fn from(err: SnapshotError) -> Self {
        CommandError::InternalError(format!("Failed to snapshot the working copy: {err}"))
    }
}

impl From<TreeMergeError> for CommandError {
    fn from(err: TreeMergeError) -> Self {
        CommandError::InternalError(format!("Merge failed: {err}"))
    }
}

impl From<ResetError> for CommandError {
    fn from(_: ResetError) -> Self {
        CommandError::InternalError("Failed to reset the working copy".to_string())
    }
}

impl From<DiffEditError> for CommandError {
    fn from(err: DiffEditError) -> Self {
        user_error(format!("Failed to edit diff: {err}"))
    }
}

impl From<ConflictResolveError> for CommandError {
    fn from(err: ConflictResolveError) -> Self {
        user_error(format!("Failed to use external tool to resolve: {err}"))
    }
}

impl From<git2::Error> for CommandError {
    fn from(err: git2::Error) -> Self {
        user_error(format!("Git operation failed: {err}"))
    }
}

impl From<GitImportError> for CommandError {
    fn from(err: GitImportError) -> Self {
        CommandError::InternalError(format!(
            "Failed to import refs from underlying Git repo: {err}"
        ))
    }
}

impl From<GitExportError> for CommandError {
    fn from(err: GitExportError) -> Self {
        CommandError::InternalError(format!(
            "Failed to export refs to underlying Git repo: {err}"
        ))
    }
}

impl From<RevsetParseError> for CommandError {
    fn from(err: RevsetParseError) -> Self {
        let message = iter::successors(Some(&err), |e| e.origin()).join("\n");
        user_error(format!("Failed to parse revset: {message}"))
    }
}

impl From<RevsetError> for CommandError {
    fn from(err: RevsetError) -> Self {
        user_error(format!("{err}"))
    }
}

impl From<TemplateParseError> for CommandError {
    fn from(err: TemplateParseError) -> Self {
        let message = iter::successors(Some(&err), |e| e.origin()).join("\n");
        user_error(format!("Failed to parse template: {message}"))
    }
}

impl From<FsPathParseError> for CommandError {
    fn from(err: FsPathParseError) -> Self {
        user_error(format!("{err}"))
    }
}

impl From<glob::PatternError> for CommandError {
    fn from(err: glob::PatternError) -> Self {
        user_error(format!("Failed to compile glob: {err}"))
    }
}

impl From<clap::Error> for CommandError {
    fn from(err: clap::Error) -> Self {
        CommandError::ClapCliError(Arc::new(err))
    }
}

/// Handle to initialize or change tracing subscription.
#[derive(Clone, Debug)]
pub struct TracingSubscription {
    reload_log_filter: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
}

impl TracingSubscription {
    /// Initializes tracing with the default configuration. This should be
    /// called as early as possible.
    pub fn init() -> Self {
        let filter = tracing_subscriber::EnvFilter::builder()
            .with_default_directive(tracing::metadata::LevelFilter::INFO.into())
            .from_env_lossy();
        let (filter, reload_log_filter) = tracing_subscriber::reload::Layer::new(filter);
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::Layer::default().with_writer(std::io::stderr))
            .init();
        TracingSubscription { reload_log_filter }
    }

    pub fn enable_verbose_logging(&self) -> Result<(), CommandError> {
        self.reload_log_filter
            .modify(|filter| {
                *filter = tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(tracing::metadata::LevelFilter::DEBUG.into())
                    .from_env_lossy()
            })
            .map_err(|err| {
                CommandError::InternalError(format!("failed to enable verbose logging: {err:?}"))
            })?;
        tracing::debug!("verbose logging enabled");
        Ok(())
    }
}

pub struct CommandHelper {
    app: Command,
    cwd: PathBuf,
    string_args: Vec<String>,
    global_args: GlobalArgs,
    settings: UserSettings,
    layered_configs: LayeredConfigs,
    maybe_workspace_loader: Result<WorkspaceLoader, CommandError>,
    store_factories: StoreFactories,
}

impl CommandHelper {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: Command,
        cwd: PathBuf,
        string_args: Vec<String>,
        global_args: GlobalArgs,
        settings: UserSettings,
        layered_configs: LayeredConfigs,
        maybe_workspace_loader: Result<WorkspaceLoader, CommandError>,
        store_factories: StoreFactories,
    ) -> Self {
        Self {
            app,
            cwd,
            string_args,
            global_args,
            settings,
            layered_configs,
            maybe_workspace_loader,
            store_factories,
        }
    }

    pub fn app(&self) -> &Command {
        &self.app
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn string_args(&self) -> &Vec<String> {
        &self.string_args
    }

    pub fn global_args(&self) -> &GlobalArgs {
        &self.global_args
    }

    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn resolved_config_values(
        &self,
        prefix: &[&str],
    ) -> Result<Vec<AnnotatedValue>, crate::config::ConfigError> {
        self.layered_configs.resolved_config_values(prefix)
    }

    fn workspace_helper_internal(
        &self,
        ui: &mut Ui,
        snapshot: bool,
    ) -> Result<WorkspaceCommandHelper, CommandError> {
        let workspace = self.load_workspace()?;
        let op_head = self.resolve_operation(ui, workspace.repo_loader())?;
        let repo = workspace.repo_loader().load_at(&op_head);
        let mut workspace_command = self.for_loaded_repo(ui, workspace, repo)?;
        if snapshot {
            workspace_command.snapshot(ui)?;
        }
        Ok(workspace_command)
    }

    pub fn workspace_helper(&self, ui: &mut Ui) -> Result<WorkspaceCommandHelper, CommandError> {
        self.workspace_helper_internal(ui, true)
    }

    pub fn workspace_helper_no_snapshot(
        &self,
        ui: &mut Ui,
    ) -> Result<WorkspaceCommandHelper, CommandError> {
        self.workspace_helper_internal(ui, false)
    }

    pub fn load_workspace(&self) -> Result<Workspace, CommandError> {
        let loader = self.maybe_workspace_loader.as_ref().map_err(Clone::clone)?;
        loader
            .load(&self.settings, &self.store_factories)
            .map_err(|e| user_error(format!("{}: {}", e, e.error)))
    }

    pub fn resolve_operation(
        &self,
        ui: &mut Ui,
        repo_loader: &RepoLoader,
    ) -> Result<Operation, OpHeadResolutionError<CommandError>> {
        if self.global_args.at_operation == "@" {
            op_heads_store::resolve_op_heads(
                repo_loader.op_heads_store().as_ref(),
                repo_loader.op_store(),
                |op_heads| {
                    writeln!(
                        ui,
                        "Concurrent modification detected, resolving automatically.",
                    )?;
                    let base_repo = repo_loader.load_at(&op_heads[0]);
                    // TODO: It may be helpful to print each operation we're merging here
                    let mut tx = start_repo_transaction(
                        &base_repo,
                        &self.settings,
                        &self.string_args,
                        "resolve concurrent operations",
                    );
                    for other_op_head in op_heads.into_iter().skip(1) {
                        tx.merge_operation(other_op_head);
                        let num_rebased = tx.mut_repo().rebase_descendants(&self.settings)?;
                        if num_rebased > 0 {
                            writeln!(
                                ui,
                                "Rebased {num_rebased} descendant commits onto commits rewritten \
                                 by other operation"
                            )?;
                        }
                    }
                    Ok(tx.write().leave_unpublished().operation().clone())
                },
            )
        } else {
            resolve_op_for_load(
                repo_loader.op_store(),
                repo_loader.op_heads_store(),
                &self.global_args.at_operation,
            )
        }
    }

    pub fn for_loaded_repo(
        &self,
        ui: &mut Ui,
        workspace: Workspace,
        repo: Arc<ReadonlyRepo>,
    ) -> Result<WorkspaceCommandHelper, CommandError> {
        WorkspaceCommandHelper::new(
            ui,
            workspace,
            self.cwd.clone(),
            self.string_args.clone(),
            &self.global_args,
            self.settings.clone(),
            repo,
        )
    }
}

// Provides utilities for writing a command that works on a workspace (like most
// commands do).
pub struct WorkspaceCommandHelper {
    cwd: PathBuf,
    string_args: Vec<String>,
    global_args: GlobalArgs,
    settings: UserSettings,
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    revset_aliases_map: RevsetAliasesMap,
    template_aliases_map: TemplateAliasesMap,
    may_update_working_copy: bool,
    working_copy_shared_with_git: bool,
}

impl WorkspaceCommandHelper {
    pub fn new(
        ui: &mut Ui,
        workspace: Workspace,
        cwd: PathBuf,
        string_args: Vec<String>,
        global_args: &GlobalArgs,
        settings: UserSettings,
        repo: Arc<ReadonlyRepo>,
    ) -> Result<Self, CommandError> {
        let revset_aliases_map = load_revset_aliases(ui, &settings)?;
        let template_aliases_map = load_template_aliases(ui, &settings)?;
        // Parse commit_summary template early to report error before starting mutable
        // operation.
        // TODO: Parsed template can be cached if it doesn't capture repo
        parse_commit_summary_template(
            repo.as_repo_ref(),
            workspace.workspace_id(),
            &template_aliases_map,
            &settings,
        )?;
        let loaded_at_head = &global_args.at_operation == "@";
        let may_update_working_copy = loaded_at_head && !global_args.ignore_working_copy;
        let mut working_copy_shared_with_git = false;
        let maybe_git_repo = repo.store().git_repo();
        if let Some(git_workdir) = maybe_git_repo
            .as_ref()
            .and_then(|git_repo| git_repo.workdir())
            .and_then(|workdir| workdir.canonicalize().ok())
        {
            working_copy_shared_with_git = git_workdir == workspace.workspace_root().as_path();
        }
        Ok(Self {
            cwd,
            string_args,
            global_args: global_args.clone(),
            settings,
            workspace,
            repo,
            revset_aliases_map,
            template_aliases_map,
            may_update_working_copy,
            working_copy_shared_with_git,
        })
    }

    pub fn check_working_copy_writable(&self) -> Result<(), CommandError> {
        if self.may_update_working_copy {
            Ok(())
        } else {
            let hint = if self.global_args.ignore_working_copy {
                "Don't use --ignore-working-copy."
            } else {
                "Don't use --at-op."
            };
            Err(user_error_with_hint(
                "This command must be able to update the working copy.",
                hint,
            ))
        }
    }

    /// Snapshot the working copy if allowed, and import Git refs if the working
    /// copy is collocated with Git.
    pub fn snapshot(&mut self, ui: &mut Ui) -> Result<(), CommandError> {
        if self.may_update_working_copy {
            if self.working_copy_shared_with_git {
                let maybe_git_repo = self.repo.store().git_repo();
                self.import_git_refs_and_head(ui, maybe_git_repo.as_ref().unwrap())?;
            }
            self.snapshot_working_copy(ui)?;
        }
        Ok(())
    }

    fn import_git_refs_and_head(
        &mut self,
        ui: &mut Ui,
        git_repo: &Repository,
    ) -> Result<(), CommandError> {
        let mut tx = self.start_transaction("import git refs").into_inner();
        git::import_refs(tx.mut_repo(), git_repo, &self.settings.git_settings())?;
        if tx.mut_repo().has_changes() {
            let old_git_head = self.repo.view().git_head().cloned();
            let new_git_head = tx.mut_repo().view().git_head().cloned();
            // If the Git HEAD has changed, abandon our old checkout and check out the new
            // Git HEAD.
            match new_git_head {
                Some(RefTarget::Normal(new_git_head_id)) if new_git_head != old_git_head => {
                    let workspace_id = self.workspace_id().to_owned();
                    let mut locked_working_copy =
                        self.workspace.working_copy_mut().start_mutation();
                    if let Some(old_wc_commit_id) = self.repo.view().get_wc_commit_id(&workspace_id)
                    {
                        tx.mut_repo()
                            .record_abandoned_commit(old_wc_commit_id.clone());
                    }
                    let new_git_head_commit = tx.mut_repo().store().get_commit(&new_git_head_id)?;
                    tx.mut_repo()
                        .check_out(workspace_id, &self.settings, &new_git_head_commit)?;
                    // The working copy was presumably updated by the git command that updated
                    // HEAD, so we just need to reset our working copy
                    // state to it without updating working copy files.
                    locked_working_copy.reset(&new_git_head_commit.tree())?;
                    tx.mut_repo().rebase_descendants(&self.settings)?;
                    self.repo = tx.commit();
                    locked_working_copy.finish(self.repo.op_id().clone());
                }
                _ => {
                    let num_rebased = tx.mut_repo().rebase_descendants(&self.settings)?;
                    if num_rebased > 0 {
                        writeln!(
                            ui,
                            "Rebased {num_rebased} descendant commits off of commits rewritten \
                             from git"
                        )?;
                    }
                    self.finish_transaction(ui, tx)?;
                }
            }
        }
        Ok(())
    }

    fn export_head_to_git(&self, mut_repo: &mut MutableRepo) -> Result<(), CommandError> {
        let git_repo = mut_repo.store().git_repo().unwrap();
        let current_git_head_ref = git_repo.find_reference("HEAD").unwrap();
        let current_git_commit_id = current_git_head_ref
            .peel_to_commit()
            .ok()
            .map(|commit| commit.id());
        if let Some(wc_commit_id) = mut_repo.view().get_wc_commit_id(self.workspace_id()) {
            let first_parent_id = mut_repo
                .index()
                .entry_by_id(wc_commit_id)
                .unwrap()
                .parents()[0]
                .commit_id();
            if first_parent_id != *mut_repo.store().root_commit_id() {
                if let Some(current_git_commit_id) = current_git_commit_id {
                    git_repo.set_head_detached(current_git_commit_id)?;
                }
                let new_git_commit_id = Oid::from_bytes(first_parent_id.as_bytes()).unwrap();
                let new_git_commit = git_repo.find_commit(new_git_commit_id)?;
                git_repo.reset(new_git_commit.as_object(), git2::ResetType::Mixed, None)?;
                mut_repo.set_git_head(RefTarget::Normal(first_parent_id));
            }
        } else {
            // The workspace was removed (maybe the user undid the
            // initialization of the workspace?), which is weird,
            // but we should probably just not do anything else here.
            // Except maybe print a note about it?
        }
        Ok(())
    }

    pub fn repo(&self) -> &Arc<ReadonlyRepo> {
        &self.repo
    }

    pub fn working_copy(&self) -> &WorkingCopy {
        self.workspace.working_copy()
    }

    pub fn unsafe_start_working_copy_mutation(
        &mut self,
    ) -> Result<(LockedWorkingCopy, Commit), CommandError> {
        self.check_working_copy_writable()?;
        let wc_commit_id = self.repo.view().get_wc_commit_id(self.workspace_id());
        let wc_commit = if let Some(wc_commit_id) = wc_commit_id {
            self.repo.store().get_commit(wc_commit_id)?
        } else {
            return Err(user_error("Nothing checked out in this workspace"));
        };

        let locked_working_copy = self.workspace.working_copy_mut().start_mutation();

        Ok((locked_working_copy, wc_commit))
    }

    pub fn start_working_copy_mutation(
        &mut self,
    ) -> Result<(LockedWorkingCopy, Commit), CommandError> {
        let (locked_working_copy, wc_commit) = self.unsafe_start_working_copy_mutation()?;
        if wc_commit.tree_id() != locked_working_copy.old_tree_id() {
            return Err(user_error("Concurrent working copy operation. Try again."));
        }
        Ok((locked_working_copy, wc_commit))
    }

    pub fn workspace_root(&self) -> &PathBuf {
        self.workspace.workspace_root()
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        self.workspace.workspace_id()
    }

    pub fn working_copy_shared_with_git(&self) -> bool {
        self.working_copy_shared_with_git
    }

    pub fn format_file_path(&self, file: &RepoPath) -> String {
        file_util::relative_path(&self.cwd, &file.to_fs_path(self.workspace_root()))
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Parses a path relative to cwd into a RepoPath, which is relative to the
    /// workspace root.
    pub fn parse_file_path(&self, input: &str) -> Result<RepoPath, FsPathParseError> {
        RepoPath::parse_fs_path(&self.cwd, self.workspace_root(), input)
    }

    pub fn matcher_from_values(&self, values: &[String]) -> Result<Box<dyn Matcher>, CommandError> {
        if values.is_empty() {
            Ok(Box::new(EverythingMatcher))
        } else {
            // TODO: Add support for globs and other formats
            let paths: Vec<_> = values
                .iter()
                .map(|v| self.parse_file_path(v))
                .try_collect()?;
            Ok(Box::new(PrefixMatcher::new(&paths)))
        }
    }

    pub fn git_config(&self) -> Result<git2::Config, git2::Error> {
        if let Some(git_repo) = self.repo.store().git_repo() {
            git_repo.config()
        } else {
            git2::Config::open_default()
        }
    }

    pub fn base_ignores(&self) -> Arc<GitIgnoreFile> {
        fn xdg_config_home() -> Result<PathBuf, VarError> {
            if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
                if !x.is_empty() {
                    return Ok(PathBuf::from(x));
                }
            }
            std::env::var("HOME").map(|x| Path::new(&x).join(".config"))
        }

        let mut git_ignores = GitIgnoreFile::empty();
        if let Ok(excludes_file_path) = self
            .git_config()
            .and_then(|git_config| {
                git_config
                    .get_string("core.excludesFile")
                    .map(expand_git_path)
            })
            .or_else(|_| xdg_config_home().map(|x| x.join("git").join("ignore")))
        {
            git_ignores = git_ignores.chain_with_file("", excludes_file_path);
        }
        if let Some(git_repo) = self.repo.store().git_repo() {
            git_ignores =
                git_ignores.chain_with_file("", git_repo.path().join("info").join("exclude"));
        }
        git_ignores
    }

    pub fn resolve_single_op(&self, op_str: &str) -> Result<Operation, CommandError> {
        // When resolving the "@" operation in a `ReadonlyRepo`, we resolve it to the
        // operation the repo was loaded at.
        resolve_single_op(
            self.repo.op_store(),
            self.repo.op_heads_store(),
            || Ok(self.repo.operation().clone()),
            op_str,
        )
    }

    pub fn resolve_single_rev(&self, revision_str: &str) -> Result<Commit, CommandError> {
        let revset_expression = self.parse_revset(revision_str)?;
        let revset = self.evaluate_revset(&revset_expression)?;
        let mut iter = revset.iter().commits(self.repo.store()).fuse();
        match (iter.next(), iter.next()) {
            (Some(commit), None) => Ok(commit?),
            (None, _) => Err(user_error(format!(
                r#"Revset "{revision_str}" didn't resolve to any revisions"#
            ))),
            (Some(commit0), Some(commit1)) => {
                let mut iter = [commit0, commit1].into_iter().chain(iter);
                let commits: Vec<_> = iter.by_ref().take(5).try_collect()?;
                let elided = iter.next().is_some();
                let hint = format!(
                    r#"The revset "{revision_str}" resolved to these revisions:{eol}{commits}{ellipsis}"#,
                    eol = "\n",
                    commits = commits
                        .iter()
                        .map(|c| self.format_commit_summary(c))
                        .join("\n"),
                    ellipsis = elided.then(|| "\n...").unwrap_or_default()
                );
                Err(user_error_with_hint(
                    format!(r#"Revset "{revision_str}" resolved to more than one revision"#),
                    hint,
                ))
            }
        }
    }

    pub fn resolve_revset(&self, revision_str: &str) -> Result<Vec<Commit>, CommandError> {
        let revset_expression = self.parse_revset(revision_str)?;
        let revset = self.evaluate_revset(&revset_expression)?;
        Ok(revset.iter().commits(self.repo.store()).try_collect()?)
    }

    pub fn parse_revset(
        &self,
        revision_str: &str,
    ) -> Result<Rc<RevsetExpression>, RevsetParseError> {
        let expression = revset::parse(
            revision_str,
            &self.revset_aliases_map,
            Some(&self.revset_context()),
        )?;
        Ok(revset::optimize(expression))
    }

    pub fn evaluate_revset<'repo>(
        &'repo self,
        revset_expression: &RevsetExpression,
    ) -> Result<Box<dyn Revset<'repo> + 'repo>, RevsetError> {
        revset_expression.evaluate(self.repo.as_repo_ref(), Some(&self.revset_context()))
    }

    fn revset_context(&self) -> RevsetWorkspaceContext {
        RevsetWorkspaceContext {
            cwd: &self.cwd,
            workspace_id: self.workspace_id(),
            workspace_root: self.workspace.workspace_root(),
        }
    }

    pub fn parse_commit_template(
        &self,
        template_text: &str,
    ) -> Result<Box<dyn Template<Commit> + '_>, TemplateParseError> {
        template_parser::parse_commit_template(
            self.repo.as_repo_ref(),
            self.workspace_id(),
            template_text,
            &self.template_aliases_map,
        )
    }

    /// Returns one-line summary of the given `commit`.
    pub fn format_commit_summary(&self, commit: &Commit) -> String {
        let mut output = Vec::new();
        self.write_commit_summary(&mut PlainTextFormatter::new(&mut output), commit)
            .expect("write() to PlainTextFormatter should never fail");
        String::from_utf8(output).expect("template output should be utf-8 bytes")
    }

    /// Writes one-line summary of the given `commit`.
    pub fn write_commit_summary(
        &self,
        formatter: &mut dyn Formatter,
        commit: &Commit,
    ) -> std::io::Result<()> {
        let template = parse_commit_summary_template(
            self.repo.as_repo_ref(),
            self.workspace_id(),
            &self.template_aliases_map,
            &self.settings,
        )
        .expect("parse error should be confined by WorkspaceCommandHelper::new()");
        template.format(commit, formatter)
    }

    pub fn check_rewritable(&self, commit: &Commit) -> Result<(), CommandError> {
        if commit.id() == self.repo.store().root_commit_id() {
            return Err(user_error("Cannot rewrite the root commit"));
        }
        Ok(())
    }

    pub fn check_non_empty(&self, commits: &[Commit]) -> Result<(), CommandError> {
        if commits.is_empty() {
            return Err(user_error("Empty revision set"));
        }
        Ok(())
    }

    pub fn snapshot_working_copy(&mut self, ui: &mut Ui) -> Result<(), CommandError> {
        let repo = self.repo.clone();
        let workspace_id = self.workspace_id().to_owned();
        let wc_commit_id = match repo.view().get_wc_commit_id(&workspace_id) {
            Some(wc_commit_id) => wc_commit_id.clone(),
            None => {
                // If the workspace has been deleted, it's unclear what to do, so we just skip
                // committing the working copy.
                return Ok(());
            }
        };
        let base_ignores = self.base_ignores();
        let mut locked_wc = self.workspace.working_copy_mut().start_mutation();
        let old_op_id = locked_wc.old_operation_id().clone();
        let wc_commit = repo.store().get_commit(&wc_commit_id)?;
        self.repo = match check_stale_working_copy(&locked_wc, &wc_commit, repo.clone()) {
            Ok(repo) => repo,
            Err(StaleWorkingCopyError::WorkingCopyStale) => {
                locked_wc.discard();
                return Err(user_error_with_hint(
                    format!(
                        "The working copy is stale (not updated since operation {}).",
                        short_operation_hash(&old_op_id)
                    ),
                    "Run `jj workspace update-stale` to update it.",
                ));
            }
            Err(StaleWorkingCopyError::SiblingOperation) => {
                locked_wc.discard();
                return Err(CommandError::InternalError(format!(
                    "The repo was loaded at operation {}, which seems to be a sibling of the \
                     working copy's operation {}",
                    short_operation_hash(repo.op_id()),
                    short_operation_hash(&old_op_id)
                )));
            }
            Err(StaleWorkingCopyError::UnrelatedOperation) => {
                locked_wc.discard();
                return Err(CommandError::InternalError(format!(
                    "The repo was loaded at operation {}, which seems unrelated to the working \
                     copy's operation {}",
                    short_operation_hash(repo.op_id()),
                    short_operation_hash(&old_op_id)
                )));
            }
        };
        let new_tree_id = locked_wc.snapshot(base_ignores)?;
        if new_tree_id != *wc_commit.tree_id() {
            let mut tx = self
                .repo
                .start_transaction(&self.settings, "snapshot working copy");
            let mut_repo = tx.mut_repo();
            let commit = mut_repo
                .rewrite_commit(&self.settings, &wc_commit)
                .set_tree(new_tree_id)
                .write()?;
            mut_repo.set_wc_commit(workspace_id, commit.id().clone())?;

            // Rebase descendants
            let num_rebased = mut_repo.rebase_descendants(&self.settings)?;
            if num_rebased > 0 {
                writeln!(
                    ui,
                    "Rebased {num_rebased} descendant commits onto updated working copy"
                )?;
            }

            if self.working_copy_shared_with_git {
                let git_repo = self.repo.store().git_repo().unwrap();
                let failed_branches = git::export_refs(mut_repo, &git_repo)?;
                print_failed_git_export(ui, &failed_branches)?;
            }

            self.repo = tx.commit();
        }
        locked_wc.finish(self.repo.op_id().clone());
        Ok(())
    }

    pub fn start_transaction(&mut self, description: &str) -> WorkspaceCommandTransaction {
        let tx = start_repo_transaction(&self.repo, &self.settings, &self.string_args, description);
        WorkspaceCommandTransaction { helper: self, tx }
    }

    fn finish_transaction(&mut self, ui: &mut Ui, mut tx: Transaction) -> Result<(), CommandError> {
        let mut_repo = tx.mut_repo();
        let store = mut_repo.store().clone();
        if !mut_repo.has_changes() {
            writeln!(ui, "Nothing changed.")?;
            return Ok(());
        }
        let num_rebased = mut_repo.rebase_descendants(&self.settings)?;
        if num_rebased > 0 {
            writeln!(ui, "Rebased {num_rebased} descendant commits")?;
        }
        if self.working_copy_shared_with_git {
            self.export_head_to_git(mut_repo)?;
            let git_repo = self.repo.store().git_repo().unwrap();
            let failed_branches = git::export_refs(mut_repo, &git_repo)?;
            print_failed_git_export(ui, &failed_branches)?;
        }
        let maybe_old_commit = tx
            .base_repo()
            .view()
            .get_wc_commit_id(self.workspace_id())
            .map(|commit_id| store.get_commit(commit_id))
            .transpose()?;
        self.repo = tx.commit();
        if self.may_update_working_copy {
            let workspace_id = self.workspace_id().to_owned();
            let summary_template = parse_commit_summary_template(
                self.repo.as_repo_ref(),
                &workspace_id,
                &self.template_aliases_map,
                &self.settings,
            )
            .expect("parse error should be confined by WorkspaceCommandHelper::new()");
            let stats = update_working_copy(
                ui,
                &self.repo,
                &workspace_id,
                self.workspace.working_copy_mut(),
                maybe_old_commit.as_ref(),
                &summary_template,
            )?;
            if let Some(stats) = stats {
                print_checkout_stats(ui, stats)?;
            }
        }
        let settings = &self.settings;
        if settings.user_name() == UserSettings::user_name_placeholder()
            || settings.user_email() == UserSettings::user_email_placeholder()
        {
            writeln!(
                ui.warning(),
                r#"Name and email not configured. Add something like the following to $HOME/.jjconfig.toml:
  user.name = "Some One"
  user.email = "someone@example.com""#
            )?;
        }
        Ok(())
    }
}

#[must_use]
pub struct WorkspaceCommandTransaction<'a> {
    helper: &'a mut WorkspaceCommandHelper,
    tx: Transaction,
}

impl WorkspaceCommandTransaction<'_> {
    /// Workspace helper that may use the base repo.
    pub fn base_workspace_helper(&self) -> &WorkspaceCommandHelper {
        self.helper
    }

    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        self.tx.base_repo()
    }

    pub fn repo(&self) -> &MutableRepo {
        self.tx.repo()
    }

    pub fn mut_repo(&mut self) -> &mut MutableRepo {
        self.tx.mut_repo()
    }

    pub fn check_out(&mut self, commit: &Commit) -> Result<Commit, CheckOutCommitError> {
        let workspace_id = self.helper.workspace_id().to_owned();
        let settings = &self.helper.settings;
        self.tx.mut_repo().check_out(workspace_id, settings, commit)
    }

    pub fn edit(&mut self, commit: &Commit) -> Result<(), EditCommitError> {
        let workspace_id = self.helper.workspace_id().to_owned();
        self.tx.mut_repo().edit(workspace_id, commit)
    }

    pub fn run_mergetool(
        &self,
        ui: &mut Ui,
        tree: &Tree,
        repo_path: &RepoPath,
    ) -> Result<TreeId, CommandError> {
        let settings = &self.helper.settings;
        Ok(crate::merge_tools::run_mergetool(
            ui, tree, repo_path, settings,
        )?)
    }

    pub fn edit_diff(
        &self,
        ui: &mut Ui,
        left_tree: &Tree,
        right_tree: &Tree,
        instructions: &str,
    ) -> Result<TreeId, CommandError> {
        let base_ignores = self.helper.base_ignores();
        let settings = &self.helper.settings;
        Ok(crate::merge_tools::edit_diff(
            ui,
            left_tree,
            right_tree,
            instructions,
            base_ignores,
            settings,
        )?)
    }

    pub fn select_diff(
        &self,
        ui: &mut Ui,
        left_tree: &Tree,
        right_tree: &Tree,
        instructions: &str,
        interactive: bool,
        matcher: &dyn Matcher,
    ) -> Result<TreeId, CommandError> {
        if interactive {
            let base_ignores = self.helper.base_ignores();
            let settings = &self.helper.settings;
            Ok(crate::merge_tools::edit_diff(
                ui,
                left_tree,
                right_tree,
                instructions,
                base_ignores,
                settings,
            )?)
        } else if matcher.visit(&RepoPath::root()) == Visit::AllRecursively {
            // Optimization for a common case
            Ok(right_tree.id().clone())
        } else {
            let mut tree_builder = self.repo().store().tree_builder(left_tree.id().clone());
            for (repo_path, diff) in left_tree.diff(right_tree, matcher) {
                match diff.into_options().1 {
                    Some(value) => {
                        tree_builder.set(repo_path, value);
                    }
                    None => {
                        tree_builder.remove(repo_path);
                    }
                }
            }
            Ok(tree_builder.write_tree())
        }
    }

    pub fn format_commit_summary(&self, commit: &Commit) -> String {
        let mut output = Vec::new();
        self.write_commit_summary(&mut PlainTextFormatter::new(&mut output), commit)
            .expect("write() to PlainTextFormatter should never fail");
        String::from_utf8(output).expect("template output should be utf-8 bytes")
    }

    pub fn write_commit_summary(
        &self,
        formatter: &mut dyn Formatter,
        commit: &Commit,
    ) -> std::io::Result<()> {
        let template = parse_commit_summary_template(
            self.tx.repo().as_repo_ref(),
            self.helper.workspace_id(),
            &self.helper.template_aliases_map,
            &self.helper.settings,
        )
        .expect("parse error should be confined by WorkspaceCommandHelper::new()");
        template.format(commit, formatter)
    }

    pub fn finish(self, ui: &mut Ui) -> Result<(), CommandError> {
        self.helper.finish_transaction(ui, self.tx)
    }

    pub fn into_inner(self) -> Transaction {
        self.tx
    }
}

fn init_workspace_loader(
    cwd: &Path,
    global_args: &GlobalArgs,
) -> Result<WorkspaceLoader, CommandError> {
    let workspace_root = if let Some(path) = global_args.repository.as_ref() {
        cwd.join(path)
    } else {
        cwd.ancestors()
            .find(|path| path.join(".jj").is_dir())
            .unwrap_or(cwd)
            .to_owned()
    };
    WorkspaceLoader::init(&workspace_root).map_err(|err| match err {
        WorkspaceLoadError::NoWorkspaceHere(wc_path) => {
            // Prefer user-specified workspace_path_str instead of absolute wc_path.
            let workspace_path_str = global_args.repository.as_deref().unwrap_or(".");
            let message = format!(r#"There is no jj repo in "{workspace_path_str}""#);
            let git_dir = wc_path.join(".git");
            if git_dir.is_dir() {
                user_error_with_hint(
                    message,
                    "It looks like this is a git repo. You can create a jj repo backed by it by \
                     running this:
jj init --git-repo=.",
                )
            } else {
                user_error(message)
            }
        }
        WorkspaceLoadError::RepoDoesNotExist(repo_dir) => user_error(format!(
            "The repository directory at {} is missing. Was it moved?",
            repo_dir.display(),
        )),
        WorkspaceLoadError::Path(e) => user_error(format!("{}: {}", e, e.error)),
        WorkspaceLoadError::NonUnicodePath => user_error(err.to_string()),
    })
}

pub fn start_repo_transaction(
    repo: &Arc<ReadonlyRepo>,
    settings: &UserSettings,
    string_args: &[String],
    description: &str,
) -> Transaction {
    let mut tx = repo.start_transaction(settings, description);
    // TODO: Either do better shell-escaping here or store the values in some list
    // type (which we currently don't have).
    let shell_escape = |arg: &String| {
        if arg.as_bytes().iter().all(|b| {
            matches!(b,
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b','
                | b'-'
                | b'.'
                | b'/'
                | b':'
                | b'@'
                | b'_'
            )
        }) {
            arg.clone()
        } else {
            format!("'{}'", arg.replace('\'', "\\'"))
        }
    };
    let mut quoted_strings = vec!["jj".to_string()];
    quoted_strings.extend(string_args.iter().skip(1).map(shell_escape));
    tx.set_tag("args".to_string(), quoted_strings.join(" "));
    tx
}

#[derive(Debug, Error)]
pub enum StaleWorkingCopyError {
    #[error("The working copy is behind the latest operation")]
    WorkingCopyStale,
    #[error("The working copy is a sibling of the latest operation")]
    SiblingOperation,
    #[error("The working copy is unrelated to the latest operation")]
    UnrelatedOperation,
}

pub fn check_stale_working_copy(
    locked_wc: &LockedWorkingCopy,
    wc_commit: &Commit,
    repo: Arc<ReadonlyRepo>,
) -> Result<Arc<ReadonlyRepo>, StaleWorkingCopyError> {
    // Check if the working copy's tree matches the repo's view
    let wc_tree_id = locked_wc.old_tree_id().clone();
    if *wc_commit.tree_id() == wc_tree_id {
        Ok(repo)
    } else {
        let wc_operation_data = repo
            .op_store()
            .read_operation(locked_wc.old_operation_id())
            .unwrap();
        let wc_operation = Operation::new(
            repo.op_store().clone(),
            locked_wc.old_operation_id().clone(),
            wc_operation_data,
        );
        let repo_operation = repo.operation();
        let maybe_ancestor_op = dag_walk::closest_common_node(
            [wc_operation.clone()],
            [repo_operation.clone()],
            &|op: &Operation| op.parents(),
            &|op: &Operation| op.id().clone(),
        );
        if let Some(ancestor_op) = maybe_ancestor_op {
            if ancestor_op.id() == repo_operation.id() {
                // The working copy was updated since we loaded the repo. We reload the repo
                // at the working copy's operation.
                Ok(repo.reload_at(&wc_operation))
            } else if ancestor_op.id() == wc_operation.id() {
                // The working copy was not updated when some repo operation committed,
                // meaning that it's stale compared to the repo view.
                Err(StaleWorkingCopyError::WorkingCopyStale)
            } else {
                Err(StaleWorkingCopyError::SiblingOperation)
            }
        } else {
            Err(StaleWorkingCopyError::UnrelatedOperation)
        }
    }
}

pub fn print_checkout_stats(ui: &mut Ui, stats: CheckoutStats) -> Result<(), std::io::Error> {
    if stats.added_files > 0 || stats.updated_files > 0 || stats.removed_files > 0 {
        writeln!(
            ui,
            "Added {} files, modified {} files, removed {} files",
            stats.added_files, stats.updated_files, stats.removed_files
        )?;
    }
    Ok(())
}

pub fn print_failed_git_export(
    ui: &mut Ui,
    failed_branches: &[String],
) -> Result<(), std::io::Error> {
    if !failed_branches.is_empty() {
        writeln!(ui.warning(), "Failed to export some branches:")?;
        let mut formatter = ui.stderr_formatter();
        for branch_name in failed_branches {
            formatter.write_str("  ")?;
            write!(formatter.labeled("branch"), "{branch_name}")?;
            formatter.write_str("\n")?;
        }
        drop(formatter);
        writeln!(
            ui.hint(),
            r#"Hint: Git doesn't allow a branch name that looks like a parent directory of
another (e.g. `foo` and `foo/bar`). Try to rename the branches that failed to
export or their "parent" branches."#,
        )?;
    }
    Ok(())
}

/// Expands "~/" to "$HOME/" as Git seems to do for e.g. core.excludesFile.
fn expand_git_path(path_str: String) -> PathBuf {
    if let Some(remainder) = path_str.strip_prefix("~/") {
        if let Ok(home_dir_str) = std::env::var("HOME") {
            return PathBuf::from(home_dir_str).join(remainder);
        }
    }
    PathBuf::from(path_str)
}

fn resolve_op_for_load(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &Arc<dyn OpHeadsStore>,
    op_str: &str,
) -> Result<Operation, OpHeadResolutionError<CommandError>> {
    let get_current_op = || {
        op_heads_store::resolve_op_heads(op_heads_store.as_ref(), op_store, |_| {
            Err(user_error(format!(
                r#"The "{op_str}" expression resolved to more than one operation"#
            )))
        })
    };
    let operation = resolve_single_op(op_store, op_heads_store, get_current_op, op_str)?;
    Ok(operation)
}

fn resolve_single_op(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &Arc<dyn OpHeadsStore>,
    get_current_op: impl FnOnce() -> Result<Operation, OpHeadResolutionError<CommandError>>,
    op_str: &str,
) -> Result<Operation, CommandError> {
    let op_symbol = op_str.trim_end_matches('-');
    let op_postfix = &op_str[op_symbol.len()..];
    let mut operation = match op_symbol {
        "@" => get_current_op(),
        s => resolve_single_op_from_store(op_store, op_heads_store, s)
            .map_err(OpHeadResolutionError::Err),
    }?;
    for _ in op_postfix.chars() {
        operation = match operation.parents().as_slice() {
            [op] => Ok(op.clone()),
            [] => Err(user_error(format!(
                r#"The "{op_str}" expression resolved to no operations"#
            ))),
            [_, _, ..] => Err(user_error(format!(
                r#"The "{op_str}" expression resolved to more than one operation"#
            ))),
        }?;
    }
    Ok(operation)
}

fn find_all_operations(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &Arc<dyn OpHeadsStore>,
) -> Vec<Operation> {
    let mut visited = HashSet::new();
    let mut work: VecDeque<_> = op_heads_store.get_op_heads().into_iter().collect();
    let mut operations = vec![];
    while let Some(op_id) = work.pop_front() {
        if visited.insert(op_id.clone()) {
            let store_operation = op_store.read_operation(&op_id).unwrap();
            work.extend(store_operation.parents.iter().cloned());
            let operation = Operation::new(op_store.clone(), op_id, store_operation);
            operations.push(operation);
        }
    }
    operations
}

fn resolve_single_op_from_store(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &Arc<dyn OpHeadsStore>,
    op_str: &str,
) -> Result<Operation, CommandError> {
    if op_str.is_empty() || !op_str.as_bytes().iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(user_error(format!(
            "Operation ID \"{op_str}\" is not a valid hexadecimal prefix"
        )));
    }
    if let Ok(binary_op_id) = hex::decode(op_str) {
        let op_id = OperationId::new(binary_op_id);
        match op_store.read_operation(&op_id) {
            Ok(operation) => {
                return Ok(Operation::new(op_store.clone(), op_id, operation));
            }
            Err(OpStoreError::NotFound) => {
                // Fall through
            }
            Err(err) => {
                return Err(CommandError::InternalError(format!(
                    "Failed to read operation: {err}"
                )));
            }
        }
    }
    let mut matches = vec![];
    for op in find_all_operations(op_store, op_heads_store) {
        if op.id().hex().starts_with(op_str) {
            matches.push(op);
        }
    }
    if matches.is_empty() {
        Err(user_error(format!("No operation ID matching \"{op_str}\"")))
    } else if matches.len() == 1 {
        Ok(matches.pop().unwrap())
    } else {
        Err(user_error(format!(
            "Operation ID prefix \"{op_str}\" is ambiguous"
        )))
    }
}

fn load_revset_aliases(
    ui: &mut Ui,
    settings: &UserSettings,
) -> Result<RevsetAliasesMap, CommandError> {
    const TABLE_KEY: &str = "revset-aliases";
    let mut aliases_map = RevsetAliasesMap::new();
    let table = settings.config().get_table(TABLE_KEY)?;
    for (decl, value) in table.into_iter().sorted_by(|a, b| a.0.cmp(&b.0)) {
        let r = value
            .into_string()
            .map_err(|e| e.to_string())
            .and_then(|v| aliases_map.insert(&decl, v).map_err(|e| e.to_string()));
        if let Err(s) = r {
            writeln!(ui.warning(), r#"Failed to load "{TABLE_KEY}.{decl}": {s}"#)?;
        }
    }
    Ok(aliases_map)
}

pub fn resolve_multiple_nonempty_revsets(
    revision_args: &[RevisionArg],
    workspace_command: &WorkspaceCommandHelper,
) -> Result<IndexSet<Commit>, CommandError> {
    let mut acc = IndexSet::new();
    for revset in revision_args {
        let revisions = workspace_command.resolve_revset(revset)?;
        workspace_command.check_non_empty(&revisions)?;
        acc.extend(revisions);
    }
    Ok(acc)
}

pub fn resolve_mutliple_nonempty_revsets_flag_guarded(
    workspace_command: &WorkspaceCommandHelper,
    revisions: &[RevisionArg],
    allow_plural_revsets: bool,
) -> Result<IndexSet<Commit>, CommandError> {
    if allow_plural_revsets {
        resolve_multiple_nonempty_revsets(revisions, workspace_command)
    } else {
        let mut commits = IndexSet::new();
        for revision_str in revisions {
            let commit = workspace_command
                .resolve_single_rev(revision_str)
                .map_err(append_large_revsets_hint_if_multiple_revisions)?;
            let commit_hash = short_commit_hash(commit.id());
            if !commits.insert(commit) {
                return Err(user_error(format!(
                    r#"More than one revset resolved to revision {commit_hash}"#,
                )));
            }
        }
        Ok(commits)
    }
}

fn append_large_revsets_hint_if_multiple_revisions(err: CommandError) -> CommandError {
    match err {
        CommandError::UserError { message, hint } if message.contains("more than one revision") => {
            CommandError::UserError {
                message,
                hint: {
                    Some(format!(
                        "{old_hint}If this was intentional, specify the `--allow-large-revsets` \
                         argument",
                        old_hint = hint.map(|h| format!("{h}\n")).unwrap_or_default()
                    ))
                },
            }
        }
        _ => err,
    }
}

pub fn update_working_copy(
    ui: &mut Ui,
    repo: &Arc<ReadonlyRepo>,
    workspace_id: &WorkspaceId,
    wc: &mut WorkingCopy,
    old_commit: Option<&Commit>,
    summary_template: &dyn Template<Commit>,
) -> Result<Option<CheckoutStats>, CommandError> {
    let new_commit_id = match repo.view().get_wc_commit_id(workspace_id) {
        Some(new_commit_id) => new_commit_id,
        None => {
            // It seems the workspace was deleted, so we shouldn't try to update it.
            return Ok(None);
        }
    };
    let new_commit = repo.store().get_commit(new_commit_id)?;
    let old_tree_id = old_commit.map(|commit| commit.tree_id().clone());
    let stats = if Some(new_commit.tree_id()) != old_tree_id.as_ref() {
        // TODO: CheckoutError::ConcurrentCheckout should probably just result in a
        // warning for most commands (but be an error for the checkout command)
        let stats = wc
            .check_out(
                repo.op_id().clone(),
                old_tree_id.as_ref(),
                &new_commit.tree(),
            )
            .map_err(|err| {
                CommandError::InternalError(format!(
                    "Failed to check out commit {}: {}",
                    new_commit.id().hex(),
                    err
                ))
            })?;
        Some(stats)
    } else {
        // Record new operation id which represents the latest working-copy state
        let locked_wc = wc.start_mutation();
        locked_wc.finish(repo.op_id().clone());
        None
    };
    if Some(&new_commit) != old_commit {
        ui.write("Working copy now at: ")?;
        summary_template.format(&new_commit, ui.stdout_formatter().as_mut())?;
        ui.write("\n")?;
    }
    Ok(stats)
}

fn load_template_aliases(
    ui: &mut Ui,
    settings: &UserSettings,
) -> Result<TemplateAliasesMap, CommandError> {
    const TABLE_KEY: &str = "template-aliases";
    let mut aliases_map = TemplateAliasesMap::new();
    let table = settings.config().get_table(TABLE_KEY)?;
    for (decl, value) in table.into_iter().sorted_by(|a, b| a.0.cmp(&b.0)) {
        let r = value
            .into_string()
            .map_err(|e| e.to_string())
            .and_then(|v| aliases_map.insert(&decl, v).map_err(|e| e.to_string()));
        if let Err(s) = r {
            writeln!(ui.warning(), r#"Failed to load "{TABLE_KEY}.{decl}": {s}"#)?;
        }
    }
    Ok(aliases_map)
}

fn parse_commit_summary_template<'a>(
    repo: RepoRef<'a>,
    workspace_id: &WorkspaceId,
    aliases_map: &TemplateAliasesMap,
    settings: &UserSettings,
) -> Result<Box<dyn Template<Commit> + 'a>, CommandError> {
    let template_text = settings.config().get_string("templates.commit_summary")?;
    Ok(template_parser::parse_commit_template(
        repo,
        workspace_id,
        &template_text,
        aliases_map,
    )?)
}

// TODO: Use a proper TOML library to serialize instead.
pub fn serialize_config_value(value: &config::Value) -> String {
    match &value.kind {
        config::ValueKind::Table(table) => format!(
            "{{{}}}",
            // TODO: Remove sorting when config crate maintains deterministic ordering.
            table
                .iter()
                .sorted_by_key(|(k, _)| *k)
                .map(|(k, v)| format!("{k}={}", serialize_config_value(v)))
                .join(", ")
        ),
        config::ValueKind::Array(vals) => {
            format!("[{}]", vals.iter().map(serialize_config_value).join(", "))
        }
        config::ValueKind::String(val) => format!("{val:?}"),
        _ => value.to_string(),
    }
}

pub fn run_ui_editor(settings: &UserSettings, edit_path: &PathBuf) -> Result<(), CommandError> {
    let editor: CommandNameAndArgs = settings
        .config()
        .get("ui.editor")
        .unwrap_or_else(|_| "pico".into());
    let exit_status = editor
        .to_command()
        .arg(edit_path)
        .status()
        .map_err(|_| user_error(format!("Failed to run editor '{editor}'")))?;
    if !exit_status.success() {
        return Err(user_error(format!(
            "Editor '{editor}' exited with an error"
        )));
    }

    Ok(())
}

pub fn short_commit_hash(commit_id: &CommitId) -> String {
    commit_id.hex()[0..12].to_string()
}

pub fn short_change_hash(change_id: &ChangeId) -> String {
    // TODO: We could avoid the unwrap() and make this more efficient by converting
    // straight from binary.
    to_reverse_hex(&change_id.hex()[0..12]).unwrap()
}

pub fn short_operation_hash(operation_id: &OperationId) -> String {
    operation_id.hex()[0..12].to_string()
}

/// Jujutsu (An experimental VCS)
///
/// To get started, see the tutorial at https://github.com/martinvonz/jj/blob/main/docs/tutorial.md.
#[derive(clap::Parser, Clone, Debug)]
#[command(
    name = "jj",
    author = "Martin von Zweigbergk <martinvonz@google.com>",
    version
)]
pub struct Args {
    #[command(flatten)]
    pub global_args: GlobalArgs,
}

#[derive(clap::Args, Clone, Debug)]
pub struct GlobalArgs {
    /// Path to repository to operate on
    ///
    /// By default, Jujutsu searches for the closest .jj/ directory in an
    /// ancestor of the current working directory.
    #[arg(
    long,
    short = 'R',
    global = true,
    help_heading = "Global Options",
    value_hint = clap::ValueHint::DirPath,
    )]
    pub repository: Option<String>,
    /// Don't snapshot the working copy, and don't update it
    ///
    /// By default, Jujutsu snapshots the working copy at the beginning of every
    /// command. The working copy is also updated at the end of the command,
    /// if the command modified the working-copy commit (`@`). If you want
    /// to avoid snapshotting the working and instead see a possibly
    /// stale working copy commit, you can use `--ignore-working-copy`.
    /// This may be useful e.g. in a command prompt, especially if you have
    /// another process that commits the working copy.
    ///
    /// Loading the repository is at a specific operation with `--at-operation`
    /// implies `--ignore-working-copy`.
    #[arg(long, global = true, help_heading = "Global Options")]
    pub ignore_working_copy: bool,
    /// Operation to load the repo at
    ///
    /// Operation to load the repo at. By default, Jujutsu loads the repo at the
    /// most recent operation. You can use `--at-op=<operation ID>` to see what
    /// the repo looked like at an earlier operation. For example `jj
    /// --at-op=<operation ID> st` will show you what `jj st` would have
    /// shown you when the given operation had just finished.
    ///
    /// Use `jj op log` to find the operation ID you want. Any unambiguous
    /// prefix of the operation ID is enough.
    ///
    /// When loading the repo at an earlier operation, the working copy will be
    /// ignored, as if `--ignore-working-copy` had been specified.
    ///
    /// It is possible to run mutating commands when loading the repo at an
    /// earlier operation. Doing that is equivalent to having run concurrent
    /// commands starting at the earlier operation. There's rarely a reason to
    /// do that, but it is possible.
    #[arg(
        long,
        visible_alias = "at-op",
        global = true,
        help_heading = "Global Options",
        default_value = "@"
    )]
    pub at_operation: String,
    /// Enable verbose logging
    #[arg(long, short = 'v', global = true, help_heading = "Global Options")]
    pub verbose: bool,

    #[command(flatten)]
    pub early_args: EarlyArgs,
}

#[derive(clap::Args, Clone, Debug)]
pub struct EarlyArgs {
    /// When to colorize output (always, never, auto)
    #[arg(
        long,
        value_name = "WHEN",
        global = true,
        help_heading = "Global Options"
    )]
    pub color: Option<ColorChoice>,
    /// Disable the pager
    #[arg(
        long,
        value_name = "WHEN",
        global = true,
        help_heading = "Global Options",
        action = ArgAction::SetTrue
    )]
    // Parsing with ignore_errors will crash if this is bool, so use
    // Option<bool>.
    pub no_pager: Option<bool>,
    /// Additional configuration options
    //  TODO: Introduce a `--config` option with simpler syntax for simple
    //  cases, designed so that `--config ui.color=auto` works
    #[arg(
        long,
        value_name = "TOML",
        global = true,
        help_heading = "Global Options"
    )]
    pub config_toml: Vec<String>,
}

/// `-m/--message` argument which should be terminated with `\n`.
///
/// Based on the Git CLI behavior. See `opt_parse_m()` and `cleanup_mode` in
/// `git/builtin/commit.c`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptionArg(String);

impl DescriptionArg {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<String> for DescriptionArg {
    fn from(s: String) -> Self {
        DescriptionArg(complete_newline(s))
    }
}

impl From<&DescriptionArg> for String {
    fn from(arg: &DescriptionArg) -> Self {
        arg.0.to_owned()
    }
}

impl AsRef<str> for DescriptionArg {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

pub fn complete_newline(s: impl Into<String>) -> String {
    let mut s = s.into();
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[derive(Clone, Debug)]
pub struct RevisionArg(String);

impl Deref for RevisionArg {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0.as_str()
    }
}

#[derive(Clone)]
pub struct RevisionArgValueParser;

impl TypedValueParser for RevisionArgValueParser {
    type Value = RevisionArg;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let string = NonEmptyStringValueParser::new().parse(cmd, arg, value.to_os_string())?;
        Ok(RevisionArg(string))
    }
}

impl ValueParserFactory for RevisionArg {
    type Parser = RevisionArgValueParser;

    fn value_parser() -> RevisionArgValueParser {
        RevisionArgValueParser
    }
}

fn resolve_aliases(
    config: &config::Config,
    app: &Command,
    string_args: &[String],
) -> Result<Vec<String>, CommandError> {
    let mut aliases_map = config.get_table("aliases")?;
    if let Ok(alias_map) = config.get_table("alias") {
        for (alias, definition) in alias_map {
            if aliases_map.insert(alias.clone(), definition).is_some() {
                return Err(user_error_with_hint(
                    format!(r#"Alias "{alias}" is defined in both [aliases] and [alias]"#),
                    "[aliases] is the preferred section for aliases. Please remove the alias from \
                     [alias].",
                ));
            }
        }
    }
    let mut resolved_aliases = HashSet::new();
    let mut string_args = string_args.to_vec();
    let mut real_commands = HashSet::new();
    for command in app.get_subcommands() {
        real_commands.insert(command.get_name().to_string());
        for alias in command.get_all_aliases() {
            real_commands.insert(alias.to_string());
        }
    }
    loop {
        let app_clone = app.clone().allow_external_subcommands(true);
        let matches = app_clone.try_get_matches_from(&string_args).ok();
        if let Some((command_name, submatches)) = matches.as_ref().and_then(|m| m.subcommand()) {
            if !real_commands.contains(command_name) {
                let alias_name = command_name.to_string();
                let alias_args = submatches
                    .get_many::<OsString>("")
                    .unwrap_or_default()
                    .map(|arg| arg.to_str().unwrap().to_string())
                    .collect_vec();
                if resolved_aliases.contains(&alias_name) {
                    return Err(user_error(format!(
                        r#"Recursive alias definition involving "{alias_name}""#
                    )));
                }
                if let Some(value) = aliases_map.remove(&alias_name) {
                    if let Ok(alias_definition) = value.try_deserialize::<Vec<String>>() {
                        assert!(string_args.ends_with(&alias_args));
                        string_args.truncate(string_args.len() - 1 - alias_args.len());
                        string_args.extend(alias_definition);
                        string_args.extend_from_slice(&alias_args);
                        resolved_aliases.insert(alias_name.clone());
                        continue;
                    } else {
                        return Err(user_error(format!(
                            r#"Alias definition for "{alias_name}" must be a string list"#
                        )));
                    }
                } else {
                    // Not a real command and not an alias, so return what we've resolved so far
                    return Ok(string_args);
                }
            }
        }
        // No more alias commands, or hit unknown option
        return Ok(string_args);
    }
}

/// Parse args that must be interpreted early, e.g. before printing help.
fn handle_early_args(
    ui: &mut Ui,
    app: &Command,
    args: &[String],
    layered_configs: &mut LayeredConfigs,
) -> Result<(), CommandError> {
    // ignore_errors() bypasses errors like "--help" or missing subcommand
    let early_matches = app.clone().ignore_errors(true).get_matches_from(args);
    let mut args: EarlyArgs = EarlyArgs::from_arg_matches(&early_matches).unwrap();

    if let Some(choice) = args.color {
        args.config_toml.push(format!(r#"ui.color="{choice}""#));
    }
    if args.no_pager.unwrap_or_default() {
        ui.set_pagination(crate::ui::PaginationChoice::No);
    }
    if !args.config_toml.is_empty() {
        layered_configs.parse_config_args(&args.config_toml)?;
        ui.reset(&layered_configs.merge())?;
    }
    Ok(())
}

pub fn expand_args(
    app: &Command,
    args_os: ArgsOs,
    config: &config::Config,
) -> Result<Vec<String>, CommandError> {
    let mut string_args: Vec<String> = vec![];
    for arg_os in args_os {
        if let Some(string_arg) = arg_os.to_str() {
            string_args.push(string_arg.to_owned());
        } else {
            return Err(CommandError::CliError("Non-utf8 argument".to_string()));
        }
    }

    resolve_aliases(config, app, &string_args)
}

pub fn parse_args(
    ui: &mut Ui,
    app: &Command,
    tracing_subscription: &TracingSubscription,
    string_args: &[String],
    layered_configs: &mut LayeredConfigs,
) -> Result<(ArgMatches, Args), CommandError> {
    handle_early_args(ui, app, string_args, layered_configs)?;
    let matches = app.clone().try_get_matches_from(string_args)?;

    let args: Args = Args::from_arg_matches(&matches).unwrap();
    if args.global_args.verbose {
        // TODO: set up verbose logging as early as possible
        tracing_subscription.enable_verbose_logging()?;
    }

    Ok((matches, args))
}

const BROKEN_PIPE_EXIT_CODE: u8 = 3;

pub fn handle_command_result(
    ui: &mut Ui,
    result: Result<(), CommandError>,
) -> std::io::Result<ExitCode> {
    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(CommandError::UserError { message, hint }) => {
            writeln!(ui.error(), "Error: {message}")?;
            if let Some(hint) = hint {
                writeln!(ui.hint(), "Hint: {hint}")?;
            }
            Ok(ExitCode::from(1))
        }
        Err(CommandError::ConfigError(message)) => {
            writeln!(ui.error(), "Config error: {message}")?;
            Ok(ExitCode::from(1))
        }
        Err(CommandError::CliError(message)) => {
            writeln!(ui.error(), "Error: {message}")?;
            Ok(ExitCode::from(2))
        }
        Err(CommandError::ClapCliError(inner)) => {
            let clap_str = if ui.color() {
                inner.render().ansi().to_string()
            } else {
                inner.render().to_string()
            };

            match inner.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    ui.request_pager()
                }
                _ => {}
            };
            // Definitions for exit codes and streams come from
            // https://github.com/clap-rs/clap/blob/master/src/error/mod.rs
            match inner.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    ui.write(&clap_str)?;
                    Ok(ExitCode::SUCCESS)
                }
                _ => {
                    ui.write_stderr(&clap_str)?;
                    Ok(ExitCode::from(2))
                }
            }
        }
        Err(CommandError::BrokenPipe) => {
            // A broken pipe is not an error, but a signal to exit gracefully.
            Ok(ExitCode::from(BROKEN_PIPE_EXIT_CODE))
        }
        Err(CommandError::InternalError(message)) => {
            writeln!(ui.error(), "Internal error: {message}")?;
            Ok(ExitCode::from(255))
        }
    }
}

/// CLI command builder and runner.
#[must_use]
pub struct CliRunner {
    tracing_subscription: TracingSubscription,
    app: Command,
    store_factories: Option<StoreFactories>,
    dispatch_fn: CliDispatchFn,
    process_global_args_fns: Vec<ProcessGlobalArgsFn>,
}

type CliDispatchFn =
    Box<dyn FnOnce(&mut Ui, &CommandHelper, &ArgMatches) -> Result<(), CommandError>>;

type ProcessGlobalArgsFn = Box<dyn FnOnce(&mut Ui, &ArgMatches) -> Result<(), CommandError>>;

impl CliRunner {
    /// Initializes CLI environment and returns a builder. This should be called
    /// as early as possible.
    pub fn init() -> Self {
        let tracing_subscription = TracingSubscription::init();
        crate::cleanup_guard::init();
        CliRunner {
            tracing_subscription,
            app: crate::commands::default_app(),
            store_factories: None,
            dispatch_fn: Box::new(crate::commands::run_command),
            process_global_args_fns: vec![],
        }
    }

    /// Replaces `StoreFactories` to be used.
    pub fn set_store_factories(mut self, store_factories: StoreFactories) -> Self {
        self.store_factories = Some(store_factories);
        self
    }

    /// Registers new subcommands in addition to the default ones.
    pub fn add_subcommand<C, F>(mut self, custom_dispatch_fn: F) -> Self
    where
        C: clap::Subcommand,
        F: FnOnce(&mut Ui, &CommandHelper, C) -> Result<(), CommandError> + 'static,
    {
        let old_dispatch_fn = self.dispatch_fn;
        let new_dispatch_fn =
            move |ui: &mut Ui, command_helper: &CommandHelper, matches: &ArgMatches| {
                match C::from_arg_matches(matches) {
                    Ok(command) => custom_dispatch_fn(ui, command_helper, command),
                    Err(_) => old_dispatch_fn(ui, command_helper, matches),
                }
            };
        self.app = C::augment_subcommands(self.app);
        self.dispatch_fn = Box::new(new_dispatch_fn);
        self
    }

    /// Registers new global arguments in addition to the default ones.
    pub fn add_global_args<A, F>(mut self, process_before: F) -> Self
    where
        A: clap::Args,
        F: FnOnce(&mut Ui, A) -> Result<(), CommandError> + 'static,
    {
        let process_global_args_fn = move |ui: &mut Ui, matches: &ArgMatches| {
            let custom_args = A::from_arg_matches(matches).unwrap();
            process_before(ui, custom_args)
        };
        self.app = A::augment_args(self.app);
        self.process_global_args_fns
            .push(Box::new(process_global_args_fn));
        self
    }

    fn run_internal(
        self,
        ui: &mut Ui,
        mut layered_configs: LayeredConfigs,
    ) -> Result<(), CommandError> {
        let cwd = env::current_dir().unwrap(); // TODO: maybe map_err to CommandError?
        layered_configs.read_user_config()?;
        let config = layered_configs.merge();
        ui.reset(&config)?;
        let string_args = expand_args(&self.app, std::env::args_os(), &config)?;
        let (matches, args) = parse_args(
            ui,
            &self.app,
            &self.tracing_subscription,
            &string_args,
            &mut layered_configs,
        )?;
        for process_global_args_fn in self.process_global_args_fns {
            process_global_args_fn(ui, &matches)?;
        }

        let maybe_workspace_loader = init_workspace_loader(&cwd, &args.global_args);
        if let Ok(loader) = &maybe_workspace_loader {
            // TODO: maybe show error/warning if repo config contained command alias
            layered_configs.read_repo_config(loader.repo_path())?;
        }
        let config = layered_configs.merge();
        ui.reset(&config)?;
        let settings = UserSettings::from_config(config);
        let command_helper = CommandHelper::new(
            self.app,
            cwd,
            string_args,
            args.global_args,
            settings,
            layered_configs,
            maybe_workspace_loader,
            self.store_factories.unwrap_or_default(),
        );
        (self.dispatch_fn)(ui, &command_helper, &matches)
    }

    #[must_use]
    pub fn run(self) -> ExitCode {
        let layered_configs = LayeredConfigs::from_environment();
        let mut ui = Ui::with_config(&layered_configs.merge())
            .expect("default config should be valid, env vars are stringly typed");
        let result = self.run_internal(&mut ui, layered_configs);
        let exit_code = handle_command_result(&mut ui, result)
            .unwrap_or_else(|_| ExitCode::from(BROKEN_PIPE_EXIT_CODE));
        ui.finalize_pager();
        exit_code
    }
}
