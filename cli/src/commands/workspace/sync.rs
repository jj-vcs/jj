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

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Synchronize jj workspaces with Git worktrees
///
/// In a colocated Git repository, this command reconciles jj workspace
/// state with Git worktree state:
///
/// * Adopts Git worktrees created externally (e.g. via `git worktree add`) by
///   creating corresponding jj workspaces.
///
/// * Forgets jj workspaces whose Git worktrees have been removed.
///
/// * Repairs workspace paths for Git worktrees that have been moved.
///
/// This is equivalent to the automatic synchronization performed when
/// `git.auto-sync-worktrees` is enabled (the default), but can be run
/// manually when that setting is disabled.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceSyncArgs {}

#[instrument(skip_all)]
pub async fn cmd_workspace_sync(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &WorkspaceSyncArgs,
) -> Result<(), CommandError> {
    let workspace = command.load_workspace_or_auto_init_git_worktree(ui).await?;
    let env = command.workspace_environment(ui, &workspace)?;
    let mut workspace_command = command.load_from_workspace(ui, workspace, env).await?;
    workspace_command.ensure_current_workspace_git_worktree(ui)?;
    workspace_command.forget_removed_git_worktrees(ui).await?;
    Ok(())
}
