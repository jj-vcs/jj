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
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::RepoLoader;
use jj_lib::revset::RevsetExpression;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_operation_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
#[cfg(feature = "git")]
use crate::commands::git::is_push_operation;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::revert_operation;
use crate::commands::operation::view_with_desired_portions_restored;
use crate::ui::Ui;

/// Undo the last operation
///
/// If used once after a normal (non-`undo`) operation, this will undo that last
/// operation by restoring its parent. If `jj undo` is used repeatedly, it will
/// restore increasingly older operations, going further back into the past.
///
/// There is also a complementary `jj redo` command that would instead move in
/// the direction of the future after one or more `jj undo`s.
///
/// Use `jj op log` to visualize the log of past operations, including a
/// detailed description of any past undo/redo operations. See also `jj op
/// restore` to explicitly restore an older operation by its id (available in
/// the operation log).
///
/// If the last operation was performed in another workspace, or is a merge of
/// concurrent operations, `jj undo` undoes the last operation performed in this
/// workspace instead. Since restoring the parent of that operation would also
/// undo the operations after it, `jj undo` reverts it like `jj op revert` does,
/// keeping the operations performed in other workspaces since. Repeated `jj
/// undo` continues with older operations of this workspace. Pass
/// `--allow-cross-workspace` to undo operations performed in other workspaces
/// as well.
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// Allow undoing an operation that was performed in another workspace
    #[arg(long)]
    allow_cross_workspace: bool,
}

pub(crate) const UNDO_OP_DESC_PREFIX: &str = "undo: restore to operation ";

/// Prefix of the description of an undo-operation that reverted the operation
/// whose id follows, instead of restoring an earlier operation.
pub(crate) const UNDO_REVERT_OP_DESC_PREFIX: &str = "undo: revert operation ";

pub async fn cmd_undo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UndoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;

    let head_op = workspace_command.repo().operation().clone();
    let mut target_op = head_op.clone();

    // Growing the "undo-stack" works as follows. See also the
    // [redo-stack](./redo.rs), which works in a similar way.
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
    if let Some(target_op_hex) = target_op
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let target_op_id = OperationId::try_from_hex(target_op_hex).ok_or_else(|| {
            internal_error("Failed to parse ID of target operation in undo-stack")
        })?;
        target_op = workspace_command
            .repo()
            .loader()
            .load_operation(&target_op_id)
            .await?;
    }

    // Restoring the parent of an operation performed in another workspace would
    // undo that workspace's work, and a merge operation has no single parent to
    // restore. In those cases, revert the latest operation performed in this
    // workspace (or in any workspace, with `--allow-cross-workspace`) instead,
    // keeping the operations performed elsewhere since. If an earlier `jj undo`
    // did that, continue with the operations before the one it reverted.
    let target_op_performed_elsewhere = target_op
        .metadata()
        .workspace_name
        .as_ref()
        .is_some_and(|name| name != workspace_command.workspace_name());
    let target_op_is_merge = target_op.parent_ids().len() > 1;
    let target_op_is_revert = target_op
        .metadata()
        .description
        .starts_with(UNDO_REVERT_OP_DESC_PREFIX);
    if target_op_is_revert
        || target_op_is_merge
        || !args.allow_cross_workspace && target_op_performed_elsewhere
    {
        let workspace_name =
            (!args.allow_cross_workspace).then(|| workspace_command.workspace_name().to_owned());
        let op_to_revert = find_op_to_undo(
            workspace_command.repo().loader(),
            &head_op,
            workspace_name.as_deref(),
        )
        .await?;
        if let Some((op, num_skipped)) = op_to_revert {
            return undo_by_reverting(ui, &mut workspace_command, &op, num_skipped).await;
        } else if target_op_is_revert {
            return Err(user_error(match &workspace_name {
                Some(name) => format!(
                    "No operation performed in workspace {} is left to undo",
                    name.as_symbol()
                ),
                None => "No operation is left to undo".to_owned(),
            }));
        }
        // Otherwise, there's no operation of this workspace to undo. Explain
        // below why the latest operation can't be undone.
    }

    if !args.allow_cross_workspace
        && let Some(op_workspace_name) = &target_op.metadata().workspace_name
        && op_workspace_name != workspace_command.workspace_name()
    {
        return Err(user_error(format!(
            "Refusing to undo operation {} because it was performed in workspace {}",
            short_operation_hash(target_op.id()),
            op_workspace_name.as_symbol()
        ))
        .hinted(
            "Use `--allow-cross-workspace` to undo it anyway, or use `jj op revert` to revert a \
             specific operation",
        ));
    }
    warn_about_undoing(ui, &target_op)?;

    let mut target_op_parent = match target_op.parents().await?.into_iter().at_most_one() {
        Ok(Some(op)) => op,
        Ok(None) => return Err(user_error("Cannot undo root operation")),
        Err(_) => {
            return Err(user_error("Cannot undo a merge operation")
                .hinted("Consider using `jj op restore` instead"));
        }
    };

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore an undo-operation. If we
    // didn't do this, repeated calls of `jj new ; jj undo` would create an
    // ever-growing linked list of undo-operations that restore each other.
    // Calling `jj undo` one more time would have to restore to the operation
    // at the very beginning of the linked list, which would require walking the
    // entire thing unnecessarily.
    if let Some(target_op_parent_hex) = target_op_parent
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let target_op_parent_id =
            OperationId::try_from_hex(target_op_parent_hex).ok_or_else(|| {
                internal_error("Failed to parse ID of target operation's parent in undo-stack")
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
        let template = tx.base_workspace_helper().operation_summary_template();

        write!(formatter, "Undid operation: ")?;
        template.format(&target_op, formatter.as_mut())?;
        writeln!(formatter)?;

        write!(formatter, "Restored to operation: ")?;
        template.format(&target_op_parent, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(
        ui,
        format!("{UNDO_OP_DESC_PREFIX}{}", target_op_parent.id().hex()),
    )
    .await?;

    Ok(())
}

/// Warns about effects of undoing `op` that the undo can't reverse.
fn warn_about_undoing(ui: &Ui, op: &Operation) -> Result<(), CommandError> {
    #[cfg(feature = "git")]
    if is_push_operation(op) {
        writeln!(
            ui.warning_default(),
            "Undoing a push operation often leads to conflicted bookmarks."
        )?;
        writeln!(ui.hint_default(), "To avoid this, run `jj redo` now.")?;
    }
    if op.metadata().description.starts_with("remove workspace") {
        writeln!(
            ui.warning_default(),
            "Undoing a workspace removal restores the workspace, but does not restore the \
             workspace directory."
        )?;
        writeln!(
            ui.hint_default(),
            "Run `jj workspace forget` then `jj workspace add` to recreate the directory."
        )?;
    }
    Ok(())
}

/// Finds the operation that `jj undo` should revert.
///
/// Walks the ancestors of `head_op` in the order `jj op log` lists them, and
/// returns the first operation performed in `workspace_name` (or in any
/// workspace if `None`) that is neither a merge nor an undo-operation, and
/// hasn't been undone already. Undo-operations of all workspaces count when
/// telling which operations have been undone.
///
/// Also returns how many operations performed in other workspaces were
/// skipped.
async fn find_op_to_undo(
    repo_loader: &RepoLoader,
    head_op: &Operation,
    workspace_name: Option<&WorkspaceName>,
) -> Result<Option<(Operation, usize)>, CommandError> {
    let mut undone_op_ids = HashSet::new();
    let mut num_skipped = 0;
    let mut ops = pin!(op_walk::walk_ancestors(slice::from_ref(head_op)));
    while let Some(op) = ops.try_next().await? {
        // Skip the root operation and merge operations, which have nothing of
        // their own to undo.
        if op.parent_ids().len() != 1 {
            continue;
        }
        let performed_here = is_performed_in(&op, workspace_name);
        if !performed_here {
            num_skipped += 1;
        }
        if undone_op_ids.contains(op.id()) {
            continue;
        }
        let description = &op.metadata().description;
        if let Some(restored_op_hex) = description.strip_prefix(UNDO_OP_DESC_PREFIX) {
            // This undo-operation undid the operations after the one it
            // restored.
            let restored_op = load_recorded_op(repo_loader, restored_op_hex).await?;
            let mut undone_ops = pin!(op_walk::walk_ancestors_range(
                slice::from_ref(&op),
                slice::from_ref(&restored_op),
            ));
            while let Some(undone_op) = undone_ops.try_next().await? {
                undone_op_ids.insert(undone_op.id().clone());
            }
        } else if let Some(reverted_op_hex) = description.strip_prefix(UNDO_REVERT_OP_DESC_PREFIX) {
            undone_op_ids.insert(parse_recorded_op_id(reverted_op_hex)?);
        } else if performed_here {
            return Ok(Some((op, num_skipped)));
        }
    }
    Ok(None)
}

/// Undoes `op` by reverting it, which keeps the operations after it.
async fn undo_by_reverting(
    ui: &Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    op: &Operation,
    num_skipped: usize,
) -> Result<(), CommandError> {
    warn_about_undoing(ui, op)?;
    let op_parent = op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Operation to undo should have a single parent"))?;
    let mut tx = workspace_command.start_transaction();
    revert_operation(&mut tx, op, &op_parent, &DEFAULT_REVERT_WHAT).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Undid operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(op, formatter.as_mut())?;
        writeln!(formatter)?;
        if num_skipped > 0 {
            writeln!(
                formatter,
                "Skipped {num_skipped} operations from other workspaces."
            )?;
        }
    }
    let old_repo = tx.base_repo().clone();
    // Record the operation even if the operation's changes had already been
    // reverted, so that the next `jj undo` continues with an older operation.
    tx.finish_even_if_unchanged(ui, format!("{UNDO_REVERT_OP_DESC_PREFIX}{}", op.id().hex()))
        .await?;
    warn_about_revert_conflicts(ui, &old_repo, workspace_command.repo(), "Undoing").await
}

/// Warns about bookmarks and changes that reverting an operation left
/// conflicted or divergent in `new_repo`, because newer operations changed
/// them too. `old_repo` is the repo before the revert.
///
/// It looks at the finished operation: until descendants are rebased, the
/// commits that the revert replaced are still visible, so a change whose
/// commit the revert put back would look divergent.
///
/// `action` names what the revert did, such as "Undoing".
pub(crate) async fn warn_about_revert_conflicts(
    ui: &Ui,
    old_repo: &ReadonlyRepo,
    new_repo: &ReadonlyRepo,
    action: &str,
) -> Result<(), CommandError> {
    let old_view = old_repo.view();
    let new_view = new_repo.view();

    let conflicted_bookmarks = new_view
        .local_bookmarks()
        .filter(|(name, target)| {
            !target.is_resolved() && old_view.get_local_bookmark(name).is_resolved()
        })
        .map(|(name, _)| name)
        .collect_vec();
    if !conflicted_bookmarks.is_empty() {
        writeln!(
            ui.warning_default(),
            "{action} the operation left these bookmarks conflicted:"
        )?;
        for name in conflicted_bookmarks {
            writeln!(ui.warning_no_heading(), "  {}", name.as_symbol())?;
        }
        writeln!(
            ui.hint_default(),
            "Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to \
             resolve."
        )?;
    }

    // Commits that the revert made visible again may be divergent with their
    // rewrites by newer operations.
    let old_heads = RevsetExpression::commits(old_view.heads().iter().cloned().collect());
    let new_heads = RevsetExpression::commits(new_view.heads().iter().cloned().collect());
    let divergent_change_ids: Vec<_> = old_heads
        .range(&new_heads)
        .intersection(&RevsetExpression::divergent())
        .evaluate(new_repo)?
        .commit_change_ids()
        .map_ok(|(_, change_id)| change_id)
        .try_collect()
        .await?;
    if !divergent_change_ids.is_empty() {
        writeln!(
            ui.warning_default(),
            "{action} the operation left these changes divergent:"
        )?;
        for change_id in divergent_change_ids.iter().unique() {
            writeln!(
                ui.warning_no_heading(),
                "  {}",
                short_change_hash(change_id)
            )?;
        }
        writeln!(
            ui.hint_default(),
            "Use `jj log -r 'divergent()'` to see details. Use `jj converge` to resolve."
        )?;
    }
    Ok(())
}

/// Returns whether `op` was performed in `workspace_name`, or in any workspace
/// if `None`.
pub(crate) fn is_performed_in(op: &Operation, workspace_name: Option<&WorkspaceName>) -> bool {
    workspace_name.is_none_or(|name| op.metadata().workspace_name.as_deref() == Some(name))
}

/// Parses the id of an operation recorded in the description of an undo- or
/// redo-operation.
pub(crate) fn parse_recorded_op_id(hex: &str) -> Result<OperationId, CommandError> {
    OperationId::try_from_hex(hex)
        .ok_or_else(|| internal_error("Failed to parse ID of operation in undo- or redo-stack"))
}

/// Loads an operation recorded in the description of an undo- or
/// redo-operation.
pub(crate) async fn load_recorded_op(
    repo_loader: &RepoLoader,
    hex: &str,
) -> Result<Operation, CommandError> {
    let op_id = parse_recorded_op_id(hex)?;
    Ok(repo_loader.load_operation(&op_id).await?)
}
