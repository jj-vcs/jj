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

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::config::existing_repo_config_file;
use crate::ui::Ui;

/// Stop trusting the repository-scoped hooks in this checkout
///
/// Clears both the enabled flag and every recorded content hash: this is a
/// clean-slate reset, not a pause. Running `jj hook enable` again always
/// re-lists and re-approves whatever is currently under `.jj-hooks/`, even
/// if it is unchanged since `jj hook disable` was run.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct HookDisableArgs {}

#[instrument(skip_all)]
pub(crate) async fn cmd_hook_disable(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &HookDisableArgs,
) -> Result<(), CommandError> {
    let Some(mut file) = existing_repo_config_file(command.raw_config()) else {
        writeln!(ui.status(), "Repository-scoped hooks were not enabled.")?;
        return Ok(());
    };
    file.data_mut().as_table_mut().remove("hooks");
    file.save()?;
    writeln!(ui.status(), "Repository-scoped hooks are now disabled.")?;
    Ok(())
}
