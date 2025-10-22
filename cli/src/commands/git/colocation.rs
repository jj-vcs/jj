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

use std::io::ErrorKind;
use std::io::Write as _;

use jj_lib::commit::Commit;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::git;
use jj_lib::op_store::RefTarget;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::is_colocated_git_workspace;
use crate::ui::Ui;

/// Show the current colocation status
#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationStatusArgs {}

/// Convert into a colocated Jujutsu/Git repository
///
/// This moves the underlying Git repository that is found inside the .jj
/// directory to the root of the Jujutsu workspace. This allows you to
/// use Git commands directly in the Jujutsu workspace.
#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationEnableArgs {}

/// Convert into a non-colocated Jujutsu/Git repository
///
/// This moves the Git repository that is at the root of the Jujutsu
/// workspace into the .jj directory. Once this is done you will no longer
/// be able to use Git commands directly in the Jujutsu workspace.
#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationDisableArgs {}

/// Manage Jujutsu repository colocation with Git
#[derive(clap::Subcommand, Clone, Debug)]
pub enum GitColocationCommand {
    Disable(GitColocationDisableArgs),
    Enable(GitColocationEnableArgs),
    Status(GitColocationStatusArgs),
}

pub fn cmd_git_colocation(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitColocationCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GitColocationCommand::Disable(args) => cmd_git_colocation_disable(ui, command, args),
        GitColocationCommand::Enable(args) => cmd_git_colocation_enable(ui, command, args),
        GitColocationCommand::Status(args) => cmd_git_colocation_status(ui, command, args),
    }
}

/// Check that the repository supports colocation commands
/// which means that the repo is backed by git, is not
/// already colocated, and is a main workspace
fn repo_supports_git_colocation_commands(
    workspace_command: &crate::cli_util::WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    // Check if backend is Git (will show an error otherwise)
    git::get_git_backend(workspace_command.repo().store())?;

    // Ensure that this is the main workspace
    if workspace_command.repo_path().is_file() {
        return Err(user_error(
            "This command cannot be used in a non-main Jujutsu workspace.",
        ));
    }
    Ok(())
}

fn cmd_git_colocation_status(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitColocationStatusArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

    // Make sure that the repository supports git colocation commands
    repo_supports_git_colocation_commands(&workspace_command)?;

    let is_colocated =
        is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo());

    if is_colocated {
        writeln!(ui.stdout(), "Repository is currently colocated with Git.")?;
        writeln!(
            ui.hint_default(),
            "To disable colocation, run: `jj git colocation disable`"
        )?;
    } else {
        writeln!(
            ui.stdout(),
            "Repository is currently not colocated with Git."
        )?;
        writeln!(
            ui.hint_default(),
            "To enable colocation, run: `jj git colocation enable`"
        )?;
    }

    Ok(())
}

fn cmd_git_colocation_enable(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitColocationEnableArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

    // Make sure that the repository supports git colocation commands
    repo_supports_git_colocation_commands(&workspace_command)?;

    // Then ensure that the repo is not already colocated before proceeding
    if is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(ui.status(), "Repository is already colocated with Git.")?;
        return Ok(());
    }

    // And that it has a working copy (whose parent we'll use later to set the git
    // HEAD)
    let wc_commit_id = workspace_command
        .get_wc_commit_id()
        .ok_or_else(|| user_error("This command requires a working copy"))?
        .clone();

    let workspace_root = workspace_command.workspace_root();
    let jj_repo_path = workspace_command.repo_path();
    let git_store_path = jj_repo_path.join("store").join("git");
    let git_target_path = jj_repo_path.join("store").join("git_target");
    let dot_git_path = workspace_root.join(".git");

    // Move the git repository from .jj/repo/store/git to .git
    std::fs::rename(&git_store_path, &dot_git_path).map_err(|err| match err.kind() {
        ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty => {
            user_error("A .git directory already exists in the workspace root. Cannot colocate.")
        }
        _ => user_error_with_message(
            "Failed to move Git repository from .jj/repo/store/git to repository root directory.",
            err,
        ),
    })?;

    // Update the git_target file to point to the new location of the git repo
    let git_target_content = "../../../.git";
    std::fs::write(&git_target_path, git_target_content).context(git_target_path)?;

    // Then we must make the Git repository non-bare
    set_git_repo_bare(&dot_git_path, false)?;

    // Reload the workspace command helper to ensure it picks up the changes
    let mut workspace_command = reload_workspace_helper(ui, command, workspace_command)?;

    // Add a .jj/.gitignore file (if needed) to ensure that the colocated Git
    // repository does not track Jujutsu's repository
    maybe_add_gitignore(&workspace_command)?;

    // Finally, update git HEAD to point to the working-copy commit's parent
    let wc_commit = workspace_command.repo().store().get_commit(&wc_commit_id)?;
    set_git_head_to_wc_parent(ui, &mut workspace_command, &wc_commit)?;

    writeln!(
        ui.status(),
        "Repository successfully converted into a colocated Jujutsu/Git repository."
    )?;

    Ok(())
}

fn cmd_git_colocation_disable(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitColocationDisableArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

    // Make sure that the repository supports git colocation commands
    repo_supports_git_colocation_commands(&workspace_command)?;

    // Then ensure that the repo is colocated before proceeding
    if !is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(ui.status(), "Repository is already not colocated with Git.")?;
        return Ok(());
    }

    let workspace_root = workspace_command.workspace_root();
    let dot_jj_path = workspace_root.join(".jj");
    let git_store_path = workspace_command.repo_path().join("store").join("git");
    let git_target_path = workspace_command
        .repo_path()
        .join("store")
        .join("git_target");
    let dot_git_path = workspace_root.join(".git");
    let jj_gitignore_path = dot_jj_path.join(".gitignore");

    // Move the Git repository from .git into .jj/repo/store/git
    std::fs::rename(&dot_git_path, &git_store_path).map_err(|e| {
        user_error_with_message("Failed to move Git repository to .jj/repo/store/git", e)
    })?;

    // Make the Git repository bare
    set_git_repo_bare(&git_store_path, true)?;

    // Update the git_target file to point to the internal git store
    let git_target_content = "git";
    std::fs::write(&git_target_path, git_target_content).context(&git_target_path)?;

    // Remove the .jj/.gitignore file if it exists
    std::fs::remove_file(&jj_gitignore_path).ok();

    // Reload the workspace command helper to ensure it picks up the changes
    let mut workspace_command = reload_workspace_helper(ui, command, workspace_command)?;

    // And finally, remove the git HEAD reference
    remove_git_head(ui, &mut workspace_command)?;

    writeln!(
        ui.status(),
        "Repository successfully converted into a non-colocated Jujutsu/Git repository."
    )?;

    Ok(())
}

/// Set the Git repository at `path` to be bare or non-bare
fn set_git_repo_bare(path: &std::path::Path, bare: bool) -> Result<(), CommandError> {
    // TODO: use gix rather than shelling out
    let bare_str = if bare { "true" } else { "false" };
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["config", "core.bare", bare_str])
        .output()
        .map_err(|e| {
            user_error_with_message(
                format!("Failed to run Git config command to set core.bare to {bare_str}."),
                e,
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(user_error_with_message(
            format!("Failed to set core.bare to {bare_str} in Git config."),
            format!("Git config failed: {}", stderr.trim()),
        ));
    }
    Ok(())
}

/// Set the git HEAD to the working copy commit's parent
fn set_git_head_to_wc_parent(
    ui: &mut Ui,
    workspace_command: &mut crate::cli_util::WorkspaceCommandHelper,
    wc_commit: &Commit,
) -> Result<(), CommandError> {
    let mut tx = workspace_command.start_transaction();
    git::reset_head(tx.repo_mut(), wc_commit)?;
    if tx.repo().has_changes() {
        tx.finish(ui, "set git head to working copy parent")?;
    }
    Ok(())
}

/// Remove the git HEAD reference
fn remove_git_head(
    ui: &mut Ui,
    workspace_command: &mut crate::cli_util::WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    let mut tx = workspace_command.start_transaction();
    tx.repo_mut().set_git_head_target(RefTarget::absent());
    if tx.repo().has_changes() {
        tx.finish(ui, "remove git head reference")?;
    }
    Ok(())
}

/// Gets an up to date workspace helper to pick up changes made to the repo
fn reload_workspace_helper(
    ui: &mut Ui,
    command: &CommandHelper,
    workspace_command: crate::cli_util::WorkspaceCommandHelper,
) -> Result<crate::cli_util::WorkspaceCommandHelper, CommandError> {
    let workspace = command.load_workspace_at(
        workspace_command.workspace_root(),
        workspace_command.settings(),
    )?;
    let op = workspace
        .repo_loader()
        .load_operation(workspace_command.repo().op_id())?;
    let repo = workspace.repo_loader().load_at(&op)?;
    let workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    Ok(workspace_command)
}
