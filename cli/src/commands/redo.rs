// Copyright 2025 The Jujutsu Authors
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

use std::collections::HashSet;
use std::pin::pin;
use std::slice;

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::op_walk;
use jj_lib::operation::Operation;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::RepoLoader;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::short_operation_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::revert_operation;
use crate::commands::operation::view_with_desired_portions_restored;
use crate::commands::undo::UNDO_OP_DESC_PREFIX;
use crate::commands::undo::UNDO_REVERT_OP_DESC_PREFIX;
use crate::commands::undo::is_performed_in;
use crate::commands::undo::load_recorded_op;
use crate::commands::undo::parse_recorded_op_id;
use crate::commands::undo::warn_about_revert_conflicts;
use crate::ui::Ui;

/// Redo the most recently undone operation
///
/// This is the natural counterpart of `jj undo`. Repeated invocations of `jj
/// undo` and `jj redo` act similarly to Undo/Redo commands in a text editor.
///
/// Use `jj op log` to visualize the log of past operations, including a
/// detailed description of any past undo/redo operations. See also `jj op
/// restore` to explicitly restore an older operation by its id (available in
/// the operation log).
///
/// If the last operation was performed in another workspace, or is a merge of
/// concurrent operations, `jj redo` redoes the operation most recently undone
/// in this workspace instead. It reverts that `jj undo` like `jj op revert`
/// does, keeping the operations performed in other workspaces since. Pass
/// `--allow-cross-workspace` to redo undo-operations performed in other
/// workspaces as well.
#[derive(clap::Args, Clone, Debug)]
pub struct RedoArgs {
    /// Allow redoing an undo-operation that was performed in another workspace
    #[arg(long)]
    allow_cross_workspace: bool,
}

const REDO_OP_DESC_PREFIX: &str = "redo: restore to operation ";

/// Prefix of the description of a redo-operation that reverted the
/// undo-operation whose id follows, instead of restoring an earlier operation.
const REDO_REVERT_OP_DESC_PREFIX: &str = "redo: revert operation ";

pub async fn cmd_redo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RedoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;

    let head_op = workspace_command.repo().operation().clone();
    let mut target_op = head_op.clone();

    // Growing the "redo-stack" works very similar to the
    // [undo-stack](./undo.rs). `jj redo` and `jj undo` track their stacks
    // separately.
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
    if let Some(target_op_hex) = target_op
        .metadata()
        .description
        .strip_prefix(REDO_OP_DESC_PREFIX)
    {
        let target_op_id = OperationId::try_from_hex(target_op_hex).ok_or_else(|| {
            internal_error("Failed to parse ID of target operation in redo-stack")
        })?;
        target_op = workspace_command
            .repo()
            .loader()
            .load_operation(&target_op_id)
            .await?;
    }

    // Like `jj undo`, if the operation to redo was performed in another
    // workspace, or is a merge, redo the latest undo-operation of this
    // workspace (or of any workspace, with `--allow-cross-workspace`) instead
    // by reverting it. Undo- and redo-operations that reverted an operation
    // continue that search.
    let target_op_performed_elsewhere = target_op
        .metadata()
        .workspace_name
        .as_ref()
        .is_some_and(|name| name != workspace_command.workspace_name());
    let target_op_is_merge = target_op.parent_ids().len() > 1;
    let target_op_description = &target_op.metadata().description;
    let target_op_is_revert = target_op_description.starts_with(UNDO_REVERT_OP_DESC_PREFIX)
        || target_op_description.starts_with(REDO_REVERT_OP_DESC_PREFIX);
    if target_op_is_revert
        || target_op_is_merge
        || !args.allow_cross_workspace && target_op_performed_elsewhere
    {
        let workspace_name =
            (!args.allow_cross_workspace).then(|| workspace_command.workspace_name().to_owned());
        let op_to_revert = find_op_to_redo(
            workspace_command.repo().loader(),
            &head_op,
            workspace_name.as_deref(),
        )
        .await?;
        if let Some((undo_op, num_skipped)) = op_to_revert {
            return redo_by_reverting(ui, &mut workspace_command, &undo_op, num_skipped).await;
        }
        // Otherwise, there's no undo-operation of this workspace to redo.
        // Explain below why the latest operation can't be redone.
    }

    if !target_op
        .metadata()
        .description
        .starts_with(UNDO_OP_DESC_PREFIX)
    {
        // cannot redo a non-undo-operation
        return Err(user_error("Nothing to redo"));
    }

    if !args.allow_cross_workspace
        && let Some(op_workspace_name) = &target_op.metadata().workspace_name
        && op_workspace_name != workspace_command.workspace_name()
    {
        return Err(user_error(format!(
            "Refusing to redo operation {} because it was performed in workspace {}",
            short_operation_hash(target_op.id()),
            op_workspace_name.as_symbol()
        ))
        .hinted(
            "Use `--allow-cross-workspace` to redo it anyway, or use `jj op revert` to revert a \
             specific undo operation",
        ));
    }

    let mut target_op_parent = target_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Undo operation should have a single parent"))?;

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore a redo-operation. If
    // we didn't do this, repeated calls of `jj undo; jj redo` would create
    // an ever-growing linked list of redo-operations that restore each
    // other. Calling `jj redo` one more time would have to redo a potential
    // undo-operation at the very beginning of the linked list, which would
    // require walking the entire thing unnecessarily.
    if let Some(target_op_parent_hex) = target_op_parent
        .metadata()
        .description
        .strip_prefix(REDO_OP_DESC_PREFIX)
    {
        let target_op_parent_id =
            OperationId::try_from_hex(target_op_parent_hex).ok_or_else(|| {
                internal_error("Failed to parse ID of target operation's parent in redo-stack")
            })?;
        target_op_parent = workspace_command
            .repo()
            .loader()
            .load_operation(&target_op_parent_id)
            .await?;
    }

    let mut tx = workspace_command.start_transaction();
    let new_view = view_with_desired_portions_restored(
        target_op_parent.view().await?.store_view(),
        tx.base_repo().view().store_view(),
        &DEFAULT_REVERT_WHAT,
    );
    tx.repo_mut().set_view(new_view);
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Restored to operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&target_op_parent, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(
        ui,
        format!("{REDO_OP_DESC_PREFIX}{}", target_op_parent.id().hex()),
    )
    .await?;

    Ok(())
}

/// Finds the undo-operation that `jj redo` should revert.
///
/// Walks the ancestors of `head_op` in the order `jj op log` lists them, and
/// returns the first undo-operation performed in `workspace_name` (or in any
/// workspace if `None`) that hasn't been redone already. Returns `None` if
/// another operation of the workspace comes first, since there's nothing to
/// redo after it. Merge operations are skipped, and redo-operations of all
/// workspaces count when telling which undo-operations have been redone.
///
/// Also returns how many operations performed in other workspaces were
/// skipped.
async fn find_op_to_redo(
    repo_loader: &RepoLoader,
    head_op: &Operation,
    workspace_name: Option<&WorkspaceName>,
) -> Result<Option<(Operation, usize)>, CommandError> {
    let mut redone_op_ids = HashSet::new();
    let mut num_skipped = 0;
    let mut ops = pin!(op_walk::walk_ancestors(slice::from_ref(head_op)));
    while let Some(op) = ops.try_next().await? {
        // Skip the root operation and merge operations.
        if op.parent_ids().len() != 1 {
            continue;
        }
        let performed_here = is_performed_in(&op, workspace_name);
        if !performed_here {
            num_skipped += 1;
        }
        if redone_op_ids.contains(op.id()) {
            continue;
        }
        let description = &op.metadata().description;
        if let Some(restored_op_hex) = description.strip_prefix(REDO_OP_DESC_PREFIX) {
            // This redo-operation redid the undo-operations after the one it
            // restored, which is where the redo-stack continues.
            let restored_op = load_recorded_op(repo_loader, restored_op_hex).await?;
            let mut redone_ops = pin!(op_walk::walk_ancestors_range(
                slice::from_ref(&op),
                slice::from_ref(&restored_op),
            ));
            while let Some(redone_op) = redone_ops.try_next().await? {
                redone_op_ids.insert(redone_op.id().clone());
            }
        } else if let Some(undo_op_hex) = description.strip_prefix(REDO_REVERT_OP_DESC_PREFIX) {
            redone_op_ids.insert(parse_recorded_op_id(undo_op_hex)?);
        } else if !performed_here {
            continue;
        } else if description.starts_with(UNDO_OP_DESC_PREFIX)
            || description.starts_with(UNDO_REVERT_OP_DESC_PREFIX)
        {
            return Ok(Some((op, num_skipped)));
        } else {
            return Ok(None);
        }
    }
    Ok(None)
}

/// Returns the operation that `undo_op` undid.
async fn op_undone_by(
    repo_loader: &RepoLoader,
    undo_op: &Operation,
) -> Result<Operation, CommandError> {
    let description = &undo_op.metadata().description;
    if let Some(reverted_op_hex) = description.strip_prefix(UNDO_REVERT_OP_DESC_PREFIX) {
        return load_recorded_op(repo_loader, reverted_op_hex).await;
    }
    // An undo-operation that restored an earlier operation undid its parent,
    // unless the parent was an undo-operation too, in which case it undid the
    // operation that the parent restored.
    let parent = undo_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Undo operation should have a single parent"))?;
    if let Some(restored_op_hex) = parent
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        return load_recorded_op(repo_loader, restored_op_hex).await;
    }
    Ok(parent)
}

/// Redoes what `undo_op` undid by reverting it, which keeps the operations
/// after it.
async fn redo_by_reverting(
    ui: &Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    undo_op: &Operation,
    num_skipped: usize,
) -> Result<(), CommandError> {
    let repo_loader = workspace_command.repo().loader();
    let undone_op = op_undone_by(repo_loader, undo_op).await?;
    let undo_op_parent = undo_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Undo operation should have a single parent"))?;
    let mut tx = workspace_command.start_transaction();
    revert_operation(&mut tx, undo_op, &undo_op_parent, &DEFAULT_REVERT_WHAT).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Redid operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&undone_op, formatter.as_mut())?;
        writeln!(formatter)?;
        if num_skipped > 0 {
            writeln!(
                formatter,
                "Skipped {num_skipped} operations from other workspaces."
            )?;
        }
    }
    let old_repo = tx.base_repo().clone();
    // Record the operation even if nothing changed, so that the next `jj redo`
    // continues with an older undo-operation.
    tx.finish_even_if_unchanged(
        ui,
        format!("{REDO_REVERT_OP_DESC_PREFIX}{}", undo_op.id().hex()),
    )
    .await?;
    warn_about_revert_conflicts(ui, &old_repo, workspace_command.repo(), "Redoing").await
}
