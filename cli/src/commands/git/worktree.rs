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
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use jj_lib::git;
use jj_lib::git::GitSettings;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::working_copy::WorkingCopyFactory;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
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

fn list_git_linked_worktrees(
    git_executable: &Path,
    main_workspace_root: &Path,
) -> Result<Vec<GitLinkedWorktree>, CommandError> {
    use std::process::Command;

    use bstr::ByteSlice as _;

    let output = Command::new(git_executable)
        .args(["worktree", "list", "--porcelain", "-z"])
        .current_dir(main_workspace_root)
        .output()
        .map_err(|err| user_error_with_message("Failed to run `git worktree list`", err))?;
    if !output.status.success() {
        return Err(user_error(format!(
            "Failed to list Git worktrees: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let main_root = dunce::canonicalize(main_workspace_root).ok();
    let mut worktrees = Vec::new();
    for block in output.stdout.split_str(b"\0\0") {
        for field in block.split_str(b"\0") {
            if let Some(path) = field.strip_prefix(b"worktree ") {
                let Ok(path_str) = path.to_str() else {
                    continue;
                };
                let Ok(worktree_root) = dunce::canonicalize(path_str) else {
                    continue;
                };
                if main_root.as_ref() == Some(&worktree_root) {
                    continue;
                }
                let Some(name) = worktree_root.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                worktrees.push(GitLinkedWorktree {
                    name: name.into(),
                    worktree_root,
                });
            }
        }
    }
    Ok(worktrees)
}

struct RepoContext<'a> {
    repo: Arc<ReadonlyRepo>,
    repo_path: PathBuf,
    main_workspace_root: PathBuf,
    git_executable: PathBuf,
    working_copy_factory: &'a dyn WorkingCopyFactory,
}

async fn resolve_repo_context<'a>(
    ui: &mut Ui,
    command: &'a CommandHelper,
) -> Result<RepoContext<'a>, CommandError> {
    let git_settings = GitSettings::from_settings(command.settings())?;
    let git_executable = git_settings.executable_path.clone();

    if let Ok(workspace) = command.load_workspace() {
        let repo = workspace.repo_loader().load_at_head().await?;
        let git_backend = git::get_git_backend(repo.store())?;
        let main_workspace_root = git_backend
            .git_workdir()
            .ok_or_else(|| user_error("Cannot adopt: bare Git repository."))?
            .to_owned();
        let repo_path = workspace.repo_path().to_owned();
        let working_copy_factory = command.get_working_copy_factory()?;
        return Ok(RepoContext {
            repo,
            repo_path,
            main_workspace_root,
            git_executable,
            working_copy_factory,
        });
    }

    let Some(git_paths) = discover_git_worktree_paths(&git_settings, command.cwd())? else {
        return Err(user_error("Not inside a jj workspace or Git worktree."));
    };
    let main_workspace_root = match git_paths.common_git_dir.parent() {
        Some(path) if path.join(".jj").is_dir() => path,
        _ => {
            return Err(user_error(
                "The Git worktree's main repository is not a colocated jj repo.",
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
        main_workspace_root: main_workspace_root.to_owned(),
        git_executable,
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

    if args.names.is_empty() && !args.all {
        return cmd_git_worktree_adopt_cwd(ui, &ctx).await;
    }

    let linked_worktrees =
        list_git_linked_worktrees(&ctx.git_executable, &ctx.main_workspace_root)?;

    let to_adopt: Vec<&GitLinkedWorktree> = if args.all {
        linked_worktrees
            .iter()
            .filter(|wt| ctx.repo.view().get_wc_commit_id(&wt.name).is_none())
            .collect()
    } else {
        let mut result = Vec::new();
        for name in &args.names {
            let wt = linked_worktrees
                .iter()
                .find(|wt| wt.name.as_str() == name)
                .ok_or_else(|| user_error(format!("Git worktree '{name}' not found.")))?;
            if ctx.repo.view().get_wc_commit_id(&wt.name).is_some() {
                return Err(user_error(format!("Workspace '{name}' already exists.",)));
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
    for wt in &to_adopt {
        let (_workspace, new_repo) = Workspace::init_workspace_with_existing_repo(
            &wt.worktree_root,
            &ctx.repo_path,
            &repo,
            ctx.working_copy_factory,
            wt.name.clone(),
        )
        .await?;
        repo = new_repo;
        writeln!(
            ui.status(),
            r#"Created jj workspace for Git worktree at "{}"."#,
            wt.worktree_root.display()
        )?;
    }
    Ok(())
}

async fn cmd_git_worktree_adopt_cwd(
    ui: &mut Ui,
    ctx: &RepoContext<'_>,
) -> Result<(), CommandError> {
    let cwd = dunce::canonicalize(
        std::env::current_dir()
            .map_err(|err| user_error_with_message("Failed to get current directory", err))?,
    )
    .map_err(|err| user_error_with_message("Failed to resolve current directory", err))?;
    let main_root = dunce::canonicalize(&ctx.main_workspace_root)
        .map_err(|err| user_error_with_message("Failed to resolve main workspace root", err))?;
    if cwd == main_root || cwd.starts_with(&main_root) && !cwd.join(".git").is_file() {
        return Err(user_error(
            "Not inside a linked Git worktree. Run this from within a Git worktree, or pass \
             worktree names to adopt.",
        ));
    }
    let worktree_root = cwd;
    let linked_worktrees =
        list_git_linked_worktrees(&ctx.git_executable, &ctx.main_workspace_root)?;
    let wt = linked_worktrees
        .iter()
        .find(|wt| wt.worktree_root == worktree_root)
        .ok_or_else(|| {
            user_error(
                "Not inside a linked Git worktree. Run this from within a Git worktree, or pass \
                 worktree names to adopt.",
            )
        })?;
    if ctx.repo.view().get_wc_commit_id(&wt.name).is_some() {
        return Err(user_error(format!(
            "Workspace '{}' already exists.",
            wt.name.as_str()
        )));
    }
    let (_workspace, _repo) = Workspace::init_workspace_with_existing_repo(
        &wt.worktree_root,
        &ctx.repo_path,
        &ctx.repo,
        ctx.working_copy_factory,
        wt.name.clone(),
    )
    .await?;
    writeln!(
        ui.status(),
        r#"Created jj workspace for Git worktree at "{}"."#,
        wt.worktree_root.display()
    )?;
    Ok(())
}
