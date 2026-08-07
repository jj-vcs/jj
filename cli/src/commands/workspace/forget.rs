// Copyright 2020 The Jujutsu Authors
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

use std::path::Path;
use std::process::Command;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Stop tracking a workspace's working-copy commit in the repo
///
/// The workspace will not be touched on disk. It can be deleted from disk
/// before or after running this command.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceForgetArgs {
    /// Names of the workspaces to forget. By default, forgets only the current
    /// workspace.
    #[arg(add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;

    let wss = if args.workspaces.is_empty() {
        vec![workspace_command.workspace_name().to_owned()]
    } else {
        args.workspaces.clone()
    };

    let mut forget_ws = Vec::new();
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
                ws.as_symbol(),
            )?;
        } else {
            forget_ws.push(ws);
        }
    }
    if forget_ws.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;

    let repo_path = workspace_command.repo_path();
    let workspace_paths: Vec<_> = forget_ws
        .iter()
        .filter_map(|ws| {
            let rel_path = workspace_store.get_workspace_path(ws).ok().flatten()?;
            dunce::canonicalize(repo_path.join(rel_path)).ok()
        })
        .collect();

    // bundle every workspace forget into a single transaction, so that e.g.
    // undo correctly restores all of them at once.
    let mut tx = workspace_command.start_transaction();

    for ws in &forget_ws {
        tx.repo_mut().remove_wc_commit(ws).await?;
    }

    workspace_store.forget(&forget_ws.iter().map(|x| x.as_ref()).collect::<Vec<_>>())?;

    let description = if let [ws] = forget_ws.as_slice() {
        format!("forget workspace {}", ws.as_symbol())
    } else {
        format!(
            "forget workspaces {}",
            forget_ws.iter().map(|ws| ws.as_symbol()).join(", ")
        )
    };

    tx.finish(ui, description).await?;

    if workspace_command.working_copy_shared_with_git() {
        for path in &workspace_paths {
            remove_git_worktree(ui, workspace_command.workspace_root(), path)?;
        }
    }

    Ok(())
}

fn remove_git_worktree(
    ui: &Ui,
    main_workspace_root: &Path,
    worktree_path: &Path,
) -> Result<(), CommandError> {
    let dot_git = worktree_path.join(".git");
    if !dot_git.is_file() {
        return Ok(());
    }
    let output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .current_dir(main_workspace_root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            writeln!(
                ui.status(),
                "Removed Git worktree at \"{}\".",
                worktree_path.display()
            )?;
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            writeln!(
                ui.warning_default(),
                "Failed to remove Git worktree at \"{}\": {stderr}",
                worktree_path.display()
            )?;
        }
        Err(err) => {
            writeln!(
                ui.warning_default(),
                "Failed to run `git worktree remove`: {err}"
            )?;
        }
    }
    Ok(())
}
