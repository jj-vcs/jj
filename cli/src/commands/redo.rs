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

use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::view_with_desired_portions_restored;
use crate::commands::undo::UNDO_OP_DESC_PREFIX;
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
#[derive(clap::Args, Clone, Debug)]
pub struct RedoArgs {}

const REDO_OP_DESC_PREFIX: &str = "redo: restore to operation ";

pub fn cmd_redo(ui: &mut Ui, command: &CommandHelper, _: &RedoArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let mut op_to_redo = workspace_command.repo().operation().clone();

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
    // The first interesting operation here is I:
    // - Attempt to redo G.
    // - G is an undo-operation, attempt to restore its parent F.
    // - F is a redo-opreation. Restore its original operation D, instead of F.
    //
    // The operation H is also noteworthy:
    // - Attempt to redo H.
    // - H is a redo-operation restoring to D, so attempt to redo D.
    // - D is an undo-operation. Redo it by restoring its parent C.
    //
    if let Some(id_of_restored_op) = op_to_redo
        .metadata()
        .description
        .strip_prefix(REDO_OP_DESC_PREFIX)
    {
        let Some(id_of_restored_op) = OperationId::try_from_hex(id_of_restored_op) else {
            return Err(internal_error(
                "Failed to parse ID of restored operation in redo-stack",
            ));
        };
        op_to_redo = workspace_command
            .repo()
            .loader()
            .load_operation(&id_of_restored_op)
            .block_on()?;
    }

    if !op_to_redo
        .metadata()
        .description
        .starts_with(UNDO_OP_DESC_PREFIX)
    {
        // cannot redo a non-undo-operation
        return Err(user_error("Nothing to redo"));
    }

    let mut op_to_restore = match op_to_redo.parents().at_most_one().ok().flatten() {
        Some(parent_of_op_to_redo) => parent_of_op_to_redo?,
        None => {
            return Err(internal_error("Undo operation should have a single parent"));
        }
    };

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore a redo-operation. If
    // we didn't to this, repeated calls of `jj undo ; jj redo` would create
    // an ever-growing linked list of redo-operations that restore each
    // other. Calling `jj redo` one more time would have to redo a potential
    // undo-operation at the very beginning of the linked list, which would
    // require walking the entire thing unnecessarily.
    if let Some(original_op) = op_to_restore
        .metadata()
        .description
        .strip_prefix(REDO_OP_DESC_PREFIX)
    {
        let Some(id_of_original_op) = OperationId::try_from_hex(original_op) else {
            return Err(internal_error(
                "Failed to parse ID of restored operation in redo-stack",
            ));
        };
        op_to_restore = workspace_command
            .repo()
            .loader()
            .load_operation(&id_of_original_op)
            .block_on()?;
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
        format!("{REDO_OP_DESC_PREFIX}{}", op_to_restore.id().hex()),
    )?;

    Ok(())
}
