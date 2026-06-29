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

use jj_lib::op_heads_store;
use jj_lib::operation::Operation;

use crate::cli_util::CommandHelper;
use crate::cli_util::command_args_to_transaction_attribute;
use crate::cli_util::merge_operations;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Make an operation part of the operation log
///
/// By default, operations are automatically integrated into the operation log,
/// but `--no-integrate-operation` or internal errors may cause that to not
/// happen. This command can then be used for making such operations part of the
/// operation log.
///
/// Running this command on an operation that is already in the operation log
/// (`jj op log`) has no effect.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationIntegrateArgs {
    /// The operation to integrate
    operation: String,
}

pub async fn cmd_op_integrate(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationIntegrateArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper_no_snapshot(ui).await?;
    let target_op = workspace_command.resolve_single_op(&args.operation)?;
    let repo_loader = workspace_command.repo().loader();
    repo_loader
        .op_heads_store()
        .update_op_heads(target_op.parent_ids(), target_op.id())
        .await?;

    op_heads_store::resolve_op_heads(
        repo_loader.op_heads_store().as_ref(),
        repo_loader.op_store(),
        async |op_heads| -> Result<Operation, CommandError> {
            // TODO: It may be helpful to print each operation we're merging here
            let transaction_description = "reconcile divergent operations";
            let transaction_attributes = [(
                "args".to_string(),
                command_args_to_transaction_attribute(command.string_args()),
            )];
            let merged_operation = merge_operations(
                Some(ui),
                repo_loader,
                op_heads,
                Some(workspace_command.workspace_name()),
                Some(transaction_description),
                transaction_attributes,
            )
            .await?;
            writeln!(
                ui.status(),
                "The specified operation has been integrated with other existing operations."
            )?;
            Ok(merged_operation)
        },
    )
    .await?;

    Ok(())
}
