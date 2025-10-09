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
use jj_lib::git::get_git_repo;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::is_colocated_git_workspace;
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationEnableArgs {}

#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationDisableArgs {}

#[derive(clap::Args, Clone, Debug)]
pub struct GitColocationStatusArgs {}

/// Manage Jujutsu repository colocation with Git
#[derive(clap::Subcommand, Clone, Debug)]
pub enum GitColocationCommand {
    /// Enable colocation (convert into a colocated Jujutsu/Git repository)
    ///
    /// This moves the underlying Git repository that is found inside the .jj
    /// directory to the root of the Jujutsu workspace. This allows you to
    /// use Git commands directly in the Jujutsu workspace.
    Enable(GitColocationEnableArgs),
    /// Disable colocation (convert into a non-colocated Jujutsu/Git
    /// repository)
    ///
    /// This moves the Git repository that is at the root of the Jujutsu
    /// workspace into the .jj directory. Once this is done you will no longer
    /// be able to use Git commands directly in the Jujutsu workspace.
    Disable(GitColocationDisableArgs),
    /// Show the current colocation status
    Status(GitColocationStatusArgs),
}

pub fn cmd_git_colocation(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitColocationCommand,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

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

    match subcommand {
        GitColocationCommand::Enable(args) => cmd_git_colocation_enable(ui, command, args),
        GitColocationCommand::Disable(args) => cmd_git_colocation_disable(ui, command, args),
        GitColocationCommand::Status(args) => cmd_git_colocation_status(ui, command, args),
    }
}

fn cmd_git_colocation_status(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitColocationStatusArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
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

    // Then we must make the Git repository non-bare
    make_git_repo_non_bare(dot_git_path)?;

    // Reload the workspace command helper to ensure it picks up the changes
    let workspace_command = reload_workspace_helper(ui, command, workspace_command)?;

    // Add a .jj/.gitignore file (if needed) to ensure that the colocated Git
    // repository does not track Jujutsu's repository
    maybe_add_gitignore(&workspace_command)?;

    // Finally, update git HEAD to point to the working-copy commit's parent
    // Get the working copy commit ID before starting the transaction to avoid
    // borrowing issues
    let wc_commit_id = workspace_command.get_wc_commit_id().unwrap().clone();
    set_git_head_to_commit_id(workspace_command, &wc_commit_id)?;

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

    make_git_repo_bare(&dot_git_path)?;

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

    // Reload the workspace command helper to ensure it picks up the changes
    let workspace_command = reload_workspace_helper(ui, command, workspace_command)?;

    // Finally, update the git HEAD to point to the main branch
    reset_git_head_to_main_branch(workspace_command)?;

    writeln!(
        ui.status(),
        "Repository successfully converted into a non-colocated regular Jujutsu repository."
    )?;

    Ok(())
}

/// Set the git HEAD to a particular commit ID
pub fn set_git_head_to_commit_id(
    workspace_command: crate::cli_util::WorkspaceCommandHelper,
    commit_id: &jj_lib::backend::CommitId,
) -> Result<(), CommandError> {
    let git_repo = get_git_repo(workspace_command.repo().store())
        .map_err(|e| user_error_with_message("Failed to access the Git repository", e))?;
    let target = gix::ObjectId::from_hex(commit_id.hex().as_bytes()).unwrap();
    let ref_edit = gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Object(target),
        },
        name: "HEAD".try_into().unwrap(),
        deref: false,
    };
    git_repo
        .edit_reference(ref_edit)
        .map_err(|e| user_error_with_message("Failed to update git HEAD", e))?;
    Ok(())
}

/// Reset the git HEAD to point to the main branch
fn reset_git_head_to_main_branch(
    workspace_command: crate::cli_util::WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    let git_repo = get_git_repo(workspace_command.repo().store())
        .map_err(|e| user_error_with_message("Failed to access the Git repository", e))?;
    let ref_edit = gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange {
                message: "jj: set HEAD to main branch after disabling colocation".into(),
                ..Default::default()
            },
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Symbolic("refs/heads/main".try_into().unwrap()),
        },
        name: "HEAD".try_into().unwrap(),
        deref: false,
    };
    git_repo
        .edit_reference(ref_edit)
        .map_err(|e| user_error_with_message("Failed to update git HEAD", e))?;
    Ok(())
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

fn make_git_repo_bare(dot_git_path: &std::path::PathBuf) -> Result<(), CommandError> {
    // TODO: use gix rather than shelling out
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dot_git_path)
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
    Ok(())
}

fn make_git_repo_non_bare(dot_git_path: std::path::PathBuf) -> Result<(), CommandError> {
    // TODO: use gix rather than shelling out
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&dot_git_path)
        .args(["config", "--unset", "core.bare"])
        .output()
        .map_err(|e| {
            user_error_with_message("Failed to run Git config command to unset core.bare.", e)
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(user_error_with_message(
            "Failed to unset core.bare in Git config.",
            format!("Git config failed: {}", stderr.trim()),
        ));
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
    let mut workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    workspace_command.maybe_snapshot(ui)?;
    Ok(workspace_command)
}
