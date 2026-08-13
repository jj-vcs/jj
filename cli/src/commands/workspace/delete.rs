// Copyright 2026 The Jujutsu Authors
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
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;
use tracing::instrument;

use super::forget::forget_workspaces;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

const USE_WORKSPACE_FORGET_HINT: &str =
    "Use `jj workspace forget` to stop tracking a workspace without deleting files.";

/// Delete a workspace and its working-copy files from disk
///
/// The workspace directory and its contents are removed from disk. The
/// working-copy state is snapshotted into a commit before the workspace is
/// deleted. The main workspace cannot be deleted.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceDeleteArgs {
    /// Names of the workspaces to delete.
    #[arg(required = true, add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceDeleteArgs,
) -> Result<(), CommandError> {
    if command.global_args().ignore_working_copy {
        return Err(
            user_error("Cannot delete workspace with --ignore-working-copy")
                .hinted(USE_WORKSPACE_FORGET_HINT),
        );
    }
    if command.global_args().at_operation.is_some() {
        return Err(user_error("Cannot delete workspace with --at-operation")
            .hinted(USE_WORKSPACE_FORGET_HINT));
    }
    if command.global_args().no_integrate_operation {
        return Err(
            user_error("Cannot delete workspace with --no-integrate-operation")
                .hinted(USE_WORKSPACE_FORGET_HINT),
        );
    }

    let mut workspace_command = command.workspace_helper(ui).await?;

    let wss = args.workspaces.clone();

    let mut delete_ws = Vec::new();
    for ws in &wss {
        if workspace_command
            .repo()
            .view()
            .get_wc_commit_id(ws)
            .is_none()
        {
            writeln!(
                ui.warning_default(),
                "No such workspace: {}",
                ws.as_symbol()
            )?;
        } else {
            delete_ws.push(ws);
        }
    }
    if delete_ws.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;
    let repo_path = workspace_command.repo_path().to_owned();

    let mut workspaces_to_delete = Vec::new();
    for ws in &delete_ws {
        let rel_path = workspace_store.get_workspace_path(ws)?.ok_or_else(|| {
            user_error(format!(
                "Cannot delete unreachable workspace '{}'",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        let abs_path = dunce::canonicalize(repo_path.join(rel_path)).map_err(|err| {
            user_error(format!(
                "Cannot access workspace '{}' directory: {err}",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        if abs_path.join(".jj").join("repo").is_dir() {
            return Err(
                user_error(format!("Cannot delete main workspace '{}'", ws.as_symbol()))
                    .hinted(USE_WORKSPACE_FORGET_HINT),
            );
        }
        #[cfg(windows)]
        if *ws == workspace_command.workspace_name() {
            return Err(user_error(format!(
                "Cannot delete current workspace '{}'",
                ws.as_symbol()
            ))
            .hinted("Run this command from another workspace."));
        }
        workspaces_to_delete.push((*ws, abs_path));
    }

    for (_ws, abs_path) in &workspaces_to_delete {
        let ws_workspace = command.load_workspace_at(abs_path, workspace_command.settings())?;
        let op = ws_workspace
            .repo_loader()
            .load_operation(workspace_command.repo().op_id())
            .await?;
        let ws_repo = ws_workspace.repo_loader().load_at(&op).await?;
        let mut ws_helper = command.for_workable_repo(ui, ws_workspace, ws_repo)?;
        ws_helper.maybe_snapshot(ui).await?;
    }

    workspace_command = command.workspace_helper_no_snapshot(ui).await?;

    forget_workspaces(ui, &mut workspace_command, &delete_ws, "delete").await?;

    for (_ws, path) in &workspaces_to_delete {
        if let Err(err) = std::fs::remove_dir_all(path) {
            writeln!(
                ui.warning_default(),
                r#"Failed to remove workspace directory "{}": {err}"#,
                path.display()
            )?;
        } else {
            writeln!(
                ui.status(),
                r#"Removed workspace directory "{}"."#,
                path.display()
            )?;
        }
    }

    Ok(())
}
