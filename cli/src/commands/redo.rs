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

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::revert_operation;
use crate::commands::undo::REDO_OP_DESC_PREFIX;
use crate::commands::undo::StackKind;
use crate::commands::undo::UNDO_OP_DESC_PREFIX;
use crate::commands::undo::find_stack_target;
use crate::ui::Ui;

/// Redo the most recently undone operation in the current workspace
///
/// This is the natural counterpart of `jj undo`. Repeated invocations of `jj
/// undo` and `jj redo` act similarly to Undo/Redo commands in a text editor.
/// `jj redo` reverts the most recent undo-operation performed in the current
/// workspace, as long as no other operation was performed in the workspace
/// after it.
///
/// Use `jj op log` to visualize the log of past operations, including a
/// detailed description of any past undo/redo operations. See also `jj op
/// revert` to revert a specific operation by its id, and `jj op restore` to
/// restore the whole repo to an older operation.
#[derive(clap::Args, Clone, Debug)]
pub struct RedoArgs {
    /// Deprecated; has no effect
    #[arg(long, hide = true)]
    allow_cross_workspace: bool,
}

pub async fn cmd_redo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RedoArgs,
) -> Result<(), CommandError> {
    if args.allow_cross_workspace {
        writeln!(
            ui.warning_default(),
            "`--allow-cross-workspace` is deprecated and has no effect; `jj redo` only considers \
             operations performed in the current workspace"
        )?;
    }
    let mut workspace_command = command.workspace_helper(ui).await?;
    let undo_op = find_stack_target(
        workspace_command.repo(),
        workspace_command.workspace_name(),
        StackKind::Redo,
    )
    .await?
    .ok_or_else(|| user_error("Nothing to redo in this workspace"))?;
    let undo_op_parent = undo_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Undo operation should have a single parent"))?;
    // The operation the undo-operation reverted, if it's known
    let undone_op = match undo_op
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        Some(hex) => {
            let id = OperationId::try_from_hex(hex)
                .ok_or_else(|| internal_error("Failed to parse ID of undone operation"))?;
            Some(
                workspace_command
                    .repo()
                    .loader()
                    .load_operation(&id)
                    .await?,
            )
        }
        None => None,
    };

    let mut tx = workspace_command.start_transaction();
    revert_operation(&mut tx, &undo_op, &undo_op_parent, &DEFAULT_REVERT_WHAT).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        let template = tx.base_workspace_helper().operation_summary_template();
        if let Some(undone_op) = &undone_op {
            write!(formatter, "Redid operation: ")?;
            template.format(undone_op, formatter.as_mut())?;
        } else {
            write!(formatter, "Reverted operation: ")?;
            template.format(&undo_op, formatter.as_mut())?;
        }
        writeln!(formatter)?;
    }
    tx.finish(ui, format!("{REDO_OP_DESC_PREFIX}{}", undo_op.id().hex()))
        .await?;

    Ok(())
}
