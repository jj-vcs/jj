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

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::repo::Repo as _;

use super::find_trackable_remote_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RemoteBookmarkNamePattern;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Stop tracking given remote bookmarks
///
/// A non-tracking remote bookmark is just a pointer to the last-fetched remote
/// bookmark. It won't be imported as a local bookmark on future pulls.
///
/// If you want to forget a local bookmark while also untracking the
/// corresponding remote bookmarks, use `jj bookmark forget` instead.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkUntrackArgs {
    /// Remote bookmarks to untrack
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarks by [wildcard pattern].
    ///
    /// Examples: bookmark@remote, glob:main@*, glob:jjfan-*@upstream
    ///
    /// [wildcard pattern]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(
        required = true,
        value_name = "BOOKMARK@REMOTE",
        add = ArgValueCandidates::new(complete::tracked_bookmarks)
    )]
    names: Vec<RemoteBookmarkNamePattern>,
}

pub fn cmd_bookmark_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkUntrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let ignored_remote = default_ignored_remote_name(repo.store());
    let mut symbols = Vec::new();
    for (symbol, remote_ref) in find_trackable_remote_bookmarks(repo.view(), &args.names)? {
        if ignored_remote.is_some_and(|ignored| symbol.remote == ignored) {
            // This restriction can be lifted if we want to support untracked @git
            // bookmarks.
            writeln!(
                ui.warning_default(),
                "Git-tracking bookmark cannot be untracked: {symbol}"
            )?;
        } else if !remote_ref.is_tracked() {
            writeln!(
                ui.warning_default(),
                "Remote bookmark not tracked yet: {symbol}"
            )?;
        } else {
            symbols.push(symbol);
        }
    }
    let mut tx = workspace_command.start_transaction();
    for &symbol in &symbols {
        tx.repo_mut().untrack_remote_bookmark(symbol);
    }
    if !symbols.is_empty() {
        writeln!(
            ui.status(),
            "Stopped tracking {} remote bookmarks.",
            symbols.len()
        )?;
    }
    tx.finish(
        ui,
        format!("untrack remote bookmark {}", symbols.iter().join(", ")),
    )?;
    Ok(())
}
