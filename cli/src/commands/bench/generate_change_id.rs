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

use super::CriterionArgs;
use super::run_bench;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Generate a change-ID
#[derive(clap::Args, Clone, Debug)]
pub struct BenchGenerateChangeIdArgs {
    #[command(flatten)]
    criterion: CriterionArgs,
}

pub async fn cmd_bench_generate_change_id(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BenchGenerateChangeIdArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let id_prefix_context = workspace_command.id_prefix_context();
    let id_prefix_index = id_prefix_context
        .populate(workspace_command.repo().as_ref())
        .unwrap();
    let rng = workspace_command.settings().get_rng();
    let length = workspace_command.repo().store().change_id_length();
    let routine = || id_prefix_index.generate_new_change_id(&rng, length);
    run_bench(ui, "generate-change-id", &args.criterion, routine)?;
    Ok(())
}
