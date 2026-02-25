// Copyright 2026 The Jujutsu Authors
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

use std::collections::HashMap;
use std::io::Write as _;

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::matchers::PrefixMatcher;
use jj_lib::merge::Diff;
use jj_lib::merged_tree::TreeDiffEntry;
use jj_lib::merged_tree::TreeDiffIterator;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::rewrite::merge_commit_trees;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Graft revisions onto new locations with path translation.
#[derive(clap::Subcommand, Clone, Debug)]
pub(crate) enum GraftCommand {
    Tree(GraftTreeArgs),
}

/// Re-root commits by translating file paths.
///
/// Creates new commits where files under the source path prefix are moved to
/// the destination path prefix. This enables vendoring workflows — e.g.
/// importing commits from an upstream repo and grafting `src/foo` into
/// `vendor/foo`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct GraftTreeArgs {
    /// The revision(s) to graft
    #[arg(long, required = true)]
    from: Vec<RevisionArg>,
    /// Source path prefix to match files from
    #[arg(long, required = true)]
    path: String,
    /// Destination path prefix to place files at
    #[arg(long, required = true)]
    onto: String,
    /// The revision to graft onto (default: @-)
    #[arg(long, default_value = "@-")]
    destination: RevisionArg,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_graft(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GraftCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GraftCommand::Tree(args) => cmd_graft_tree(ui, command, args).await,
    }
}

#[instrument(skip_all)]
async fn cmd_graft_tree(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GraftTreeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let source_prefix = workspace_command.parse_file_path(&args.path)?;
    let dest_prefix = workspace_command.parse_file_path(&args.onto)?;

    let destination_commit = workspace_command
        .resolve_single_rev(ui, &args.destination)
        .await?;

    // Evaluate revset — returns commits in reverse topological order
    let commit_ids: Vec<CommitId> = workspace_command
        .parse_union_revsets(ui, &args.from)?
        .evaluate_to_commit_ids()?
        .try_collect()
        .await?;

    if commit_ids.is_empty() {
        writeln!(ui.status(), "No revisions to graft.")?;
        return Ok(());
    }

    // Reverse to get parents-first (topological) order
    let commit_ids: Vec<CommitId> = commit_ids.into_iter().rev().collect();

    // Filter out root commit
    let root_commit_id = workspace_command.repo().store().root_commit_id().clone();
    let commit_ids: Vec<CommitId> = commit_ids
        .into_iter()
        .filter(|id| *id != root_commit_id)
        .collect();

    if commit_ids.is_empty() {
        writeln!(ui.status(), "No revisions to graft.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    let store = tx.repo().store().clone();
    let matcher = PrefixMatcher::new([&source_prefix]);

    // Maps old commit ID → new commit ID (or inherited parent for skipped commits)
    let mut old_to_new: HashMap<CommitId, CommitId> = HashMap::new();
    let mut grafted_count = 0;

    for old_id in &commit_ids {
        let old_commit = store.get_commit(old_id)?;
        let commit_tree = old_commit.tree();

        // Determine new parents: map all original parents through old_to_new,
        // dedup, and fallback to destination if none were mapped.
        let new_parent_ids: Vec<CommitId> = old_commit
            .parent_ids()
            .iter()
            .filter_map(|pid| old_to_new.get(pid).cloned())
            .unique()
            .collect();
        let new_parent_ids = if new_parent_ids.is_empty() {
            vec![destination_commit.id().clone()]
        } else {
            new_parent_ids
        };
        let is_merge = new_parent_ids.len() > 1;

        let new_parent_commits: Vec<_> = new_parent_ids
            .iter()
            .map(|id| store.get_commit(id))
            .try_collect()?;
        let new_parent_tree = merge_commit_trees(tx.repo(), &new_parent_commits).block_on()?;

        // Compute the diff base from only the old parents that are mapped
        // (present in old_to_new). Files from unmapped parents (not in the
        // revset) will appear as additions in the diff, ensuring they are
        // included in the new tree.
        let mapped_old_parents: Vec<_> = old_commit
            .parent_ids()
            .iter()
            .filter(|pid| old_to_new.contains_key(*pid))
            .map(|pid| store.get_commit(pid))
            .try_collect()?;
        let base_tree = if mapped_old_parents.is_empty() {
            store.empty_merged_tree()
        } else {
            merge_commit_trees(tx.repo(), &mapped_old_parents).block_on()?
        };

        // Build new tree incrementally: start from the merged new parent tree
        // and apply only the changed files (under the source prefix) with
        // translated paths. This is O(changed_files) per commit instead of
        // O(total_files).
        let mut tree_builder = MergedTreeBuilder::new(new_parent_tree.clone());
        let mut has_changes = false;

        for entry in TreeDiffIterator::new(&base_tree, &commit_tree, &matcher) {
            let TreeDiffEntry { path, values } = entry;
            let Diff { before: _, after } = values?;
            if let Some(new_path) = translate_path(&path, &source_prefix, &dest_prefix) {
                tree_builder.set_or_remove(new_path, after);
                has_changes = true;
            }
        }

        if !has_changes && !is_merge {
            // Skip non-merge commit with no changes under the source prefix.
            // Map to first parent so children chain correctly.
            old_to_new.insert(old_id.clone(), new_parent_ids[0].clone());
            continue;
        }

        let new_tree = tree_builder.write_tree().block_on()?;

        // Skip non-merge commits where the tree is unchanged from the parent.
        // Merge commits are always preserved to maintain DAG structure.
        if !is_merge && new_tree.tree_ids() == new_parent_tree.tree_ids() {
            old_to_new.insert(old_id.clone(), new_parent_ids[0].clone());
            continue;
        }

        let new_commit = tx
            .repo_mut()
            .new_commit(new_parent_ids, new_tree)
            .generate_new_change_id()
            .set_description(old_commit.description())
            .set_author(old_commit.author().clone())
            .set_committer(old_commit.committer().clone())
            .write()
            .block_on()?;

        if let Some(mut formatter) = ui.status_formatter() {
            write!(formatter, "Grafted ")?;
            tx.write_commit_summary(formatter.as_mut(), &old_commit)?;
            write!(formatter, " as ")?;
            tx.write_commit_summary(formatter.as_mut(), &new_commit)?;
            writeln!(formatter)?;
        }

        old_to_new.insert(old_id.clone(), new_commit.id().clone());
        grafted_count += 1;
    }

    if grafted_count == 0 {
        writeln!(ui.status(), "No revisions to graft.")?;
        tx.finish(ui, "graft (no changes)").await?;
        return Ok(());
    }

    tx.finish(ui, format!("graft {grafted_count} commit(s)")).await?;
    Ok(())
}

/// Translates a file path from source prefix to destination prefix.
fn translate_path(
    original: &jj_lib::repo_path::RepoPath,
    source_prefix: &RepoPathBuf,
    dest_prefix: &RepoPathBuf,
) -> Option<RepoPathBuf> {
    let suffix = original.strip_prefix(source_prefix)?;
    let dest_str = dest_prefix.as_internal_file_string();
    let suffix_str = suffix.as_internal_file_string();
    let new_str = if suffix_str.is_empty() {
        dest_str.to_owned()
    } else if dest_str.is_empty() {
        suffix_str.to_owned()
    } else {
        format!("{dest_str}/{suffix_str}")
    };
    RepoPathBuf::from_internal_string(new_str).ok()
}
