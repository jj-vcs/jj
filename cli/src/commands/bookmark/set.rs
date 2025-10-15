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
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;

use super::is_fast_forward;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::has_tracked_remote_bookmarks;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::revset_util;
use crate::ui::Ui;

/// Create or update a bookmark to point to a certain commit
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkSetArgs {
    /// The bookmark's target revision
    #[arg(
        long, short,
        default_value = "@",
        visible_alias = "to",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revision: RevisionArg,

    /// Allow moving the bookmark backwards or sideways
    #[arg(long, short = 'B')]
    allow_backwards: bool,

    /// The bookmarks to update
    #[arg(
        required = true,
        value_parser = revset_util::parse_bookmark_name,
        add = ArgValueCandidates::new(complete::local_bookmarks),
    )]
    names: Vec<RefNameBuf>,
}

pub fn cmd_bookmark_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let repo = workspace_command.repo().as_ref();
    let bookmark_names = &args.names;
    let mut new_bookmarks = HashSet::new();
    let mut moved_bookmark_count = 0;
    for name in bookmark_names {
        let old_target = repo.view().get_local_bookmark(name);
        // If a bookmark is absent locally but is still tracking remote bookmarks,
        // we are resurrecting the local bookmark, not "creating" a new bookmark.
        if old_target.is_absent() && !has_tracked_remote_bookmarks(repo, name) {
            new_bookmarks.insert(name);
        } else if old_target.as_normal() != Some(target_commit.id()) {
            moved_bookmark_count += 1;
        }
        if !args.allow_backwards && !is_fast_forward(repo, old_target, target_commit.id())? {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to move bookmark backwards or sideways: {name}",
                    name = name.as_symbol()
                ),
                "Use --allow-backwards to allow it.",
            ));
        }
    }
    if target_commit.is_discardable(repo)? {
        writeln!(ui.warning_default(), "Target revision is empty.")?;
    }

    let mut tx = workspace_command.start_transaction();
    let remote_settings = tx.settings().remote_settings()?;
    let readonly_repo = tx.base_repo().clone();
    for name in bookmark_names {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::normal(target_commit.id().clone()));
        if new_bookmarks.contains(name) {
            for (remote_name, settings) in &remote_settings {
                if !settings.auto_track_bookmarks.is_match(name.as_str()) {
                    continue;
                }
                let Some(view) = readonly_repo.view().get_remote_view(remote_name) else {
                    continue;
                };
                let symbol = name.to_remote_symbol(remote_name);
                if view.bookmarks.contains_key(name) {
                    writeln!(
                        ui.warning_default(),
                        "Auto-tracking bookmark that exists on the remote: {symbol}"
                    )?;
                }
                tx.repo_mut().track_remote_bookmark(symbol)?;
            }
        }
    }

    if let Some(mut formatter) = ui.status_formatter() {
        let new_bookmark_count = new_bookmarks.len();
        if new_bookmark_count > 0 {
            write!(
                formatter,
                "Created {new_bookmark_count} bookmarks pointing to "
            )?;
            tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
            writeln!(formatter)?;
        }
        if moved_bookmark_count > 0 {
            write!(formatter, "Moved {moved_bookmark_count} bookmarks to ")?;
            tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
            writeln!(formatter)?;
        }
    }

    tx.finish(
        ui,
        format!(
            "point bookmark {names} to commit {id}",
            names = bookmark_names.iter().map(|n| n.as_symbol()).join(", "),
            id = target_commit.id().hex()
        ),
    )?;
    Ok(())
}
