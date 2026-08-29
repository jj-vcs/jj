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

/// List files ignored by .gitignore in the working copy
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileListIgnoredArgs {}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_list_ignored(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &FileListIgnoredArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats, _) = command.workspace_helper_with_stats(ui).await?;
    if snapshot_stats.ignored_paths.is_empty() {
        return Ok(());
    }
    let wc_commit_id = workspace_command
        .get_wc_commit_id()
        .ok_or_else(|| user_error("Nothing checked out in this workspace"))?;
    let wc_commit = workspace_command.repo().store().get_commit_async(wc_commit_id).await?;
    let tree = wc_commit.tree();
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    visit_collapsed_untracked_files(&snapshot_stats.ignored_paths, tree, |path, is_dir| {
        let mut ui_path = workspace_command.format_file_path(path);
        if is_dir {
            ui_path.push(std::path::MAIN_SEPARATOR);
        }
        writeln!(formatter, "{ui_path}")?;
        Ok(())
    })
    .await?;
    Ok(())
}
