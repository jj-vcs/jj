// Copyright 2020-2023 The Jujutsu Authors
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
use jj_lib::op_store::OpStoreError;
use jj_lib::operation::Operation;
use jj_lib::repo::Repo as _;

use super::view_with_desired_portions_restored;
use super::UndoWhatToRestore;
use super::DEFAULT_UNDO_WHAT;
use crate::cli_util::CommandHelper;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Create a new operation that undoes an earlier operation
///
/// This undoes an individual operation by applying the inverse of the
/// operation.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationUndoArgs {
    /// The operation to undo
    ///
    /// Use `jj op log` to find an operation to undo.
    #[arg(default_value = "@", add = ArgValueCandidates::new(complete::operations))]
    operation: String,

    /// What portions of the local state to restore (can be repeated)
    ///
    /// This option is EXPERIMENTAL.
    #[arg(long, value_enum, default_values_t = DEFAULT_UNDO_WHAT)]
    what: Vec<UndoWhatToRestore>,
}

fn is_undo(op: &Operation, parent_op: &Operation) -> Result<bool, OpStoreError> {
    let grand_parents: Vec<_> = parent_op.parents().try_collect()?;
    if let [grand_parent_op] = &grand_parents[..] {
        Ok(op.view_id() == grand_parent_op.view_id())
    } else {
        Ok(false)
    }
}

pub fn cmd_op_undo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationUndoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let bad_op = workspace_command.resolve_single_op(&args.operation)?;
    let parent_of_bad_op = match bad_op.parents().at_most_one() {
        Ok(Some(parent_of_bad_op)) => parent_of_bad_op?,
        Ok(None) => return Err(user_error("Cannot undo root operation")),
        Err(_) => return Err(user_error("Cannot undo a merge operation")),
    };

    let mut tx = workspace_command.start_transaction();
    let repo_loader = tx.base_repo().loader();
    let bad_repo = repo_loader.load_at(&bad_op)?;
    let parent_repo = repo_loader.load_at(&parent_of_bad_op)?;
    tx.repo_mut().merge(&bad_repo, &parent_repo)?;
    let new_view = view_with_desired_portions_restored(
        tx.repo().view().store_view(),
        tx.base_repo().view().store_view(),
        &args.what,
    );
    tx.repo_mut().set_view(new_view);
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Undid operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&bad_op, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(ui, format!("undo operation {}", bad_op.id().hex()))?;

    if args.operation == "@" && is_undo(&bad_op, &parent_of_bad_op)? {
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
