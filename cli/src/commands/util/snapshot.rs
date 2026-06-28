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

use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Snapshot the working copy if needed
///
/// Snapshots the working copy and updates the working-copy commit if the
/// working copy has changed since the last snapshot. Since almost every command
/// snapshots the working copy, there is very little reason to run this command
/// as a human; it is mostly meant for scripts. It prints the resulting
/// working-copy state together with the current operation summary.
///
/// If you want to query the current operation ID directly, run
/// `jj operation log --limit 1`. However, since that command also snapshots the
/// working copy, there would often be no need to run `jj util snapshot` first.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilSnapshotArgs {}

pub async fn cmd_util_snapshot(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &UtilSnapshotArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper_no_snapshot(ui).await?;
    let old_wc_commit_id = workspace_command.get_wc_commit_id().cloned();

    // Trigger the snapshot if needed.
    let did_operation_change = workspace_command.maybe_snapshot(ui).await?;
    let Some(mut formatter) = ui.status_formatter() else {
        return Ok(());
    };
    let status = if did_operation_change {
        "Snapshot complete."
    } else {
        "No snapshot needed."
    };

    writeln!(formatter, "{status}")?;

    if let Some(commit_id) = workspace_command.get_wc_commit_id() {
        let commit = workspace_command
            .repo()
            .store()
            .get_commit_async(commit_id)
            .await?;
        if Some(commit.id()) != old_wc_commit_id.as_ref() {
            write!(formatter, "Working copy  (@) now at: ")?;
            workspace_command
                .commit_summary_template()
                .format(&commit, formatter.as_mut())?;
            writeln!(formatter)?;
            for parent in commit.parents().await? {
                write!(formatter, "Parent commit (@-)      : ")?;
                workspace_command
                    .commit_summary_template()
                    .format(&parent, formatter.as_mut())?;
                writeln!(formatter)?;
            }
        } else {
            write!(formatter, "Working copy change (@): ")?;
            workspace_command
                .short_change_id_template()
                .format(&commit, formatter.as_mut())?;
            writeln!(formatter)?;
        }
    }

    write!(formatter, "Current operation: ")?;
    workspace_command
        .operation_summary_template()
        .format(workspace_command.repo().operation(), formatter.as_mut())?;
    writeln!(formatter)?;

    Ok(())
}
