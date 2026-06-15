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
pub async fn cmd_debug_watchman(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &DebugWatchmanCommand,
) -> Result<(), CommandError> {
    use jj_lib::local_working_copy::LockedLocalWorkingCopy;

    let mut workspace_command = command.workspace_helper(ui).await?;
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
                        "Background snapshotting is {}. Use \
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
            wc.query_watchman(&config)
                .await
                .map_err(annotate_watchman_query_error)?;
            writeln!(
                ui.stdout(),
                "The watchman server seems to be installed and working correctly."
            )?;
            writeln!(
                ui.stdout(),
                "Background snapshotting is currently {}.",
                if wc.is_watchman_trigger_registered(&config).await? {
                    "active"
                } else {
                    "inactive"
                }
            )?;
        }
        DebugWatchmanCommand::QueryClock => {
            let wc = check_local_disk_wc(workspace_command.working_copy())?;
            let (clock, _changed_files) = wc
                .query_watchman(&watchman_config)
                .await
                .map_err(annotate_watchman_query_error)?;
            writeln!(ui.stdout(), "Clock: {clock:?}")?;
        }
        DebugWatchmanCommand::QueryChangedFiles => {
            let wc = check_local_disk_wc(workspace_command.working_copy())?;
            let (_clock, changed_files) = wc
                .query_watchman(&watchman_config)
                .await
                .map_err(annotate_watchman_query_error)?;
            writeln!(ui.stdout(), "Changed files: {changed_files:?}")?;
        }
        DebugWatchmanCommand::ResetClock => {
            let (mut locked_ws, _commit) = workspace_command.start_working_copy_mutation().await?;
            let Some(locked_local_wc): Option<&mut LockedLocalWorkingCopy> =
                locked_ws.locked_wc().downcast_mut()
            else {
                return Err(user_error(
                    "This command requires a standard local-disk working copy",
                ));
            };
            locked_local_wc.reset_watchman()?;
            locked_ws.finish(repo.op_id().clone()).await?;
            writeln!(ui.status(), "Reset Watchman clock")?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "watchman"))]
pub async fn cmd_debug_watchman(
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

/// Returns `true` if `err` (or any error in its source chain) looks like the
/// macOS Homebrew Watchman failure where `~/Library/LaunchAgents` is owned by
/// root, which prevents Watchman from writing its `*.plist` state file.
/// See <https://github.com/jj-vcs/jj/issues/4064>.
///
/// This is best-effort. The telltale text ("LaunchAgents" + "Permission
/// denied") comes from the Watchman CLI's stderr, which reaches us only via the
/// `Display` of `watchman_client::Error`'s connection-discovery variant (the
/// `#[source]` of `jj_lib::fsmonitor::watchman::Error::WatchmanConnectError`).
/// A future `watchman_client` change to that wording would silently disable the
/// hint; it would not break anything.
#[cfg(feature = "watchman")]
fn is_launchagents_permission_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(err);
    while let Some(err) = source {
        let message = err.to_string();
        if message.contains("LaunchAgents") && message.contains("Permission denied") {
            return true;
        }
        source = err.source();
    }
    false
}

/// Converts a `query_watchman` failure into a [`CommandError`], attaching a
/// macOS remediation hint when the failure looks like the
/// `~/Library/LaunchAgents` ownership problem described in
/// <https://github.com/jj-vcs/jj/issues/4064>.
#[cfg(feature = "watchman")]
fn annotate_watchman_query_error(err: jj_lib::working_copy::WorkingCopyStateError) -> CommandError {
    let show_hint = cfg!(target_os = "macos") && is_launchagents_permission_error(&err);
    let mut cmd_err = CommandError::from(err);
    if show_hint {
        cmd_err.add_hint(
            "On macOS, `watchman` may be unable to write to `~/Library/LaunchAgents` if that \
             directory is owned by root. If so, run `sudo chown $USER ~/Library/LaunchAgents` \
             and re-run. See https://github.com/facebook/watchman/issues/326.",
        );
    }
    cmd_err
}

#[cfg(all(test, feature = "watchman"))]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct ChainError {
        message: &'static str,
        source: Option<Box<Self>>,
    }

    impl std::fmt::Display for ChainError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for ChainError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_deref()
                .map(|err| err as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn detects_launchagents_permission_error_in_source_chain() {
        let err = ChainError {
            message: "Failed to query watchman",
            source: Some(Box::new(ChainError {
                message: "Failed to open \
                          /Users/me/Library/LaunchAgents/com.github.facebook.watchman.plist for \
                          write: Permission denied",
                source: None,
            })),
        };
        assert!(is_launchagents_permission_error(&err));
    }

    #[test]
    fn ignores_unrelated_watchman_error() {
        let err = ChainError {
            message: "Could not connect to Watchman",
            source: None,
        };
        assert!(!is_launchagents_permission_error(&err));
    }

    #[test]
    fn requires_both_launchagents_and_permission_denied() {
        let err = ChainError {
            message: "Failed to open /Users/me/Library/LaunchAgents/foo.plist for write",
            source: None,
        };
        assert!(!is_launchagents_permission_error(&err));
    }
}
