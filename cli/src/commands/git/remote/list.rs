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

use std::io::Write as _;

use bstr::BString;
use gix::Remote;
use jj_lib::git;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// List Git remotes
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteListArgs {}

pub fn cmd_git_remote_list(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitRemoteListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let git_repo = git::get_git_repo(workspace_command.repo().store())?;
    for remote_name in git_repo.remote_names() {
        let remote = match git_repo.try_find_remote(&*remote_name) {
            Some(Ok(remote)) => remote,
            Some(Err(err)) => {
                return Err(user_error_with_message(
                    format!("Failed to load configured remote {remote_name}"),
                    err,
                ));
            }
            None => continue, // ignore empty [remote "<name>"] section
        };
        let fetch_url = get_url(&remote, gix::remote::Direction::Fetch);
        let push_url = get_url(&remote, gix::remote::Direction::Push);
        if fetch_url == push_url {
            writeln!(ui.stdout(), "{remote_name} {fetch_url}")?;
        } else {
            writeln!(ui.stdout(), "{remote_name} {fetch_url} (push: {push_url})")?;
        }
    }
    Ok(())
}

fn get_url(remote: &Remote, direction: gix::remote::Direction) -> BString {
    remote
        .url(direction)
        .map(|url| url.to_bstring())
        .unwrap_or_else(|| "<no URL>".into())
}
