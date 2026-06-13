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

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Snapshot the working copy if needed
///
/// Snapshots the working copy and updates the working-copy commit if the
/// working copy has changed since the last snapshot. Since almost every command
/// snapshots the working copy, there is very little reason to run this command
/// as a human; it is mostly meant for scripts where you want to ensure a
/// snapshot has been created and you don't care about the result other than
/// that a snapshot was created (if necessary).
///
/// If you want to see the ID of the current operation after this command, run
/// `jj operation log --limit 1`. However, since that command also snapshots the
/// working copy, there would be no need to run `jj util snapshot` first.
///
/// If no snapshotting happened, there will be no new commit or operation
/// created, so it is not correct to assume that this command will create a new
/// commit and operation. Also note that other commands may happen almost
/// concurrently with this command, so even `echo foo >> file; jj util snapshot`
/// is not guaranteed to produce a new commit and operation.
///
/// Whether this command resulted in snapshotting or not, it may result in new
/// operations for reconciling concurrent operations and/or importing Git refs
/// (if in a colocated workspace), so don't rely on any new operations being
/// related to snapshotting.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilSnapshotArgs {}

pub async fn cmd_util_snapshot(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &UtilSnapshotArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper_no_snapshot(ui).await?;

    // Trigger the snapshot if needed.
    let was_snapshot_taken = workspace_command.maybe_snapshot(ui).await?;
    if was_snapshot_taken {
        writeln!(ui.status(), "Snapshot complete.")?;
    } else {
        writeln!(ui.status(), "No snapshot needed.")?;
    }

    Ok(())
}
