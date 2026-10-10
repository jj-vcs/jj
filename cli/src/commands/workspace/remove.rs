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

use std::sync::Arc;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
#[cfg(feature = "git")]
use jj_lib::git::GitSubprocessOptions;
use jj_lib::op_heads_store;
use jj_lib::op_store::OpStoreError;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::merge_operations;
use crate::cli_util::print_snapshot_stats;
use crate::cli_util::short_commit_hash;
use crate::cli_util::start_repo_transaction;
use crate::command_error::CommandError;
use crate::command_error::print_error_sources;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
#[cfg(feature = "git")]
use crate::git_util::unlink_git_worktree;
use crate::ui::Ui;

const USE_WORKSPACE_FORGET_HINT: &str =
    "Use `jj workspace forget` to stop tracking a workspace without deleting files.";

/// Remove a workspace and its working-copy files from disk
///
/// The workspace directory and its contents are removed from disk. The
/// working-copy state is snapshotted into a commit before the workspace is
/// removed. The main workspace cannot be removed.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceRemoveArgs {
    /// Names of the workspaces to remove.
    #[arg(required = true, add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_remove(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceRemoveArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    workspace_command
        .check_working_copy_writable()
        .map_err(|err| err.hinted(USE_WORKSPACE_FORGET_HINT))?;

    let wss = args.workspaces.clone();

    let mut remove_ws = Vec::new();
    for ws in &wss {
        if workspace_command
            .repo()
            .view()
            .get_wc_commit_id(ws)
            .is_none()
        {
            writeln!(
                ui.warning_default(),
                "No such workspace: {}",
                ws.as_symbol()
            )?;
        } else {
            remove_ws.push(ws);
        }
    }
    if remove_ws.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let workspace_store = workspace_command.repo().loader().workspace_store().clone();
    let repo_path = workspace_command.repo_path();

    let mut workspaces_to_remove = Vec::new();
    for ws in &remove_ws {
        let ws_path = workspace_store.get_workspace_path(ws)?.ok_or_else(|| {
            user_error(format!(
                "Cannot remove unreachable workspace '{}'",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        let ws_path = dunce::canonicalize(&ws_path).map_err(|err| {
            user_error_with_message(
                format!("Cannot access workspace '{}' directory", ws.as_symbol()),
                err,
            )
            .hinted(USE_WORKSPACE_FORGET_HINT)
        })?;
        // The repository itself lives under the main workspace (in `.jj/repo`),
        // so removing that directory would destroy the repository. Don't
        // suggest `jj workspace forget` here: forgetting the main workspace
        // isn't a useful thing to do either.
        if repo_path.starts_with(&ws_path) {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it contains the repository",
                ws.as_symbol()
            )));
        }
        // The recorded path may since have been replaced by a workspace of an
        // unrelated repository, which we must not remove.
        let ws_workspace = command.load_workspace_at(&ws_path, workspace_command.settings())?;
        if ws_workspace.repo_path() != repo_path {
            return Err(user_error(format!(
                "Cannot remove workspace '{}' because it belongs to another repository",
                ws.as_symbol()
            ))
            .hinted(USE_WORKSPACE_FORGET_HINT));
        }
        workspaces_to_remove.push((ws_path, ws_workspace));
    }

    let auto_tracking_matcher = workspace_command.auto_tracking_matcher(ui)?;
    let snapshot_options =
        workspace_command.snapshot_options_with_start_tracking_matcher(&auto_tracking_matcher)?;

    let head_repo = workspace_command.repo().clone();
    let mut paths_to_remove = Vec::new();
    for (abs_path, mut ws_workspace) in workspaces_to_remove {
        snapshot_at_workspace_op(
            ui,
            command,
            &head_repo,
            &mut ws_workspace,
            &snapshot_options,
        )
        .await?;
        paths_to_remove.push(abs_path);
    }

    // A snapshot of a stale working copy diverges from the head operation, so
    // the operation heads have to be merged back together before the workspaces
    // can be removed.
    let loader = head_repo.loader();
    let op = op_heads_store::resolve_op_heads(
        loader.op_heads_store().as_ref(),
        loader.op_store(),
        async |op_heads| {
            merge_operations(
                None,
                loader,
                op_heads,
                Some(workspace_command.workspace_name()),
                Some("reconcile working-copy snapshots"),
                command.string_args(),
            )
            .await
        },
    )
    .await?;

    let workspace = command.load_workspace()?;
    let repo = workspace.repo_loader().load_at(&op).await?;
    workspace_command = command.for_workable_repo(ui, workspace, repo)?;

    let mut tx = workspace_command.start_transaction();
    for ws in &remove_ws {
        tx.repo_mut().remove_workspace(ws).await?;
    }
    let names = remove_ws.iter().map(|ws| ws.as_symbol()).join(", ");
    let description = if remove_ws.len() == 1 {
        format!("remove workspace {names}")
    } else {
        format!("remove workspaces {names}")
    };
    tx.finish(ui, description).await?;

    workspace_store.forget(&remove_ws.iter().map(|ws| ws.as_ref()).collect_vec())?;

    #[cfg(feature = "git")]
    {
        let subprocess_options = GitSubprocessOptions::from_settings(workspace_command.settings())?;
        let store = workspace_command.repo().store();
        for path in &paths_to_remove {
            unlink_git_worktree(ui, store, subprocess_options.clone(), path)?;
        }
    }

    for path in &paths_to_remove {
        if let Err(err) = std::fs::remove_dir_all(path) {
            writeln!(
                ui.warning_default(),
                r#"Failed to remove workspace directory "{}"."#,
                path.display()
            )?;
            print_error_sources(ui, Some(&err))?;
        } else {
            writeln!(
                ui.status(),
                r#"Removed workspace directory "{}"."#,
                path.display()
            )?;
        }
    }

    Ok(())
}

/// Snapshots the working-copy files of `workspace` into a new commit, so that
/// they aren't lost when the workspace directory is removed.
///
/// The snapshot operation is written on top of the operation the working copy
/// was last updated to, which may leave divergent operation heads behind.
#[instrument(skip_all)]
async fn snapshot_at_workspace_op(
    ui: &Ui,
    command: &CommandHelper,
    head_repo: &Arc<ReadonlyRepo>,
    workspace: &mut Workspace,
    options: &SnapshotOptions<'_>,
) -> Result<(), CommandError> {
    let workspace_name = workspace.workspace_name().to_owned();
    let workspace_root = workspace.workspace_root().to_owned();
    // Snapshot onto the operation this working copy was last updated to. The
    // working copy may be stale, in which case the commit it was updated to has
    // since been rewritten, and snapshotting onto the head operation would
    // apply the on-disk changes to the wrong commit.
    let wc_op_id = workspace.working_copy().operation_id().clone();
    let repo = match workspace.repo_loader().load_operation(&wc_op_id).await {
        Ok(wc_op) => workspace.repo_loader().load_at(&wc_op).await?,
        Err(err @ OpStoreError::ObjectNotFound { .. }) => {
            writeln!(
                ui.warning_default(),
                "Failed to read the operation workspace {} was updated to; snapshotting onto its \
                 current working-copy commit instead. Error message from read attempt: {err}",
                workspace_name.as_symbol(),
            )?;
            head_repo.clone()
        }
        Err(err) => return Err(err.into()),
    };
    let Some(wc_commit_id) = repo.view().get_wc_commit_id(&workspace_name) else {
        return Ok(());
    };
    let wc_commit = repo.store().get_commit_async(wc_commit_id).await?;

    let mut locked_ws = workspace.start_working_copy_mutation().await?;
    let (new_tree, stats) = {
        let progress = crate::progress::snapshot_progress(ui);
        let options = SnapshotOptions {
            progress: progress.as_ref().map(|x| x as _),
            ..options.clone()
        };
        locked_ws.locked_wc().snapshot(&options).await?
    };
    let path_converter = RepoPathUiConverter::Fs {
        cwd: command.cwd().to_owned(),
        base: workspace_root,
    };
    print_snapshot_stats(ui, &stats, &path_converter)?;
    if new_tree.tree_ids_and_labels() == wc_commit.tree().tree_ids_and_labels() {
        return Ok(());
    }

    // The new commit isn't checked out because the workspace directory is about
    // to be removed. It's written as a child of the working-copy commit instead
    // of rewriting it, so that the preserved changes don't disappear if that
    // commit is immutable or has descendants.
    let mut tx = start_repo_transaction(&repo, &workspace_name, command.string_args());
    tx.set_is_snapshot(true);
    let new_commit = tx
        .repo_mut()
        .new_commit(vec![wc_commit.id().clone()], new_tree)
        .set_description("RECOVERY COMMIT FROM `jj workspace remove`\n")
        .write()
        .await?;
    tx.commit("snapshot working copy").await?;
    writeln!(
        ui.status(),
        "Preserved working-copy changes of workspace {} in commit {}.",
        workspace_name.as_symbol(),
        short_commit_hash(new_commit.id()),
    )?;
    Ok(())
}
