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

use clap_complete::ArgValueCandidates;
use jj_lib::git;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::git_util::absolute_git_url;
use crate::ui::Ui;

/// Set the URL of a Git remote
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteSetUrlArgs {
    /// The remote's name
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remote: RemoteNameBuf,
    /// The desired URL or path for `remote`
    ///
    /// Local path will be resolved to absolute form.
    #[arg(value_hint = clap::ValueHint::Url)]
    url: String,
}

pub fn cmd_git_remote_set_url(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteSetUrlArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let url = absolute_git_url(command.cwd(), &args.url)?;
    git::set_remote_url(workspace_command.repo().store(), &args.remote, &url)?;
    Ok(())
}
