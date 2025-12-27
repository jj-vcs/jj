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

mod list;

use tracing::instrument;

use self::list::AliasListArgs;
use self::list::cmd_alias_list;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Manage command aliases
///
/// Display and manage command aliases from configuration.
#[derive(clap::Subcommand, Clone, Debug)]
pub(crate) enum AliasCommand {
    #[command(visible_alias("l"))]
    List(AliasListArgs),
}

#[instrument(skip_all)]
pub(crate) fn cmd_alias(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &AliasCommand,
) -> Result<(), CommandError> {
    match subcommand {
        AliasCommand::List(args) => cmd_alias_list(ui, command, args),
    }
}
