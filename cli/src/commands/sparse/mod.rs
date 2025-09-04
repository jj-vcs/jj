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

mod edit;
mod list;
mod reset;
mod set;

use clap::Subcommand;
use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use self::edit::SparseEditArgs;
use self::edit::cmd_sparse_edit;
use self::list::SparseListArgs;
use self::list::cmd_sparse_list;
use self::reset::SparseResetArgs;
use self::reset::cmd_sparse_reset;
use self::set::SparseSetArgs;
use self::set::cmd_sparse_set;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::print_checkout_stats;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::ui::Ui;

/// Manage which paths from the working-copy commit are present in the working
/// copy
#[derive(Subcommand, Clone, Debug)]
pub(crate) enum SparseCommand {
    Edit(SparseEditArgs),
    List(SparseListArgs),
    Reset(SparseResetArgs),
    Set(SparseSetArgs),
}

#[instrument(skip_all)]
pub(crate) fn cmd_sparse(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &SparseCommand,
) -> Result<(), CommandError> {
    match subcommand {
        SparseCommand::Edit(args) => cmd_sparse_edit(ui, command, args),
        SparseCommand::List(args) => cmd_sparse_list(ui, command, args),
        SparseCommand::Reset(args) => cmd_sparse_reset(ui, command, args),
        SparseCommand::Set(args) => cmd_sparse_set(ui, command, args),
    }
}

fn update_sparse_patterns_with(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    f: impl FnOnce(&mut Ui, &[RepoPathBuf]) -> Result<Vec<RepoPathBuf>, CommandError>,
) -> Result<(), CommandError> {
    let (mut locked_ws, wc_commit) = workspace_command.start_working_copy_mutation()?;
    let new_patterns = f(ui, locked_ws.locked_wc().sparse_patterns()?)?;
    let stats = locked_ws
        .locked_wc()
        .set_sparse_patterns(new_patterns)
        .map_err(|err| internal_error_with_message("Failed to update working copy paths", err))?;
    let operation_id = locked_ws.locked_wc().old_operation_id().clone();
    locked_ws.finish(operation_id)?;
    print_checkout_stats(ui, &stats, &wc_commit)?;
    Ok(())
}
