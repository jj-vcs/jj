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

//! Restoring and reverting earlier operations.

use itertools::Itertools as _;
use jj_lib::op_store;
use jj_lib::op_store::OpStoreError;
use jj_lib::operation::Operation;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo::RepoLoaderError;
use thiserror::Error;

/// Portions of the repo state to restore from an earlier operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewPortions {
    /// The jj repo state, including the local bookmarks and tags.
    pub repo: bool,
    /// The remote-tracking bookmarks and tags. Users who want to push after
    /// restoring an operation might not want to restore these.
    pub remote_tracking: bool,
}

impl ViewPortions {
    /// All portions.
    pub const ALL: Self = Self {
        repo: true,
        remote_tracking: true,
    };
}

/// Returns `current_view` with the given `portions` replaced by the
/// corresponding portions of `view_being_restored`. The Git refs and Git HEAD
/// are always kept from `current_view`.
pub fn view_with_portions_restored(
    view_being_restored: &op_store::View,
    current_view: &op_store::View,
    portions: ViewPortions,
) -> op_store::View {
    let repo_source = if portions.repo {
        view_being_restored
    } else {
        current_view
    };
    let remote_source = if portions.remote_tracking {
        view_being_restored
    } else {
        current_view
    };
    op_store::View {
        head_ids: repo_source.head_ids.clone(),
        local_bookmarks: repo_source.local_bookmarks.clone(),
        local_tags: repo_source.local_tags.clone(),
        remote_views: remote_source.remote_views.clone(),
        git_refs: current_view.git_refs.clone(),
        git_heads: current_view.git_heads.clone(),
        wc_commit_ids: repo_source.wc_commit_ids.clone(),
    }
}

/// Restores the given `portions` of the repo state at `target_op` in `repo`.
/// This effectively undoes all operations after `target_op`.
pub async fn restore_operation(
    repo: &mut MutableRepo,
    target_op: &Operation,
    portions: ViewPortions,
) -> Result<(), OpStoreError> {
    let new_view = view_with_portions_restored(
        target_op.view().await?.store_view(),
        repo.base_repo().view().store_view(),
        portions,
    );
    repo.set_view(new_view);
    Ok(())
}

/// Error from [`revert_operation()`].
#[derive(Debug, Error)]
pub enum RevertOperationError {
    /// The root operation can't be reverted.
    #[error("Cannot revert root operation")]
    RootOperation,
    /// Merge operations can't be reverted.
    #[error("Cannot revert a merge operation")]
    MergeOperation,
    /// Failed to read an operation.
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
    /// Failed to load the repo at an operation.
    #[error(transparent)]
    RepoLoader(#[from] RepoLoaderError),
}

/// Applies the inverse of `target_op` to `repo`, but only to the given
/// `portions` of the repo state.
pub async fn revert_operation(
    repo: &mut MutableRepo,
    target_op: &Operation,
    portions: ViewPortions,
) -> Result<(), RevertOperationError> {
    let target_op_parent = match target_op.parents().await?.into_iter().at_most_one() {
        Ok(Some(op)) => op,
        Ok(None) => return Err(RevertOperationError::RootOperation),
        Err(_) => return Err(RevertOperationError::MergeOperation),
    };
    let repo_loader = repo.base_repo().loader().clone();
    let repo_at_target_op = repo_loader.load_at(target_op).await?;
    let repo_at_target_op_parent = repo_loader.load_at(&target_op_parent).await?;
    repo.merge(&repo_at_target_op, &repo_at_target_op_parent)
        .await?;
    let new_view = view_with_portions_restored(
        repo.view().store_view(),
        repo.base_repo().view().store_view(),
        portions,
    );
    repo.set_view(new_view);
    Ok(())
}
