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
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandEnvironment;
use crate::cli_util::print_snapshot_stats;
use crate::command_error::CommandError;
use crate::command_error::print_error_sources;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
#[cfg(feature = "git")]
use crate::git_util::unlink_git_worktree;
use crate::progress::snapshot_progress;
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
    let mut workspace_command = command.workspace_helper(ui).await?;
    workspace_command
        .check_working_copy_writable()
        .map_err(|err| err.hinted(USE_WORKSPACE_FORGET_HINT))?;

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

    let workspace_store = workspace_command.repo().loader().workspace_store().clone();
    let repo_path = workspace_command.repo_path();

    let mut paths_to_remove = Vec::new();
    let mut workspaces_to_remove = Vec::new();
    for ws in &remove_ws {
        let ws_path = workspace_store.get_workspace_path(ws)?.ok_or_else(|| {
            user_error(format!(
                "Cannot remove unreachable workspace '{}'",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        let ws_path = dunce::canonicalize(&ws_path).map_err(|err| {
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
        if repo_path.starts_with(&ws_path) {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it contains the repository",
                ws.as_symbol()
            )));
        }
        // The recorded path may since have been replaced by a workspace of an
        // unrelated repository, which we must not remove.
        let ws_workspace = command.load_workspace_at(&ws_path, workspace_command.settings())?;
        if ws_workspace.repo_path() != repo_path {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it belongs to another repository",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT));
        }
        paths_to_remove.push(ws_path);
        workspaces_to_remove.push(ws_workspace);
    }

    // Snapshot each workspace before its directory goes away, so that tracked
    // working-copy changes that were never committed survive as a commit.
    // Untracked and ignored files are not part of the snapshot and are lost
    // along with the directory.
    let mut new_snapshot_commit_ids = Vec::new();
    for mut ws_workspace in workspaces_to_remove {
        let env = command.workspace_environment(ui, &ws_workspace)?;
        if let Some(new_commit) = snapshot_workspace(ui, &mut ws_workspace, &env).await? {
            // Drop commit object since the underlying store isn't shared.
            new_snapshot_commit_ids.push(new_commit.id().clone());
        }
    }

    let mut tx = workspace_command.start_transaction();
    for commit_id in &new_snapshot_commit_ids {
        let commit = tx.repo().store().get_commit_async(commit_id).await?;
        tx.repo_mut().add_head(&commit).await?;
    }
    for ws in &remove_ws {
        tx.repo_mut().remove_workspace(ws).await?;
    }
    let names = remove_ws.iter().map(|ws| ws.as_symbol()).join(", ");
    let description = if remove_ws.len() == 1 {
        format!("remove workspace {names}")
    } else {
        format!("remove workspaces {names}")
    };
    tx.finish(ui, description).await?;

    workspace_store.forget(&remove_ws.iter().map(|ws| ws.as_ref()).collect_vec())?;

    #[cfg(feature = "git")]
    {
        let store = workspace_command.repo().store();
        for path in &paths_to_remove {
            unlink_git_worktree(ui, store, path)?;
        }
    }

    for path in &paths_to_remove {
        if let Err(err) = std::fs::remove_dir_all(path) {
            writeln!(
                ui.warning_default(),
                r#"Failed to remove workspace directory "{}"."#,
                path.display()
            )?;
            print_error_sources(ui, Some(&err))?;
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

/// Snapshots working-copy changes to a new commit.
async fn snapshot_workspace(
    ui: &Ui,
    workspace: &mut Workspace,
    env: &WorkspaceCommandEnvironment,
) -> Result<Option<Commit>, CommandError> {
    let repo_loader = workspace.repo_loader().clone();
    let mut locked_ws = workspace.start_working_copy_mutation().await?;
    let op_id = locked_ws.locked_wc().old_operation_id();
    let op = repo_loader.load_operation(op_id).await?;
    let repo = repo_loader.load_at(&op).await?;
    let workspace_name = env.workspace_name();
    let Some(wc_commit_id) = repo.view().get_wc_commit_id(workspace_name) else {
        return Ok(None);
    };

    let wc_commit = repo.store().get_commit_async(wc_commit_id).await?;
    let (new_tree, stats) = {
        let matcher = env.auto_tracking_matcher(ui)?;
        let mut options =
            env.snapshot_options_with_start_tracking_matcher(repo.store(), &matcher)?;
        let progress = snapshot_progress(ui);
        options.progress = progress.as_ref().map(|x| x as _);
        locked_ws.locked_wc().snapshot(&options).await?
    };
    print_snapshot_stats(ui, &stats, env.path_converter())?;
    if !stats.untracked_paths.is_empty() || !stats.invalid_utf8_paths.is_empty() {
        return Err(user_error(format!(
            "Cannot remove workspace '{name}' with unsnapshotted changes",
            name = workspace_name.as_symbol()
        )));
    }
    if new_tree.tree_ids_and_labels() == wc_commit.tree().tree_ids_and_labels() {
        return Ok(None);
    }

    // Simply create a new commit since we don't check the immutability of the
    // working-copy commit. It's also safer not to rewrite and rebase commits
    // during workspace removal.
    let mut tx = repo.start_transaction();
    let new_commit = tx
        .repo_mut()
        .new_commit(vec![wc_commit.id().clone()], new_tree.clone())
        .detach()
        .set_description(format!(
            "SNAPSHOT OF REMOVED WORKSPACE `{name}`\n",
            name = workspace_name.as_symbol()
        ))
        .write_hidden()
        .await?;
    writeln!(
        ui.status(),
        "Snapshot commit created for workspace '{name}': {commit_id:.12}",
        name = workspace_name.as_symbol(),
        commit_id = new_commit.id()
    )?;
    Ok(Some(new_commit))
}
