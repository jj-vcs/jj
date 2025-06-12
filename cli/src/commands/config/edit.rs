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
    let file = args.level.edit_config_file(ui, command)?;
    if !file.path().exists() {
        file.save()?;
    }

    // Editing again and again until either of these conditions is met
    // 1. The config is OK
    // 2. The user restores previous one
    writeln!(ui.status(), "Editing file: {}", file.path())?;
    loop {
        editor.edit_file(file.path())?;

        // Trying to load back config. If error, prompt to continue editing
        if let Err(e) = ConfigLayer::load_from_file(file.layer().source, file.path()) {
            writeln!(
                ui.warning_default(),
                "An error has been found inside the config:"
            )?;
            print_error_sources(ui, Some(&e))?;
            let continue_editing = ui.prompt_yes_no(
                "Do you want to keep editing the file? If not, previous config will be restored.",
                Some(true),
            )?;
            if !continue_editing {
                // Saving back previous config
                file.save()?;
                break;
            }
        } else {
            // config is OK
            break;
        }
    }
    Ok(())
}
