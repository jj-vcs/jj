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

use jj_lib::protos::secure_config::TrustLevel;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Start an editor on a jj config file.
///
/// Creates the file if it doesn't already exist regardless of what the editor
/// does.
#[derive(clap::Args, Clone, Debug)]
pub struct ConfigManagedArgs {
    /// Ignore the managed config for this repo.
    /// Use this if you don't trust the managed config and don't want to go
    /// through the process of reviewing it.
    #[arg(long)]
    ignore: bool,

    /// Notify the user when the managed config has been updated more recently
    /// than the repo config.
    /// Use this if you want the managed config but don't trust that the repo
    /// won't have malicious changes added to it down the line.
    #[arg(long)]
    notify: bool,

    /// Trust the managed config.
    /// Use this only if you trust that the authors of the repo will ensure
    /// that the managed config is safe.
    #[arg(long)]
    trust: bool,
}

#[instrument(skip_all)]
pub fn cmd_config_managed(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConfigManagedArgs,
) -> Result<(), CommandError> {
    let want = match (args.ignore, args.notify, args.trust) {
        (true, false, false) => TrustLevel::Ignored,
        (false, true, false) => TrustLevel::Notify,
        (false, false, true) => TrustLevel::Trusted,
        _ => {
            return Err(user_error(
                "Please specify exactly one of --ignore, --notify, or --trust",
            ));
        }
    };
    // Verify that we're in a workspace.
    command.load_workspace()?;

    command.config_env().update_repo_metadata(ui, |m| {
        m.set_trust_level(want);
    })
}
