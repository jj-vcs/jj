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

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use bstr::ByteSlice as _;
use jj_lib::file_util;
use jj_lib::git;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::simple_workspace_store::SimpleWorkspaceStore;
use jj_lib::working_copy::WorkingCopyFactory;
use jj_lib::workspace::Workspace;
use jj_lib::workspace_store::WorkspaceStore as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::find_workspace_dir;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::GitLinkedWorktree;
use crate::git_util::discover_git_worktree_paths;
use crate::git_util::list_git_linked_worktrees;
use crate::git_util::repair_jj_repo_link;
use crate::git_util::unique_workspace_name;
use crate::git_util::workspace_name_for_path;
use crate::ui::Ui;

/// Adopt existing Git worktrees as jj workspaces
///
/// With no arguments, adopts the Git worktree at the current directory.
/// With worktree names, adopts those specific worktrees. With `--all`,
/// adopts all unadopted Git worktrees.
#[derive(clap::Args, Clone, Debug)]
pub struct GitWorktreeAdoptArgs {
    /// Git worktree IDs to adopt
    #[arg(conflicts_with = "all")]
    names: Vec<String>,

    /// Name for the adopted jj workspace
    #[arg(long, conflicts_with = "all")]
    name: Option<WorkspaceNameBuf>,

    /// Adopt all unadopted Git worktrees
    #[arg(long)]
    all: bool,
}

/// Synchronize jj workspaces with Git worktrees
///
/// In a colocated Git repository, this command reconciles jj workspace
/// state with Git worktree state. It adopts externally-created Git
/// worktrees as jj workspaces, forgets workspaces whose Git worktrees
/// have been removed, and repairs paths for moved worktrees.
///
/// Set `git.auto-sync-worktrees = true` to run this synchronization
/// automatically on every jj invocation.
#[derive(clap::Args, Clone, Debug)]
pub struct GitWorktreeSyncArgs {}

/// Manage Git worktrees
#[derive(clap::Subcommand, Clone, Debug)]
pub enum GitWorktreeCommand {
    Adopt(GitWorktreeAdoptArgs),
    Sync(GitWorktreeSyncArgs),
}

pub async fn cmd_git_worktree(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitWorktreeCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GitWorktreeCommand::Adopt(args) => cmd_git_worktree_adopt(ui, command, args).await,
        GitWorktreeCommand::Sync(args) => cmd_git_worktree_sync(ui, command, args).await,
    }
}

struct RepoContext<'a> {
    repo: Arc<ReadonlyRepo>,
    repo_path: PathBuf,
    working_copy_factory: &'a dyn WorkingCopyFactory,
}

fn adopted_workspace_name(
    command: &CommandHelper,
    ctx: &RepoContext<'_>,
    workspace_store: &SimpleWorkspaceStore,
    wt: &GitLinkedWorktree,
) -> Result<Option<WorkspaceNameBuf>, CommandError> {
    if let Some(name) = workspace_name_for_path(
        &ctx.repo,
        &ctx.repo_path,
        workspace_store,
        &wt.worktree_root,
    )? {
        return Ok(Some(name));
    }
    if !wt.worktree_root.join(".jj").is_dir() {
        return Ok(None);
    }
    repair_jj_repo_link(&wt.worktree_root, &ctx.repo_path)?;
    let workspace = command.load_workspace_at(&wt.worktree_root, command.settings())?;
    if workspace.repo_path() != ctx.repo_path
        || ctx
            .repo
            .view()
            .get_wc_commit_id(workspace.workspace_name())
            .is_none()
    {
        return Ok(None);
    }
    workspace_store.add(workspace.workspace_name(), workspace.workspace_root())?;
    Ok(Some(workspace.workspace_name().to_owned()))
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
    let workspace_store = SimpleWorkspaceStore::load(&ctx.repo_path)?;
    if args.name.is_some() && args.names.len() > 1 {
        return Err(user_error(
            "--name can only be used when adopting one Git worktree",
        ));
    }

    let to_adopt: Vec<&GitLinkedWorktree> = if args.all {
        let mut result = Vec::new();
        for wt in &linked_worktrees {
            if adopted_workspace_name(command, &ctx, &workspace_store, wt)?.is_none() {
                result.push(wt);
            }
        }
        result
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
        if let Some(workspace_name) = adopted_workspace_name(command, &ctx, &workspace_store, wt)? {
            return Err(user_error(format!(
                "Git worktree '{}' is already adopted as workspace '{}'",
                wt.id.to_str_lossy(),
                workspace_name.as_symbol()
            )));
        }
        vec![wt]
    } else {
        let mut result = Vec::new();
        let mut selected_paths = std::collections::HashSet::new();
        for name in &args.names {
            let wt = linked_worktrees
                .iter()
                .find(|wt| wt.id.as_slice() == name.as_bytes())
                .ok_or_else(|| user_error(format!("Git worktree '{name}' not found")))?;
            if let Some(workspace_name) =
                adopted_workspace_name(command, &ctx, &workspace_store, wt)?
            {
                return Err(user_error(format!(
                    "Git worktree '{name}' is already adopted as workspace '{}'",
                    workspace_name.as_symbol()
                )));
            }
            if selected_paths.insert(&wt.worktree_root) {
                result.push(wt);
            }
        }
        result
    };

    if to_adopt.is_empty() {
        writeln!(ui.status(), "No unadopted Git worktrees found.")?;
        return Ok(());
    }

    let mut repo = ctx.repo.clone();
    for (index, wt) in to_adopt.into_iter().enumerate() {
        let workspace_name = if index == 0 { args.name.clone() } else { None };
        repo = adopt_worktree(ui, command, &ctx, repo, wt, workspace_name).await?;
    }
    Ok(())
}

async fn adopt_worktree(
    ui: &mut Ui,
    command: &CommandHelper,
    ctx: &RepoContext<'_>,
    repo: Arc<ReadonlyRepo>,
    wt: &GitLinkedWorktree,
    workspace_name: Option<WorkspaceNameBuf>,
) -> Result<Arc<ReadonlyRepo>, CommandError> {
    let workspace_name = if let Some(name) = workspace_name {
        if name.as_str().is_empty() {
            return Err(user_error("New workspace name cannot be empty"));
        }
        if repo.view().get_wc_commit_id(&name).is_some() {
            return Err(user_error(format!(
                "Workspace named '{}' already exists",
                name.as_symbol()
            )));
        }
        name
    } else {
        unique_workspace_name(&repo, &wt.worktree_root)?
    };
    let (workspace, repo) = Workspace::init_workspace_with_existing_repo(
        &wt.worktree_root,
        &ctx.repo_path,
        &repo,
        ctx.working_copy_factory,
        workspace_name,
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

#[instrument(skip_all)]
async fn cmd_git_worktree_sync(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitWorktreeSyncArgs,
) -> Result<(), CommandError> {
    let ctx = resolve_repo_context(ui, command).await?;
    let linked_worktrees = list_git_linked_worktrees(&ctx.repo)?;
    let workspace_store = SimpleWorkspaceStore::load(&ctx.repo_path)?;
    let mut repo = ctx.repo.clone();
    for wt in &linked_worktrees {
        if adopted_workspace_name(command, &ctx, &workspace_store, wt)?.is_none() {
            repo = adopt_worktree(ui, command, &ctx, repo, wt, None).await?;
        }
    }
    let workspace = command.load_workspace_or_auto_init_git_worktree(ui).await?;
    let env = command.workspace_environment(ui, &workspace)?;
    let mut workspace_command = command.load_from_workspace(ui, workspace, env).await?;
    workspace_command.ensure_current_workspace_git_worktree(ui)?;
    workspace_command.forget_removed_git_worktrees(ui).await?;
    Ok(())
}
