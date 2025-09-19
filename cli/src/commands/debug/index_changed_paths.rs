// Copyright 2025 The Jujutsu Authors
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

use jj_lib::default_index::DefaultIndexStore;
use jj_lib::repo::Repo as _;
use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Build changed-path index
#[derive(clap::Args, Clone, Debug)]
pub struct DebugIndexChangedPathsArgs {
    /// Limit number of revisions to index
    #[arg(long, short = 'n', default_value_t = u32::MAX)]
    limit: u32,
}

pub fn cmd_debug_index_changed_paths(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &DebugIndexChangedPathsArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let repo_loader = workspace_command.workspace().repo_loader();
    let index_store = repo_loader.index_store();
    let Some(default_index_store) = index_store.downcast_ref::<DefaultIndexStore>() else {
        return Err(user_error(format!(
            "Unsupported index type '{}'",
            index_store.name()
        )));
    };
    let index = default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), args.limit)
        .block_on()
        .map_err(internal_error)?;
    let stats = index.stats();
    writeln!(
        ui.status(),
        "Finished indexing {:?} commits.",
        stats.changed_path_commits_range.unwrap()
    )?;
    Ok(())
}
