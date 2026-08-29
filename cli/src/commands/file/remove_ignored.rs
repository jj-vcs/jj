// Copyright 2020 The Jujutsu Authors
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

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Remove files ignored by .gitignore in the working copy
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileRemoveIgnoredArgs {}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_remove_ignored(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &FileRemoveIgnoredArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats, _) = command.workspace_helper_with_stats(ui).await?;
    let workspace_root = workspace_command.workspace_root();
    let mut removed_count = 0;
    for path in &snapshot_stats.ignored_paths {
        let disk_path = path
            .to_fs_path(workspace_root)
            .map_err(|err| user_error(format!("Invalid path: {err}")))?;
        let Ok(metadata) = disk_path.symlink_metadata() else {
            // The file was already removed (possibly concurrently). Treat it as
            // already deleted.
            continue;
        };
        if metadata.is_dir() {
            std::fs::remove_dir_all(&disk_path).map_err(|err| {
                user_error(format!(
                    "Failed to remove directory {}: {err}",
                    disk_path.display()
                ))
            })?;
        } else {
            std::fs::remove_file(&disk_path).map_err(|err| {
                user_error(format!(
                    "Failed to remove file {}: {err}",
                    disk_path.display()
                ))
            })?;
        }
        removed_count += 1;
    }
    writeln!(
        ui.status(),
        "Removed {removed_count} ignored file(s) from the working copy."
    )?;
    Ok(())
}
