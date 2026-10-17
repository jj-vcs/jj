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

use std::io::Write as _;

use tracing::instrument;

use super::format_sparse_rules;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Show the rules that decide which paths are present in the working copy
///
/// The rules are printed in the format accepted by `jj sparse edit` (see
/// `jj sparse --help`). A newly cloned or initialized repo has the single
/// rule `+ all()`.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseShowArgs {}

#[instrument(skip_all)]
pub async fn cmd_sparse_show(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseShowArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let rules = workspace_command.working_copy().sparse_patterns()?;
    write!(ui.stdout(), "{}", format_sparse_rules(rules))?;
    Ok(())
}
