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
use itertools::Either;
use itertools::Itertools as _;
use jj_lib::op_store::BookmarkTarget;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::refs::RemoteRefSymbol;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use super::find_bookmarks_with;
use super::find_remote_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::LocalOrRemoteBookmarkNamePattern;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Forget a bookmark without marking it as a deletion to be pushed
///
/// If a local bookmark is forgotten, any corresponding remote bookmarks will
/// become untracked to ensure that the forgotten bookmark will not impact
/// remotes on future pushes.
///
/// Remote bookmarks can also be forgotten using the normal `bookmark@remote`
/// syntax. If a remote bookmark is forgotten, it will be recreated on future
/// fetches if it still exists on the remote.
///
/// Git-tracking bookmarks (e.g. `bookmark@git`) can also be forgotten. If a
/// Git-tracking bookmark is forgotten, it will be deleted from the underlying
/// Git repo on the next `jj git export`. Otherwise, the bookmark will be
/// recreated on the next `jj git import` if it still exists in the underlying
/// Git repo. In colocated repos, `jj git export` is run automatically after
/// every command, so forgetting a Git-tracking bookmark has no effect in a
/// colocated repo.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkForgetArgs {
    /// When forgetting a local bookmark, also forget any corresponding remote
    /// bookmarks
    ///
    /// If there is a corresponding Git-tracking remote bookmark, it will also
    /// be forgotten.
    #[arg(long)]
    include_remotes: bool,
    /// The bookmarks to forget
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarks by [wildcard pattern].
    ///
    /// [wildcard pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets/#string-patterns
    #[arg(
        required = true,
        add = ArgValueCandidates::new(complete::local_and_remote_bookmarks),
    )]
    names: Vec<LocalOrRemoteBookmarkNamePattern>,
}

pub fn cmd_bookmark_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let (local_bookmarks, remote_bookmarks): (Vec<_>, Vec<_>) =
        args.names.iter().cloned().partition_map(|name| match name {
            LocalOrRemoteBookmarkNamePattern::Local(local) => Either::Left(local),
            LocalOrRemoteBookmarkNamePattern::Remote(remote) => Either::Right(remote),
        });
    let matched_bookmarks = find_forgettable_bookmarks(repo.view(), &local_bookmarks)?;
    let matched_remote_bookmarks = find_remote_bookmarks(repo.view(), &remote_bookmarks)?;
    let mut tx = workspace_command.start_transaction();
    let mut forgotten_remote: usize = 0;
    for &(symbol, _) in &matched_remote_bookmarks {
        tx.repo_mut()
            .set_remote_bookmark(symbol, RemoteRef::absent());
        forgotten_remote += 1;
    }
    for (name, bookmark_target) in &matched_bookmarks {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::absent());
        for (remote, _) in &bookmark_target.remote_refs {
            let symbol = RemoteRefSymbol { name, remote };
            // If the remote bookmark was already deleted explicitly, skip it
            if tx.repo().get_remote_bookmark(symbol).is_absent() {
                continue;
            }
            // If `--include-remotes` is specified, we forget the corresponding remote
            // bookmarks instead of untracking them
            if args.include_remotes {
                tx.repo_mut()
                    .set_remote_bookmark(symbol, RemoteRef::absent());
                forgotten_remote += 1;
                continue;
            }
            // Git-tracking remote bookmarks cannot be untracked currently, so skip them
            if jj_lib::git::is_special_git_remote(symbol.remote) {
                continue;
            }
            tx.repo_mut().untrack_remote_bookmark(symbol);
        }
    }
    writeln!(
        ui.status(),
        "Forgot {} local bookmarks.",
        matched_bookmarks.len()
    )?;
    if forgotten_remote != 0 {
        writeln!(ui.status(), "Forgot {forgotten_remote} remote bookmarks.")?;
    }
    let forgotten_bookmarks = matched_bookmarks
        .iter()
        .map(|(name, _)| name.to_string())
        .chain(
            matched_remote_bookmarks
                .iter()
                .map(|(name, _)| name.to_string()),
        )
        .join(", ");
    tx.finish(ui, format!("forget bookmark {forgotten_bookmarks}"))?;
    Ok(())
}

fn find_forgettable_bookmarks<'a>(
    view: &'a View,
    name_patterns: &[StringPattern],
) -> Result<Vec<(&'a str, BookmarkTarget<'a>)>, CommandError> {
    find_bookmarks_with(name_patterns, |pattern| {
        view.bookmarks()
            .filter(|(name, _)| pattern.matches(name))
            .map(Ok)
    })
}
