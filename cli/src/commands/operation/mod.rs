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
mod log;
mod restore;
pub mod revert;
mod show;

use abandon::OperationAbandonArgs;
use abandon::cmd_op_abandon;
use clap::Subcommand;
use diff::OperationDiffArgs;
use diff::cmd_op_diff;
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
use crate::commands::renamed_cmd;
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
    Log(OperationLogArgs),
    Restore(OperationRestoreArgs),
    Revert(OperationRevertArgs),
    Show(OperationShowArgs),
    // TODO: Delete in jj 0.39.0+
    #[command(hide = true)]
    Undo(OperationRevertArgs),
}

pub fn cmd_operation(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &OperationCommand,
) -> Result<(), CommandError> {
    match subcommand {
        OperationCommand::Abandon(args) => cmd_op_abandon(ui, command, args),
        OperationCommand::Diff(args) => cmd_op_diff(ui, command, args),
        OperationCommand::Log(args) => cmd_op_log(ui, command, args),
        OperationCommand::Restore(args) => cmd_op_restore(ui, command, args),
        OperationCommand::Revert(args) => cmd_op_revert(ui, command, args),
        OperationCommand::Show(args) => cmd_op_show(ui, command, args),
        OperationCommand::Undo(args) => {
            let cmd = renamed_cmd("op undo", "op revert", cmd_op_revert);
            cmd(ui, command, args)
        }
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
pub(crate) fn view_with_desired_portions_restored(
    view_being_restored: &jj_lib::op_store::View,
    current_view: &jj_lib::op_store::View,
    what: &[RevertWhatToRestore],
) -> jj_lib::op_store::View {
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
    jj_lib::op_store::View {
        head_ids: repo_source.head_ids.clone(),
        local_bookmarks: repo_source.local_bookmarks.clone(),
        local_tags: repo_source.local_tags.clone(),
        remote_views: remote_source.remote_views.clone(),
        git_refs: current_view.git_refs.clone(),
        git_head: current_view.git_head.clone(),
        wc_commit_ids: repo_source.wc_commit_ids.clone(),
    }
}
