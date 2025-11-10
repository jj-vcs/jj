// Copyright 2025 The Jujutsu Authors
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

use bstr::ByteVec as _;
use clap::ArgGroup;
use clap_complete::ArgValueCompleter;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::merge_commit_trees;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::compute_commit_location;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Apply the reverse of the given revision(s)
///
/// The reverse of each of the given revisions is applied sequentially in
/// reverse topological order at the given location.
///
/// The description of the new revisions can be customized with the
/// `templates.revert_description` config variable.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("location").args(&["onto", "insert_after", "insert_before"]).required(true).multiple(true)))]
pub(crate) struct RevertArgs {
    /// The revision(s) to apply the reverse of
    #[arg(
        long, short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revisions: Vec<RevisionArg>,
    /// The revision(s) to apply the reverse changes on top of
    #[arg(
        long,
        alias = "destination",
        short,
        short_alias = 'd',
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    onto: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert the reverse changes after (can be repeated to
    /// create a merge commit)
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "onto",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    insert_after: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert the reverse changes before (can be repeated to
    /// create a merge commit)
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "onto",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    insert_before: Option<Vec<RevisionArg>>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_revert(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RevertArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let to_revert: Vec<_> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?; // in reverse topological order
    if to_revert.is_empty() {
        writeln!(ui.status(), "No revisions to revert.")?;
        return Ok(());
    }
    let (new_parent_ids, new_child_ids) = compute_commit_location(
        ui,
        &workspace_command,
        args.onto.as_deref(),
        args.insert_after.as_deref(),
        args.insert_before.as_deref(),
        "reverted commits",
    )?;
    let transaction_description = if to_revert.len() == 1 {
        format!("revert commit {}", to_revert[0].id().hex())
    } else {
        format!(
            "revert commit {} and {} more",
            to_revert[0].id().hex(),
            to_revert.len() - 1
        )
    };
    let commits_to_revert_with_new_commit_descriptions = {
        let template_text = command
            .settings()
            .get_string("templates.revert_description")?;
        let template = workspace_command.parse_commit_template(ui, &template_text)?;

        to_revert
            .into_iter()
            .map(|commit| {
                let commit_description = template.format_plain_text(&commit).into_string_lossy();
                (commit, commit_description)
            })
            .collect_vec()
    };
    let mut tx = workspace_command.start_transaction();
    let original_parent_commit_ids: HashSet<_> = new_parent_ids.iter().cloned().collect();
    let new_parents: Vec<_> = new_parent_ids
        .iter()
        .map(|id| tx.repo().store().get_commit(id))
        .try_collect()?;
    let mut new_base_tree = merge_commit_trees(tx.repo(), &new_parents).block_on()?;
    let mut parent_ids = new_parent_ids;

    let mut reverted_commits = vec![];
    for (commit_to_revert, new_commit_description) in
        &commits_to_revert_with_new_commit_descriptions
    {
        let old_base_tree = commit_to_revert.parent_tree(tx.repo())?;
        let old_tree = commit_to_revert.tree();
        let new_tree = new_base_tree
            .merge_unlabeled(old_tree, old_base_tree)
            .block_on()?;
        let new_parent_ids = parent_ids.clone();
        let new_commit = tx
            .repo_mut()
            .new_commit(new_parent_ids, new_tree.clone())
            .set_description(new_commit_description)
            .write()?;
        parent_ids = vec![new_commit.id().clone()];
        reverted_commits.push(new_commit);
        new_base_tree = new_tree;
    }

    // Rebase new children onto the reverted commit.
    let new_head_ids: Vec<_> = parent_ids;
    let children_commit_ids_set: HashSet<CommitId> = new_child_ids.iter().cloned().collect();
    let mut num_rebased = 0;
    tx.repo_mut()
        .transform_descendants(new_child_ids, async |mut rewriter| {
            if children_commit_ids_set.contains(rewriter.old_commit().id()) {
                let mut child_new_parent_ids = IndexSet::new();
                for old_parent_id in rewriter.old_commit().parent_ids() {
                    // If the original parents of the new children are the new parents of
                    // `target_head_ids`, replace them with `new_head_ids` since we are
                    // "inserting" the new commits in between the new parents and the new
                    // children.
                    if original_parent_commit_ids.contains(old_parent_id) {
                        child_new_parent_ids.extend(new_head_ids.clone());
                    } else {
                        child_new_parent_ids.insert(old_parent_id.clone());
                    }
                }
                // If not already present, add `new_head_ids` as parents of the new child
                // commit.
                child_new_parent_ids.extend(new_head_ids.clone());
                rewriter.set_new_parents(child_new_parent_ids.into_iter().collect());
            }
            num_rebased += 1;
            rewriter.rebase().await?.write()?;
            Ok(())
        })?;

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Reverted {} commits as follows:",
            reverted_commits.len()
        )?;
        print_updated_commits(
            formatter.as_mut(),
            &tx.commit_summary_template(),
            &reverted_commits,
        )?;
        if num_rebased > 0 {
            writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
        }
    }
    tx.finish(ui, transaction_description)?;

    Ok(())
}
