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

//! Running operations on a workspace: snapshotting, importing from and
//! exporting to Git, finishing transactions, and updating the working copy.

use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigGetError;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::gitignore::GitIgnoreError;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::lock::FileLock;
use jj_lib::lock::FileLockError;
use jj_lib::matchers::Matcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_heads_store;
use jj_lib::op_heads_store::OpHeadsStoreError;
use jj_lib::op_store::OpStoreError;
use jj_lib::op_store::OperationId;
use jj_lib::operation::Operation;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::CheckOutCommitError;
#[cfg(feature = "git")]
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo::RepoLoader;
use jj_lib::repo::RepoLoaderError;
use jj_lib::repo::RewriteRootCommit;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use jj_lib::transaction::TransactionCommitError;
use jj_lib::working_copy;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::LockedWorkingCopy;
use jj_lib::working_copy::RecoverWorkspaceError;
use jj_lib::working_copy::ResetError;
use jj_lib::working_copy::SnapshotError;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::SnapshotProgress;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::working_copy::WorkingCopyFreshness;
use jj_lib::working_copy::WorkingCopyStateError;
use jj_lib::workspace::LockedWorkspace;
use jj_lib::workspace::Workspace;
use thiserror::Error;
use tracing::instrument;

use crate::config_util::RemoteSettingDiagnostics;
use crate::config_util::RemoteSettingParseError;
use crate::workspace_env::WorkspaceEnvironment;
use crate::workspace_env::WorkspaceEnvironmentDiagnostics;
use crate::workspace_env::WorkspaceEnvironmentError;

/// A token that holds a lock for git import/export operations in colocated
/// repositories. For non-colocated repos, this is an empty token (no actual
/// lock held). The lock is automatically released when this token is dropped.
pub struct GitImportExportLock {
    pub(crate) _lock: Option<FileLock>,
}

/// Options controlling how a [`WorkspaceOperationRunner`] interacts with the
/// working copy and the operation log.
#[derive(Clone, Debug)]
pub struct RunnerOptions {
    /// Don't snapshot or update the working copy.
    pub ignore_working_copy: bool,
    /// Publish new operations to the operation log. If false, operations are
    /// written but left unpublished, and the working copy isn't updated.
    pub integrate_operation: bool,
    /// Allow rewriting immutable commits.
    pub ignore_immutable: bool,
    /// Merge concurrent operations when loading the repo at the head
    /// operation. If false, loading fails if there are concurrent operations.
    pub merge_concurrent_operations: bool,
    /// Attributes to record in the metadata of new operations.
    pub transaction_attributes: Vec<(String, String)>,
}

impl RunnerOptions {
    /// Starts a transaction on `repo` for the workspace `workspace_name`, with
    /// the configured operation attributes.
    pub fn start_transaction(
        &self,
        repo: &Arc<ReadonlyRepo>,
        workspace_name: &WorkspaceName,
    ) -> Transaction {
        let mut tx = repo.start_transaction();
        tx.set_workspace_name(workspace_name);
        for (key, value) in &self.transaction_attributes {
            tx.set_attribute(key.clone(), value.clone());
        }
        tx
    }

    /// Writes the operation of `tx`, and publishes it unless
    /// [`Self::integrate_operation`] is false.
    pub async fn commit_transaction(
        &self,
        tx: Transaction,
        description: impl Into<String>,
    ) -> Result<Arc<ReadonlyRepo>, TransactionCommitError> {
        let unpublished_op = tx.write(description).await?;
        if self.integrate_operation {
            unpublished_op.publish().await
        } else {
            Ok(unpublished_op.leave_unpublished())
        }
    }
}

/// Something noteworthy that happened while snapshotting the working copy,
/// importing from or exporting to Git, or finishing a transaction.
#[derive(Debug)]
pub enum RunnerEvent<'a> {
    /// The `trunk()` revset alias couldn't be resolved in the loaded repo, so
    /// it was temporarily set to `fallback`.
    TrunkAliasFallback {
        /// Why `trunk()` couldn't be resolved.
        error: &'a RevsetResolutionError,
        /// The definition `trunk()` was set to.
        fallback: &'a str,
    },
    /// Problems found while reloading the [`WorkspaceEnvironment`].
    EnvironmentDiagnostics(&'a WorkspaceEnvironmentDiagnostics),
    /// Concurrent operations were found when loading the repo and are about
    /// to be merged.
    MergingConcurrentOperations {
        /// The concurrent operations.
        operations: &'a [Operation],
    },
    /// Descendants of rewritten commits were rebased.
    DescendantsRebased {
        /// Number of rebased commits.
        num_rebased: usize,
        /// Why the commits were rebased.
        reason: RebaseReason,
    },
    /// A new operation was written but not published because
    /// [`RunnerOptions::integrate_operation`] is false.
    OperationLeftUnpublished {
        /// ID of the unpublished operation.
        op_id: &'a OperationId,
        /// Whether the operation snapshotted the working copy.
        is_snapshot: bool,
    },
    /// The working copy had changes but the working-copy commit was immutable,
    /// so the changes were snapshotted into a new commit on top of it.
    SnapshotOnImmutableWorkingCopy {
        /// The new working-copy commit.
        new_wc_commit: &'a Commit,
    },
    /// The working-copy commit became immutable, so a new working-copy commit
    /// was created on top of it.
    WorkingCopyBecameImmutable {
        /// The new working-copy commit.
        new_wc_commit: &'a Commit,
    },
    /// The working copy was updated from `old_commit` to `new_commit`.
    WorkingCopyUpdated {
        /// The repo containing the commits.
        repo: &'a Arc<ReadonlyRepo>,
        /// The previous working-copy commit, if any.
        old_commit: Option<&'a Commit>,
        /// The new working-copy commit.
        new_commit: &'a Commit,
        /// Statistics about the files updated in the working copy.
        stats: &'a CheckoutStats,
    },
    /// A transaction was committed, changing the repo from `old_repo` to
    /// `new_repo`.
    TransactionFinished {
        /// The repo before the transaction.
        old_repo: &'a Arc<ReadonlyRepo>,
        /// The repo after the transaction.
        new_repo: &'a Arc<ReadonlyRepo>,
    },
    /// The user's name or email isn't configured, so new commits will have an
    /// empty author and committer.
    UserIdentityNotConfigured {
        /// Whether `user.name` is missing.
        missing_name: bool,
        /// Whether `user.email` is missing.
        missing_email: bool,
    },
    /// The working copy was found to be stale, but by the time it was locked,
    /// it had already been updated.
    WorkingCopyNotStale,
    /// The stale working copy was updated to `new_commit`.
    StaleWorkingCopyUpdated {
        /// The new working-copy commit.
        new_commit: &'a Commit,
    },
    /// The operation the working copy was last updated at couldn't be found, so
    /// a recovery commit will be created.
    WorkingCopyOperationNotFound(&'a OpStoreError),
    /// A recovery commit with the contents of the working copy was created and
    /// checked out.
    RecoveryCommitCreated {
        /// The recovery commit.
        commit: &'a Commit,
    },
    /// The working copy contains `.jjconflict*` files, which are used to
    /// represent conflicts in Git commits. This likely means that a conflicted
    /// commit was checked out by Git.
    #[cfg(feature = "git")]
    JjConflictFilesInWorkingCopy,
    /// The working-copy commit was moved because Git HEAD had changed.
    /// `first_import` is true if Git HEAD hadn't been imported before.
    #[cfg(feature = "git")]
    GitHeadImported {
        /// Whether Git HEAD hadn't been imported before.
        first_import: bool,
    },
    /// Refs were imported from Git. There may also have been failures.
    #[cfg(feature = "git")]
    GitRefsImported(&'a jj_lib::git::GitImportStats),
    /// Changes imported from Git have been committed.
    #[cfg(feature = "git")]
    GitRefsImportFinished,
    /// Refs were exported to Git. There may also have been failures.
    #[cfg(feature = "git")]
    GitRefsExported(&'a jj_lib::git::GitExportStats),
    /// Git HEAD couldn't be updated to the working-copy commit's parent.
    #[cfg(feature = "git")]
    GitHeadResetFailed(&'a jj_lib::git::GitResetHeadError),
}

/// Why descendant commits were rebased.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RebaseReason {
    /// Commits were rewritten by concurrent operations that were merged.
    ConcurrentOperations,
    /// Commits were rewritten by a transaction.
    RewrittenCommits,
    /// Commits were rewritten in the Git repo.
    RewrittenInGit,
    /// The working-copy commit was rewritten by a snapshot.
    WorkingCopySnapshot,
}

/// Receives [`RunnerEvent`]s from a [`WorkspaceOperationRunner`].
pub trait RunnerEvents {
    /// Handles `event`. Returning an error aborts the operation, and the error
    /// is returned from the runner as [`RunnerError::EventSink`].
    fn event(&self, event: RunnerEvent<'_>)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Returns a callback for reporting progress while snapshotting the working
    /// copy.
    fn snapshot_progress(&self) -> Option<Box<SnapshotProgress<'_>>> {
        None
    }
}

/// Error from snapshotting the working copy, importing from or exporting to
/// Git, or finishing a transaction.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// There are concurrent operations, and
    /// [`RunnerOptions::merge_concurrent_operations`] isn't set.
    #[error("Found {} concurrent operations", op_ids.len())]
    ConcurrentOperations {
        /// IDs of the concurrent operations.
        op_ids: Vec<OperationId>,
    },
    /// The working copy needs to be updated, but it can't be.
    #[error("The working copy can't be updated")]
    WorkingCopyNotWritable(WorkingCopyNotWritableReason),
    /// The root commit was going to be rewritten.
    #[error("The root commit {commit_id:.12} is immutable")]
    RootCommitImmutable {
        /// ID of the root commit.
        commit_id: CommitId,
    },
    /// An immutable commit was going to be rewritten.
    #[error("Commit {commit_id:.12} is immutable")]
    ImmutableCommit {
        /// ID of one of the immutable commits.
        commit_id: CommitId,
        /// Estimated number of immutable commits that would be rewritten,
        /// including descendants, as a lower bound and an optional upper bound.
        num_immutable: (usize, Option<usize>),
    },
    /// There's no working-copy commit for the workspace.
    #[error("Nothing checked out in this workspace")]
    NothingCheckedOut,
    /// The working copy was changed concurrently.
    #[error("Concurrent working copy operation. Try again.")]
    ConcurrentWorkingCopyOperation,
    /// The lock for importing from and exporting to Git couldn't be taken.
    #[error("Failed to take lock for Git import/export")]
    GitImportExportLock(#[source] FileLockError),
    /// The configured `immutable_heads()` couldn't be resolved.
    #[error("Invalid `revset-aliases.immutable_heads()`")]
    InvalidImmutableHeads(#[source] RevsetResolutionError),
    /// A commit couldn't be checked out in the working copy.
    #[error("Failed to check out commit {}", commit_id.hex())]
    CheckOut {
        /// The commit that was being checked out.
        commit_id: CommitId,
        /// The underlying error.
        #[source]
        source: CheckoutError,
    },
    /// The working copy is stale.
    #[error(transparent)]
    StaleWorkingCopy(#[from] StaleWorkingCopyError),
    /// The workspace environment couldn't be loaded.
    #[error(transparent)]
    Environment(#[from] WorkspaceEnvironmentError),
    /// Error from the commit backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// Error from the operation store.
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
    /// Error from the operation heads store.
    #[error(transparent)]
    OpHeadsStore(#[from] OpHeadsStoreError),
    /// Error loading the repo.
    #[error(transparent)]
    RepoLoader(#[from] RepoLoaderError),
    /// Error snapshotting the working copy.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// Error reading or writing the working-copy state.
    #[error(transparent)]
    WorkingCopyState(#[from] WorkingCopyStateError),
    /// Error evaluating a revset.
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
    /// Attempted to rewrite the root commit.
    #[error(transparent)]
    RewriteRootCommit(#[from] RewriteRootCommit),
    /// Error creating a new working-copy commit.
    #[error(transparent)]
    CheckOutCommit(#[from] CheckOutCommitError),
    /// Error resetting the working-copy state.
    #[error(transparent)]
    Reset(#[from] ResetError),
    /// Error creating a recovery commit.
    #[error(transparent)]
    RecoverWorkspace(#[from] RecoverWorkspaceError),
    /// Error committing a transaction.
    #[error(transparent)]
    TransactionCommit(#[from] TransactionCommitError),
    /// Error importing from Git.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitImport(#[from] jj_lib::git::GitImportError),
    /// Error exporting to Git.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitExport(#[from] jj_lib::git::GitExportError),
    /// Error resetting Git HEAD.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitResetHead(#[from] jj_lib::git::GitResetHeadError),
    /// The caller failed to handle a [`RunnerEvent`].
    #[error(transparent)]
    EventSink(Box<dyn std::error::Error + Send + Sync>),
}

/// The working copy can't be snapshotted because it's not at the repo's
/// operation.
#[derive(Debug, Error)]
pub enum StaleWorkingCopyError {
    /// The working copy was last updated at an operation that's older than the
    /// repo's operation.
    #[error("The working copy is stale (not updated since operation {})", wc_op_id.hex())]
    Stale {
        /// The operation the working copy was last updated at.
        wc_op_id: OperationId,
    },
    /// The working copy was last updated at an operation that's not an
    /// ancestor of the repo's operation. That operation may need to be
    /// integrated into the operation log.
    #[error(
        "The repo was loaded at operation {}, which seems to be a sibling of the working copy's \
         operation {}",
        repo_op_id.hex(),
        wc_op_id.hex()
    )]
    SiblingOperation {
        /// The operation the repo was loaded at.
        repo_op_id: OperationId,
        /// The operation the working copy was last updated at.
        wc_op_id: OperationId,
    },
    /// The working copy's operation couldn't be found.
    #[error("Could not read working copy's operation")]
    OperationNotFound(#[source] OpStoreError),
}

/// Why the working copy can't be updated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkingCopyNotWritableReason {
    /// [`RunnerOptions::ignore_working_copy`] is set.
    IgnoringWorkingCopy,
    /// [`RunnerOptions::integrate_operation`] is not set.
    NotIntegratingOperations,
    /// The repo was loaded at an operation other than the head operation.
    NotAtHeadOperation,
}

/// Runs the operations that commands (or other user interfaces) perform on a
/// workspace before and after modifying the repo: snapshotting the working
/// copy, importing from and exporting to Git, finishing transactions, and
/// updating the working copy.
pub struct WorkspaceOperationRunner {
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    env: Arc<WorkspaceEnvironment>,
    options: RunnerOptions,
    may_snapshot_working_copy: bool,
    may_update_working_copy: bool,
}

impl WorkspaceOperationRunner {
    /// Creates a runner for `workspace` with `repo` loaded. If
    /// `may_snapshot_working_copy` is false, the working copy will be neither
    /// snapshotted nor updated.
    pub fn new(
        workspace: Workspace,
        repo: Arc<ReadonlyRepo>,
        env: Arc<WorkspaceEnvironment>,
        options: RunnerOptions,
        may_snapshot_working_copy: bool,
    ) -> Self {
        let may_update_working_copy = may_snapshot_working_copy && options.integrate_operation;
        Self {
            workspace,
            repo,
            env,
            options,
            may_snapshot_working_copy,
            may_update_working_copy,
        }
    }

    /// Loads the repo for `workspace` at the head operation. If there are
    /// concurrent operations, they're merged if
    /// [`RunnerOptions::merge_concurrent_operations`] is set. Otherwise,
    /// [`RunnerError::ConcurrentOperations`] is returned.
    pub async fn load_at_head(
        events: &dyn RunnerEvents,
        workspace: Workspace,
        env: Arc<WorkspaceEnvironment>,
        options: RunnerOptions,
    ) -> Result<Self, RunnerError> {
        let repo_loader = workspace.repo_loader();
        let op_head =
            resolve_head_operation(events, repo_loader, workspace.workspace_name(), &options)
                .await?;
        let repo = repo_loader.load_at(&op_head).await?;
        let may_snapshot_working_copy = !options.ignore_working_copy;
        Self::load_at(
            events,
            workspace,
            repo,
            env,
            options,
            may_snapshot_working_copy,
        )
    }

    /// Creates a runner like [`Self::new()`], but first checks that the
    /// `trunk()` revset alias can be resolved in `repo`. If it can't, it's
    /// temporarily set to `root()` so revsets that use it can still be
    /// evaluated.
    pub fn load_at(
        events: &dyn RunnerEvents,
        workspace: Workspace,
        repo: Arc<ReadonlyRepo>,
        mut env: Arc<WorkspaceEnvironment>,
        options: RunnerOptions,
        may_snapshot_working_copy: bool,
    ) -> Result<Self, RunnerError> {
        apply_trunk_alias_fallback(events, &repo, &mut env)?;
        Ok(Self::new(
            workspace,
            repo,
            env,
            options,
            may_snapshot_working_copy,
        ))
    }

    /// Reloads the repo at the head operation, e.g. after another process
    /// has modified the repo. Concurrent operations are handled like in
    /// [`Self::load_at_head()`], and the `trunk()` revset alias is checked
    /// like in [`Self::load_at()`].
    pub async fn reload_at_head(&mut self, events: &dyn RunnerEvents) -> Result<(), RunnerError> {
        let repo_loader = self.workspace.repo_loader();
        let op_head = resolve_head_operation(
            events,
            repo_loader,
            self.workspace.workspace_name(),
            &self.options,
        )
        .await?;
        let repo = repo_loader.load_at(&op_head).await?;
        apply_trunk_alias_fallback(events, &repo, &mut self.env)?;
        self.repo = repo;
        Ok(())
    }

    /// The workspace.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns the workspace, e.g. to recover from a stale working copy with
    /// [`Self::recover_stale_working_copy()`].
    pub fn into_workspace(self) -> Workspace {
        self.workspace
    }

    /// Whether the working copy may be snapshotted.
    pub fn may_snapshot_working_copy(&self) -> bool {
        self.may_snapshot_working_copy
    }

    /// The current repo.
    pub fn repo(&self) -> &Arc<ReadonlyRepo> {
        &self.repo
    }

    /// The workspace environment.
    pub fn env(&self) -> &Arc<WorkspaceEnvironment> {
        &self.env
    }

    /// The options this runner was created with.
    pub fn options(&self) -> &RunnerOptions {
        &self.options
    }

    /// Sets [`RunnerOptions::ignore_immutable`], e.g. for a single
    /// transaction where the user has chosen to rewrite immutable commits.
    pub fn set_ignore_immutable(&mut self, ignore_immutable: bool) {
        self.options.ignore_immutable = ignore_immutable;
    }

    /// The name of the workspace.
    pub fn workspace_name(&self) -> &WorkspaceName {
        self.workspace.workspace_name()
    }

    /// The root directory of the workspace.
    pub fn workspace_root(&self) -> &Path {
        self.workspace.workspace_root()
    }

    /// Settings for this workspace.
    pub fn settings(&self) -> &UserSettings {
        self.workspace.settings()
    }

    /// Starts a transaction on the current repo, with this workspace's name
    /// and the operation attributes from [`RunnerOptions`].
    pub fn start_transaction(&self) -> Transaction {
        self.options
            .start_transaction(&self.repo, self.workspace.workspace_name())
    }

    /// Finishes a transaction started by [`Self::start_transaction()`]:
    /// rebases descendants of rewritten commits, exports to Git, commits the
    /// operation, and updates the working copy.
    pub async fn finish_transaction(
        &mut self,
        events: &dyn RunnerEvents,
        mut tx: Transaction,
        description: impl Into<String>,
    ) -> Result<(), RunnerError> {
        let num_rebased = self.rebase_mutable_descendants(&mut tx).await?;
        if num_rebased > 0 {
            events
                .event(RunnerEvent::DescendantsRebased {
                    num_rebased,
                    reason: RebaseReason::RewrittenCommits,
                })
                .map_err(RunnerError::EventSink)?;
        }
        // Acquire git import/export lock before finishing the transaction to ensure
        // Git HEAD export happens atomically with the transaction commit.
        let git_import_export_lock = self.lock_git_import_export()?;
        self.finish_transaction_locked(events, tx, description, &git_import_export_lock)
            .await
    }

    /// Recovers a stale working copy in `workspace`. See
    /// [`Self::recover_stale()`]. Returns the runner at the head operation and
    /// the stats from snapshotting.
    pub async fn recover_stale_working_copy(
        events: &dyn RunnerEvents,
        workspace: Workspace,
        env: Arc<WorkspaceEnvironment>,
        options: RunnerOptions,
        config: &SnapshotConfig,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(Self, SnapshotStats), RunnerError> {
        // Start at the working copy's operation if possible, so
        // recover_stale() doesn't have to load the repo again.
        let repo_loader = workspace.repo_loader();
        let op = match repo_loader
            .load_operation(workspace.working_copy().operation_id())
            .await
        {
            Ok(op) => op,
            Err(_) => repo_loader.root_operation().await,
        };
        let repo = repo_loader.load_at(&op).await?;
        let may_snapshot_working_copy = !options.ignore_working_copy;
        let mut runner = Self::new(workspace, repo, env, options, may_snapshot_working_copy);
        let stats = runner
            .recover_stale(events, config, git_import_export_lock)
            .await?;
        Ok((runner, stats))
    }

    /// Recovers a stale working copy. The working copy is snapshotted on top
    /// of the operation it was last updated at. Then the repo is loaded at the
    /// head operation and the working copy is updated to the working-copy
    /// commit there. If the working copy's operation can't be found, a
    /// recovery commit with the working copy's contents is created instead.
    /// Returns the stats from snapshotting.
    pub async fn recover_stale(
        &mut self,
        events: &dyn RunnerEvents,
        config: &SnapshotConfig,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<SnapshotStats, RunnerError> {
        let repo_loader = self.workspace.repo_loader();
        let op_id = self.workspace.working_copy().operation_id();
        match repo_loader.load_operation(op_id).await {
            Ok(op) => {
                if self.repo.op_id() != op.id() {
                    self.repo = repo_loader.load_at(&op).await?;
                }
                self.check_working_copy_writable()?;

                // Snapshot the current working copy on top of the last known working-copy
                // operation, then merge the divergent operations. The wc_commit_id of the
                // merged repo wouldn't change because the old one wins, but it's probably
                // fine if we picked the new wc_commit_id.
                let stale_stats = self
                    .snapshot_working_copy(events, config, git_import_export_lock)
                    .await?;

                let stale_wc_commit = self.get_wc_commit().await?.unwrap();

                self.reload_at_head(events).await?;
                self.update_stale_working_copy(events, &stale_wc_commit, git_import_export_lock)
                    .await?;

                // There may be Git refs to import, so snapshot again. Git HEAD
                // will also be imported if it was updated after the working
                // copy became stale. The result wouldn't be ideal, but there
                // should be no data loss at least.
                let fresh_stats = self
                    .snapshot(events, config, git_import_export_lock)
                    .await?;
                let SnapshotStats {
                    mut untracked_paths,
                    mut invalid_utf8_paths,
                } = stale_stats;
                untracked_paths.extend(fresh_stats.untracked_paths);
                invalid_utf8_paths.extend(fresh_stats.invalid_utf8_paths);
                Ok(SnapshotStats {
                    untracked_paths,
                    invalid_utf8_paths,
                })
            }
            Err(err @ OpStoreError::ObjectNotFound { .. }) => {
                events
                    .event(RunnerEvent::WorkingCopyOperationNotFound(&err))
                    .map_err(RunnerError::EventSink)?;
                self.reload_at_head(events).await?;
                self.create_and_check_out_recovery_commit(events, config, git_import_export_lock)
                    .await
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Updates the stale working copy from `stale_wc_commit` to the current
    /// working-copy commit.
    async fn update_stale_working_copy(
        &mut self,
        events: &dyn RunnerEvents,
        stale_wc_commit: &Commit,
        #[cfg_attr(not(feature = "git"), expect(unused_variables))]
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), RunnerError> {
        let desired_wc_commit = self.prepare_working_copy_mutation().await?;
        let mut locked_ws = self.workspace.start_working_copy_mutation().await?;
        match WorkingCopyFreshness::check_stale(
            locked_ws.locked_wc(),
            &desired_wc_commit,
            &self.repo,
        )
        .await?
        {
            WorkingCopyFreshness::Fresh | WorkingCopyFreshness::Updated(_) => {
                drop(locked_ws);
                events
                    .event(RunnerEvent::WorkingCopyNotStale)
                    .map_err(RunnerError::EventSink)?;
            }
            WorkingCopyFreshness::WorkingCopyStale | WorkingCopyFreshness::SiblingOperation => {
                // Reset Git HEAD first if the repo is colocated
                #[cfg(feature = "git")]
                if self.env.working_copy_shared_with_git && self.options.integrate_operation {
                    let workspace_name = self.env.workspace_name();
                    let workspace_root = self.env.workspace_root();
                    let mut tx = self.options.start_transaction(&self.repo, workspace_name);
                    try_reset_git_head(
                        events,
                        tx.repo_mut(),
                        workspace_name,
                        workspace_root,
                        &desired_wc_commit,
                        git_import_export_lock,
                    )
                    .await?;
                    if tx.repo().has_changes() {
                        self.repo = self
                            .options
                            .commit_transaction(tx, "reset git head")
                            .await?;
                    }
                }

                let stats = update_stale_working_copy(
                    locked_ws,
                    self.repo.op_id().clone(),
                    stale_wc_commit,
                    &desired_wc_commit,
                )
                .await?;
                events
                    .event(RunnerEvent::WorkingCopyUpdated {
                        repo: &self.repo,
                        old_commit: Some(stale_wc_commit),
                        new_commit: &desired_wc_commit,
                        stats: &stats,
                    })
                    .map_err(RunnerError::EventSink)?;
                events
                    .event(RunnerEvent::StaleWorkingCopyUpdated {
                        new_commit: &desired_wc_commit,
                    })
                    .map_err(RunnerError::EventSink)?;
            }
        }
        Ok(())
    }

    /// Creates a commit with the contents of the working copy on top of the
    /// current working-copy commit and checks it out. This is used when the
    /// operation the working copy was last updated at has been lost.
    async fn create_and_check_out_recovery_commit(
        &mut self,
        events: &dyn RunnerEvents,
        config: &SnapshotConfig,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<SnapshotStats, RunnerError> {
        self.check_working_copy_writable()?;

        let workspace_name = self.workspace.workspace_name().to_owned();
        let mut locked_ws = self.workspace.start_working_copy_mutation().await?;
        let (repo, new_commit) = working_copy::create_and_check_out_recovery_commit(
            locked_ws.locked_wc(),
            &self.repo,
            workspace_name,
            "RECOVERY COMMIT FROM `jj workspace update-stale`

This commit contains changes that were written to the working copy by an
operation that was subsequently lost (or was at least unavailable when you ran
`jj workspace update-stale`). Because the operation was lost, we don't know
what the parent commits are supposed to be. That means that the diff compared
to the current parents may contain changes from multiple commits.
",
        )
        .await?;

        events
            .event(RunnerEvent::RecoveryCommitCreated {
                commit: &new_commit,
            })
            .map_err(RunnerError::EventSink)?;
        locked_ws.finish(repo.op_id().clone()).await?;
        self.repo = repo;

        self.snapshot(events, config, git_import_export_lock).await
    }

    /// Returns an error if any of `commits` are immutable, unless
    /// [`RunnerOptions::ignore_immutable`] is set (the root commit is always
    /// immutable).
    pub async fn check_rewritable<'a>(
        &self,
        commits: impl IntoIterator<Item = &'a CommitId>,
    ) -> Result<(), RunnerError> {
        let commit_ids = commits.into_iter().cloned().collect_vec();
        let to_rewrite_expr = RevsetExpression::commits(commit_ids);
        self.check_rewritable_expr(&to_rewrite_expr).await
    }

    /// Returns an error if any of the commits in `to_rewrite_expr` are
    /// immutable, unless [`RunnerOptions::ignore_immutable`] is set (the root
    /// commit is always immutable).
    pub async fn check_rewritable_expr(
        &self,
        to_rewrite_expr: &Arc<ResolvedRevsetExpression>,
    ) -> Result<(), RunnerError> {
        let repo = self.repo.as_ref();
        let immutable_expr = self
            .env
            .resolve_immutable_expression(repo, self.options.ignore_immutable)?;
        let Some(commit_id) = immutable_expr
            .intersection(to_rewrite_expr)
            .evaluate(repo)?
            .stream()
            .try_next()
            .await?
        else {
            return Ok(());
        };
        if &commit_id == repo.store().root_commit_id() {
            return Err(RunnerError::RootCommitImmutable { commit_id });
        }
        let num_immutable = immutable_expr
            .intersection(&to_rewrite_expr.descendants())
            .evaluate(repo)?
            .count_estimate()
            .await?;
        Err(RunnerError::ImmutableCommit {
            commit_id,
            num_immutable,
        })
    }

    /// Rebases descendants of rewritten commits, except those that are
    /// immutable.
    pub async fn rebase_mutable_descendants(
        &self,
        tx: &mut Transaction,
    ) -> Result<usize, RunnerError> {
        rebase_mutable_descendants(&self.env, &self.options, tx).await
    }

    /// Returns an error if the working copy can't be updated, e.g. because
    /// [`RunnerOptions::ignore_working_copy`] is set.
    pub fn check_working_copy_writable(&self) -> Result<(), RunnerError> {
        if self.may_update_working_copy {
            Ok(())
        } else {
            let reason = if self.options.ignore_working_copy {
                WorkingCopyNotWritableReason::IgnoringWorkingCopy
            } else if !self.options.integrate_operation {
                WorkingCopyNotWritableReason::NotIntegratingOperations
            } else {
                WorkingCopyNotWritableReason::NotAtHeadOperation
            };
            Err(RunnerError::WorkingCopyNotWritable(reason))
        }
    }

    /// Acquires a lock for git import/export operations if the workspace is
    /// colocated with Git. Returns a token that can be passed to functions
    /// that need to import from or export to Git. For non-colocated repos,
    /// returns a token with no lock inside.
    pub fn lock_git_import_export(&self) -> Result<GitImportExportLock, RunnerError> {
        self.env.lock_git_import_export(&self.workspace)
    }

    /// Snapshots the working copy, importing Git HEAD and refs first and after
    /// if the working copy is shared with Git.
    ///
    /// The returned [`SnapshotStats`] include files that weren't tracked, which
    /// should usually be presented to the user.
    #[instrument(skip_all)]
    pub async fn snapshot(
        &mut self,
        events: &dyn RunnerEvents,
        config: &SnapshotConfig,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<SnapshotStats, RunnerError> {
        assert!(self.may_snapshot_working_copy);
        #[cfg(feature = "git")]
        if self.env.working_copy_shared_with_git {
            self.import_git_head(events, git_import_export_lock).await?;
        }
        // Because the Git refs (except HEAD) aren't imported yet, the ref
        // pointing to the new working-copy commit might not be exported.
        // In that situation, the ref would be conflicted anyway, so export
        // failure is okay.
        let stats = self
            .snapshot_working_copy(events, config, git_import_export_lock)
            .await?;

        // import_git_refs() can rebase the working-copy commit.
        #[cfg(feature = "git")]
        if let Some(import_options) = &config.git_import_options {
            self.import_git_refs(events, import_options, git_import_export_lock)
                .await?;
        }
        Ok(stats)
    }

    /// Imports new HEAD from the colocated Git repo.
    ///
    /// If the Git HEAD has changed, this function checks out the new Git HEAD.
    /// The old working-copy commit will be abandoned if it's discardable. The
    /// working-copy state will be reset to point to the new Git HEAD. The
    /// working-copy contents won't be updated.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    async fn import_git_head(
        &mut self,
        events: &dyn RunnerEvents,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), RunnerError> {
        assert!(self.may_snapshot_working_copy);
        let workspace_name = self.workspace_name().to_owned();
        let workspace_root = self.workspace_root().to_owned();
        let mut tx = self.start_transaction();
        jj_lib::git::import_head(tx.repo_mut(), &workspace_name, &workspace_root).await?;
        if !tx.repo().has_changes() {
            return Ok(());
        }

        let old_git_head = self.repo().view().git_head(&workspace_name).clone();
        let new_git_head = tx.repo().view().git_head(&workspace_name);
        if let Some(new_git_head_id) = new_git_head.as_normal() {
            let new_git_head_commit = tx.repo().store().get_commit_async(new_git_head_id).await?;
            let wc_commit = tx
                .repo_mut()
                .check_out(workspace_name, &new_git_head_commit)
                .await?;
            let mut locked_ws = self.workspace.start_working_copy_mutation().await?;
            // The working copy was presumably updated by the git command that updated
            // HEAD, so we just need to reset our working copy
            // state to it without updating working copy files.
            locked_ws.locked_wc().reset(&wc_commit).await?;
            tx.repo_mut().rebase_descendants().await?;
            let repo = self
                .options
                .commit_transaction(tx, "import git head")
                .await?;
            self.repo = repo;
            if self.options.integrate_operation {
                locked_ws.finish(self.repo.op_id().clone()).await?;
            }
            events
                .event(RunnerEvent::GitHeadImported {
                    first_import: old_git_head.is_absent(),
                })
                .map_err(RunnerError::EventSink)?;
            if !self.options.integrate_operation {
                events
                    .event(RunnerEvent::OperationLeftUnpublished {
                        op_id: self.repo().op_id(),
                        is_snapshot: false,
                    })
                    .map_err(RunnerError::EventSink)?;
            }
        } else {
            // Unlikely, but the HEAD ref got deleted by git?
            let num_rebased = tx.repo_mut().rebase_descendants().await?;
            if num_rebased > 0 {
                events
                    .event(RunnerEvent::DescendantsRebased {
                        num_rebased,
                        reason: RebaseReason::RewrittenCommits,
                    })
                    .map_err(RunnerError::EventSink)?;
            }
            self.finish_transaction_locked(events, tx, "import git head", git_import_export_lock)
                .await?;
        }
        Ok(())
    }

    /// Imports branches and tags from the underlying Git repo, abandons old
    /// bookmarks.
    ///
    /// If the working-copy branch is rebased, and if update is allowed, the
    /// new working-copy commit will be checked out.
    ///
    /// This function does not import the Git HEAD, but the HEAD may be reset to
    /// the working copy parent if the repository is colocated.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    async fn import_git_refs(
        &mut self,
        events: &dyn RunnerEvents,
        import_options: &jj_lib::git::GitImportOptions,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), RunnerError> {
        let mut tx = self.start_transaction();
        let stats = jj_lib::git::import_refs(tx.repo_mut(), import_options).await?;
        events
            .event(RunnerEvent::GitRefsImported(&stats))
            .map_err(RunnerError::EventSink)?;
        if !tx.repo().has_changes() {
            return Ok(());
        }

        let num_rebased = self.rebase_mutable_descendants(&mut tx).await?;
        if num_rebased > 0 {
            events
                .event(RunnerEvent::DescendantsRebased {
                    num_rebased,
                    reason: RebaseReason::RewrittenInGit,
                })
                .map_err(RunnerError::EventSink)?;
        }
        self.finish_transaction_locked(events, tx, "import git refs", git_import_export_lock)
            .await?;
        events
            .event(RunnerEvent::GitRefsImportFinished)
            .map_err(RunnerError::EventSink)?;
        Ok(())
    }

    async fn prepare_working_copy_mutation(&self) -> Result<Commit, RunnerError> {
        self.check_working_copy_writable()?;
        self.get_wc_commit()
            .await?
            .ok_or(RunnerError::NothingCheckedOut)
    }

    /// Locks the working copy for modification. Returns the locked workspace
    /// and the working-copy commit.
    pub async fn start_working_copy_mutation(
        &mut self,
    ) -> Result<(LockedWorkspace<'_>, Commit), RunnerError> {
        let wc_commit = self.prepare_working_copy_mutation().await?;
        let mut locked_ws = self.workspace.start_working_copy_mutation().await?;
        if wc_commit.tree().tree_ids_and_labels()
            != locked_ws.locked_wc().old_tree().tree_ids_and_labels()
        {
            return Err(RunnerError::ConcurrentWorkingCopyOperation);
        }
        Ok((locked_ws, wc_commit))
    }

    /// The ID of the workspace's working-copy commit in the current repo.
    pub fn get_wc_commit_id(&self) -> Option<&CommitId> {
        self.repo().view().get_wc_commit_id(self.workspace_name())
    }

    /// The workspace's working-copy commit in the current repo.
    pub async fn get_wc_commit(&self) -> BackendResult<Option<Commit>> {
        if let Some(id) = self.get_wc_commit_id() {
            let commit = self.repo().store().get_commit_async(id).await?;
            Ok(Some(commit))
        } else {
            Ok(None)
        }
    }

    /// Snapshots the working copy without importing from Git. Most callers
    /// should use [`Self::snapshot()`] instead.
    #[instrument(skip_all)]
    pub async fn snapshot_working_copy(
        &mut self,
        events: &dyn RunnerEvents,
        config: &SnapshotConfig,
        #[cfg_attr(not(feature = "git"), expect(unused_variables))]
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<SnapshotStats, RunnerError> {
        let workspace_name = self.workspace_name().to_owned();
        let repo = self.repo().clone();
        let options = config.snapshot_options();

        // Compare working-copy tree and operation with repo's, and reload as needed.
        let mut locked_ws = self.workspace.start_working_copy_mutation().await?;

        let Some((repo, wc_commit)) =
            handle_stale_working_copy(locked_ws.locked_wc(), repo, &workspace_name).await?
        else {
            // If the workspace has been deleted, it's unclear what to do, so we just skip
            // committing the working copy.
            return Ok(SnapshotStats::default());
        };

        self.repo = repo;
        let (new_tree, stats) = {
            let mut options = options;
            let progress = events.snapshot_progress();
            options.progress = progress.as_deref();
            locked_ws.locked_wc().snapshot(&options).await?
        };
        if new_tree.tree_ids_and_labels() != wc_commit.tree().tree_ids_and_labels() {
            let mut tx = self.options.start_transaction(&self.repo, &workspace_name);
            tx.set_is_snapshot(true);
            let immutable_expr = self
                .env
                .resolve_immutable_expression(tx.repo(), self.options.ignore_immutable)?;
            let wc_immutable = !immutable_expr
                .intersection(&RevsetExpression::commit(wc_commit.id().clone()))
                .evaluate(tx.repo())?
                .is_empty()
                .await?;
            let mut_repo = tx.repo_mut();
            let new_wc_commit;
            if wc_immutable {
                new_wc_commit = mut_repo
                    .new_commit(vec![wc_commit.id().clone()], new_tree.clone())
                    .write()
                    .await?;
                events
                    .event(RunnerEvent::SnapshotOnImmutableWorkingCopy {
                        new_wc_commit: &new_wc_commit,
                    })
                    .map_err(RunnerError::EventSink)?;
            } else {
                new_wc_commit = mut_repo
                    .rewrite_commit(&wc_commit)
                    .set_tree(new_tree.clone())
                    .write()
                    .await?;
            }
            mut_repo.set_wc_commit(workspace_name.clone(), new_wc_commit.id().clone())?;

            // Rebase descendants
            let num_rebased = mut_repo.rebase_descendants().await?;
            if num_rebased > 0 {
                events
                    .event(RunnerEvent::DescendantsRebased {
                        num_rebased,
                        reason: RebaseReason::WorkingCopySnapshot,
                    })
                    .map_err(RunnerError::EventSink)?;
            }

            #[cfg(feature = "git")]
            if self.env.working_copy_shared_with_git && self.options.integrate_operation {
                let workspace_root = self.env.workspace_root();
                if wc_immutable {
                    // New working-copy commit is created on top. Reset Git HEAD and index.
                    try_reset_git_head(
                        events,
                        mut_repo,
                        &workspace_name,
                        workspace_root,
                        &new_wc_commit,
                        git_import_export_lock,
                    )
                    .await?;
                    // export_refs() is probably unnecessary because there should be no
                    // rewritten descendants, but it's harmless.
                    let stats = jj_lib::git::export_refs(mut_repo)?;
                    events
                        .event(RunnerEvent::GitRefsExported(&stats))
                        .map_err(RunnerError::EventSink)?;
                } else {
                    let old_tree = wc_commit.tree();
                    let new_tree = new_wc_commit.tree();
                    jj_lib::git::update_intent_to_add(
                        mut_repo.base_repo().as_ref(),
                        workspace_root,
                        &old_tree,
                        &new_tree,
                    )
                    .await?;
                    let stats = jj_lib::git::export_refs(mut_repo)?;
                    events
                        .event(RunnerEvent::GitRefsExported(&stats))
                        .map_err(RunnerError::EventSink)?;
                }
            }

            let repo = self
                .options
                .commit_transaction(tx, "snapshot working copy")
                .await?;
            self.repo = repo;
            if !self.options.integrate_operation {
                events
                    .event(RunnerEvent::OperationLeftUnpublished {
                        op_id: self.repo.op_id(),
                        is_snapshot: true,
                    })
                    .map_err(RunnerError::EventSink)?;
            }
        }

        #[cfg(feature = "git")]
        if self.env.working_copy_shared_with_git
            && let Ok(resolved_tree) = new_tree.trees().await?.into_resolved()
            && resolved_tree
                .entries_non_recursive()
                .any(|entry| entry.name().as_internal_str().starts_with(".jjconflict"))
        {
            events
                .event(RunnerEvent::JjConflictFilesInWorkingCopy)
                .map_err(RunnerError::EventSink)?;
        }

        if self.options.integrate_operation {
            locked_ws.finish(self.repo.op_id().clone()).await?;
        }
        Ok(stats)
    }

    async fn finish_transaction_locked(
        &mut self,
        events: &dyn RunnerEvents,
        mut tx: Transaction,
        description: impl Into<String>,
        #[cfg_attr(not(feature = "git"), expect(unused_variables))]
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), RunnerError> {
        let old_repo = tx.base_repo().clone();

        let maybe_old_wc_commit = old_repo
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.base_repo().store().get_commit(commit_id))
            .transpose()?;
        let maybe_new_wc_commit = tx
            .repo()
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.repo().store().get_commit(commit_id))
            .transpose()?;
        // Create a new mutable working-copy commit to reduce unintended states.
        // This isn't strictly required for correctness, so symbol resolution
        // failures can be ignored. snapshot_working_copy() ensures that the
        // working-copy commit is mutable.
        let maybe_new_wc_commit = if let Some(wc_commit) = &maybe_new_wc_commit
            && let Ok(immutable_expr) = self
                .env
                .resolve_immutable_expression(tx.repo(), self.options.ignore_immutable)
            && !immutable_expr
                .intersection(&RevsetExpression::commit(wc_commit.id().clone()))
                .evaluate(tx.repo())?
                .is_empty()
                .await?
        {
            let new_wc_commit = tx
                .repo_mut()
                .new_commit(vec![wc_commit.id().clone()], wc_commit.tree())
                .write()
                .await?;
            tx.repo_mut()
                .set_wc_commit(self.workspace_name().to_owned(), new_wc_commit.id().clone())?;
            events
                .event(RunnerEvent::WorkingCopyBecameImmutable {
                    new_wc_commit: &new_wc_commit,
                })
                .map_err(RunnerError::EventSink)?;
            Some(new_wc_commit)
        } else {
            maybe_new_wc_commit
        };

        #[cfg(feature = "git")]
        if self.env.working_copy_shared_with_git && self.options.integrate_operation {
            if let Some(wc_commit) = &maybe_new_wc_commit {
                try_reset_git_head(
                    events,
                    tx.repo_mut(),
                    self.workspace_name(),
                    self.workspace_root(),
                    wc_commit,
                    git_import_export_lock,
                )
                .await?;
            }
            let stats = jj_lib::git::export_refs(tx.repo_mut())?;
            events
                .event(RunnerEvent::GitRefsExported(&stats))
                .map_err(RunnerError::EventSink)?;
        }

        let repo = self.options.commit_transaction(tx, description).await?;
        self.repo = repo;

        // Update working copy before reporting repo changes, so that
        // potential errors while reporting changes (broken pipe, etc)
        // don't leave the working copy in a stale state.
        if self.may_update_working_copy {
            if let Some(new_commit) = &maybe_new_wc_commit {
                let stats = update_working_copy(
                    &self.repo,
                    &mut self.workspace,
                    maybe_old_wc_commit.as_ref(),
                    new_commit,
                )
                .await?;
                events
                    .event(RunnerEvent::WorkingCopyUpdated {
                        repo: &self.repo,
                        old_commit: maybe_old_wc_commit.as_ref(),
                        new_commit,
                        stats: &stats,
                    })
                    .map_err(RunnerError::EventSink)?;
            } else {
                // It seems the workspace was deleted, so we shouldn't try to
                // update it.
            }
        }

        events
            .event(RunnerEvent::TransactionFinished {
                old_repo: &old_repo,
                new_repo: &self.repo,
            })
            .map_err(RunnerError::EventSink)?;

        if !self.options.integrate_operation {
            events
                .event(RunnerEvent::OperationLeftUnpublished {
                    op_id: self.repo().op_id(),
                    is_snapshot: false,
                })
                .map_err(RunnerError::EventSink)?;
        }

        let settings = self.settings();
        let missing_name = settings.user_name().is_empty();
        let missing_email = settings.user_email().is_empty();
        if missing_name || missing_email {
            events
                .event(RunnerEvent::UserIdentityNotConfigured {
                    missing_name,
                    missing_email,
                })
                .map_err(RunnerError::EventSink)?;
        }
        Ok(())
    }
}

/// Resolves the head operation of the operation log. If there are concurrent
/// operations, they're merged if [`RunnerOptions::merge_concurrent_operations`]
/// is set. Otherwise, [`RunnerError::ConcurrentOperations`] is returned.
pub async fn resolve_head_operation(
    events: &dyn RunnerEvents,
    repo_loader: &RepoLoader,
    workspace_name: &WorkspaceName,
    options: &RunnerOptions,
) -> Result<Operation, RunnerError> {
    op_heads_store::resolve_op_heads(
        repo_loader.op_heads_store().as_ref(),
        repo_loader.op_store(),
        async |op_heads| -> Result<Operation, RunnerError> {
            if !options.merge_concurrent_operations {
                return Err(RunnerError::ConcurrentOperations {
                    op_ids: op_heads.iter().map(|op| op.id().clone()).collect(),
                });
            }
            events
                .event(RunnerEvent::MergingConcurrentOperations {
                    operations: &op_heads,
                })
                .map_err(RunnerError::EventSink)?;
            let (merged_repo, num_rebased) = repo_loader
                .merge_operations(
                    op_heads,
                    Some(workspace_name),
                    Some("reconcile divergent operations"),
                    options.transaction_attributes.iter().cloned(),
                )
                .await?;
            if num_rebased > 0 {
                events
                    .event(RunnerEvent::DescendantsRebased {
                        num_rebased,
                        reason: RebaseReason::ConcurrentOperations,
                    })
                    .map_err(RunnerError::EventSink)?;
            }
            Ok(merged_repo.operation().clone())
        },
    )
    .await
}

/// Configuration used when snapshotting the working copy and importing changes
/// from Git.
pub struct SnapshotConfig {
    /// Matches new files that should be tracked automatically.
    pub auto_tracking_matcher: Box<dyn Matcher>,
    /// Ignore patterns in addition to the working copy's `.gitignore` files.
    pub base_ignores: Arc<GitIgnoreFile>,
    /// New files larger than this aren't tracked automatically.
    pub max_new_file_size: u64,
    /// Options for importing Git refs. Only present if the working copy is
    /// shared with Git.
    #[cfg(feature = "git")]
    pub git_import_options: Option<jj_lib::git::GitImportOptions>,
}

/// Non-fatal problems found while loading a [`SnapshotConfig`].
#[derive(Debug)]
pub struct SnapshotConfigDiagnostics {
    /// Diagnostics from parsing `snapshot.auto-track`.
    pub auto_track: FilesetDiagnostics,
    /// Diagnostics from parsing `remotes.<name>.auto-track-bookmarks`.
    pub remote_settings: Vec<RemoteSettingDiagnostics>,
}

impl SnapshotConfigDiagnostics {
    /// Creates empty diagnostics.
    pub fn new() -> Self {
        Self {
            auto_track: FilesetDiagnostics::new(),
            remote_settings: vec![],
        }
    }
}

impl Default for SnapshotConfigDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

/// Error from loading a [`SnapshotConfig`].
#[derive(Debug, Error)]
pub enum SnapshotConfigError {
    /// A config value couldn't be read.
    #[error(transparent)]
    Config(#[from] ConfigGetError),
    /// The workspace configuration couldn't be loaded.
    #[error(transparent)]
    Environment(#[from] WorkspaceEnvironmentError),
    /// An ignore file couldn't be loaded.
    #[error(transparent)]
    GitIgnore(#[from] GitIgnoreError),
    /// A `remotes.<name>` setting couldn't be parsed.
    #[error(transparent)]
    RemoteSetting(#[from] RemoteSettingParseError),
}

impl SnapshotConfig {
    /// Loads the configuration from the settings of the workspace in `env`.
    /// `store` is used for finding the Git repo's ignore files, and `home_dir`
    /// for expanding `~` in Git's `core.excludesFile`.
    pub fn load(
        diagnostics: &mut SnapshotConfigDiagnostics,
        env: &WorkspaceEnvironment,
        store: &Store,
        home_dir: Option<&Path>,
    ) -> Result<Self, SnapshotConfigError> {
        let auto_tracking_matcher = env.auto_tracking_matcher(&mut diagnostics.auto_track)?;
        let base_ignores = env.base_ignores(store, home_dir)?;
        let max_new_file_size = env.max_new_file_size()?;
        #[cfg(feature = "git")]
        let git_import_options = if env.working_copy_shared_with_git() {
            let settings = env.settings();
            let git_settings = jj_lib::git::GitSettings::from_settings(settings)?;
            let remote_settings = settings.remote_settings()?;
            Some(crate::git_util::load_git_import_options(
                &mut diagnostics.remote_settings,
                &git_settings,
                &remote_settings,
            )?)
        } else {
            None
        };
        Ok(Self {
            auto_tracking_matcher,
            base_ignores,
            max_new_file_size,
            #[cfg(feature = "git")]
            git_import_options,
        })
    }

    fn snapshot_options(&self) -> SnapshotOptions<'_> {
        SnapshotOptions {
            base_ignores: self.base_ignores.clone(),
            progress: None,
            start_tracking_matcher: self.auto_tracking_matcher.as_ref(),
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size: self.max_new_file_size,
        }
    }
}

#[cfg(feature = "git")]
async fn try_reset_git_head(
    events: &dyn RunnerEvents,
    mut_repo: &mut MutableRepo,
    workspace_name: &WorkspaceName,
    workspace_root: &Path,
    wc_commit: &Commit,
    _git_import_export_lock: &GitImportExportLock,
) -> Result<(), RunnerError> {
    // Export Git HEAD while holding the git-head lock to prevent races:
    // - Between two finish_transaction calls updating HEAD
    // - With import_git_head importing HEAD concurrently
    // This can still fail if HEAD was updated concurrently by another JJ process
    // (overlapping transaction) or a non-JJ process (e.g., git checkout). In that
    // case, the actual state will be imported on the next snapshot.
    match jj_lib::git::reset_head(mut_repo, workspace_name, workspace_root, wc_commit).await {
        Ok(()) => Ok(()),
        Err(err @ jj_lib::git::GitResetHeadError::UpdateHeadRef(_)) => {
            events
                .event(RunnerEvent::GitHeadResetFailed(&err))
                .map_err(RunnerError::EventSink)?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// Checks that the `trunk()` revset alias can be resolved in `repo`. If it
/// can't, it's temporarily set to `root()` in `env` so revsets that use it can
/// still be evaluated.
fn apply_trunk_alias_fallback(
    events: &dyn RunnerEvents,
    repo: &ReadonlyRepo,
    env: &mut Arc<WorkspaceEnvironment>,
) -> Result<(), RunnerError> {
    if let Err(error) =
        crate::revset_util::try_resolve_trunk_alias(repo, &env.revset_parse_context())
    {
        // The fallback can be builtin_trunk() if we're willing to support
        // inferred trunk forever. (#7990)
        let fallback = "root()";
        events
            .event(RunnerEvent::TrunkAliasFallback {
                error: &error,
                fallback,
            })
            .map_err(RunnerError::EventSink)?;
        let env = Arc::make_mut(env);
        env.revset_aliases_map
            .insert("trunk()", fallback, None)
            .expect("valid syntax");
        let mut diagnostics = WorkspaceEnvironmentDiagnostics::new();
        let result = env.reload_revset_expressions(&mut diagnostics);
        events
            .event(RunnerEvent::EnvironmentDiagnostics(&diagnostics))
            .map_err(RunnerError::EventSink)?;
        result?;
    }
    Ok(())
}

async fn rebase_mutable_descendants(
    env: &WorkspaceEnvironment,
    options: &RunnerOptions,
    tx: &mut Transaction,
) -> Result<usize, RunnerError> {
    // Commands like "jj git fetch" can update immutable commits to reflect the
    // remote changes. Their immutable descendants shouldn't be rebased. We use
    // tx.base_repo() here because we're interested in existing immutable
    // commits that are still reachable.
    let mut num_rebased = 0;
    let immutable =
        env.resolve_immutable_expression(tx.base_repo().as_ref(), options.ignore_immutable)?;
    tx.repo_mut()
        .rebase_descendants_with_options(
            &immutable,
            &RebaseOptions::default(),
            |_old_commit, _rebased_commit| num_rebased += 1,
        )
        .await?;
    Ok(num_rebased)
}

/// Check if the working copy is stale and reload the repo if the repo is ahead
/// of the working copy.
///
/// Returns Ok(None) if the workspace doesn't exist in the repo (presumably
/// because it was deleted).
async fn handle_stale_working_copy(
    locked_wc: &mut dyn LockedWorkingCopy,
    repo: Arc<ReadonlyRepo>,
    workspace_name: &WorkspaceName,
) -> Result<Option<(Arc<ReadonlyRepo>, Commit)>, RunnerError> {
    let get_wc_commit = |repo: &ReadonlyRepo| -> BackendResult<Option<_>> {
        repo.view()
            .get_wc_commit_id(workspace_name)
            .map(|id| repo.store().get_commit(id))
            .transpose()
    };
    let Some(wc_commit) = get_wc_commit(&repo)? else {
        return Ok(None);
    };
    let old_op_id = locked_wc.old_operation_id().clone();
    match WorkingCopyFreshness::check_stale(locked_wc, &wc_commit, &repo).await {
        Ok(WorkingCopyFreshness::Fresh) => Ok(Some((repo, wc_commit))),
        Ok(WorkingCopyFreshness::Updated(wc_operation)) => {
            let repo = repo.reload_at(&wc_operation).await?;
            if let Some(wc_commit) = get_wc_commit(&repo)? {
                Ok(Some((repo, wc_commit)))
            } else {
                Ok(None)
            }
        }
        Ok(WorkingCopyFreshness::WorkingCopyStale) => Err(StaleWorkingCopyError::Stale {
            wc_op_id: old_op_id,
        }
        .into()),
        Ok(WorkingCopyFreshness::SiblingOperation) => {
            Err(StaleWorkingCopyError::SiblingOperation {
                repo_op_id: repo.op_id().clone(),
                wc_op_id: old_op_id,
            }
            .into())
        }
        Err(err @ OpStoreError::ObjectNotFound { .. }) => {
            Err(StaleWorkingCopyError::OperationNotFound(err).into())
        }
        Err(err) => Err(err.into()),
    }
}

async fn update_stale_working_copy(
    mut locked_ws: LockedWorkspace<'_>,
    op_id: OperationId,
    stale_commit: &Commit,
    new_commit: &Commit,
) -> Result<CheckoutStats, RunnerError> {
    // The same check as start_working_copy_mutation(), but with the stale
    // working-copy commit.
    if stale_commit.tree().tree_ids_and_labels()
        != locked_ws.locked_wc().old_tree().tree_ids_and_labels()
    {
        return Err(RunnerError::ConcurrentWorkingCopyOperation);
    }
    let stats = locked_ws
        .locked_wc()
        .check_out(new_commit)
        .await
        .map_err(|source| RunnerError::CheckOut {
            commit_id: new_commit.id().clone(),
            source,
        })?;
    locked_ws.finish(op_id).await?;

    Ok(stats)
}

/// Updates the working copy of `workspace` from `old_commit` to `new_commit`.
pub async fn update_working_copy(
    repo: &Arc<ReadonlyRepo>,
    workspace: &mut Workspace,
    old_commit: Option<&Commit>,
    new_commit: &Commit,
) -> Result<CheckoutStats, RunnerError> {
    let old_tree = old_commit.map(|commit| commit.tree());
    // TODO: CheckoutError::ConcurrentCheckout should probably just result in a
    // warning for most commands (but be an error for the checkout command)
    let stats = workspace
        .check_out(repo.op_id().clone(), old_tree.as_ref(), new_commit)
        .await
        .map_err(|source| RunnerError::CheckOut {
            commit_id: new_commit.id().clone(),
            source,
        })?;
    Ok(stats)
}
