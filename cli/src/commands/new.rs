// Copyright 2020 The Jujutsu Authors
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
use std::io::Write;

use clap_complete::ArgValueCandidates;
use itertools::Itertools;
use jj_lib::backend::CommitId;
use jj_lib::repo::Repo;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::rewrite::rebase_commit;
use tracing::instrument;

use crate::cli_util::compute_commit_location;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::description_util::join_message_paragraphs;
use crate::ui::Ui;

/// Create a new, empty change and (by default) edit it in the working copy
///
/// By default, `jj` will edit the new change, making the [working copy]
/// represent the new commit. This can be avoided with `--no-edit`.
///
/// Note that you can create a merge commit by specifying multiple revisions as
/// argument. For example, `jj new @ main` will create a new commit with the
/// working copy and the `main` bookmark as parents.
///
/// [working copy]:
///     https://jj-vcs.github.io/jj/latest/working-copy/
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct NewArgs {
    /// Parent(s) of the new change
    #[arg(
        default_value = "@",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::all_revisions)
    )]
    revisions: Option<Vec<RevisionArg>>,
    /// Ignored (but lets you pass `-d`/`-r` for consistency with other
    /// commands)
    #[arg(short = 'd', hide = true, short_alias = 'r',  action = clap::ArgAction::Count)]
    unused_destination: u8,
    /// The change description to use
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,
    /// Do not edit the newly created change
    #[arg(long, conflicts_with = "_edit")]
    no_edit: bool,
    /// No-op flag to pair with --no-edit
    #[arg(long, hide = true)]
    _edit: bool,
    /// Insert the new change after the given commit(s)
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "revisions",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::all_revisions),
    )]
    insert_after: Option<Vec<RevisionArg>>,
    /// Insert the new change before the given commit(s)
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "revisions",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::mutable_revisions),
    )]
    insert_before: Option<Vec<RevisionArg>>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_new(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &NewArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let (parent_commit_ids, child_commit_ids) = compute_commit_location(
        ui,
        &workspace_command,
        // HACK: `args.revisions` will always have a value due to the `default_value`, however
        // `compute_commit_location` requires that the `destination` argument is mutually exclusive
        // to `insert_after` and `insert_before` arguments.
        if args.insert_before.is_some() || args.insert_after.is_some() {
            None
        } else {
            args.revisions.as_deref()
        },
        args.insert_after.as_deref(),
        args.insert_before.as_deref(),
        "new commit",
    )?;
    let parent_commits: Vec<_> = parent_commit_ids
        .iter()
        .map(|commit_id| workspace_command.repo().store().get_commit(commit_id))
        .try_collect()?;
    let mut advance_bookmarks_target = None;
    let mut advanceable_bookmarks = vec![];

    if args.insert_before.is_none() && args.insert_after.is_none() {
        let should_advance_bookmarks = parent_commits.len() == 1;
        if should_advance_bookmarks {
            advance_bookmarks_target = Some(parent_commit_ids[0].clone());
            advanceable_bookmarks =
                workspace_command.get_advanceable_bookmarks(parent_commits[0].parent_ids())?;
        }
    };

    let parent_commit_ids_set: HashSet<CommitId> = parent_commit_ids.iter().cloned().collect();

    let mut tx = workspace_command.start_transaction();
    let merged_tree = merge_commit_trees(tx.repo(), &parent_commits)?;
    let new_commit = tx
        .repo_mut()
        .new_commit(parent_commit_ids, merged_tree.id())
        .set_description(join_message_paragraphs(&args.message_paragraphs))
        .write()?;

    let mut num_rebased = 0;
    for child_commit_id in child_commit_ids {
        let child_commit = tx.repo_mut().store().get_commit(&child_commit_id)?;
        let new_parent_ids = child_commit
            .parent_ids()
            .iter()
            .filter(|id| !parent_commit_ids_set.contains(id))
            .cloned()
            .chain(std::iter::once(new_commit.id().clone()))
            .collect_vec();
        rebase_commit(tx.repo_mut(), child_commit, new_parent_ids)?;
        num_rebased += 1;
    }
    num_rebased += tx.repo_mut().rebase_descendants()?;

    if args.no_edit {
        if let Some(mut formatter) = ui.status_formatter() {
            write!(formatter, "Created new commit ")?;
            tx.write_commit_summary(formatter.as_mut(), &new_commit)?;
            writeln!(formatter)?;
        }
    } else {
        tx.edit(&new_commit)?;
        // The description of the new commit will be printed by tx.finish()
    }
    if num_rebased > 0 {
        writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
    }

    // Does nothing if there's no bookmarks to advance.
    if let Some(target) = advance_bookmarks_target {
        tx.advance_bookmarks(advanceable_bookmarks, &target);
    }

    tx.finish(ui, "new empty commit")?;
    Ok(())
}
