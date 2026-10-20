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

mod abandon;
mod diff;
mod integrate;
mod log;
mod restore;
mod revert;
mod show;

use std::path::PathBuf;

use abandon::OperationAbandonArgs;
use abandon::cmd_op_abandon;
use clap::Subcommand;
use diff::OperationDiffArgs;
use diff::cmd_op_diff;
use integrate::OperationIntegrateArgs;
use integrate::cmd_op_integrate;
use itertools::Itertools as _;
use jj_lib::op_store::View;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::workspace_store::WorkspaceStore;
use jj_lib::workspace_store::WorkspaceStoreError;
use log::OperationLogArgs;
use log::cmd_op_log;
use restore::OperationRestoreArgs;
use restore::cmd_op_restore;
use revert::OperationRevertArgs;
use revert::cmd_op_revert;
use show::OperationShowArgs;
use show::cmd_op_show;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Commands for working with the operation log
///
/// See the [operation log documentation] for more information.
///
/// [operation log documentation]:
///     https://docs.jj-vcs.dev/latest/operation-log/
#[derive(Subcommand, Clone, Debug)]
pub enum OperationCommand {
    Abandon(OperationAbandonArgs),
    Diff(OperationDiffArgs),
    Integrate(OperationIntegrateArgs),
    Log(OperationLogArgs),
    Restore(OperationRestoreArgs),
    Revert(OperationRevertArgs),
    Show(OperationShowArgs),
}

pub async fn cmd_operation(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &OperationCommand,
) -> Result<(), CommandError> {
    match subcommand {
        OperationCommand::Abandon(args) => cmd_op_abandon(ui, command, args).await,
        OperationCommand::Diff(args) => cmd_op_diff(ui, command, args).await,
        OperationCommand::Integrate(args) => cmd_op_integrate(ui, command, args).await,
        OperationCommand::Log(args) => cmd_op_log(ui, command, args).await,
        OperationCommand::Restore(args) => cmd_op_restore(ui, command, args).await,
        OperationCommand::Revert(args) => cmd_op_revert(ui, command, args).await,
        OperationCommand::Show(args) => cmd_op_show(ui, command, args).await,
    }
}

// pub for `jj undo`
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
pub(crate) enum RevertWhatToRestore {
    /// The jj repo state and local bookmarks
    Repo,
    /// The remote-tracking bookmarks. Do not restore these if you'd like to
    /// push after the undo
    RemoteTracking,
}

// pub for `jj undo`
pub(crate) const DEFAULT_REVERT_WHAT: [RevertWhatToRestore; 2] = [
    RevertWhatToRestore::Repo,
    RevertWhatToRestore::RemoteTracking,
];

/// Restore only the portions of the view specified by the `what` argument
///
/// Workspaces that have been renamed since `view_being_restored` keep their
/// current names. The name is also recorded in the workspace's working copy,
/// which can't be updated from here, so restoring the old name would leave the
/// workspace without a working-copy commit.
pub(crate) fn view_with_desired_portions_restored(
    ui: &Ui,
    view_being_restored: &View,
    current_view: &View,
    what: &[RevertWhatToRestore],
    workspace_store: &dyn WorkspaceStore,
) -> Result<View, CommandError> {
    let repo_source = if what.contains(&RevertWhatToRestore::Repo) {
        view_being_restored
    } else {
        current_view
    };
    let remote_source = if what.contains(&RevertWhatToRestore::RemoteTracking) {
        view_being_restored
    } else {
        current_view
    };
    let mut wc_commit_ids = repo_source.wc_commit_ids.clone();
    for (old_name, new_name) in find_renamed_workspaces(repo_source, current_view, workspace_store)?
    {
        let commit_id = wc_commit_ids.remove(&old_name).unwrap();
        wc_commit_ids.insert(new_name.clone(), commit_id);
        writeln!(
            ui.warning_default(),
            "Workspace {new} keeps its name instead of being renamed back to {old}.",
            new = new_name.as_symbol(),
            old = old_name.as_symbol(),
        )?;
        writeln!(
            ui.hint_default(),
            "Run `jj workspace rename {old}` in that workspace to rename it.",
            old = old_name.as_symbol(),
        )?;
    }
    Ok(View {
        head_ids: repo_source.head_ids.clone(),
        local_bookmarks: repo_source.local_bookmarks.clone(),
        local_tags: repo_source.local_tags.clone(),
        remote_views: remote_source.remote_views.clone(),
        git_refs: current_view.git_refs.clone(),
        git_heads: current_view.git_heads.clone(),
        wc_commit_ids,
    })
}

/// Finds workspaces named differently in `restored_view` and `current_view`,
/// returning `(old_name, new_name)` pairs.
///
/// A workspace only in `restored_view` and one only in `current_view` are the
/// same workspace if the workspace store records the same path for both names.
/// `jj workspace rename` keeps the old name in the store for this reason.
fn find_renamed_workspaces(
    restored_view: &View,
    current_view: &View,
    workspace_store: &dyn WorkspaceStore,
) -> Result<Vec<(WorkspaceNameBuf, WorkspaceNameBuf)>, CommandError> {
    let restored_names = &restored_view.wc_commit_ids;
    let current_names = &current_view.wc_commit_ids;
    let old_names = restored_names
        .keys()
        .filter(|name| !current_names.contains_key(*name))
        .collect_vec();
    let new_names = current_names
        .keys()
        .filter(|name| !restored_names.contains_key(*name))
        .collect_vec();
    // Most operations don't add or remove workspaces, so avoid reading the
    // workspace store unless they did.
    if old_names.is_empty() || new_names.is_empty() {
        return Ok(vec![]);
    }
    fn with_paths<'a>(
        names: Vec<&'a WorkspaceNameBuf>,
        workspace_store: &dyn WorkspaceStore,
    ) -> Result<Vec<(&'a WorkspaceNameBuf, PathBuf)>, WorkspaceStoreError> {
        let mut named_paths = vec![];
        for name in names {
            if let Some(path) = workspace_store.get_workspace_path(name)? {
                named_paths.push((name, path));
            }
        }
        Ok(named_paths)
    }
    let old_paths = with_paths(old_names, workspace_store)?;
    let new_paths = with_paths(new_names, workspace_store)?;
    // Only pair names if their path doesn't also belong to another name.
    let renames = new_paths
        .iter()
        .filter_map(|(new_name, path)| {
            let (old_name, _) = old_paths
                .iter()
                .filter(|(_, old_path)| old_path == path)
                .exactly_one()
                .ok()?;
            new_paths
                .iter()
                .filter(|(_, new_path)| new_path == path)
                .exactly_one()
                .ok()?;
            Some(((*old_name).clone(), (*new_name).clone()))
        })
        .collect();
    Ok(renames)
}
