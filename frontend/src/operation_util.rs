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

//! Restoring, reverting, undoing, and redoing earlier operations.

use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store;
use jj_lib::op_store::OpStoreError;
use jj_lib::op_store::OperationId;
use jj_lib::operation::Operation;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
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

const UNDO_OP_DESC_PREFIX: &str = "undo: restore to operation ";
const REDO_OP_DESC_PREFIX: &str = "redo: restore to operation ";

/// Error from [`find_undo_target()`] or [`find_redo_target()`].
#[derive(Debug, Error)]
pub enum UndoError {
    /// The root operation can't be undone.
    #[error("Cannot undo root operation")]
    RootOperation,
    /// Merge operations can't be undone.
    #[error("Cannot undo a merge operation")]
    MergeOperation,
    /// The operation to redo isn't an undo operation.
    #[error("Nothing to redo")]
    NothingToRedo,
    /// The operation to undo or redo was performed in another workspace.
    #[error("Operation {operation} was performed in workspace {}", workspace_name.as_symbol())]
    OtherWorkspace {
        /// The operation to undo or redo.
        operation: OperationId,
        /// The workspace the operation was performed in.
        workspace_name: WorkspaceNameBuf,
    },
    /// The description of an undo or redo operation doesn't contain a valid
    /// operation ID.
    #[error("Failed to parse ID of the operation restored by operation {operation}")]
    InvalidStackEntry {
        /// The undo or redo operation.
        operation: OperationId,
    },
    /// An undo operation unexpectedly has multiple parents.
    #[error("Undo operation {operation} should have a single parent")]
    UndoOperationWithMultipleParents {
        /// The undo operation.
        operation: OperationId,
    },
    /// Failed to read an operation.
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
}

/// The operations involved in undoing or redoing.
#[derive(Clone, Debug)]
pub struct UndoTarget {
    /// The operation that is undone or redone.
    pub operation: Operation,
    /// The operation whose repo state should be restored.
    pub restore_to: Operation,
    description_prefix: &'static str,
}

impl UndoTarget {
    /// Returns the description for the new operation, which is needed for
    /// repeated undo or redo to work.
    pub fn description(&self) -> String {
        format!("{}{}", self.description_prefix, self.restore_to.id().hex())
    }
}

/// If `op` was created by undo or redo (as indicated by `prefix`), returns the
/// operation it restored to.
async fn restored_operation(
    repo: &ReadonlyRepo,
    op: &Operation,
    prefix: &str,
) -> Result<Option<Operation>, UndoError> {
    let Some(op_hex) = op.metadata().description.strip_prefix(prefix) else {
        return Ok(None);
    };
    let op_id = OperationId::try_from_hex(op_hex).ok_or_else(|| UndoError::InvalidStackEntry {
        operation: op.id().clone(),
    })?;
    Ok(Some(repo.loader().load_operation(&op_id).await?))
}

fn check_workspace(
    op: &Operation,
    workspace_name: &WorkspaceName,
    allow_cross_workspace: bool,
) -> Result<(), UndoError> {
    if !allow_cross_workspace
        && let Some(op_workspace_name) = &op.metadata().workspace_name
        && op_workspace_name != workspace_name
    {
        return Err(UndoError::OtherWorkspace {
            operation: op.id().clone(),
            workspace_name: op_workspace_name.clone(),
        });
    }
    Ok(())
}

/// Finds the operation to undo in `repo`, which is in the workspace named
/// `workspace_name`, and the operation to restore to.
///
/// If used once after a normal (non-undo) operation, this undoes that operation
/// by restoring its parent. If used repeatedly, it restores increasingly older
/// operations. Operations performed in other workspaces are refused unless
/// `allow_cross_workspace` is set.
pub async fn find_undo_target(
    repo: &ReadonlyRepo,
    workspace_name: &WorkspaceName,
    allow_cross_workspace: bool,
) -> Result<UndoTarget, UndoError> {
    let mut target_op = repo.operation().clone();

    // Growing the "undo-stack" works as follows. See also the redo-stack in
    // `find_redo_target()`, which works in a similar way.
    //
    // - If the operation to undo is a regular one (not an undo-operation), simply
    //   undo it (== restore its parent).
    // - If the operation to undo is an undo-operation itself, undo that operation
    //   to which the previous undo-operation restored the repo.
    // - If the operation to restore to is an undo-operation, restore directly to
    //   the original operation. This avoids creating a linked list of
    //   undo-operations, which subsequently may have to be walked with an
    //   inefficient loop.
    //
    // This described behavior leads to "jumping over" old undo-stacks if the
    // current one grows into it. Consider this op-log example:
    //
    // * G "undo: restore A" -------+
    // |                            |
    // * F "undo: restore B" -----+ |
    // |                          | |
    // * E                        | |
    // |                          | |
    // * D "undo: restore B" -+   | |
    // |                      |   | |
    // * C                    |   | |
    // |                      |   | |
    // * B   <----------------+ <-+ |
    // |                            |
    // * A   <----------------------+
    //
    // It was produced by the following sequence of events:
    // - do normal operations A, B and C
    // - undo C, restoring to B
    // - do normal operation E
    // - undo E, restoring to B again (NOT to D)
    // - undo F, restoring to A
    //
    // Notice that running `undo` after having undone E leads to A being
    // restored (as opposed to C). The undo-stack spanning from F to B was
    // "jumped over".
    //
    if let Some(op) = restored_operation(repo, &target_op, UNDO_OP_DESC_PREFIX).await? {
        target_op = op;
    }
    check_workspace(&target_op, workspace_name, allow_cross_workspace)?;

    let mut target_op_parent = match target_op.parents().await?.into_iter().at_most_one() {
        Ok(Some(op)) => op,
        Ok(None) => return Err(UndoError::RootOperation),
        Err(_) => return Err(UndoError::MergeOperation),
    };

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore an undo-operation. If we
    // didn't do this, repeated calls of `jj new ; jj undo` would create an
    // ever-growing linked list of undo-operations that restore each other.
    // Calling `jj undo` one more time would have to restore to the operation
    // at the very beginning of the linked list, which would require walking the
    // entire thing unnecessarily.
    if let Some(op) = restored_operation(repo, &target_op_parent, UNDO_OP_DESC_PREFIX).await? {
        target_op_parent = op;
    }

    Ok(UndoTarget {
        operation: target_op,
        restore_to: target_op_parent,
        description_prefix: UNDO_OP_DESC_PREFIX,
    })
}

/// Finds the undo operation to redo in `repo`, which is in the workspace named
/// `workspace_name`, and the operation to restore to.
///
/// Repeated undo and redo act similarly to Undo/Redo commands in a text
/// editor. Undo operations performed in other workspaces are refused unless
/// `allow_cross_workspace` is set.
pub async fn find_redo_target(
    repo: &ReadonlyRepo,
    workspace_name: &WorkspaceName,
    allow_cross_workspace: bool,
) -> Result<UndoTarget, UndoError> {
    let mut target_op = repo.operation().clone();

    // Growing the "redo-stack" works very similar to the undo-stack in
    // `find_undo_target()`. Undo and redo track their stacks separately.
    //
    // - If the operation to redo is a regular one (neither an undo- or
    //   redo-operation): Fail, because there is nothing to redo.
    // - If the operation to redo is an undo-operation, try to redo it (by restoring
    //   its parent operation).
    // - If the operation to redo is a redo-operation itself, redo the operation the
    //   early redo-operation restored to.
    // - If the operation to restore to is a redo-operation itself, restore directly
    //   to the original operation. This avoids creating a linked list of
    //   redo-operations, which subsequently may have to be walked with an
    //   inefficient loop.
    //
    // This described behavior leads to "jumping over" old redo-stacks if the
    // current one grows into it. Consider the following op-log example, where
    // redo-stacks are shown on the left and undo-stacks on the right:
    //
    // +------- "redo: restore C" * I
    // |                          |
    // | +----- "redo: restore D" * H
    // | |                        |
    // | |                        * G "undo: restore A" -------+
    // | |                        |                            |
    // | |   +- "redo: restore D" * F                          |
    // | |   |                    |                            |
    // | |   |                    * E "undo: restore A" ---+   |
    // | |   |                    |                        |   |
    // | +-> +---------------->   * D "undo: restore B" -+ |   |
    // |                          |                      | |   |
    // +---------------------->   * C                    | |   |
    //                            |                      | |   |
    //                            * B   <----------------+ |   |
    //                            |                        |   |
    //                            * A   <------------------+ <-+
    //
    // The first interesting operation here is H:
    // - Attempt to redo G.
    // - G is an undo-operation, so attempt to restore its parent F.
    // - F is a redo-operation. Restore its original operation D, instead of F.
    //
    // The operation I is also noteworthy:
    // - Attempt to redo H.
    // - H is a redo-operation restoring to D, so attempt to redo D.
    // - D is an undo-operation. Redo it by restoring its parent C.
    //
    if let Some(op) = restored_operation(repo, &target_op, REDO_OP_DESC_PREFIX).await? {
        target_op = op;
    }

    if !target_op
        .metadata()
        .description
        .starts_with(UNDO_OP_DESC_PREFIX)
    {
        // cannot redo a non-undo-operation
        return Err(UndoError::NothingToRedo);
    }
    check_workspace(&target_op, workspace_name, allow_cross_workspace)?;

    let mut target_op_parent = target_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| UndoError::UndoOperationWithMultipleParents {
            operation: target_op.id().clone(),
        })?;

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore a redo-operation. If
    // we didn't do this, repeated calls of `jj undo; jj redo` would create
    // an ever-growing linked list of redo-operations that restore each
    // other. Calling `jj redo` one more time would have to redo a potential
    // undo-operation at the very beginning of the linked list, which would
    // require walking the entire thing unnecessarily.
    if let Some(op) = restored_operation(repo, &target_op_parent, REDO_OP_DESC_PREFIX).await? {
        target_op_parent = op;
    }

    Ok(UndoTarget {
        operation: target_op,
        restore_to: target_op_parent,
        description_prefix: REDO_OP_DESC_PREFIX,
    })
}
