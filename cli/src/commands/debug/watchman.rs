// Copyright 2023 The Jujutsu Authors
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

#[cfg(feature = "watchman")]
use std::io::Write as _;

use clap::Subcommand;
#[cfg(feature = "watchman")]
use jj_lib::fsmonitor::FsmonitorSettings;
#[cfg(feature = "watchman")]
use jj_lib::fsmonitor::WatchmanConfig;
#[cfg(feature = "watchman")]
use jj_lib::local_working_copy::LocalWorkingCopy;
#[cfg(feature = "watchman")]
use jj_lib::working_copy::WorkingCopy;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

#[derive(Subcommand, Clone, Debug)]
pub enum DebugWatchmanCommand {
    /// Check whether `watchman` is enabled and whether it's correctly installed
    Status,
    QueryClock,
    QueryChangedFiles,
    ResetClock,
}

#[cfg(feature = "watchman")]
pub fn cmd_debug_watchman(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &DebugWatchmanCommand,
) -> Result<(), CommandError> {
    use jj_lib::local_working_copy::LockedLocalWorkingCopy;

    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let watchman_config = WatchmanConfig {
        // The value is likely irrelevant here. TODO(ilyagr): confirm
        register_trigger: false,
    };
    match subcommand {
        DebugWatchmanCommand::Status => {
            // TODO(ilyagr): It would be nice to add colors here
            let config = match FsmonitorSettings::from_settings(workspace_command.settings())? {
                FsmonitorSettings::Watchman(config) => {
                    writeln!(ui.stdout(), "Watchman is enabled via `fsmonitor.backend`.")?;
                    writeln!(
                        ui.stdout(),
                        r"Background snapshotting is {}. Use \
                          `fsmonitor.watchman.register-snapshot-trigger` to control it.",
                        if config.register_trigger {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )?;
                    config
                }
                FsmonitorSettings::None => {
                    writeln!(
                        ui.stdout(),
                        r#"Watchman is disabled. Set `fsmonitor.backend="watchman"` to enable."#
                    )?;
                    writeln!(
                        ui.stdout(),
                        "Attempting to contact the `watchman` CLI regardless..."
                    )?;
                    watchman_config
                }
                other_fsmonitor => {
                    return Err(user_error(format!(
                        r"This command does not support the currently enabled filesystem monitor: {other_fsmonitor:?}."
                    )));
                }
            };
            let wc = check_local_disk_wc(workspace_command.working_copy())?;
            wc.query_watchman(&config)?;
            writeln!(
                ui.stdout(),
                "The watchman server seems to be installed and working correctly."
            )?;
            writeln!(
                ui.stdout(),
                "Background snapshotting is currently {}.",
                if wc.is_watchman_trigger_registered(&config)? {
                    "active"
                } else {
                    "inactive"
                }
            )?;
        }
        DebugWatchmanCommand::QueryClock => {
            let wc = check_local_disk_wc(workspace_command.working_copy())?;
            let (clock, _changed_files) = wc.query_watchman(&watchman_config)?;
            writeln!(ui.stdout(), "Clock: {clock:?}")?;
        }
        DebugWatchmanCommand::QueryChangedFiles => {
            let wc = check_local_disk_wc(workspace_command.working_copy())?;
            let (_clock, changed_files) = wc.query_watchman(&watchman_config)?;
            writeln!(ui.stdout(), "Changed files: {changed_files:?}")?;
        }
        DebugWatchmanCommand::ResetClock => {
            let (mut locked_ws, _commit) = workspace_command.start_working_copy_mutation()?;
            let Some(locked_local_wc): Option<&mut LockedLocalWorkingCopy> =
                locked_ws.locked_wc().downcast_mut()
            else {
                return Err(user_error(
                    "This command requires a standard local-disk working copy",
                ));
            };
            locked_local_wc.reset_watchman()?;
            locked_ws.finish(repo.op_id().clone())?;
            writeln!(ui.status(), "Reset Watchman clock")?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "watchman"))]
pub fn cmd_debug_watchman(
    _ui: &mut Ui,
    _command: &CommandHelper,
    _subcommand: &DebugWatchmanCommand,
) -> Result<(), CommandError> {
    Err(user_error(
        "Cannot query Watchman because jj was not compiled with the `watchman` feature",
    ))
}

#[cfg(feature = "watchman")]
fn check_local_disk_wc(x: &dyn WorkingCopy) -> Result<&LocalWorkingCopy, CommandError> {
    x.downcast_ref()
        .ok_or_else(|| user_error("This command requires a standard local-disk working copy"))
}
