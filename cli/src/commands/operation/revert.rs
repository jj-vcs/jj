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
use jj_lib::operation::Operation;
use jj_lib::repo::Repo as _;

use super::DEFAULT_REVERT_WHAT;
use super::RevertWhatToRestore;
use super::view_with_desired_portions_restored;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Create a new operation that reverts an earlier operation
///
/// This reverts an individual operation by applying the inverse of the
/// operation.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationRevertArgs {
    /// The operation to revert
    ///
    /// Use `jj op log` to find an operation to revert.
    #[arg(default_value = "@")]
    #[arg(add = ArgValueCandidates::new(complete::operations))]
    pub(crate) operation: String, // pub for `jj undo`

    /// What portions of the local state to restore (can be repeated)
    ///
    /// This option is EXPERIMENTAL.
    #[arg(long, value_enum, default_values_t = DEFAULT_REVERT_WHAT)]
    pub(crate) what: Vec<RevertWhatToRestore>, // pub for `jj undo`
}

fn tx_description(op: &Operation) -> String {
    format!("revert operation {}", op.id().hex())
}

pub fn cmd_op_revert(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationRevertArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let bad_op = workspace_command.resolve_single_op(&args.operation)?;
    let parent_of_bad_op = match bad_op.parents().at_most_one() {
        Ok(Some(parent_of_bad_op)) => parent_of_bad_op?,
        Ok(None) => return Err(user_error("Cannot revert root operation")),
        Err(_) => return Err(user_error("Cannot revert a merge operation")),
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
        write!(formatter, "Reverted operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&bad_op, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(ui, tx_description(&bad_op))?;

    Ok(())
}
