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
use std::path::Path;

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// List the paths that are currently present in the working copy
///
/// By default, a newly cloned or initialized repo will have have a pattern
/// matching all files from the repo root. That pattern is rendered as `.` (a
/// single period). Selections that exclude paths or match them by glob have
/// no path-list form; use `jj sparse show` for those.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseListArgs {}

#[instrument(skip_all)]
pub async fn cmd_sparse_list(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let rules = workspace_command.working_copy().sparse_patterns()?;
    let paths = rules.to_path_prefixes().ok_or_else(|| {
        user_error("The sparse patterns aren't a plain list of paths")
            .hinted("Use `jj sparse show` instead.")
    })?;
    for path in paths {
        writeln!(
            ui.stdout(),
            "{}",
            path.to_fs_path_unchecked(Path::new("")).display()
        )?;
    }
    Ok(())
}
