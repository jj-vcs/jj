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

use clap::parser::ValueSource;
use jj_lib::commit::Commit;
use jj_lib::matchers::Matcher;
use jj_lib::object_id::ObjectId;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::settings::UserSettings;
use tracing::instrument;

use crate::cli_util::{CommandHelper, DiffSelector, RevisionArg, WorkspaceCommandTransaction};
use crate::command_error::{user_error, CommandError};
use crate::description_util::{combine_messages, join_message_paragraphs};
use crate::ui::Ui;

/// Move changes from a revision into its parent
///
/// After moving the changes into the parent, the child revision will have the
/// same content state as before. If that means that the change is now empty
/// compared to its parent, it will be abandoned.
/// Without `--interactive`, the child change will always be empty.
///
/// If the source became empty and both the source and destination had a
/// non-empty description, you will be asked for the combined description. If
/// either was empty, then the other one will be used.
///
/// If a working-copy commit gets abandoned, it will be given a new, empty
/// commit. This is true in general; it is not specific to this command.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct SquashArgs {
    #[arg(long, short, default_value = "@")]
    revision: RevisionArg,
    /// The description to use for squashed revision (don't open editor)
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,
    /// Interactively choose which parts to squash
    #[arg(long, short)]
    interactive: bool,
    /// Specify diff editor to be used (implies --interactive)
    #[arg(long, value_name = "NAME")]
    tool: Option<String>,
    /// Move only changes to these paths (instead of all paths)
    #[arg(conflicts_with_all = ["interactive", "tool"], value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_squash(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SquashArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let commit = workspace_command.resolve_single_rev(&args.revision)?;
    workspace_command.check_rewritable([&commit])?;
    let parents = commit.parents();
    if parents.len() != 1 {
        return Err(user_error("Cannot squash merge commits"));
    }
    let parent = &parents[0];
    workspace_command.check_rewritable(&parents[..1])?;
    let matcher = workspace_command.matcher_from_values(&args.paths)?;
    let diff_selector =
        workspace_command.diff_selector(ui, args.tool.as_deref(), args.interactive)?;
    let mut tx = workspace_command.start_transaction();
    let instructions = format!(
        "\
You are moving changes from: {}
into its parent: {}

The left side of the diff shows the contents of the parent commit. The
right side initially shows the contents of the commit you're moving
changes from.

Adjust the right side until the diff shows the changes you want to move
to the destination. If you don't make any changes, then all the changes
from the source will be moved into the parent.
",
        tx.format_commit_summary(&commit),
        tx.format_commit_summary(parent)
    );
    let parent_tree = parent.tree()?;
    let tree = commit.tree()?;
    let new_parent_tree_id =
        diff_selector.select(&parent_tree, &tree, matcher.as_ref(), Some(&instructions))?;
    if &new_parent_tree_id == parent.tree_id() {
        if diff_selector.is_interactive() {
            return Err(user_error("No changes selected"));
        }

        if let [only_path] = &args.paths[..] {
            let (_, matches) = command.matches().subcommand().unwrap();
            if matches.value_source("revision").unwrap() == ValueSource::DefaultValue
                && revset::parse(
                    only_path,
                    &tx.base_workspace_helper().revset_parse_context(),
                )
                .is_ok()
            {
                writeln!(
                    ui.warning(),
                    "warning: The argument {only_path:?} is being interpreted as a path. To \
                     specify a revset, pass -r {only_path:?} instead."
                )?;
            }
        }
    }
    // Abandon the child if the parent now has all the content from the child
    // (always the case in the non-interactive case).
    let abandon_child = &new_parent_tree_id == commit.tree_id();
    let description = if !args.message_paragraphs.is_empty() {
        join_message_paragraphs(&args.message_paragraphs)
    } else {
        combine_messages(
            tx.base_repo(),
            &commit,
            parent,
            command.settings(),
            abandon_child,
        )?
    };
    let mut_repo = tx.mut_repo();
    let new_parent = mut_repo
        .rewrite_commit(command.settings(), parent)
        .set_tree_id(new_parent_tree_id)
        .set_predecessors(vec![parent.id().clone(), commit.id().clone()])
        .set_description(description)
        .write()?;
    if abandon_child {
        mut_repo.record_abandoned_commit(commit.id().clone());
    } else {
        // Commit the remainder on top of the new parent commit.
        mut_repo
            .rewrite_commit(command.settings(), &commit)
            .set_parents(vec![new_parent.id().clone()])
            .write()?;
    }
    tx.finish(ui, format!("squash commit {}", commit.id().hex()))?;
    Ok(())
}

pub fn move_diff(
    tx: &mut WorkspaceCommandTransaction,
    settings: &UserSettings,
    source: Commit,
    mut destination: Commit,
    matcher: &dyn Matcher,
    diff_selector: &DiffSelector,
) -> Result<(), CommandError> {
    tx.base_workspace_helper()
        .check_rewritable([&source, &destination])?;
    let parent_tree = merge_commit_trees(tx.repo(), &source.parents())?;
    let source_tree = source.tree()?;
    let instructions = format!(
        "\
You are moving changes from: {}
into commit: {}

The left side of the diff shows the contents of the parent commit. The
right side initially shows the contents of the commit you're moving
changes from.

Adjust the right side until the diff shows the changes you want to move
to the destination. If you don't make any changes, then all the changes
from the source will be moved into the destination.
",
        tx.format_commit_summary(&source),
        tx.format_commit_summary(&destination)
    );
    let new_parent_tree_id =
        diff_selector.select(&parent_tree, &source_tree, matcher, Some(&instructions))?;
    if diff_selector.is_interactive() && new_parent_tree_id == parent_tree.id() {
        return Err(user_error("No changes to move"));
    }
    let new_parent_tree = tx.repo().store().get_root_tree(&new_parent_tree_id)?;
    // Apply the reverse of the selected changes onto the source
    let new_source_tree = source_tree.merge(&new_parent_tree, &parent_tree)?;
    let abandon_source = new_source_tree.id() == parent_tree.id();
    if abandon_source {
        tx.mut_repo().record_abandoned_commit(source.id().clone());
    } else {
        tx.mut_repo()
            .rewrite_commit(settings, &source)
            .set_tree_id(new_source_tree.id().clone())
            .write()?;
    }
    if tx.repo().index().is_ancestor(source.id(), destination.id()) {
        // If we're moving changes to a descendant, first rebase descendants onto the
        // rewritten source. Otherwise it will likely already have the content
        // changes we're moving, so applying them will have no effect and the
        // changes will disappear.
        let rebase_map = tx.mut_repo().rebase_descendants_return_map(settings)?;
        let rebased_destination_id = rebase_map.get(destination.id()).unwrap().clone();
        destination = tx.mut_repo().store().get_commit(&rebased_destination_id)?;
    }
    // Apply the selected changes onto the destination
    let destination_tree = destination.tree()?;
    let new_destination_tree = destination_tree.merge(&parent_tree, &new_parent_tree)?;
    let description = combine_messages(
        tx.base_repo(),
        &source,
        &destination,
        settings,
        abandon_source,
    )?;
    tx.mut_repo()
        .rewrite_commit(settings, &destination)
        .set_tree_id(new_destination_tree.id().clone())
        .set_predecessors(vec![destination.id().clone(), source.id().clone()])
        .set_description(description)
        .write()?;
    Ok(())
}
