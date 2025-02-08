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
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::RemoteRefSymbolBuf;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringExpression;
use jj_lib::view::View;

use super::warn_unmatched_local_or_remote_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::complete;
use crate::revset_util::parse_name_patterns_or_remote_symbols;
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
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// `BOOKMARK@REMOTE` resolves to a remote bookmark exactly.
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(required = true, value_name = "BOOKMARK[@REMOTE]")]
    #[arg(add = ArgValueCandidates::new(complete::local_and_remote_bookmarks))]
    names: Vec<String>,
}

pub async fn cmd_bookmark_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let repo = workspace_command.repo().clone();
    let (bookmark_exprs, remote_symbols) = parse_name_patterns_or_remote_symbols(ui, &args.names)?;
    let bookmark_expr = StringExpression::union_all(bookmark_exprs);
    let matched_bookmarks = find_forgettable_bookmarks(ui, repo.view(), &bookmark_expr)?;
    let matched_remote_bookmarks = find_remote_bookmarks(ui, repo.view(), &remote_symbols)?;

    if matched_bookmarks.is_empty() && matched_remote_bookmarks.is_empty() {
        writeln!(ui.status(), "No bookmarks to forget.")?;
        return Ok(());
    }

    let ignored_remote = default_ignored_remote_name(repo.store());
    let mut tx = workspace_command.start_transaction();
    let mut forgotten_local: usize = 0;
    let mut forgotten_remote: usize = 0;

    for &(symbol, remote_ref) in &matched_remote_bookmarks {
        // If the remote bookmark was already deleted explicitly, there is
        // nothing left for `forget` to do.
        if remote_ref.is_absent() {
            continue;
        }
        tx.repo_mut()
            .set_remote_bookmark(symbol, RemoteRef::absent());
        forgotten_remote += 1;
    }

    for (name, bookmark_target) in &matched_bookmarks {
        if bookmark_target.local_target.is_present() {
            forgotten_local += 1;
        }
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::absent());
        for (remote, _) in &bookmark_target.remote_refs {
            let symbol = name.to_remote_symbol(remote);
            // If the remote bookmark was already deleted explicitly, skip it.
            if tx.repo().get_remote_bookmark(symbol).is_absent() {
                continue;
            }
            // If `--include-remotes` is specified, we forget the corresponding
            // remote bookmarks instead of untracking them.
            if args.include_remotes {
                tx.repo_mut()
                    .set_remote_bookmark(symbol, RemoteRef::absent());
                forgotten_remote += 1;
                continue;
            }
            // Git-tracking remote bookmarks cannot be untracked currently, so
            // skip them.
            if ignored_remote.is_some_and(|ignored| symbol.remote == ignored) {
                continue;
            }
            tx.repo_mut().untrack_remote_bookmark(symbol);
        }
    }

    if forgotten_local != 0 {
        writeln!(ui.status(), "Forgot {forgotten_local} local bookmarks.")?;
    }
    if forgotten_remote != 0 {
        writeln!(ui.status(), "Forgot {forgotten_remote} remote bookmarks.")?;
    }

    let forgotten_bookmarks = matched_bookmarks
        .iter()
        .map(|(name, _)| name.as_symbol().to_string())
        .chain(
            matched_remote_bookmarks
                .iter()
                .map(|(symbol, _)| symbol.to_string()),
        )
        .join(", ");
    tx.finish(ui, format!("forget bookmark {forgotten_bookmarks}"))
        .await?;
    Ok(())
}

fn find_forgettable_bookmarks<'a>(
    ui: &Ui,
    view: &'a View,
    name_expr: &StringExpression,
) -> Result<Vec<(&'a RefName, LocalRemoteRefTarget<'a>)>, CommandError> {
    let name_matcher = name_expr.to_matcher();
    let matched_bookmarks = view
        .bookmarks()
        .filter(|(name, _)| name_matcher.is_match(name.as_str()))
        .collect();
    warn_unmatched_local_or_remote_bookmarks(ui, view, name_expr)?;
    Ok(matched_bookmarks)
}

fn find_remote_bookmarks<'a>(
    ui: &Ui,
    view: &'a View,
    symbols: &'a [RemoteRefSymbolBuf],
) -> Result<Vec<(RemoteRefSymbol<'a>, &'a RemoteRef)>, CommandError> {
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for symbol in symbols {
        let symbol = symbol.as_ref();
        let remote_ref = view.get_remote_bookmark(symbol);
        let has_entry = view
            .get_remote_view(symbol.remote)
            .is_some_and(|remote_view| remote_view.bookmarks.contains_key(symbol.name));
        if remote_ref.is_present() || has_entry {
            matched.push((symbol, remote_ref));
        } else {
            unmatched.push(symbol);
        }
    }
    matched.sort_unstable_by_key(|(symbol, _)| *symbol);
    matched.dedup_by(|(symbol1, _), (symbol2, _)| symbol1 == symbol2);
    if !unmatched.is_empty() {
        writeln!(
            ui.warning_default(),
            "No matching remote bookmarks for names: {}",
            unmatched.iter().join(", ")
        )?;
    }
    Ok(matched)
}
