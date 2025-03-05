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

use std::io::Write;

use jj_lib::file_util;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::cli_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::user_error_with_message;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Create a new repo in the given directory
///
/// If the given directory does not exist, it will be created. If no directory
/// is given, the current directory is used.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct InitArgs {
    /// The destination directory
    #[arg(default_value = ".", value_hint = clap::ValueHint::DirPath)]
    destination: String,
}

#[instrument(skip_all)]
pub(crate) fn cmd_init(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &InitArgs,
) -> Result<(), CommandError> {
    if command.global_args().ignore_working_copy {
        return Err(cli_error("--ignore-working-copy is not respected"));
    }
    if command.global_args().at_operation.is_some() {
        return Err(cli_error("--at-op is not respected"));
    }
    let cwd = command.cwd();
    let wc_path = cwd.join(&args.destination);
    let wc_path = file_util::create_or_reuse_dir(&wc_path)
        .and_then(|_| dunce::canonicalize(wc_path))
        .map_err(|e| user_error_with_message("Failed to create workspace", e))?;

    // TODO: remove ui.allow-init-native after jj 0.33+ and inline the condition
    // again
    let config_disabled = {
        let old_name = !command.settings().get_bool("ui.allow-init-native")?;
        if old_name {
            writeln!(
                ui.warning_default(),
                "--ui.allow-init-native is deprecated; use ui.allow-init-toy instead."
            )?;
        }
        old_name || !command.settings().get_bool("ui.allow-init-toy")?
    };
    if config_disabled {
        return Err(user_error_with_hint(
            "The toy backend is disallowed by default.",
            "Did you mean to call `jj git init`?
Set `ui.allow-init-toy` to allow initializing a repo with the toy backend.",
        ));
    }
    Workspace::init_local(&command.settings_for_new_workspace(&wc_path)?, &wc_path)?;

    let relative_wc_path = file_util::relative_path(cwd, &wc_path);
    writeln!(
        ui.status(),
        "Initialized repo in \"{}\"",
        relative_wc_path.display()
    )?;
    Ok(())
}
