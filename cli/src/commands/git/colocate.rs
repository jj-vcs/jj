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

use std::path::Path;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::git_util::is_colocated_git_workspace;
use crate::ui::Ui;

/// Manage Jujutsu repository colocation with Git
#[derive(clap::Args, Clone, Debug)]
pub struct GitColocateArgs {
    /// Enable co-location (convert into a co-located Jujutsu/Git repository)
    #[arg(long)]
    enable: bool,

    /// Disable co-location (convert into a non co-located Jujutsu/Git
    /// repository)
    #[arg(long)]
    disable: bool,
}

pub fn cmd_git_colocate(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitColocateArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    // Validate that both flags aren't used together
    if args.enable && args.disable {
        return Err(user_error(
            "Cannot specify both --enable and --disable flags",
        ));
    }

    if args.enable {
        enable_repository_colocation(ui, &mut workspace_command)
    } else if args.disable {
        disable_repository_colocation(ui, &mut workspace_command)
    } else {
        show_status(ui, &mut workspace_command)
    }
}

fn show_status(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    let is_colocated =
        is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo());

    if is_colocated {
        writeln!(ui.status(), "Repository is currently co-located with Git")?;
        writeln!(
            ui.status(),
            "To disable co-location, run: jj git colocate --disable"
        )?;
    } else {
        writeln!(
            ui.status(),
            "Repository is currently not co-located with Git"
        )?;
        writeln!(
            ui.status(),
            "To enable co-location, run: jj git colocate --enable"
        )?;
    }

    Ok(())
}

fn enable_repository_colocation(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    if is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(ui.status(), "Repository is already co-located with Git.")?;
        return Ok(());
    }

    let workspace_root = workspace_command.workspace_root();
    let dot_jj_path = workspace_root.join(".jj");
    let jj_repo_path = dot_jj_path.join("repo");
    let git_store_path = jj_repo_path.join("store").join("git");
    let git_target_path = jj_repo_path.join("store").join("git_target");
    let dot_git_path = workspace_root.join(".git");

    // Bail out if a git repo already exist at the root folder
    if dot_git_path.exists() {
        return Err(user_error(
            "A .git directory already exists in the workspace root. Cannot co-locate.",
        ));
    }
    // or if the Jujutsu repo is a workspace
    if jj_repo_path.is_file() {
        return Err(user_error("Cannot co-locate a Jujutsu workspace."));
    }
    // or if it is not backed by git
    if !git_store_path.exists() {
        return Err(user_error(
            "git store not found. This repository might not be using the git back-end.",
        ));
    }

    // Create a .gitignore file in the .jj directory that ensures that the root
    // git repo completely ignores the .jj directory
    // Note that if a .jj/.gitignore already exists it will be overwritten
    // This should be fine since it does not make sense to only ignore parts of
    // the .jj directory
    let jj_gitignore_path = dot_jj_path.join(".gitignore");
    std::fs::write(&jj_gitignore_path, "/*\n")
        .map_err(|e| user_error_with_message("Failed to create .jj/.gitignore file.", e))?;

    // Create a git_target file pointing to the new location
    // Note that we do this first so that it is easier to revert the operation
    // in case there is a failure in this step or the next
    let git_target_content = "../../../.git";
    std::fs::write(&git_target_path, git_target_content)
        .map_err(|e| user_error_with_message("Failed to create git_target file.", e))?;

    // Move the git repository from .jj/repo/store/git to .git
    if let Err(e) = move_directory(&git_store_path, &dot_git_path) {
        // Attempt to delete git_target_path if move fails and show an error message
        let _ = std::fs::remove_file(&git_target_path);
        return Err(user_error_with_message(
            "Failed to move git repository from .jj/repo/store/git to repository root directory.",
            e,
        ));
    }

    // Make the co-located git repository non-bare
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
                "Failed to unset core.bare in git config.",
                format!("git config failed: {}", stderr.trim()),
            ));
        }
        Err(e) => {
            return Err(user_error_with_message(
                "Failed to run git config command to unset core.bare.",
                e,
            ));
        }
    }

    // Finally, update git HEAD by taking a snapshot which triggers git export
    // This will update .git/HEAD to point to the working-copy commit's parent
    workspace_command.maybe_snapshot(ui)?;

    writeln!(
        ui.status(),
        "Repository successfully converted into a co-located Jujutsu/git repository."
    )?;

    Ok(())
}

fn disable_repository_colocation(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    // Check if the repo is colocated before proceeding
    if !is_colocated_git_workspace(workspace_command.workspace(), workspace_command.repo()) {
        writeln!(
            ui.status(),
            "Repository is already not co-located with Git."
        )?;
        return Ok(());
    }

    let workspace_root = workspace_command.workspace_root();
    let dot_jj_path = workspace_root.join(".jj");
    let git_store_path = dot_jj_path.join("repo").join("store").join("git");
    let git_target_path = dot_jj_path.join("repo").join("store").join("git_target");
    let dot_git_path = workspace_root.join(".git");
    let jj_gitignore_path = dot_jj_path.join(".gitignore");

    // Do not proceed if there is no .git directory at the root folder
    if !dot_git_path.exists() {
        return Err(user_error("No .git directory found in workspace root."));
    }

    // Or if a git repo already exist inside Jujutsu's repo store
    if git_store_path.exists() {
        return Err(user_error(
            "git store already exists at .jj/repo/store/git. Cannot disable co-location.",
        ));
    }

    // Make the git repository bare
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&dot_git_path)
        .args(["config", "core.bare", "true"])
        .output()
        .map_err(|e| {
            user_error_with_message("Failed to run git config command to set core.bare.", e)
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(user_error_with_message(
            "Failed to set core.bare in git config.",
            format!("git config failed: {}", stderr.trim()),
        ));
    }

    // Move the git repository from .git into .jj/repo/store/git
    move_directory(&dot_git_path, &git_store_path).map_err(|e| {
        user_error_with_message("Failed to move git repository to .jj/repo/store/git", e)
    })?;

    // Update the git_target file to point to the internal git store
    let git_target_content = "git";
    std::fs::write(&git_target_path, git_target_content)
        .map_err(|e| user_error_with_message("Failed to update git_target file.", e))?;

    // Remove the .jj/.gitignore file if it exists
    if jj_gitignore_path.exists() {
        std::fs::remove_file(&jj_gitignore_path)
            .map_err(|e| user_error_with_message("Failed to remove .jj/.gitignore file.", e))?;
    }

    writeln!(
        ui.status(),
        "Repository successfully converted into a non co-located regular Jujutsu repository."
    )?;

    Ok(())
}

/// Cross-platform directory move operation
fn move_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    // Try a rename first, falling back to copy + remove in case of failure
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // If rename fails, do a recursive copy and delete
            copy_dir_recursive(from, to)?;
            std::fs::remove_dir_all(from)?;
            Ok(())
        }
    }
}

/// Recursively copy a directory to handle cross-filesystem moves
fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::fs;

    if !to.exists() {
        fs::create_dir_all(to)?;
    }

    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dest_path = to.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path)?;
        }
    }

    Ok(())
}
