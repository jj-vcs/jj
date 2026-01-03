// Copyright 2020-2023 The Jujutsu Authors
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

use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Make an operation part of the operation log
///
/// Sometimes an operation does not make it into the operation log for some
/// reason. This command can then be used for making that operation part of the
/// operation log.
///
/// Running this command on an operation that is already in the operation log
/// (`jj op log`) has no effect. Since operations should currently always be
/// integrated into the operation log (until e.g. [#2562] gets implemented),
/// this command should always be a no-op. It would indicate a bug if this
/// command is not a no-op. Only use it if you are told to by an error message.
///
/// [#2562]:
///     https://github.com/jj-vcs/jj/issues/2562
#[derive(clap::Args, Clone, Debug)]
pub struct OperationIntegrateArgs {
    /// The operation to integrate
    operation: String,
}

pub fn cmd_op_integrate(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationIntegrateArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper_no_snapshot(ui)?;
    let target_op = workspace_command.resolve_single_op(&args.operation)?;
    let op_heads_store = workspace_command.repo().op_heads_store();
    op_heads_store
        .update_op_heads(target_op.parent_ids(), target_op.id())
        .block_on()?;

    Ok(())
}
