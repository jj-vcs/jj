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

use jj_lib::repo::Repo;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::visit_collapsed_untracked_files;
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
    if snapshot_stats.ignored_paths.is_empty() {
        writeln!(
            ui.status(),
            "Removed 0 ignored file(s) from the working copy."
        )?;
        return Ok(());
    }
    let workspace_root = workspace_command.workspace_root().to_owned();
    let wc_commit_id = workspace_command
        .get_wc_commit_id()
        .ok_or_else(|| user_error("Nothing checked out in this workspace"))?;
    let wc_commit = workspace_command
        .repo()
        .store()
        .get_commit_async(wc_commit_id)
        .await?;
    let tree = wc_commit.tree();
    let mut removed_count = 0;
    // Reuse the same collapsing traversal as `file list-ignored` so that a
    // directory containing no tracked files is removed as a single unit
    // (via `remove_dir_all`), instead of only deleting the individual
    // ignored files it happens to contain and leaving an empty directory
    // behind.
    visit_collapsed_untracked_files(&snapshot_stats.ignored_paths, tree, |path, is_dir| {
        let disk_path = path
            .to_fs_path(&workspace_root)
            .map_err(|err| user_error(format!("Invalid path: {err}")))?;
        let Ok(metadata) = disk_path.symlink_metadata() else {
            // The file or directory was already removed (possibly
            // concurrently). Treat it as already deleted.
            return Ok(());
        };
        if is_dir || metadata.is_dir() {
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
        Ok(())
    })
    .await?;
    writeln!(
        ui.status(),
        "Removed {removed_count} ignored file(s) from the working copy."
    )?;
    Ok(())
}
