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

use super::format_sparse_rules;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// List the patterns that are currently present in the working copy
///
/// The patterns are printed as a list of rules (see `jj sparse --help`). By
/// default, a newly cloned or initialized repo has a single rule, `+ all()`.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseListArgs {}

#[instrument(skip_all)]
pub async fn cmd_sparse_list(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let sparse_patterns = workspace_command.working_copy().sparse_patterns()?;
    write!(ui.stdout(), "{}", format_sparse_rules(sparse_patterns))?;
    Ok(())
}
