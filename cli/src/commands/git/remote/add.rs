// Copyright 2020-2023 The Jujutsu Authors
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

use clap::ValueEnum;
use jj_lib::git;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::git_util::absolute_git_url;
use crate::ui::Ui;

/// Add a Git remote
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteAddArgs {
    /// The remote's name
    remote: RemoteNameBuf,
    /// The remote's URL or path
    ///
    /// Local path will be resolved to absolute form.
    #[arg(value_hint = clap::ValueHint::Url)]
    url: String,

    /// Configure when to fetch tags
    #[arg(long, value_enum, default_value_t = RemoteFetchTagsMode::Included)]
    fetch_tags: RemoteFetchTagsMode,
}

/// Configure the `tagOpt` setting of the remote
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum RemoteFetchTagsMode {
    /// Always fetch all tags
    All,

    /// Only fetch tags that point to objects that are already being
    /// transmitted.
    Included,

    /// Do not fetch any tags
    None,
}

impl RemoteFetchTagsMode {
    fn as_fetch_tags(self) -> gix::remote::fetch::Tags {
        match self {
            Self::All => gix::remote::fetch::Tags::All,
            Self::Included => gix::remote::fetch::Tags::Included,
            Self::None => gix::remote::fetch::Tags::None,
        }
    }
}

pub fn cmd_git_remote_add(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteAddArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let url = absolute_git_url(command.cwd(), &args.url)?;
    git::add_remote(
        workspace_command.repo().store(),
        &args.remote,
        &url,
        args.fetch_tags.as_fetch_tags(),
    )?;
    Ok(())
}
