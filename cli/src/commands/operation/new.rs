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

use crate::cli_util::CommandHelper;
use crate::cli_util::start_repo_transaction;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Create a new operation with a custom description
///
/// Unconditionally creates a no-op operation with the provided description.
///
/// Unless --ignore-working-copy is specified, this command will still first
/// snapshot any changes.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationNewArgs {
    /// The description of the operation to create
    description: String,
}

pub fn cmd_op_new(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationNewArgs,
) -> Result<(), CommandError> {
    // Will snapshot any changes if applicable
    let workspace_helper = command.workspace_helper(ui)?;

    let workspace = workspace_helper.workspace();
    let repo_loader = workspace.repo_loader();

    let op = command.resolve_operation(ui, repo_loader)?;
    let repo = repo_loader.load_at(&op)?;
    let transaction = start_repo_transaction(&repo, command.string_args());
    transaction.commit(&args.description)?;
    Ok(())
}
