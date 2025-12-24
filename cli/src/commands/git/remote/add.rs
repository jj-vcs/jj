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

use jj_lib::git;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::str_util::StringExpression;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::commands::git::FetchTagsMode;
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
    #[arg(long, value_enum, default_value_t = FetchTagsMode::Included)]
    fetch_tags: FetchTagsMode,

    /// The URL used for push
    ///
    /// Local path will be resolved to absolute form
    #[arg(long, value_hint = clap::ValueHint::Url)]
    push_url: Option<String>,
}

pub fn cmd_git_remote_add(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteAddArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let url = absolute_git_url(command.cwd(), &args.url)?;
    let push_url = args
        .push_url
        .as_deref()
        .map(|url| absolute_git_url(command.cwd(), url))
        .transpose()?;

    let mut tx = workspace_command.start_transaction();
    let bookmark_expr = StringExpression::all(); // TODO: add command arg?

    git::add_remote(
        tx.repo_mut(),
        &args.remote,
        &url,
        push_url.as_deref(),
        args.fetch_tags.as_fetch_tags(),
        &bookmark_expr,
    )?;
    tx.finish(ui, format!("add git remote {}", args.remote.as_symbol()))?;
    Ok(())
}
