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

use std::io::Write as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::gc_util::reindex_at_operation;
use crate::ui::Ui;

/// Rebuild commit index
#[derive(clap::Args, Clone, Debug)]
pub struct DebugReindexArgs {}

pub async fn cmd_debug_reindex(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &DebugReindexArgs,
) -> Result<(), CommandError> {
    // Resolve the operation without loading the repo. The index might have to
    // be rebuilt while loading the repo.
    let workspace = command.load_workspace()?;
    let repo_loader = workspace.repo_loader();

    let op = command.resolve_operation(ui, repo_loader, workspace.workspace_name())?;
    let default_index = reindex_at_operation(repo_loader, &op).await?;

    writeln!(
        ui.status(),
        "Finished indexing {} commits.",
        default_index.num_commits()
    )?;

    Ok(())
}
