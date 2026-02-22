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

use std::collections::HashSet;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;

use crate::cli_util::CommandHelper;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::revset_util;
use crate::ui::Ui;

/// Rename `old` bookmark name to `new` bookmark name
///
/// The new bookmark name points at the same commit as the old bookmark name.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkRenameArgs {
    /// The old name of the bookmark
    #[arg(value_parser = revset_util::parse_bookmark_name)]
    #[arg(add = ArgValueCandidates::new(complete::local_bookmarks))]
    old: RefNameBuf,

    /// The new name of the bookmark
    #[arg(value_parser = revset_util::parse_bookmark_name)]
    new: RefNameBuf,

    /// Allow renaming even if the new bookmark name already exists
    #[arg(long)]
    overwrite_existing: bool,
}

pub fn cmd_bookmark_rename(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkRenameArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    let old_bookmark = &args.old;
    let ref_target = view.get_local_bookmark(old_bookmark).clone();
    if ref_target.is_absent() {
        return Err(user_error(format!(
            "No such bookmark: {old_bookmark}",
            old_bookmark = old_bookmark.as_symbol()
        )));
    }

    let new_bookmark = &args.new;
    if !args.overwrite_existing && view.get_local_bookmark(new_bookmark).is_present() {
        return Err(user_error(format!(
            "Bookmark already exists: {new_bookmark}",
            new_bookmark = new_bookmark.as_symbol()
        )));
    }

    let mut tx = workspace_command.start_transaction();
    tx.repo_mut()
        .set_local_bookmark_target(new_bookmark, ref_target);
    tx.repo_mut()
        .set_local_bookmark_target(old_bookmark, RefTarget::absent());

    let remote_matcher = match default_ignored_remote_name(tx.repo().store()) {
        Some(remote) => StringExpression::exact(remote).negated().to_matcher(),
        None => StringMatcher::all(),
    };
    let mut tracked_present_remote_bookmarks_exist_for_old_bookmark = false;
    let old_tracked_remotes = tx
        .base_repo()
        .view()
        .remote_bookmarks_matching(&StringMatcher::exact(old_bookmark), &remote_matcher)
        .filter(|(_, remote_ref)| {
            if remote_ref.is_tracked() && remote_ref.is_present() {
                tracked_present_remote_bookmarks_exist_for_old_bookmark = true;
            }
            remote_ref.is_tracked()
        })
        .map(|(symbol, _)| symbol.remote.to_owned())
        .collect_vec();
    let mut existing_tracked_remotes = HashSet::new();
    let existing_untracked_remotes = tx
        .base_repo()
        .view()
        .remote_bookmarks_matching(&StringMatcher::exact(new_bookmark), &remote_matcher)
        .filter_map(|(symbol, remote_ref)| {
            if remote_ref.is_tracked() {
                existing_tracked_remotes.insert(symbol.remote.to_owned());
                None
            } else {
                Some(symbol.remote.to_owned())
            }
        })
        .collect::<HashSet<_>>();
    // Untrack the overwritten bookmark's tracked remotes so that the tracking
    // state transfer from the old bookmark starts from a clean base.
    if args.overwrite_existing {
        for remote in &existing_tracked_remotes {
            tx.repo_mut()
                .untrack_remote_bookmark(new_bookmark.to_remote_symbol(remote));
        }
    }
    // preserve tracking state of old bookmark
    for old_remote in old_tracked_remotes {
        let new_remote_bookmark = new_bookmark.to_remote_symbol(&old_remote);
        if existing_untracked_remotes.contains(new_remote_bookmark.remote) {
            writeln!(
                ui.warning_default(),
                "The renamed bookmark already exists on the remote '{remote}', tracking state was \
                 dropped.",
                remote = new_remote_bookmark.remote.as_symbol(),
            )?;
            writeln!(
                ui.hint_default(),
                "To track the existing remote bookmark, run `jj bookmark track {name} \
                 --remote={remote}`",
                name = new_remote_bookmark.name.as_symbol(),
                remote = new_remote_bookmark.remote.as_symbol()
            )?;
            continue;
        }
        // Skip re-tracking remotes that belonged to the overwritten bookmark.
        // Their remote refs still contain the overwritten bookmark's data, so
        // track_remote_bookmark would incorrectly merge that into the local.
        if existing_tracked_remotes.contains(new_remote_bookmark.remote) {
            continue;
        }
        tx.repo_mut().track_remote_bookmark(new_remote_bookmark)?;
    }

    tx.finish(
        ui,
        format!(
            "rename bookmark {old_bookmark} to {new_bookmark}",
            old_bookmark = old_bookmark.as_symbol(),
            new_bookmark = new_bookmark.as_symbol()
        ),
    )?;

    if tracked_present_remote_bookmarks_exist_for_old_bookmark {
        writeln!(
            ui.warning_default(),
            "Tracked remote bookmarks for bookmark {old_bookmark} were not renamed.",
            old_bookmark = old_bookmark.as_symbol(),
        )?;
        writeln!(
            ui.hint_default(),
            "To rename the bookmark on the remote, you can `jj git push --bookmark \
             {old_bookmark}` first (to delete it on the remote), and then `jj git push --bookmark \
             {new_bookmark}`. `jj git push --all --deleted` would also be sufficient.",
            old_bookmark = old_bookmark.as_symbol(),
            new_bookmark = new_bookmark.as_symbol()
        )?;
    }
    if !existing_tracked_remotes.is_empty() && !args.overwrite_existing {
        // This isn't an error because bookmark renaming can't be propagated to
        // the remote immediately. "rename old new && rename new old" should be
        // allowed even if the original old bookmark had tracked remotes.
        writeln!(
            ui.warning_default(),
            "Tracked remote bookmarks for bookmark {new_bookmark} exist.",
            new_bookmark = new_bookmark.as_symbol()
        )?;
        writeln!(
            ui.hint_default(),
            "Run `jj bookmark untrack {new_bookmark}` to disassociate them.",
            new_bookmark = new_bookmark.as_symbol()
        )?;
    }

    Ok(())
}
