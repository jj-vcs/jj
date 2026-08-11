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

use std::io::ErrorKind;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use bstr::ByteSlice as _;
use jj_lib::file_util;
use jj_lib::git;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::working_copy::WorkingCopyFactory;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::find_workspace_dir;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::discover_git_worktree_paths;
use crate::ui::Ui;

/// Adopt existing Git worktrees as jj workspaces
///
/// With no arguments, adopts the Git worktree at the current directory.
/// With worktree names, adopts those specific worktrees. With `--all`,
/// adopts all unadopted Git worktrees.
#[derive(clap::Args, Clone, Debug)]
pub struct GitWorktreeAdoptArgs {
    /// Names of Git worktrees to adopt
    #[arg(conflicts_with = "all")]
    names: Vec<String>,

    /// Adopt all unadopted Git worktrees
    #[arg(long)]
    all: bool,
}

/// Manage Git worktrees
#[derive(clap::Subcommand, Clone, Debug)]
pub enum GitWorktreeCommand {
    Adopt(GitWorktreeAdoptArgs),
}

pub async fn cmd_git_worktree(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitWorktreeCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GitWorktreeCommand::Adopt(args) => cmd_git_worktree_adopt(ui, command, args).await,
    }
}

struct GitLinkedWorktree {
    name: WorkspaceNameBuf,
    worktree_root: PathBuf,
}

fn list_git_linked_worktrees(repo: &ReadonlyRepo) -> Result<Vec<GitLinkedWorktree>, CommandError> {
    let git_repo = git::get_git_backend(repo.store())?.git_repo();
    // Only linked worktrees are listed, so the main workspace never appears.
    let proxies = git_repo
        .worktrees()
        .map_err(|err| user_error_with_message("Failed to list Git worktrees", err))?;
    let mut worktrees = Vec::new();
    for proxy in proxies {
        // Skip worktrees whose checkout has gone missing.
        let Ok(base) = proxy.base() else {
            continue;
        };
        let worktree_root = match dunce::canonicalize(&base) {
            Ok(path) => path,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(user_error_with_message(
                    format!("Failed to resolve Git worktree '{}'", base.display()),
                    err,
                ));
            }
        };
        let Some(name) = proxy.id().to_str().ok().filter(|name| !name.is_empty()) else {
            continue;
        };
        worktrees.push(GitLinkedWorktree {
            name: name.into(),
            worktree_root,
        });
    }
    Ok(worktrees)
}

struct RepoContext<'a> {
    repo: Arc<ReadonlyRepo>,
    repo_path: PathBuf,
    working_copy_factory: &'a dyn WorkingCopyFactory,
}

async fn resolve_repo_context<'a>(
    ui: &mut Ui,
    command: &'a CommandHelper,
) -> Result<RepoContext<'a>, CommandError> {
    if find_workspace_dir(command.cwd()).join(".jj").is_dir() {
        let workspace = command.load_workspace()?;
        let repo = workspace.repo_loader().load_at_head().await?;
        git::get_git_backend(repo.store())?;
        let repo_path = workspace.repo_path().to_owned();
        let working_copy_factory = command.get_working_copy_factory()?;
        return Ok(RepoContext {
            repo,
            repo_path,
            working_copy_factory,
        });
    }

    let Some(git_paths) = discover_git_worktree_paths(command.cwd())? else {
        return Err(user_error("Not inside a jj workspace or Git worktree"));
    };
    let main_workspace_root = match git_paths.common_git_dir.parent() {
        Some(path) if path.join(".jj").is_dir() => path,
        _ => {
            return Err(user_error(
                "The Git worktree's main repository is not a colocated jj repo",
            ));
        }
    };
    let (main_settings, _) = command.settings_for_new_workspace(ui, main_workspace_root)?;
    let main_workspace = command.load_workspace_at(main_workspace_root, &main_settings)?;
    let working_copy_factory = command.get_working_copy_factory_at(main_workspace_root)?;
    let repo = main_workspace.repo_loader().load_at_head().await?;
    let repo_path = main_workspace.repo_path().to_owned();
    Ok(RepoContext {
        repo,
        repo_path,
        working_copy_factory,
    })
}

#[instrument(skip_all)]
async fn cmd_git_worktree_adopt(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitWorktreeAdoptArgs,
) -> Result<(), CommandError> {
    let ctx = resolve_repo_context(ui, command).await?;
    let linked_worktrees = list_git_linked_worktrees(&ctx.repo)?;

    let to_adopt: Vec<&GitLinkedWorktree> = if args.all {
        linked_worktrees
            .iter()
            .filter(|wt| ctx.repo.view().get_wc_commit_id(&wt.name).is_none())
            .collect()
    } else if args.names.is_empty() {
        let Some(git_paths) = discover_git_worktree_paths(command.cwd())? else {
            return Err(user_error(
                "Not inside a linked Git worktree. Run this from within a Git worktree, or pass \
                 worktree names to adopt.",
            ));
        };
        let wt = linked_worktrees
            .iter()
            .find(|wt| wt.worktree_root == git_paths.worktree_root)
            .ok_or_else(|| {
                user_error(
                    "Not inside a linked Git worktree. Run this from within a Git worktree, or \
                     pass worktree names to adopt.",
                )
            })?;
        if ctx.repo.view().get_wc_commit_id(&wt.name).is_some() {
            return Err(user_error(format!(
                "Workspace named '{name}' already exists",
                name = wt.name.as_symbol()
            )));
        }
        vec![wt]
    } else {
        let mut result = Vec::new();
        for name in &args.names {
            let wt = linked_worktrees
                .iter()
                .find(|wt| wt.name.as_str() == name)
                .ok_or_else(|| user_error(format!("Git worktree '{name}' not found")))?;
            if ctx.repo.view().get_wc_commit_id(&wt.name).is_some() {
                return Err(user_error(format!(
                    "Workspace named '{name}' already exists"
                )));
            }
            result.push(wt);
        }
        result
    };

    if to_adopt.is_empty() {
        writeln!(ui.status(), "No unadopted Git worktrees found.")?;
        return Ok(());
    }

    let mut repo = ctx.repo.clone();
    for wt in to_adopt {
        repo = adopt_worktree(ui, command, &ctx, repo, wt).await?;
    }
    Ok(())
}

async fn adopt_worktree(
    ui: &mut Ui,
    command: &CommandHelper,
    ctx: &RepoContext<'_>,
    repo: Arc<ReadonlyRepo>,
    wt: &GitLinkedWorktree,
) -> Result<Arc<ReadonlyRepo>, CommandError> {
    let (workspace, repo) = Workspace::init_workspace_with_existing_repo(
        &wt.worktree_root,
        &ctx.repo_path,
        &repo,
        ctx.working_copy_factory,
        wt.name.clone(),
    )
    .await?;
    let mut workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    // The adopted directory is a Git worktree, so keep Git from tracking
    // the workspace's own .jj directory.
    maybe_add_gitignore(&workspace_command)?;
    // Import Git HEAD so the working-copy commit sits on the worktree's
    // checked-out revision; snapshot then picks up uncommitted Git changes.
    workspace_command.maybe_snapshot(ui).await?;
    writeln!(
        ui.status(),
        "Created workspace in \"{}\"",
        file_util::relative_path(command.cwd(), &wt.worktree_root).display()
    )?;
    Ok(workspace_command.repo().clone())
}
