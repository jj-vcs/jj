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

use jj_lib::protos::secure_config::TrustLevel;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::config::MANAGED_CONFIG_PATH;
use crate::config::review_managed_config_approval;
use crate::ui::Ui;

/// Sets the trust level of jj's managed in-repo configuration.
///
/// When no arguments are provided, will take you through the
/// review process for the current managed config.
///
/// Managed configuration is like jj's repo configuration, but is stored in
/// the repository itself, in `.config/jj/config.toml`. It is used for
/// configuration specific to a repository but not a user, such
/// as formatter configuration.
///
/// Because it is stored in-repo, a malicious actor could modify it to
/// execute arbitrary code. For this reason, managed configuration is untrusted
/// by default.
#[derive(clap::Args, Clone, Debug)]
#[group(multiple = false)]
pub struct ConfigManagedArgs {
    /// Ignore the managed config for this repo.
    ///
    /// Use this if you don't trust the managed config and don't want to go
    /// through the process of reviewing it.
    #[arg(long)]
    ignore: bool,

    /// The user will explicitly either approve or reject each change.
    #[arg(long)]
    review: bool,

    /// Trust the managed config.
    ///
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
    let want = match (args.ignore, args.review, args.trust) {
        (true, false, false) => Some(TrustLevel::Ignored),
        (false, true, false) => Some(TrustLevel::Review),
        (false, false, true) => Some(TrustLevel::Trusted),
        (false, false, false) => None,
        _ => unreachable!(),
    };

    let workspace = command.load_workspace()?;
    let config_file = workspace.workspace_root().join(MANAGED_CONFIG_PATH);

    command.config_env().update_repo_metadata(ui, |m| {
        if let Some(want) = want {
            m.set_trust_level(want);
            Ok(())
        } else if !Ui::can_prompt() {
            Err(user_error(
                "`jj config managed` must be run in an interactive terminal",
            ))
        } else if m.trust_level() != TrustLevel::Review {
            Err(
                user_error("Cannot manually approve or reject when not in review mode")
                    .hinted("Run `jj config managed --review` to set it to review"),
            )
        } else {
            match std::fs::read_to_string(&config_file) {
                Ok(content) => {
                    review_managed_config_approval(ui, command.settings(), m, &content)?;
                    Ok(())
                }
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(internal_error(
                    format!("Failed to read managed config: {err}"),
                )),
                Err(_) => {
                    // File not found, nothing to approve.
                    Ok(())
                }
            }
        }
    })?;
    Ok(())
}
