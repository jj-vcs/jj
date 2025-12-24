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

    /// The URL or path to fetch from
    ///
    /// This is a short form, equivalent to using the explicit --fetch.
    ///
    /// Local path will be resolved to absolute form.
    #[arg(value_hint = clap::ValueHint::Url)]
    url: Option<String>,

    /// The URL or path to push to
    ///
    /// Local path will be resolved to absolute form.
    #[arg(long, value_hint = clap::ValueHint::Url)]
    push: Option<String>,

    /// The URL or path to fetch from
    ///
    /// Local path will be resolved to absolute form.
    #[arg(long, value_hint = clap::ValueHint::Url, conflicts_with = "url")]
    fetch: Option<String>,
}

pub fn cmd_git_remote_set_url(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteSetUrlArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

    let process_url = |url: Option<&String>| {
        url.map(|url| absolute_git_url(command.cwd(), url))
            .transpose()
    };

    let fetch_url = process_url(args.url.as_ref().or(args.fetch.as_ref()))?;
    let push_url = process_url(args.push.as_ref())?;

    git::set_remote_urls(
        workspace_command.repo().store(),
        &args.remote,
        fetch_url.as_deref(),
        push_url.as_deref(),
    )?;
    Ok(())
}
