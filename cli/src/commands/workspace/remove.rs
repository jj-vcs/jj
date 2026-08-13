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

use super::forget::WorkspaceRemoval;
use super::forget::forget_workspaces;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
use crate::ui::Ui;

const USE_WORKSPACE_FORGET_HINT: &str =
    "Use `jj workspace forget` to stop tracking a workspace without deleting files.";

/// Remove a workspace and its working-copy files from disk
///
/// The workspace directory and its contents are removed from disk. The
/// working-copy state is snapshotted into a commit before the workspace is
/// removed. The main workspace cannot be removed.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceRemoveArgs {
    /// Names of the workspaces to remove.
    #[arg(required = true, add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_remove(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceRemoveArgs,
) -> Result<(), CommandError> {
    if command.global_args().ignore_working_copy {
        return Err(
            user_error("Cannot remove workspace with --ignore-working-copy")
                .hinted(USE_WORKSPACE_FORGET_HINT),
        );
    }
    if command.global_args().at_operation.is_some() {
        return Err(user_error("Cannot remove workspace with --at-operation")
            .hinted(USE_WORKSPACE_FORGET_HINT));
    }
    if command.global_args().no_integrate_operation {
        return Err(
            user_error("Cannot remove workspace with --no-integrate-operation")
                .hinted(USE_WORKSPACE_FORGET_HINT),
        );
    }

    let mut workspace_command = command.workspace_helper(ui).await?;

    let wss = args.workspaces.clone();

    let mut remove_ws = Vec::new();
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
            remove_ws.push(ws);
        }
    }
    if remove_ws.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;
    let repo_path = workspace_command.repo_path().to_owned();
    let canonical_repo_path = dunce::canonicalize(&repo_path).unwrap_or_else(|_| repo_path.clone());

    let mut workspaces_to_remove = Vec::new();
    for ws in &remove_ws {
        let rel_path = workspace_store.get_workspace_path(ws)?.ok_or_else(|| {
            user_error(format!(
                "Cannot remove unreachable workspace '{}'",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        let abs_path = dunce::canonicalize(repo_path.join(rel_path)).map_err(|err| {
            user_error_with_message(
                format!("Cannot access workspace '{}' directory", ws.as_symbol()),
                err,
            )
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        // The repository itself lives under the main workspace (in `.jj/repo`),
        // so removing that directory would destroy the repository. Don't
        // suggest `jj workspace forget` here: forgetting the main workspace
        // isn't a useful thing to do either.
        if canonical_repo_path.starts_with(&abs_path) {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it contains the repository",
                ws.as_symbol()
            )));
        }
        // The recorded path may since have been replaced by a workspace of an
        // unrelated repository, which we must not remove.
        let ws_workspace = command.load_workspace_at(&abs_path, workspace_command.settings())?;
        let ws_repo_path = dunce::canonicalize(ws_workspace.repo_path())
            .unwrap_or_else(|_| ws_workspace.repo_path().to_owned());
        if ws_repo_path != canonical_repo_path {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it belongs to another repository",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT));
        }
        workspaces_to_remove.push((abs_path, ws_workspace));
    }

    // Snapshot each workspace before its directory goes away, so that tracked
    // working-copy changes that were never committed survive as a commit.
    // Untracked and ignored files are not part of the snapshot and are lost
    // along with the directory.
    let mut op_id = workspace_command.repo().op_id().clone();
    let mut paths_to_remove = Vec::new();
    for (abs_path, ws_workspace) in workspaces_to_remove {
        let op = ws_workspace.repo_loader().load_operation(&op_id).await?;
        let ws_repo = ws_workspace.repo_loader().load_at(&op).await?;
        let mut ws_helper = command.for_workable_repo(ui, ws_workspace, ws_repo)?;
        ws_helper.maybe_snapshot(ui).await?;
        // Continue from the operation the snapshot created, so that the next
        // snapshot and the removal itself build on it.
        op_id = ws_helper.repo().op_id().clone();
        paths_to_remove.push(abs_path);
    }

    let workspace = command.load_workspace()?;
    let op = workspace.repo_loader().load_operation(&op_id).await?;
    let repo = workspace.repo_loader().load_at(&op).await?;
    workspace_command = command.for_workable_repo(ui, workspace, repo)?;

    forget_workspaces(
        ui,
        &mut workspace_command,
        &remove_ws,
        WorkspaceRemoval::Remove,
    )
    .await?;

    for path in &paths_to_remove {
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
