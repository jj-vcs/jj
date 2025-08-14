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
use jj_lib::op_store::OpStoreError;
use jj_lib::operation::Operation;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::RevertWhatToRestore;
use crate::commands::operation::revert::OperationRevertArgs;
use crate::commands::operation::revert::cmd_op_revert;
use crate::commands::operation::revert::tx_description;
use crate::complete;
use crate::ui::Ui;

// Checks whether `op` resets the view of `parent_op` to the view of the
// grandparent op.
//
// This is a necessary condition for `op` to be a revert of `parent_op` but is
// not sufficient. For example, deleting a bookmark also resets the view
// similarly but is not a literal `revert` operation.
fn resets_view_of(op: &Operation, parent_op: &Operation) -> Result<bool, OpStoreError> {
    let Ok(grandparent_op) = parent_op.parents().exactly_one() else {
        return Ok(false);
    };
    Ok(op.view_id() == grandparent_op?.view_id())
}

/// Create a new operation that undoes an earlier operation
///
/// This undoes an individual operation by applying the inverse of the
/// operation.
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// The operation to undo
    ///
    /// Use `jj op log` to find an operation to undo.
    #[arg(default_value = "@", add = ArgValueCandidates::new(complete::operations))]
    operation: String,

    /// What portions of the local state to restore (can be repeated)
    ///
    /// This option is EXPERIMENTAL.
    #[arg(long, value_enum, default_values_t = DEFAULT_REVERT_WHAT)]
    what: Vec<RevertWhatToRestore>,
}

pub fn cmd_undo(ui: &mut Ui, command: &CommandHelper, args: &UndoArgs) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let bad_op = workspace_command.resolve_single_op(&args.operation)?;
    let parent_of_bad_op = match bad_op.parents().at_most_one() {
        Ok(Some(parent_of_bad_op)) => parent_of_bad_op?,
        Ok(None) => return Err(user_error("Cannot undo root operation")),
        Err(_) => return Err(user_error("Cannot undo a merge operation")),
    };

    let args = OperationRevertArgs {
        operation: args.operation.clone(),
        what: args.what.clone(),
    };
    cmd_op_revert(ui, command, &args)?;

    // Check if the user performed a "double undo", i.e. the current `undo` (C)
    // reverts an immediately preceding `undo` (B) that is itself an `undo` of the
    // operation preceding it (A).
    //
    //    C (undo of B)
    // @  B (`bad_op` = undo of A)
    // ○  A
    //
    // An exception is made for when the user specified the immediately preceding
    // `undo` with an op set. In this situation, the user's intent is clear, so
    // a warning is not shown.
    //
    // Note that undoing an older `undo` does not constitute a "double undo". For
    // example, the current `undo` (D) here reverts an `undo` B that is not the
    // immediately preceding operation (C). A warning is not shown in this case.
    //
    //    D (undo of B)
    // @  C (unrelated operation)
    // ○  B (`bad_op` = undo of A)
    // ○  A
    if args.operation == "@"
        && resets_view_of(&bad_op, &parent_of_bad_op)?
        && bad_op.metadata().description == tx_description(&parent_of_bad_op)
    {
        writeln!(
            ui.warning_default(),
            "The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in \
             the same state as it was before the second-last `jj undo`."
        )?;
        writeln!(
            ui.hint_default(),
            "To undo multiple operations, use `jj op log` to see past states and `jj op restore` \
             to restore one of these states."
        )?;
    }

    Ok(())
}
