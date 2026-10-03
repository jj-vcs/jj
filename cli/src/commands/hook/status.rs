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

use jj_lib::hooks;
use jj_lib::hooks::HookKind;
use jj_lib::hooks::HookTrustState;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Show which hooks are configured and which one would run
///
/// For each hook name, shows whether a global and/or a repository-scoped
/// file exists, whether the repository-scoped one is currently trusted,
/// and which one (if any) jj would actually run.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct HookStatusArgs {}

#[instrument(skip_all)]
pub(crate) async fn cmd_hook_status(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &HookStatusArgs,
) -> Result<(), CommandError> {
    let workspace_root = command.workspace_loader()?.workspace_root();
    let root_config_dir = command
        .config_env()
        .root_config_dir()
        .ok_or_else(|| user_error("No jj configuration directory found"))?;
    let trust_state = HookTrustState::from_settings(command.settings())?;

    let Some(mut formatter) = ui.status_formatter() else {
        return Ok(());
    };
    for kind in HookKind::ALL {
        let global_path = hooks::global_hook_path(root_config_dir, kind);
        let repo_path = hooks::repo_hook_path(workspace_root, kind);
        let global_exists = global_path.is_file();
        let repo_exists = repo_path.is_file();
        let resolved = hooks::resolve_hook(root_config_dir, workspace_root, kind, |path| {
            trust_state.check(path)
        });

        writeln!(formatter, "{}:", kind.file_name())?;
        writeln!(
            formatter,
            "  global: {}",
            if global_exists { "present" } else { "none" }
        )?;
        write!(formatter, "  local: ")?;
        if !repo_exists {
            writeln!(formatter, "none")?;
        } else {
            match trust_state.check(&repo_path) {
                hooks::RepoHookTrust::Trusted => writeln!(formatter, "present, trusted")?,
                hooks::RepoHookTrust::NotEnabled => {
                    writeln!(formatter, "present, not enabled (`jj hook enable`)")?;
                }
                hooks::RepoHookTrust::Stale => {
                    writeln!(
                        formatter,
                        "present, untrusted (`jj hook enable` to re-approve)"
                    )?;
                }
            }
        }
        match &resolved.path {
            Some(path) if *path == repo_path => writeln!(formatter, "  would run: local")?,
            Some(_) => writeln!(formatter, "  would run: global")?,
            None => writeln!(formatter, "  would run: (nothing)")?,
        }
    }
    Ok(())
}
