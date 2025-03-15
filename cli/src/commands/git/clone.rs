// Copyright 2020-2023 The Jujutsu Authors
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
use std::io;
use std::io::Write as _;
use std::num::NonZeroU32;
use std::path::Path;

use jj_lib::git;
use jj_lib::git::GitFetch;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;
use jj_lib::workspace::Workspace;

use super::write_repository_level_trunk_alias;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::command_error::CommandError;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::absolute_git_url;
use crate::git_util::print_git_import_stats;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

/// Create a new repo backed by a clone of a Git repo
///
/// The Git repo will be a bare git repo stored inside the `.jj/` directory.
#[derive(clap::Args, Clone, Debug)]
pub struct GitCloneArgs {
    /// URL or path of the Git repo to clone
    ///
    /// Local path will be resolved to absolute form.
    #[arg(value_hint = clap::ValueHint::Url)]
    source: String,
    /// Specifies the target directory for the Jujutsu repository clone.
    /// If not provided, defaults to a directory named after the last component
    /// of the source URL. The full directory path will be created if it
    /// doesn't exist.
    #[arg(value_hint = clap::ValueHint::DirPath)]
    destination: Option<String>,
    /// Name of the newly created remote
    #[arg(long = "remote", default_value = "origin")]
    remote_name: RemoteNameBuf,
    /// Whether or not to colocate the Jujutsu repo with the git repo
    #[arg(long)]
    colocate: bool,
    /// Create a shallow clone of the given depth
    #[arg(long)]
    depth: Option<NonZeroU32>,
}

fn clone_destination_for_source(source: &str) -> Option<&str> {
    let destination = source.strip_suffix(".git").unwrap_or(source);
    let destination = destination.strip_suffix('/').unwrap_or(destination);
    destination
        .rsplit_once(&['/', '\\', ':'][..])
        .map(|(_, name)| name)
}

fn is_empty_dir(path: &Path) -> bool {
    if let Ok(mut entries) = path.read_dir() {
        entries.next().is_none()
    } else {
        false
    }
}

pub fn cmd_git_clone(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitCloneArgs,
) -> Result<(), CommandError> {
    let remote_name = &args.remote_name;
    if command.global_args().at_operation.is_some() {
        return Err(cli_error("--at-op is not respected"));
    }
    let source = absolute_git_url(command.cwd(), &args.source)?;
    let wc_path_str = args
        .destination
        .as_deref()
        .or_else(|| clone_destination_for_source(&source))
        .ok_or_else(|| user_error("No destination specified and wasn't able to guess it"))?;
    let wc_path = command.cwd().join(wc_path_str);

    let wc_path_existed = wc_path.exists();
    if wc_path_existed && !is_empty_dir(&wc_path) {
        return Err(user_error(
            "Destination path exists and is not an empty directory",
        ));
    }

    // will create a tree dir in case if was deleted after last check
    fs::create_dir_all(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;

    // Canonicalize because fs::remove_dir_all() doesn't seem to like e.g.
    // `/some/path/.`
    let canonical_wc_path = dunce::canonicalize(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;

    let clone_result = (|| -> Result<_, CommandError> {
        let workspace_command = init_workspace(ui, command, &canonical_wc_path, args.colocate)?;
        let mut workspace_command =
            configure_remote(ui, command, workspace_command, remote_name, &source)?;
        let default_branch = fetch_new_remote(ui, &mut workspace_command, remote_name, args.depth)?;
        Ok((workspace_command, default_branch))
    })();
    if clone_result.is_err() {
        let clean_up_dirs = || -> io::Result<()> {
            fs::remove_dir_all(canonical_wc_path.join(".jj"))?;
            if args.colocate {
                fs::remove_dir_all(canonical_wc_path.join(".git"))?;
            }
            if !wc_path_existed {
                fs::remove_dir(&canonical_wc_path)?;
            }
            Ok(())
        };
        if let Err(err) = clean_up_dirs() {
            writeln!(
                ui.warning_default(),
                "Failed to clean up {}: {}",
                canonical_wc_path.display(),
                err
            )
            .ok();
        }
    }

    let (mut workspace_command, default_branch) = clone_result?;
    if let Some(name) = &default_branch {
        let default_symbol = name.to_remote_symbol(remote_name);
        write_repository_level_trunk_alias(ui, workspace_command.repo_path(), default_symbol)?;

        let default_branch_remote_ref = workspace_command
            .repo()
            .view()
            .get_remote_bookmark(default_symbol);
        if let Some(commit_id) = default_branch_remote_ref.target.as_normal().cloned() {
            let mut checkout_tx = workspace_command.start_transaction();
            // For convenience, create local bookmark as Git would do.
            checkout_tx.repo_mut().track_remote_bookmark(default_symbol);
            if let Ok(commit) = checkout_tx.repo().store().get_commit(&commit_id) {
                checkout_tx.check_out(&commit)?;
            }
            checkout_tx.finish(ui, "check out git remote's default branch")?;
        }
    }
    Ok(())
}

fn init_workspace(
    ui: &Ui,
    command: &CommandHelper,
    wc_path: &Path,
    colocate: bool,
) -> Result<WorkspaceCommandHelper, CommandError> {
    let settings = command.settings_for_new_workspace(wc_path)?;
    let (workspace, repo) = if colocate {
        Workspace::init_colocated_git(&settings, wc_path)?
    } else {
        Workspace::init_internal_git(&settings, wc_path)?
    };
    let workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    maybe_add_gitignore(&workspace_command)?;
    Ok(workspace_command)
}

fn configure_remote(
    ui: &Ui,
    command: &CommandHelper,
    workspace_command: WorkspaceCommandHelper,
    remote_name: &RemoteName,
    source: &str,
) -> Result<WorkspaceCommandHelper, CommandError> {
    git::add_remote(workspace_command.repo().store(), remote_name, source)?;
    // Reload workspace to apply new remote configuration to
    // gix::ThreadSafeRepository behind the store.
    let workspace = command.load_workspace_at(
        workspace_command.workspace_root(),
        workspace_command.settings(),
    )?;
    let op = workspace
        .repo_loader()
        .load_operation(workspace_command.repo().op_id())?;
    let repo = workspace.repo_loader().load_at(&op)?;
    command.for_workable_repo(ui, workspace, repo)
}

fn fetch_new_remote(
    ui: &Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    remote_name: &RemoteName,
    depth: Option<NonZeroU32>,
) -> Result<Option<RefNameBuf>, CommandError> {
    writeln!(
        ui.status(),
        r#"Fetching into new repo in "{}""#,
        workspace_command.workspace_root().display()
    )?;
    let git_settings = workspace_command.settings().git_settings()?;
    let mut fetch_tx = workspace_command.start_transaction();
    let mut git_fetch = GitFetch::new(fetch_tx.repo_mut(), &git_settings)?;
    with_remote_git_callbacks(ui, |cb| {
        git_fetch.fetch(remote_name, &[StringPattern::everything()], cb, depth)
    })?;
    let default_branch = git_fetch.get_default_branch(remote_name)?;
    let import_stats = git_fetch.import_refs()?;
    print_git_import_stats(ui, fetch_tx.repo(), &import_stats, true)?;
    fetch_tx.finish(ui, "fetch from git remote into empty repo")?;
    Ok(default_branch)
}
