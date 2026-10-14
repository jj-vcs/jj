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

use std::sync::Arc;

use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::lock::FileLock;
use jj_lib::lock::FileLockError;
use jj_lib::matchers::Matcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OpStoreError;
use jj_lib::op_store::OperationId;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::CheckOutCommitError;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::RepoLoaderError;
use jj_lib::repo::RewriteRootCommit;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::transaction::Transaction;
use jj_lib::transaction::TransactionCommitError;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::ResetError;
use jj_lib::working_copy::SnapshotError;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::SnapshotProgress;
use jj_lib::working_copy::WorkingCopyStateError;
use thiserror::Error;

use crate::workspace_env::WorkspaceEnvironmentError;

/// A token that holds a lock for git import/export operations in colocated
/// repositories. For non-colocated repos, this is an empty token (no actual
/// lock held). The lock is automatically released when this token is dropped.
pub struct GitImportExportLock {
    pub(crate) _lock: Option<FileLock>,
}

/// Options controlling how operations on a workspace interact with the
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
    /// Commits were rewritten by a transaction.
    RewrittenCommits,
    /// Commits were rewritten in the Git repo.
    RewrittenInGit,
    /// The working-copy commit was rewritten by a snapshot.
    WorkingCopySnapshot,
}

/// Receives [`RunnerEvent`]s from a `WorkspaceOperationRunner`.
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

impl SnapshotConfig {
    /// Returns the options for snapshotting the working copy with this
    /// configuration.
    pub fn snapshot_options(&self) -> SnapshotOptions<'_> {
        SnapshotOptions {
            base_ignores: self.base_ignores.clone(),
            progress: None,
            start_tracking_matcher: self.auto_tracking_matcher.as_ref(),
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size: self.max_new_file_size,
        }
    }
}

/// Error from snapshotting the working copy, importing from or exporting to
/// Git, or finishing a transaction.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// The working copy needs to be updated, but it can't be.
    #[error("The working copy can't be updated")]
    WorkingCopyNotWritable(WorkingCopyNotWritableReason),
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
