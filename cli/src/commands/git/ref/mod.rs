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

mod fetch;

use clap::Subcommand;

use self::fetch::cmd_git_ref_fetch;
use self::fetch::GitRefFetchArgs;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Manage Git refs
#[derive(Subcommand, Clone, Debug)]
pub enum GitRefCommand {
    Fetch(GitRefFetchArgs),
}

pub fn cmd_git_ref(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitRefCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GitRefCommand::Fetch(args) => cmd_git_ref_fetch(ui, command, args),
    }
}
