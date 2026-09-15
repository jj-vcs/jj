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

use jj_lib::config::ConfigNamePathBuf;
use jj_lib::hooks;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Trust the repository-scoped hooks under `.jj-hooks/` in this checkout
///
/// Lists every file currently under `.jj-hooks/` and records the enabled
/// flag and each file's content hash in this repository's local, secure
/// config. From then on, jj runs these hooks automatically for this
/// checkout, for every hook name it lists.
///
/// This approval is per repository and per clone: cloning a repository that
/// already has `.jj-hooks/` populated does not enable execution by itself.
/// It is also tied to content, not to a one-time decision: if a hook
/// file's content changes afterward (a local edit, or content pulled in by
/// `jj git fetch`, a rebase, or a checkout of a different bookmark), jj
/// treats it as untrusted again until `jj hook enable` is run once more.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct HookEnableArgs {}

#[instrument(skip_all)]
pub(crate) async fn cmd_hook_enable(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &HookEnableArgs,
) -> Result<(), CommandError> {
    let workspace_root = command.workspace_loader()?.workspace_root();
    let hashes = hooks::current_repo_hook_hashes(workspace_root)
        .map_err(|err| user_error_with_message("Failed to read .jj-hooks/", err))?;
    if hashes.is_empty() {
        return Err(user_error("No hooks found under .jj-hooks/"));
    }

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "jj will now execute the following hooks automatically in this checkout:"
        )?;
        for name in hashes.keys() {
            writeln!(formatter, "  {name}")?;
        }
    }

    let config = command.raw_config();
    let mut file = command
        .config_env()
        .repo_config_files(ui, config)?
        .into_iter()
        .next()
        .ok_or_else(|| user_error("No repo config path found"))?;

    file.set_value(ConfigNamePathBuf::from_iter(["hooks", "enabled"]), true)
        .map_err(|err| user_error_with_message("Failed to set hooks.enabled", err))?;
    for (name, hash) in &hashes {
        file.set_value(
            ConfigNamePathBuf::from_iter(["hooks", "trusted-hashes", name.as_str()]),
            hash.as_str(),
        )
        .map_err(|err| user_error_with_message(format!("Failed to trust hook {name}"), err))?;
    }
    file.save()?;
    Ok(())
}
