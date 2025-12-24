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
use std::io::Write as _;

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::rewrite::rebase_commit;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::compute_commit_location;
use crate::cli_util::merge_args_with;
use crate::command_error::CommandError;
use crate::complete;
use crate::description_util::add_trailers;
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
///     https://docs.jj-vcs.dev/latest/working-copy/
#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("revisions").multiple(true)))]
pub(crate) struct NewArgs {
    /// Parent(s) of the new change [default: @] [aliases: -o, -r]
    #[arg(group = "revisions", value_name = "REVSETS")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions_pos: Option<Vec<RevisionArg>>,

    #[arg(
        short = 'o',
        group = "revisions",
        hide = true,
        short_aliases = ['d', 'r'],
        value_name = "REVSETS",

    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions_opt: Option<Vec<RevisionArg>>,

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
    ///
    /// Example: `jj new --insert-after A` creates a new change between `A` and
    /// its children:
    ///
    /// ```text
    ///                 B   C
    ///                  \ /
    ///     B   C   =>    @
    ///      \ /          |
    ///       A           A
    /// ```
    ///
    /// Specifying `--insert-after` multiple times will relocate all children of
    /// the given commits.
    ///
    /// Example: `jj new --insert-after A --insert-after X` creates a change
    /// with `A` and `X` as parents, and rebases all children on top of the new
    /// change:
    ///
    /// ```text
    ///                 B   Y
    ///                  \ /
    ///     B  Y    =>    @
    ///     |  |         / \
    ///     A  X        A   X
    /// ```
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "revisions",
        value_name = "REVSETS",
        verbatim_doc_comment
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    insert_after: Option<Vec<RevisionArg>>,

    /// Insert the new change before the given commit(s)
    ///
    /// Example: `jj new --insert-before C` creates a new change between `C` and
    /// its parents:
    ///
    /// ```text
    ///                    C
    ///                    |
    ///       C     =>     @
    ///      / \          / \
    ///     A   B        A   B
    /// ```
    ///
    /// `--insert-after` and `--insert-before` can be combined.
    ///
    /// Example: `jj new --insert-after A --insert-before D`:
    ///
    /// ```text
    /// 
    ///     D            D
    ///     |           / \
    ///     C          |   C
    ///     |    =>    @   |
    ///     B          |   B
    ///     |           \ /
    ///     A            A
    /// ```
    ///
    /// Similar to `--insert-after`, you can specify `--insert-before` multiple
    /// times.
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "revisions",
        value_name = "REVSETS",
        verbatim_doc_comment
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    insert_before: Option<Vec<RevisionArg>>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_new(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &NewArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let revision_args = match (&args.revisions_pos, &args.revisions_opt) {
        (None, None) => (args.insert_before.is_none() && args.insert_after.is_none())
            .then(|| vec![RevisionArg::AT]),
        (None, Some(args)) | (Some(args), None) => Some(args.clone()),
        (Some(pos), Some(opt)) => Some(merge_args_with(
            command.matches().subcommand_matches("new").unwrap(),
            &[("revisions_pos", pos), ("revisions_opt", opt)],
            |_id, value| value.clone(),
        )),
    };
    let (parent_commit_ids, child_commit_ids) = compute_commit_location(
        ui,
        &workspace_command,
        revision_args.as_deref(),
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
                workspace_command.get_advanceable_bookmarks(ui, parent_commits[0].parent_ids())?;
        }
    };

    let parent_commit_ids_set: HashSet<CommitId> = parent_commit_ids.iter().cloned().collect();

    let mut tx = workspace_command.start_transaction();
    let merged_tree = merge_commit_trees(tx.repo(), &parent_commits).block_on()?;
    let mut commit_builder = tx
        .repo_mut()
        .new_commit(parent_commit_ids, merged_tree)
        .detach();
    let mut description = join_message_paragraphs(&args.message_paragraphs);
    if !description.is_empty() {
        // The first trailer would become the first line of the description.
        // Also, a commit with no description is treated in a special way in jujutsu: it
        // can be discarded as soon as it's no longer the working copy. Adding a
        // trailer to an empty description would break that logic.
        commit_builder.set_description(description);
        description = add_trailers(ui, &tx, &commit_builder)?;
    }
    commit_builder.set_description(&description);
    let new_commit = commit_builder.write(tx.repo_mut())?;

    let child_commits: Vec<_> = child_commit_ids
        .iter()
        .map(|commit_id| tx.repo().store().get_commit(commit_id))
        .try_collect()?;
    let mut num_rebased = 0;
    for child_commit in child_commits {
        let new_parent_ids = child_commit
            .parent_ids()
            .iter()
            .filter(|id| !parent_commit_ids_set.contains(id))
            .cloned()
            .chain(std::iter::once(new_commit.id().clone()))
            .collect_vec();
        rebase_commit(tx.repo_mut(), child_commit, new_parent_ids).block_on()?;
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
        tx.advance_bookmarks(advanceable_bookmarks, &target)?;
    }

    tx.finish(ui, "new empty commit")?;
    Ok(())
}
