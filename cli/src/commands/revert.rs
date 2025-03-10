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
use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::object_id::ObjectId;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::rewrite::rebase_commit;
use tracing::instrument;

use crate::cli_util::compute_commit_location;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::formatter::PlainTextFormatter;
use crate::ui::Ui;

/// Apply the reverse of the given revision(s)
///
/// The reverse of each of the given revisions is applied sequentially in
/// reverse topological order at the given location.
///
/// The description of the new revisions can be customized with the
/// `templates.revert_description` config variable.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("location").args(&["destination", "insert_after", "insert_before"]).required(true).multiple(true)))]
pub(crate) struct RevertArgs {
    /// The revision(s) to apply the reverse of
    #[arg(
        long, short,
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::all_revisions),
    )]
    revisions: Vec<RevisionArg>,
    /// The revision to apply the reverse changes on top of
    #[arg(
        long, short,
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::all_revisions),
    )]
    destination: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert the reverse changes after (can be repeated to
    /// create a merge commit)
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "destination",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::all_revisions),
    )]
    insert_after: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert the reverse changes before (can be repeated to
    /// create a merge commit)
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "destination",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::mutable_revisions)
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
    let (new_parents, new_children) = compute_commit_location(
        ui,
        &workspace_command,
        args.destination.as_deref(),
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
                let mut output = Vec::new();
                template
                    .format(&commit, &mut PlainTextFormatter::new(&mut output))
                    .expect("write() to vec backed formatter should never fail");
                // Template output is usually UTF-8, but it can contain file content.
                let commit_description = output.into_string_lossy();
                (commit, commit_description)
            })
            .collect_vec()
    };
    let mut tx = workspace_command.start_transaction();
    let mut parents = new_parents;
    let original_parent_commit_ids: HashSet<_> = parents.iter().ids().cloned().collect();
    let mut new_base_tree = merge_commit_trees(tx.repo(), &parents)?;

    for (commit_to_revert, new_commit_description) in
        &commits_to_revert_with_new_commit_descriptions
    {
        let old_base_tree = commit_to_revert.parent_tree(tx.repo())?;
        let old_tree = commit_to_revert.tree()?;
        let new_tree = new_base_tree.merge(&old_tree, &old_base_tree)?;
        let new_parent_ids = parents.iter().map(|commit| commit.id().clone()).collect();
        let new_commit = tx
            .repo_mut()
            .new_commit(new_parent_ids, new_tree.id())
            .set_description(new_commit_description)
            .write()?;
        parents = vec![new_commit];
        new_base_tree = new_tree;
    }

    let mut num_rebased = 0;
    for child_commit in new_children {
        let new_parent_ids = child_commit
            .parent_ids()
            .iter()
            .filter(|id| !original_parent_commit_ids.contains(id))
            .cloned()
            .chain(parents.iter().ids().cloned())
            .collect_vec();
        rebase_commit(tx.repo_mut(), child_commit, new_parent_ids)?;
        num_rebased += 1;
    }
    num_rebased += tx.repo_mut().rebase_descendants()?;

    writeln!(
        ui.status(),
        "Reverted {} commits",
        commits_to_revert_with_new_commit_descriptions.len()
    )?;
    if num_rebased > 0 {
        writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
    }
    tx.finish(ui, transaction_description)?;

    Ok(())
}
