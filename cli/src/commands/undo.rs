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

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
#[cfg(feature = "git")]
use crate::commands::git::is_push_operation;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::RevertWhatToRestore;
use crate::commands::operation::revert::OperationRevertArgs;
use crate::commands::operation::revert::cmd_op_revert;
use crate::commands::operation::view_with_desired_portions_restored;
use crate::complete;
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
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// (deprecated, use `jj op revert <operation>`)
    ///
    /// The operation to undo
    ///
    /// Use `jj op log` to find an operation to undo.
    // TODO: Delete in jj 0.39+
    #[arg(default_value = "@")]
    #[arg(add = ArgValueCandidates::new(complete::operations))]
    operation: String,

    /// (deprecated, use `jj op revert --what`)
    ///
    /// What portions of the local state to restore (can be repeated)
    ///
    /// This option is EXPERIMENTAL.
    #[arg(long, value_enum, hide = true, default_values_t = DEFAULT_REVERT_WHAT)]
    what: Vec<RevertWhatToRestore>,
}

pub(crate) const UNDO_OP_DESC_PREFIX: &str = "undo: restore to operation ";

pub fn cmd_undo(ui: &mut Ui, command: &CommandHelper, args: &UndoArgs) -> Result<(), CommandError> {
    if args.operation != "@" {
        writeln!(
            ui.warning_default(),
            "`jj undo <operation>` is deprecated; use `jj op revert <operation>` instead"
        )?;
        let args = OperationRevertArgs {
            operation: args.operation.clone(),
            what: args.what.clone(),
        };
        return cmd_op_revert(ui, command, &args);
    }
    if args.what != DEFAULT_REVERT_WHAT {
        writeln!(
            ui.warning_default(),
            "`jj undo --what` is deprecated; use `jj op revert --what` instead"
        )?;
        let args = OperationRevertArgs {
            operation: args.operation.clone(),
            what: args.what.clone(),
        };
        return cmd_op_revert(ui, command, &args);
    }

    let mut workspace_command = command.workspace_helper(ui)?;

    let mut op_to_undo = workspace_command.resolve_single_op(&args.operation)?;

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
    if let Some(id_of_restored_op) = op_to_undo
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let Some(id_of_restored_op) = OperationId::try_from_hex(id_of_restored_op) else {
            return Err(internal_error(
                "Failed to parse ID of restored operation in undo-stack",
            ));
        };
        op_to_undo = workspace_command
            .repo()
            .loader()
            .load_operation(&id_of_restored_op)?;
    }
    #[cfg(feature = "git")]
    if is_push_operation(&op_to_undo) {
        writeln!(
            ui.warning_default(),
            "Undoing a push operation often leads to conflicted bookmarks."
        )?;
        writeln!(ui.hint_default(), "To avoid this, run `jj redo` now.")?;
    };

    let mut op_to_restore = match op_to_undo.parents().at_most_one() {
        Ok(Some(parent_of_op_to_undo)) => parent_of_op_to_undo?,
        Ok(None) => return Err(user_error("Cannot undo root operation")),
        Err(_) => {
            return Err(user_error_with_hint(
                "Cannot undo a merge operation",
                "Consider using `jj op restore` instead",
            ));
        }
    };

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore an undo-operation. If we
    // didn't to this, repeated calls of `jj new ; jj undo` would create an
    // ever-growing linked list of undo-operations that restore each other.
    // Calling `jj undo` one more time would have to restore to the operation
    // at the very beginning of the linked list, which would require walking the
    // entire thing unnecessarily.
    if let Some(original_op) = op_to_restore
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let Some(id_of_original_op) = OperationId::try_from_hex(original_op) else {
            return Err(internal_error(
                "Failed to parse ID of restored operation in undo-stack",
            ));
        };
        op_to_restore = workspace_command
            .repo()
            .loader()
            .load_operation(&id_of_original_op)?;
    }

    let mut tx = workspace_command.start_transaction();
    let new_view = view_with_desired_portions_restored(
        op_to_restore.view()?.store_view(),
        tx.base_repo().view().store_view(),
        &DEFAULT_REVERT_WHAT,
    );
    tx.repo_mut().set_view(new_view);
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Restored to operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&op_to_restore, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(
        ui,
        format!("{UNDO_OP_DESC_PREFIX}{}", op_to_restore.id().hex()),
    )?;

    Ok(())
}
