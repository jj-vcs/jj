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

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use clap_complete::ArgValueCandidates;
use jj_lib::file_util;
#[cfg(feature = "git")]
use jj_lib::git::GitSubprocessOptions;
use jj_lib::ref_name::WorkspaceNameBuf;
#[cfg(feature = "git")]
use jj_lib::repo::Repo as _;
use jj_lib::workspace::write_workspace_repo_link;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
#[cfg(feature = "git")]
use crate::git_util::is_colocated_git_workspace;
use crate::ui::Ui;

/// Move a workspace to a new path
///
/// The workspace containing the repository cannot be moved. The source and
/// destination must be on the same filesystem. On Windows, the command must be
/// run from outside the workspace being moved.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceMoveArgs {
    /// Workspace to move
    #[arg(add = ArgValueCandidates::new(complete::workspaces))]
    workspace: WorkspaceNameBuf,

    /// New path for the workspace
    destination: PathBuf,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_move(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceMoveArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    workspace_command.check_working_copy_writable()?;
    let repo = workspace_command.repo();
    let workspace_name = &*args.workspace;

    if repo.view().get_wc_commit_id(workspace_name).is_none() {
        return Err(user_error(format!(
            "No such workspace: {}",
            workspace_name.as_symbol()
        )));
    }

    let workspace_store = repo.loader().workspace_store();
    let source_path = workspace_store
        .get_workspace_path(workspace_name)?
        .ok_or_else(|| {
            user_error(format!(
                "Workspace has no recorded path: {}",
                workspace_name.as_symbol()
            ))
        })?;
    let source_path = dunce::canonicalize(workspace_command.repo_path().join(source_path))
        .map_err(|err| user_error_with_message("Cannot access workspace directory", err))?;
    if workspace_command.repo_path().starts_with(&source_path) {
        return Err(user_error(format!(
            "Cannot move workspace '{}' because it contains the repository",
            workspace_name.as_symbol()
        )));
    }

    let source_workspace = command.load_workspace_at(&source_path, workspace_command.settings())?;
    if source_workspace.repo_path() != workspace_command.repo_path()
        || source_workspace.workspace_name() != workspace_name
    {
        return Err(user_error(format!(
            "Recorded path for workspace '{}' belongs to another workspace",
            workspace_name.as_symbol()
        )));
    }

    #[cfg(feature = "git")]
    let is_colocated = is_colocated_git_workspace(&source_workspace)?;
    #[cfg(feature = "git")]
    let git_subprocess_options = is_colocated
        .then(|| GitSubprocessOptions::from_settings(workspace_command.settings()))
        .transpose()?;

    let mut destination_path = normalize_destination(command.cwd(), &args.destination)?;
    match destination_path.symlink_metadata() {
        Ok(_) => {
            if !dunce::canonicalize(&destination_path).is_ok_and(|path| path == source_path) {
                return Err(user_error("Destination path already exists"));
            }
            destination_path = source_path.clone();
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(user_error_with_message(
                "Cannot access destination path",
                err,
            ));
        }
    }

    if destination_path == source_path {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    #[cfg(windows)]
    if command.cwd().starts_with(&source_path) {
        return Err(user_error(
            "Cannot move a workspace containing the current directory on Windows",
        )
        .hinted("Run the command from outside the workspace directory."));
    }

    fs::rename(&source_path, &destination_path)
        .map_err(|err| user_error_with_message("Failed to move workspace directory", err))?;
    if let Err(err) = write_workspace_repo_link(&destination_path, workspace_command.repo_path()) {
        if let Err(rollback_err) = rollback_move(
            &destination_path,
            &source_path,
            workspace_command.repo_path(),
        ) {
            return Err(rollback_err.hinted(format!("The original error was: {err}")));
        }
        return Err(err.into());
    }

    #[cfg(feature = "git")]
    if let Some(subprocess_options) = &git_subprocess_options
        && let Err(err) = jj_lib::git::repair_worktree(
            repo.store(),
            subprocess_options.clone(),
            &destination_path,
        )
    {
        if let Err(rollback_err) = rollback_move(
            &destination_path,
            &source_path,
            workspace_command.repo_path(),
        ) {
            return Err(rollback_err.hinted(format!("The original error was: {err}")));
        }
        if let Err(rollback_err) =
            jj_lib::git::repair_worktree(repo.store(), subprocess_options.clone(), &source_path)
        {
            let rollback_err: CommandError = rollback_err.into();
            return Err(rollback_err.hinted(format!("The original error was: {err}")));
        }
        return Err(err.into());
    }

    if let Err(err) = workspace_store.add(workspace_name, &destination_path) {
        if let Err(rollback_err) = rollback_move(
            &destination_path,
            &source_path,
            workspace_command.repo_path(),
        ) {
            return Err(rollback_err.hinted(format!("The original error was: {err}")));
        }
        #[cfg(feature = "git")]
        if let Some(subprocess_options) = &git_subprocess_options
            && let Err(rollback_err) =
                jj_lib::git::repair_worktree(repo.store(), subprocess_options.clone(), &source_path)
        {
            let rollback_err: CommandError = rollback_err.into();
            return Err(rollback_err.hinted(format!("The original error was: {err}")));
        }
        return Err(err.into());
    }

    writeln!(
        ui.status(),
        r#"Moved workspace '{}' to "{}"."#,
        workspace_name.as_symbol(),
        file_util::relative_path(command.cwd(), &destination_path).display()
    )?;
    Ok(())
}

fn normalize_destination(cwd: &Path, destination: &Path) -> Result<PathBuf, CommandError> {
    let destination = cwd.join(destination);
    let parent = destination
        .parent()
        .ok_or_else(|| user_error("Destination path has no parent directory"))?;
    let file_name = destination
        .file_name()
        .ok_or_else(|| user_error("Destination path has no file name"))?;
    let parent = dunce::canonicalize(parent).map_err(|err| {
        user_error_with_message("Cannot access destination parent directory", err)
    })?;
    Ok(parent.join(file_name))
}

fn rollback_move(destination: &Path, source: &Path, repo_path: &Path) -> Result<(), CommandError> {
    fs::rename(destination, source)
        .map_err(|err| user_error_with_message("Failed to restore workspace directory", err))?;
    write_workspace_repo_link(source, repo_path)?;
    Ok(())
}
