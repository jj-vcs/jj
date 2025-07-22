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

use clap_complete::ArgValueCandidates;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::commands::operation::DEFAULT_UNDO_WHAT;
use crate::commands::operation::UndoWhatToRestore;
use crate::commands::operation::undo::OperationUndoArgs;
use crate::commands::operation::undo::cmd_op_undo;
use crate::complete;
use crate::ui::Ui;

/// Create a new operation that undoes an earlier operation
///
/// This undoes an individual operation by applying the inverse of the
/// operation.
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// The operation to undo
    ///
    /// Use `jj op log` to find an operation to undo.
    #[arg(default_value = "@", add = ArgValueCandidates::new(complete::operations))]
    operation: String,

    /// What portions of the local state to restore (can be repeated)
    ///
    /// This option is EXPERIMENTAL.
    #[arg(long, value_enum, default_values_t = DEFAULT_UNDO_WHAT)]
    what: Vec<UndoWhatToRestore>,
}

pub fn cmd_undo(ui: &mut Ui, command: &CommandHelper, args: &UndoArgs) -> Result<(), CommandError> {
    let args = OperationUndoArgs {
        operation: args.operation.clone(),
        what: args.what.clone(),
    };
    cmd_op_undo(ui, command, &args)?;

    Ok(())
}
