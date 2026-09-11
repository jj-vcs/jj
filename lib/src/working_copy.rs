// Copyright 2023 The Jujutsu Authors
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

//! Defines the interface for the working copy. See `LocalWorkingCopy` for the
//! default local-disk implementation.

use std::any::Any;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
pub use jj_core::working_copy::CheckoutError;
pub use jj_core::working_copy::CheckoutStats;
pub use jj_core::working_copy::LockedWorkingCopy;
pub use jj_core::working_copy::ResetError;
pub use jj_core::working_copy::SnapshotError;
pub use jj_core::working_copy::SnapshotStats;
pub use jj_core::working_copy::UntrackedReason;
pub use jj_core::working_copy::WorkingCopy;
pub use jj_core::working_copy::WorkingCopyStateError;
use thiserror::Error;
use tracing::instrument;

use crate::backend::BackendError;
use crate::commit::Commit;
use crate::gitignore::GitIgnoreError;
use crate::ignore::Ignore;
use crate::matchers::Matcher;
use crate::merged_tree::MergedTree;
use crate::op_store::OpStoreError;
use crate::op_store::OperationId;
use crate::op_walk;
use crate::operation::Operation;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RewriteRootCommit;
use crate::repo_path::InvalidRepoPathError;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::settings::UserSettings;
use crate::store::Store;
use crate::transaction::TransactionCommitError;

/// The factory which creates and loads a specific type of working copy.
pub trait WorkingCopyFactory {
    /// Create a new working copy from scratch.
    fn init_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        operation_id: OperationId,
        workspace_name: WorkspaceNameBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError>;

    /// Load an existing working copy.
    fn load_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError>;
}

/// Whether the working copy is stale or not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkingCopyFreshness {
    /// The working copy isn't stale, and no need to reload the repo.
    Fresh,
    /// The working copy was updated since we loaded the repo. The repo must be
    /// reloaded at the working copy's operation.
    Updated(Box<Operation>),
    /// The working copy is behind the latest operation.
    WorkingCopyStale,
    /// The working copy is a sibling of the latest operation.
    SiblingOperation,
}

impl WorkingCopyFreshness {
    /// Determine the freshness of the provided working copy relative to the
    /// target commit.
    #[instrument(skip_all)]
    pub async fn check_stale(
        locked_wc: &dyn LockedWorkingCopy,
        wc_commit: &Commit,
        repo: &ReadonlyRepo,
    ) -> Result<Self, OpStoreError> {
        // Check if the working copy's operation matches the repo's operation
        if locked_wc.old_operation_id() == repo.op_id() {
            // The working copy isn't stale, and no need to reload the repo.
            Ok(Self::Fresh)
        } else {
            let wc_operation = repo
                .loader()
                .load_operation(locked_wc.old_operation_id())
                .await?;
            let repo_operation = repo.operation();
            let ancestor_ops =
                op_walk::closest_common_ancestors([wc_operation.clone()], [repo_operation.clone()])
                    .await?;
            // TODO: test all operations instead of using only a single common operation
            let ancestor_op = ancestor_ops.into_iter().next().unwrap();
            if ancestor_op.id() == repo_operation.id() {
                // The working copy was updated since we loaded the repo. The repo must be
                // reloaded at the working copy's operation.
                Ok(Self::Updated(Box::new(wc_operation)))
            } else if ancestor_op.id() == wc_operation.id() {
                // The working copy was not updated when some repo operation committed,
                // meaning that it's stale compared to the repo view.
                if locked_wc.old_tree().tree_ids_and_labels()
                    == wc_commit.tree().tree_ids_and_labels()
                {
                    // The working copy doesn't require any changes
                    Ok(Self::Fresh)
                } else {
                    Ok(Self::WorkingCopyStale)
                }
            } else {
                Ok(Self::SiblingOperation)
            }
        }
    }
}

/// An error while recovering a stale working copy.
#[derive(Debug, Error)]
pub enum RecoverWorkspaceError {
    /// Backend error.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// Error during checkout.
    #[error(transparent)]
    Reset(#[from] ResetError),
    /// Checkout attempted to modify the root commit.
    #[error(transparent)]
    RewriteRootCommit(#[from] RewriteRootCommit),
    /// Error during transaction.
    #[error(transparent)]
    TransactionCommit(#[from] TransactionCommitError),
    /// Working copy commit is missing.
    #[error(r#""{}" doesn't have a working-copy commit"#, .0.as_symbol())]
    WorkspaceMissingWorkingCopy(WorkspaceNameBuf),
}

/// Recover this workspace to its last known checkout.
pub async fn create_and_check_out_recovery_commit(
    locked_wc: &mut dyn LockedWorkingCopy,
    repo: &Arc<ReadonlyRepo>,
    workspace_name: WorkspaceNameBuf,
    description: &str,
) -> Result<(Arc<ReadonlyRepo>, Commit), RecoverWorkspaceError> {
    let mut tx = repo.start_transaction();
    let repo_mut = tx.repo_mut();

    let commit_id = repo
        .view()
        .get_wc_commit_id(&workspace_name)
        .ok_or_else(|| {
            RecoverWorkspaceError::WorkspaceMissingWorkingCopy(workspace_name.clone())
        })?;
    let commit = repo.store().get_commit_async(commit_id).await?;
    let new_commit = repo_mut
        .new_commit(vec![commit_id.clone()], commit.tree())
        .set_description(description)
        .write()
        .await?;
    repo_mut.set_wc_commit(workspace_name, new_commit.id().clone())?;

    let repo = tx.commit("recovery commit").await?;
    locked_wc.recover(&new_commit).await?;

    Ok((repo, new_commit))
}
