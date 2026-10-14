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
use jj_lib::lock::FileLockError;
use jj_lib::matchers::Matcher;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::transaction::Transaction;
use jj_lib::transaction::TransactionCommitError;
use jj_lib::working_copy::CheckoutError;
use thiserror::Error;

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
    pub async fn maybe_commit_transaction(
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
pub enum RunnerEvent {
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
        op_id: OperationId,
        /// Whether the operation snapshotted the working copy.
        is_snapshot: bool,
    },
    /// The working copy had changes but the working-copy commit was immutable,
    /// so the changes were snapshotted into a new commit on top of it.
    SnapshotOnImmutableWorkingCopy {
        /// The new working-copy commit.
        new_wc_commit: Commit,
    },
    /// The working-copy commit became immutable, so a new working-copy commit
    /// was created on top of it.
    WorkingCopyBecameImmutable {
        /// The new working-copy commit.
        new_wc_commit: Commit,
    },
    /// The working copy contains `.jjconflict*` files, which are used to
    /// represent conflicts in Git commits. This likely means that a conflicted
    /// commit was checked out by Git.
    #[cfg(feature = "git")]
    JjConflictFilesInWorkingCopy,
    /// The working-copy commit was moved because Git HEAD had changed.
    #[cfg(feature = "git")]
    GitHeadImported {
        /// Whether Git HEAD hadn't been imported before.
        first_import: bool,
    },
    /// Refs were imported from Git. There may also have been failures.
    #[cfg(feature = "git")]
    GitRefsImported(jj_lib::git::GitImportStats),
    /// Changes imported from Git have been committed.
    #[cfg(feature = "git")]
    GitRefsImportFinished,
    /// Refs were exported to Git. There may also have been failures.
    #[cfg(feature = "git")]
    GitRefsExported(jj_lib::git::GitExportStats),
    /// Git HEAD couldn't be updated to the working-copy commit's parent.
    #[cfg(feature = "git")]
    GitHeadResetFailed(jj_lib::git::GitResetHeadError),
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

/// Configuration used when snapshotting the working copy and importing changes
/// from Git.
pub struct SnapshotConfig {
    /// Matches new files that should be tracked automatically.
    pub auto_tracking_matcher: Box<dyn Matcher>,
    /// Options for importing Git refs. Only present if the working copy is
    /// shared with Git.
    #[cfg(feature = "git")]
    pub git_import_options: Option<jj_lib::git::GitImportOptions>,
}

/// Error from snapshotting the working copy, importing from or exporting to
/// Git, or finishing a transaction.
#[derive(Debug, Error)]
pub enum RunnerError {
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
    /// Error from the commit backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// Error resetting Git HEAD.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitResetHead(#[from] jj_lib::git::GitResetHeadError),
    /// The caller failed to handle a [`RunnerEvent`].
    #[error(transparent)]
    EventSink(Box<dyn std::error::Error + Send + Sync>),
}
