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

use jj_frontend::operation_util::UndoError;
use jj_frontend::operation_util::ViewPortions;
use jj_frontend::operation_util::find_redo_target;
use jj_frontend::operation_util::restore_operation;

use crate::cli_util::CommandHelper;
use crate::cli_util::short_operation_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
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
/// By default, `jj redo` refuses to redo an undo-operation that was performed
/// in another workspace, since that is usually not what you meant to redo. Pass
/// `--allow-cross-workspace` to redo it anyway, or use `jj op revert` to revert
/// a specific undo operation.
#[derive(clap::Args, Clone, Debug)]
pub struct RedoArgs {
    /// Allow redoing an undo-operation that was performed in another workspace
    #[arg(long)]
    allow_cross_workspace: bool,
}

pub async fn cmd_redo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RedoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let target = find_redo_target(
        workspace_command.repo(),
        workspace_command.workspace_name(),
        args.allow_cross_workspace,
    )
    .await
    .map_err(|err| match err {
        UndoError::OtherWorkspace {
            operation,
            workspace_name,
        } => user_error(format!(
            "Refusing to redo operation {} because it was performed in workspace {}",
            short_operation_hash(&operation),
            workspace_name.as_symbol()
        ))
        .hinted(
            "Use `--allow-cross-workspace` to redo it anyway, or use `jj op revert` to revert a \
             specific undo operation",
        ),
        err => err.into(),
    })?;

    let mut tx = workspace_command.start_transaction();
    restore_operation(tx.repo_mut(), &target.restore_to, ViewPortions::ALL).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Restored to operation: ")?;
        let template = tx.base_workspace_helper().operation_summary_template();
        template.format(&target.restore_to, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(ui, target.description()).await?;

    Ok(())
}
