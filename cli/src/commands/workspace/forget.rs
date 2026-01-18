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

#[cfg(feature = "git")]
use std::path::Path;
#[cfg(feature = "git")]
use std::path::PathBuf;
#[cfg(feature = "git")]
use std::process::Command;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
#[cfg(feature = "git")]
use jj_lib::git;
#[cfg(feature = "git")]
use jj_lib::protos::local_working_copy::Checkout;
use jj_lib::ref_name::WorkspaceNameBuf;
#[cfg(feature = "git")]
use jj_lib::repo::Repo as _;
#[cfg(feature = "git")]
use prost::Message as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Stop tracking a workspace's working-copy commit in the repo
///
/// For colocated workspaces, this also removes the associated Git worktree.
/// By default, removal will fail if the worktree has uncommitted changes.
/// Use --force to remove the worktree even with uncommitted changes.
///
/// For non-colocated workspaces, the workspace directory is not touched on
/// disk. It can be deleted from disk before or after running this command.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceForgetArgs {
    /// Names of the workspaces to forget. By default, forgets only the current
    /// workspace.
    #[arg(add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,

    /// Force removal of Git worktrees even if they have uncommitted changes
    #[arg(long)]
    force: bool,
}

#[instrument(skip_all)]
pub fn cmd_workspace_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

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

    // Collect worktrees to remove BEFORE committing the transaction.
    // We need to read the checkout protobuf while the .jj directories still exist.
    #[cfg(feature = "git")]
    let worktrees_to_remove =
        if let Ok(git_backend) = git::get_git_backend(workspace_command.repo().store()) {
            let git_repo = git_backend.git_repo();
            let common_dir = git_repo.common_dir().to_path_buf();
            let worktrees = find_worktrees_for_workspaces(&common_dir, &forget_ws);
            Some((common_dir, worktrees, args.force))
        } else {
            None
        };

    // bundle every workspace forget into a single transaction, so that e.g.
    // undo correctly restores all of them at once.
    let mut tx = workspace_command.start_transaction();
    forget_ws
        .iter()
        .try_for_each(|ws| tx.repo_mut().remove_wc_commit(ws))?;
    let description = if let [ws] = forget_ws.as_slice() {
        format!("forget workspace {}", ws.as_symbol())
    } else {
        format!(
            "forget workspaces {}",
            forget_ws.iter().map(|ws| ws.as_symbol()).join(", ")
        )
    };

    tx.finish(ui, description)?;

    // Clean up git worktrees AFTER the transaction commits successfully.
    // This ensures that if the transaction fails, the worktrees remain intact.
    #[cfg(feature = "git")]
    if let Some((common_dir, worktrees, force)) = worktrees_to_remove {
        for (ws, worktree_path) in worktrees {
            // Run git worktree remove
            let mut cmd = Command::new("git");
            cmd.arg("-C").arg(&common_dir).arg("worktree").arg("remove");
            if force {
                cmd.arg("--force");
            }
            cmd.arg(&worktree_path);
            let result = cmd.output();

            match result {
                Ok(output) if !output.status.success() => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    // Check if worktree is already gone
                    if stderr.contains("is not a working tree") {
                        continue;
                    }
                    // Check if it's a dirty worktree error (only happens without --force)
                    if !force
                        && (stderr.contains("contains modified or untracked files")
                            || stderr.contains("is dirty"))
                    {
                        writeln!(
                            ui.warning_default(),
                            "Git worktree for workspace {} has uncommitted changes and was not \
                             removed.",
                            ws.as_symbol(),
                        )?;
                        writeln!(
                            ui.hint_default(),
                            "Use --force to remove it anyway, or manually clean up with `git \
                             worktree remove --force {}`",
                            worktree_path.display()
                        )?;
                    } else {
                        writeln!(
                            ui.warning_default(),
                            "Failed to remove Git worktree for workspace {}: {}",
                            ws.as_symbol(),
                            stderr.trim()
                        )?;
                    }
                }
                Err(e) => {
                    writeln!(
                        ui.warning_default(),
                        "Failed to run git worktree remove for workspace {}: {}",
                        ws.as_symbol(),
                        e
                    )?;
                }
                Ok(_) => {
                    // Success - worktree was removed
                }
            }
        }
    }

    Ok(())
}

/// Finds git worktrees that correspond to the given jj workspaces.
///
/// Enumerates all git worktrees and checks each one's jj checkout state
/// to match workspace names, since the git worktree directory name may
/// differ from the jj workspace name (e.g., when using --name flag).
#[cfg(feature = "git")]
fn find_worktrees_for_workspaces<'a>(
    common_dir: &Path,
    workspaces: &'a [&WorkspaceNameBuf],
) -> Vec<(&'a WorkspaceNameBuf, PathBuf)> {
    let worktrees_dir = common_dir.join("worktrees");
    let Ok(entries) = std::fs::read_dir(&worktrees_dir) else {
        return Vec::new();
    };

    let mut results = Vec::new();

    for entry in entries.flatten() {
        let worktree_admin_dir = entry.path();
        if !worktree_admin_dir.is_dir() {
            continue;
        }

        // Read the gitdir file to find the worktree path
        let gitdir_path = worktree_admin_dir.join("gitdir");
        let Ok(content) = std::fs::read_to_string(&gitdir_path) else {
            continue;
        };

        // gitdir contains path to the .git file in the worktree
        let git_file = content.trim();
        // Get the parent directory (the worktree root)
        let Some(worktree_path) = Path::new(git_file).parent() else {
            continue;
        };

        // Check if this worktree has a jj workspace
        let checkout_path = worktree_path
            .join(".jj")
            .join("working_copy")
            .join("checkout");
        let Ok(checkout_bytes) = std::fs::read(&checkout_path) else {
            continue;
        };

        // Decode the checkout protobuf to get the workspace name
        let Ok(checkout) = Checkout::decode(checkout_bytes.as_slice()) else {
            continue;
        };

        // Check if this workspace name matches any we're forgetting
        for ws in workspaces {
            if checkout.workspace_name == ws.as_str() {
                results.push((*ws, worktree_path.to_path_buf()));
                break;
            }
        }
    }

    results
}
