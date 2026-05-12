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

use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Print the name of the backend used in the current repo
#[derive(clap::Args, Clone, Debug)]
pub struct UtilBackendNameArgs {}

pub async fn cmd_util_backend_name(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &UtilBackendNameArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper_no_snapshot(ui).await?;
    writeln!(
        ui.stdout(),
        "{}",
        workspace_command.repo().store().backend().name()
    )?;
    Ok(())
}
