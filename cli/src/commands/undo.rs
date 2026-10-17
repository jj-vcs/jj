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
use jj_frontend::operation_util::find_undo_target;
use jj_frontend::operation_util::restore_operation;

use crate::cli_util::CommandHelper;
use crate::cli_util::short_operation_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
#[cfg(feature = "git")]
use crate::commands::git::is_push_operation;
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
/// By default, `jj undo` refuses to undo an operation that was performed in
/// another workspace, since that is usually not what you meant to undo. Pass
/// `--allow-cross-workspace` to undo it anyway, or use `jj op revert` to revert
/// a specific operation.
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// Allow undoing an operation that was performed in another workspace
    #[arg(long)]
    allow_cross_workspace: bool,
}

pub async fn cmd_undo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UndoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let target = find_undo_target(
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
            "Refusing to undo operation {} because it was performed in workspace {}",
            short_operation_hash(&operation),
            workspace_name.as_symbol()
        ))
        .hinted(
            "Use `--allow-cross-workspace` to undo it anyway, or use `jj op revert` to revert a \
             specific operation",
        ),
        UndoError::MergeOperation => {
            user_error(err).hinted("Consider using `jj op restore` instead")
        }
        err => err.into(),
    })?;
    #[cfg(feature = "git")]
    if is_push_operation(&target.operation) {
        writeln!(
            ui.warning_default(),
            "Undoing a push operation often leads to conflicted bookmarks."
        )?;
        writeln!(ui.hint_default(), "To avoid this, run `jj redo` now.")?;
    }
    if target
        .operation
        .metadata()
        .description
        .starts_with("remove workspace")
    {
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

    let mut tx = workspace_command.start_transaction();
    restore_operation(tx.repo_mut(), &target.restore_to, ViewPortions::ALL).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        let template = tx.base_workspace_helper().operation_summary_template();

        write!(formatter, "Undid operation: ")?;
        template.format(&target.operation, formatter.as_mut())?;
        writeln!(formatter)?;

        write!(formatter, "Restored to operation: ")?;
        template.format(&target.restore_to, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(ui, target.description()).await?;

    Ok(())
}
