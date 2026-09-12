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
use crate::ui::Ui;

/// List files ignored by .gitignore in the working copy
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileListIgnoredArgs {
    /// Only list ignored paths matching these filesets
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_list_ignored(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileListIgnoredArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats, _) = command.workspace_helper_with_stats(ui).await?;
    if snapshot_stats.ignored_paths.is_empty() {
        return Ok(());
    }
    let matcher = workspace_command
        .parse_file_patterns(ui, &args.paths)?
        .to_matcher();
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    for ignored in &snapshot_stats.ignored_paths {
        let path_matches = if ignored.is_dir {
            !matcher.visit(&ignored.path).is_nothing()
        } else {
            matcher.matches(&ignored.path)
        };
        if !path_matches {
            continue;
        }
        let mut ui_path = workspace_command.format_file_path(&ignored.path);
        if ignored.is_dir {
            // Always use '/' regardless of platform, matching jj's other
            // path output (e.g. `jj status`), rather than
            // `std::path::MAIN_SEPARATOR` which would be '\' on Windows.
            ui_path.push('/');
        }
        writeln!(formatter, "{ui_path}")?;
    }
    Ok(())
}
