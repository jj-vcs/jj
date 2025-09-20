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

use std::fs;
use std::io::Write as _;
use std::path::Path;

use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::git;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::git_util::is_colocated_git_workspace;
use crate::ui::Ui;

/// Manage Jujutsu repository colocation with Git
#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationArgs {
    #[command(subcommand)]
    command: GitColocationCommand,
}

#[derive(clap::Subcommand, Clone, Debug)]
pub enum GitColocationCommand {
    /// Enable colocation (convert into a colocated Jujutsu/Git repository)
    ///
    /// This moves the underlying Git repository that is found inside the .jj
    /// directory to the root of the Jujutsu workspace. This allows you to
    /// use Git commands directly in the Jujutsu workspace.
    Enable,
    /// Disable colocation (convert into a non-colocated Jujutsu/Git
    /// repository)
    ///
    /// This moves the Git repository that is at the root of the Jujutsu
    /// workspace into the .jj directory. Once this is done you will no longer
    /// be able to use Git commands directly in the Jujutsu workspace.
    Disable,
    /// Show the current colocation status
    Status,
}

/// Cross-platform directory move operation
fn move_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    // Try a rename first, falling back to copy + remove in case of failure
    if std::fs::rename(from, to).is_err() {
        // If rename fails, do a recursive copy and delete
        copy_dir_recursive(from, to).map_err(|e| e.source)?;
        std::fs::remove_dir_all(from)?;
    }
    Ok(())
}

/// Recursively copy a directory to handle cross-filesystem moves
fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), PathError> {
    if !to.exists() {
        fs::create_dir_all(to).context(to)?;
    }

    for entry in fs::read_dir(from).context(from)? {
        let entry = entry.context(from)?;
        let file_type = entry.file_type().context(entry.path())?;
        let src_path = entry.path();
        let dest_path = to.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path).context(&dest_path)?;
        }
    }

    Ok(())
}

pub fn cmd_git_colocation(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitColocationArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    // Check if backend is Git
    if git::get_git_backend(workspace_command.repo().store()).is_err() {
        return Err(user_error(
            "This command requires a repository backed by Git. This repository appears to be \
             using a different backend.",
        ));
    }

    // Ensure that this is the main workspace
    let workspace_root = workspace_command.workspace_root();
    let jj_repo_path = workspace_root.join(".jj").join("repo");
    if jj_repo_path.is_file() {
        return Err(user_error(
            "This command cannot be used in a non-main Jujutsu workspace.",
        ));
    }

    match &args.command {
        GitColocationCommand::Enable => cmd_git_colocation_enable(ui, &mut workspace_command),
        GitColocationCommand::Disable => cmd_git_colocation_disable(ui, &mut workspace_command),
        GitColocationCommand::Status => cmd_git_colocation_status(ui, &mut workspace_command),
    }
}

fn cmd_git_colocation_status(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
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
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    // Check if the repo is already colocated before proceeding
    if is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(ui.status(), "Repository is already colocated with Git.")?;
        return Ok(());
    }

    let workspace_root = workspace_command.workspace_root();
    let dot_jj_path = workspace_root.join(".jj");
    let jj_repo_path = dot_jj_path.join("repo");
    let git_store_path = jj_repo_path.join("store").join("git");
    let git_target_path = jj_repo_path.join("store").join("git_target");
    let dot_git_path = workspace_root.join(".git");

    // This is called after checking that the repo is backed by git, is not
    // already colocated, and is a main workspace, but we must still bail out
    // if a git repo already exist at the root folder
    if dot_git_path.exists() {
        return Err(user_error(
            "A .git directory already exists in the workspace root. Cannot colocate.",
        ));
    }

    // Create a .gitignore file in the .jj directory that ensures that the root
    // git repo completely ignores the .jj directory
    // Note that if a .jj/.gitignore already exists it will be overwritten
    // This should be fine since it does not make sense to only ignore parts of
    // the .jj directory
    let jj_gitignore_path = dot_jj_path.join(".gitignore");
    std::fs::write(&jj_gitignore_path, "/*\n").context(&jj_gitignore_path)?;

    // Update the git_target file to point to the new location of the git repo
    // Note that we do this first so that it is easier to revert the operation
    // in case there is a failure in this step or the next
    let git_target_content = "../../../.git";
    std::fs::write(&git_target_path, git_target_content).context(git_target_content)?;

    // Move the git repository from .jj/repo/store/git to .git
    if let Err(e) = move_directory(&git_store_path, &dot_git_path) {
        // Attempt to delete git_target_path if move fails and show an error message
        let _ = std::fs::remove_file(&git_target_path);
        return Err(user_error_with_message(
            "Failed to move Git repository from .jj/repo/store/git to repository root directory.",
            e,
        ));
    }

    // Make the colocated git repository non-bare
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&dot_git_path)
        .args(["config", "--unset", "core.bare"])
        .output();

    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(user_error_with_message(
                "Failed to unset core.bare in Git config.",
                format!("Git config failed: {}", stderr.trim()),
            ));
        }
        Err(e) => {
            return Err(user_error_with_message(
                "Failed to run Git config command to unset core.bare.",
                e,
            ));
        }
    }

    // Finally, update git HEAD by taking a snapshot which triggers git export
    // This will update .git/HEAD to point to the working-copy commit's parent
    workspace_command.maybe_snapshot(ui)?;

    writeln!(
        ui.status(),
        "Repository successfully converted into a colocated Jujutsu/git repository."
    )?;

    Ok(())
}

fn cmd_git_colocation_disable(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    // Check if the repo is not colocated before proceeding
    if !is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(ui.status(), "Repository is already not colocated with Git.")?;
        return Ok(());
    }

    let workspace_root = workspace_command.workspace_root();
    let dot_jj_path = workspace_root.join(".jj");
    let git_store_path = dot_jj_path.join("repo").join("store").join("git");
    let git_target_path = dot_jj_path.join("repo").join("store").join("git_target");
    let dot_git_path = workspace_root.join(".git");
    let jj_gitignore_path = dot_jj_path.join(".gitignore");

    // This is called after checking that the repo is backed by git, is not
    // already colocated, and is a main workspace, but we must still bail out
    // if there is no .git directory at the root folder
    if !dot_git_path.exists() {
        return Err(user_error("No .git directory found in workspace root."));
    }

    // Or if a git repo already exist inside Jujutsu's repo store
    if git_store_path.exists() {
        return Err(user_error(
            "Git store already exists at .jj/repo/store/git. Cannot disable colocation.",
        ));
    }

    // Make the git repository bare
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&dot_git_path)
        .args(["config", "core.bare", "true"])
        .output()
        .map_err(|e| {
            user_error_with_message("Failed to run Git config command to set core.bare.", e)
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(user_error_with_message(
            "Failed to set core.bare in Git config.",
            format!("Git config failed: {}", stderr.trim()),
        ));
    }

    // Move the Git repository from .git into .jj/repo/store/git
    move_directory(&dot_git_path, &git_store_path).map_err(|e| {
        user_error_with_message("Failed to move Git repository to .jj/repo/store/git", e)
    })?;

    // Update the git_target file to point to the internal git store
    let git_target_content = "git";
    std::fs::write(&git_target_path, git_target_content).context(&git_target_path)?;

    // Remove the .jj/.gitignore file if it exists
    if jj_gitignore_path.exists() {
        std::fs::remove_file(&jj_gitignore_path).context(&jj_gitignore_path)?;
    }

    writeln!(
        ui.status(),
        "Repository successfully converted into a non colocated regular Jujutsu repository."
    )?;

    Ok(())
}
