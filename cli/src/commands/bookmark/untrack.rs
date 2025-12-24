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
use jj_lib::str_util::StringExpression;

use super::find_trackable_remote_bookmarks;
use super::trackable_remote_bookmarks_matching;
use super::warn_unmatched_local_or_remote_bookmarks;
use super::warn_unmatched_remotes;
use crate::cli_util::CommandHelper;
use crate::cli_util::RemoteBookmarkNamePattern;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
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
    /// Bookmark names to untrack
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(required = true, value_name = "BOOKMARK")]
    #[arg(add = ArgValueCandidates::new(complete::tracked_bookmarks))]
    names: Vec<String>,

    /// Remote names to untrack
    ///
    /// By default, the specified pattern matches remote names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// If no remote names are given, all remote bookmarks matching the bookmark
    /// names will be untracked.
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long = "remote", value_name = "REMOTE")]
    remotes: Option<Vec<String>>,
}

pub fn cmd_bookmark_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkUntrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let view = repo.view();
    let ignored_remote = default_ignored_remote_name(repo.store())
        // suppress unmatched remotes warning for default-ignored remote
        .filter(|name| view.get_remote_view(name).is_some());
    let matched_refs = if args.remotes.is_none() && args.names.iter().all(|s| s.contains('@')) {
        // TODO: Delete in jj 0.43+
        writeln!(
            ui.warning_default(),
            "<bookmark>@<remote> syntax is deprecated, use `<bookmark> --remote=<remote>` instead."
        )?;
        let name_patterns: Vec<RemoteBookmarkNamePattern> = args
            .names
            .iter()
            .map(|s| s.parse())
            .try_collect()
            .map_err(cli_error)?;
        find_trackable_remote_bookmarks(ui, view, &name_patterns)?
    } else {
        let bookmark_expr = parse_union_name_patterns(ui, &args.names)?;
        let remote_expr = match (&args.remotes, ignored_remote) {
            (Some(text), _) => parse_union_name_patterns(ui, text)?,
            (None, Some(ignored)) => StringExpression::exact(ignored).negated(),
            (None, None) => StringExpression::all(),
        };
        let bookmark_matcher = bookmark_expr.to_matcher();
        let remote_matcher = remote_expr.to_matcher();
        let matched_refs =
            trackable_remote_bookmarks_matching(view, &bookmark_matcher, &remote_matcher).collect();
        warn_unmatched_local_or_remote_bookmarks(ui, view, &bookmark_expr)?;
        warn_unmatched_remotes(ui, view, &remote_expr)?;
        matched_refs
    };
    let mut symbols = Vec::new();
    for (symbol, remote_ref) in matched_refs {
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
