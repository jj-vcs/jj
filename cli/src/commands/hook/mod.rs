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

mod disable;
mod enable;
mod status;

use tracing::instrument;

use self::disable::HookDisableArgs;
use self::disable::cmd_hook_disable;
use self::enable::HookEnableArgs;
use self::enable::cmd_hook_enable;
use self::status::HookStatusArgs;
use self::status::cmd_hook_status;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Manage jj hooks
///
/// See the "Hooks" section of [`jj help -k config`] for the hook
/// mechanism, hook kinds, and the trust model for repository-scoped hooks.
///
/// [`jj help -k config`]:
///     https://docs.jj-vcs.dev/latest/config/#hooks
#[derive(clap::Subcommand, Clone, Debug)]
pub(crate) enum HookCommand {
    Disable(HookDisableArgs),
    Enable(HookEnableArgs),
    Status(HookStatusArgs),
}

#[instrument(skip_all)]
pub(crate) async fn cmd_hook(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &HookCommand,
) -> Result<(), CommandError> {
    match subcommand {
        HookCommand::Disable(args) => cmd_hook_disable(ui, command, args).await,
        HookCommand::Enable(args) => cmd_hook_enable(ui, command, args).await,
        HookCommand::Status(args) => cmd_hook_status(ui, command, args).await,
    }
}
