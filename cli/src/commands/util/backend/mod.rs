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

mod name;

use clap::Subcommand;
use tracing::instrument;

use self::name::UtilBackendNameArgs;
use self::name::cmd_util_backend_name;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Commands relating to the backend used in the current repo
#[derive(Subcommand, Clone, Debug)]
pub(crate) enum UtilBackendCommand {
    Name(UtilBackendNameArgs),
}

#[instrument(skip_all)]
pub(crate) async fn cmd_util_backend(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &UtilBackendCommand,
) -> Result<(), CommandError> {
    match subcommand {
        UtilBackendCommand::Name(args) => cmd_util_backend_name(ui, command, args).await,
    }
}
