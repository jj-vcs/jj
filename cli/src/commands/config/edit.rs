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

use jj_lib::config::ConfigLayer;
use tracing::instrument;

use super::ConfigLevelArgs;
use crate::cli_util::CommandHelper;
use crate::command_error::print_error_sources;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Start an editor on a jj config file.
///
/// Creates the file if it doesn't already exist regardless of what the editor
/// does.
#[derive(clap::Args, Clone, Debug)]
pub struct ConfigEditArgs {
    #[command(flatten)]
    pub level: ConfigLevelArgs,
}

#[instrument(skip_all)]
pub fn cmd_config_edit(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConfigEditArgs,
) -> Result<(), CommandError> {
    let editor = command.text_editor()?;

    // Determine which config file to edit (user, repo, or workspace)
    let file = if args.level.workspace {
        // Workspace-level: set up workspace path and reload
        let mut temp_env = command.config_env().clone();
        let cwd = std::env::current_dir()
            .map_err(|e| user_error(format!("Unable to get cwd: {}", e)))?;
        let ws_dir = cwd.join(".jj");
        temp_env.reset_workspace_path(&ws_dir);
        let mut raw = command.raw_config().clone();
        temp_env
            .reload_workspace_config(&mut raw)
            .map_err(|e| user_error(format!("Failed to load workspace config: {}", e)))?;
        let mut files = temp_env
            .workspace_config_files(&raw)
            .map_err(|e| user_error(format!("No workspace config path: {}", e)))?;
        if files.is_empty() {
            return Err(user_error("No workspace config path found"));
        }
        files.remove(0)
    } else {
        // User or repo-level
        args.level.edit_config_file(ui, command)?
    };

    // Create the file if it doesn't exist yet
    if !file.path().exists() {
        file.save()?;
    }

    writeln!(ui.status(), "Editing file: {}", file.path().display())?;
    loop {
        editor.edit_file(file.path())?;

        // Validate the edited config
        if let Err(e) = ConfigLayer::load_from_file(file.layer().source, file.path().to_path_buf()) {
            writeln!(ui.warning_default(), "An error has been found inside the config:")?;
            print_error_sources(ui, Some(&e))?;
            let continue_editing = ui.prompt_yes_no(
                "Do you want to keep editing the file? If not, previous config will be restored.",
                Some(true),
            )?;
            if !continue_editing {
                // Restore previous content
                file.save()?;
                break;
            }
        } else {
            // Config is valid
            break;
        }
    }
    Ok(())
}