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
use jj_lib::op_store::LocalRemoteRefTarget;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RefName;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use super::find_bookmarks_with;
use crate::cli_util::CommandHelper;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Forget a bookmark without marking it as a deletion to be pushed
///
/// If a local bookmark is forgotten, any corresponding remote bookmarks will
/// become untracked to ensure that the forgotten bookmark will not impact
/// remotes on future pushes.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkForgetArgs {
    /// When forgetting a local bookmark, also forget any corresponding remote
    /// bookmarks
    ///
    /// A forgotten remote bookmark will not impact remotes on future pushes. It
    /// will be recreated on future fetches if it still exists on the remote. If
    /// there is a corresponding Git-tracking remote bookmark, it will also be
    /// forgotten.
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
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    names: Vec<StringPattern>,
}

pub fn cmd_bookmark_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let ignored_remote = default_ignored_remote_name(repo.store());
    let matched_bookmarks = find_forgettable_bookmarks(repo.view(), &args.names)?;
    let mut tx = workspace_command.start_transaction();
    let mut forgotten_remote: usize = 0;
    for (name, bookmark_target) in &matched_bookmarks {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::absent());
        for (remote, _) in &bookmark_target.remote_refs {
            let symbol = name.to_remote_symbol(remote);
            // If `--include-remotes` is specified, we forget the corresponding remote
            // bookmarks instead of untracking them
            if args.include_remotes {
                tx.repo_mut()
                    .set_remote_bookmark(symbol, RemoteRef::absent());
                forgotten_remote += 1;
                continue;
            }
            // Git-tracking remote bookmarks cannot be untracked currently, so skip them
            if ignored_remote.is_some_and(|ignored| symbol.remote == ignored) {
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
        .map(|(name, _)| name.as_symbol())
        .join(", ");
    tx.finish(ui, format!("forget bookmark {forgotten_bookmarks}"))?;
    Ok(())
}

fn find_forgettable_bookmarks<'a>(
    view: &'a View,
    name_patterns: &[StringPattern],
) -> Result<Vec<(&'a RefName, LocalRemoteRefTarget<'a>)>, CommandError> {
    find_bookmarks_with(name_patterns, |matcher| {
        let bookmarks = view
            .bookmarks()
            .filter(|(name, _)| matcher.is_match(name.as_str()))
            .collect();
        Ok(bookmarks)
    })
}
