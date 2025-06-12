// Copyright 2025 The Jujutsu Authors
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

use std::io::Write as _;

use jj_lib::repo::Repo as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Show the underlying Git directory of a repository using the Git backend
#[derive(clap::Args, Clone, Debug)]
pub struct GitRootArgs {}

#[instrument(skip_all)]
pub fn cmd_git_root(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitRootArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let store = workspace_command.repo().store();
    let git_backend = jj_lib::git::get_git_backend(store)?;
    let root = git_backend.git_repo_path();
    writeln!(ui.stdout(), "{root}")?;
    Ok(())
}
