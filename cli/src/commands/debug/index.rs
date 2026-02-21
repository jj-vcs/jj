// Copyright 2023 The Jujutsu Authors
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

use std::fmt::Debug;
use std::io::Write as _;

use jj_lib::default_index::DefaultReadonlyIndex;
use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Show commit index stats
#[derive(clap::Args, Clone, Debug)]
pub struct DebugIndexArgs {}

pub fn cmd_debug_index(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &DebugIndexArgs,
) -> Result<(), CommandError> {
    // Resolve the operation without loading the repo, so this command won't
    // update the index.
    let workspace = command.load_workspace()?;
    let repo_loader = workspace.repo_loader();
    let op = command.resolve_operation(ui, repo_loader)?;
    let index_store = repo_loader.index_store();
    let index = index_store
        .get_index_at_op(&op, repo_loader.store())
        .block_on()
        .map_err(internal_error)?;
    if let Some(default_index) = index.downcast_ref::<DefaultReadonlyIndex>() {
        let stats = default_index.stats();
        writeln!(ui.stdout(), "=== Commits ===")?;
        writeln!(ui.stdout(), "Number of commits: {}", stats.num_commits)?;
        writeln!(ui.stdout(), "Number of merges: {}", stats.num_merges)?;
        writeln!(
            ui.stdout(),
            "Max generation number: {}",
            stats.max_generation_number
        )?;
        writeln!(ui.stdout(), "Number of heads: {}", stats.num_heads)?;
        writeln!(ui.stdout(), "Number of changes: {}", stats.num_changes)?;
        writeln!(ui.stdout(), "Stats per level:")?;
        for (i, level) in stats.commit_levels.iter().enumerate() {
            writeln!(ui.stdout(), "  Level {i}:")?;
            writeln!(ui.stdout(), "    Number of commits: {}", level.num_commits)?;
            writeln!(ui.stdout(), "    Name: {}", level.name)?;
        }

        writeln!(ui.stdout(), "=== Changed paths ===")?;
        if let Some(range) = &stats.changed_path_commits_range {
            writeln!(ui.stdout(), "Indexed commits: {range:?}")?;
        } else {
            writeln!(ui.stdout(), "Indexed commits: none")?;
        }
        writeln!(ui.stdout(), "Stats per level:")?;
        for (i, level) in stats.changed_path_levels.iter().enumerate() {
            writeln!(ui.stdout(), "  Level {i}:")?;
            writeln!(ui.stdout(), "    Number of commits: {}", level.num_commits)?;
            writeln!(
                ui.stdout(),
                "    Number of changed paths: {}",
                level.num_changed_paths
            )?;
            writeln!(ui.stdout(), "    Number of paths: {}", level.num_paths)?;
            writeln!(ui.stdout(), "    Name: {}", level.name)?;
        }
    } else {
        return Err(user_error(format!(
            "Cannot get stats for indexes of type '{}'",
            index_store.name()
        )));
    }
    Ok(())
}
