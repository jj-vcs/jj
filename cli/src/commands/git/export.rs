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

use jj_lib::git;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::git_util::print_git_export_stats;
use crate::ui::Ui;

/// Update the underlying Git repo with changes made in the repo
///
/// By default, this command does nothing in colocated workspaces because the
/// export happens automatically. Use `--ignore-working-copy` to forcibly export
/// changes.
#[derive(clap::Args, Clone, Debug)]
pub struct GitExportArgs {}

pub async fn cmd_git_export(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitExportArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    if command.is_working_copy_writable() && workspace_command.working_copy_shared_with_git() {
        // Git refs are imported during the snapshot, so there are no ref
        // changes to export.
        writeln!(ui.status(), "No export needed in colocated workspaces.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    let stats = git::export_refs(tx.repo_mut())?;
    tx.finish(ui, "export git refs").await?;
    print_git_export_stats(ui, &stats)?;
    Ok(())
}
