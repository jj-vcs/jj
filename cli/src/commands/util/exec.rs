// Copyright 2024 The Jujutsu Authors
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

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Execute an external command via jj
///
/// This command will have access to the environment variable JJ_WORKSPACE_ROOT.
///
/// This is useful for arbitrary aliases.
///
/// !! WARNING !!
///
/// The following technique just provides a convenient syntax for running
/// arbitrary code on your system. Using it irresponsibly may cause damage
/// ranging from breaking the behavior of `jj undo` to wiping your file system.
/// Exercise the same amount of caution while writing these aliases as you would
/// when typing commands into the terminal!
///
/// This feature may be removed or replaced by an embedded scripting language in
/// the future.
///
/// Let's assume you have a script called "my-jj-script" in you $PATH and you
/// would like to execute it as "jj my-script". You would add the following line
/// to your configuration file to achieve that:
///
/// ```toml
/// [aliases]
/// my-script = ["util", "exec", "--", "my-jj-script"]
/// #                            ^^^^
/// # This makes sure that flags are passed to your script instead of parsed by jj.
/// ```
///
/// If you don't want to manage your script as a separate file, you can even
/// inline it into your config file:
///
/// ```toml
/// [aliases]
/// my-inline-script = ["util", "exec", "--", "bash", "-c", """
/// set -euo pipefail
/// echo "Look Ma, everything in one file!"
/// echo "args: $@"
/// """, ""]
/// #    ^^
/// # This last empty string will become "$0" in bash, so your actual arguments
/// # are all included in "$@" and start at "$1" as expected.
/// ```
///
/// > Note: Shebangs (e.g. `#!/usr/bin/env`) aren't necessary since you're
/// > already explicitly passing your script into the right shell.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub(crate) struct UtilExecArgs {
    /// External command to execute
    command: String,

    /// Arguments to pass to the external command
    #[arg(value_hint = clap::ValueHint::FilePath)]
    args: Vec<String>,
}

pub fn cmd_util_exec(
    _: &mut Ui,
    command: &CommandHelper,
    args: &UtilExecArgs,
) -> Result<(), CommandError> {
    let workspace_root = command
        .workspace_loader()
        .ok()
        .map(|loader| loader.workspace_root());
    let mut cmd = std::process::Command::new(&args.command);
    cmd.args(&args.args);

    if let Some(workspace_root) = workspace_root {
        cmd.env("JJ_WORKSPACE_ROOT", workspace_root);
    }

    let status = cmd.status().map_err(|err| {
        user_error_with_message(
            format!("Failed to execute external command '{}'", &args.command),
            err,
        )
    })?;

    // Try to match the exit status of the executed process.
    if let Some(exit_code) = status.code() {
        std::process::exit(exit_code);
    }
    if !status.success() {
        // signal
        return Err(user_error(format!(
            "External command was terminated by {status}"
        )));
    }
    Ok(())
}
