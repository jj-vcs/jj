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

use clap::ArgGroup;
use clap_complete::ArgValueCompleter;
use futures::TryStreamExt as _;
use futures::future::try_join_all;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::commit::conflict_label_for_commits;
use jj_lib::copies::CopyRecords;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo;
use jj_lib::rewrite::merge_commit_trees;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::diff_util::DUMMY_DESCRIPTION_PATH;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::check_diff_revset_has_no_gaps;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Show differences between the diffs of two revsets
///
/// This is like running `jj diff -r` on each side, then comparing those
/// results. It answers: "How do the modifications introduced by revision A
/// differ from the modifications introduced by revision B?"
///
/// For example, if two changes both add a feature but implement it
/// differently, `jj interdiff --from @- --to other` shows what one
/// implementation adds or removes that the other doesn't.
///
/// A common use of this command is to compare how a change has changed
/// since the last push to a remote:
///
/// ```sh
/// $ jj interdiff --from push-xyz@origin --to push-xyz
/// ```
///
/// This command is different from `jj diff --from A --to B`, which compares
/// file contents directly. `interdiff` compares what the changes do in terms of
/// their patches, rather than their file contents. This makes a difference when
/// the two revisions have different parents: `jj diff --from A --to B` will
/// include the changes between their parents while `jj interdiff --from A --to
/// B` will not.
///
/// Technically, this works by rebasing `--from` onto `--to`'s parents and
/// comparing the result to `--to`.
///
/// `--from` and `--to` may also resolve to multiple revisions; each side is
/// then treated as if its revisions were squashed into a single revision
/// first. For example, if revisions `A::B` were rebased and modified to `C::D`,
/// then `jj interdiff --from A::B --to C::D` shows how the changes evolved.
/// Multiple heads and/or roots are supported, but gaps in the revsets are not
/// supported (e.g. `--from 'A|C'` in a linear chain `A::C`).
///
/// To see the changes throughout the whole evolution of a single change
/// instead, use `jj evolog --patch`.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("to_diff").args(&["from", "to"]).multiple(true).required(true)))]
#[command(mut_arg("ignore_all_space", |a| a.short('w')))]
#[command(mut_arg("ignore_space_change", |a| a.short('b')))]
pub(crate) struct InterdiffArgs {
    /// The first revision(s) to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Option<Vec<RevisionArg>>,

    /// The second revision(s) to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    to: Option<Vec<RevisionArg>>,

    /// Restrict the diff to these paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::interdiff_files))]
    paths: Vec<String>,

    #[command(flatten)]
    format: DiffFormatArgs,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_interdiff(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &InterdiffArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let from = resolve_revset(ui, &workspace_command, args.from.as_deref()).await?;
    let to = resolve_revset(ui, &workspace_command, args.to.as_deref()).await?;
    let repo = workspace_command.repo();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let (from_roots, from_heads) = roots_and_heads(&from);
    let (to_roots, to_heads) = roots_and_heads(&to);
    print_unmatched_explicit_paths(
        ui,
        &workspace_command,
        &fileset_expression,
        &get_all_trees(
            repo.as_ref(),
            (&from_roots, &from_heads),
            (&to_roots, &to_heads),
        )
        .await?,
    )
    .await?;

    let diff_renderer = workspace_command.diff_renderer_for(&args.format)?;
    ui.request_pager();

    let (from_tree, to_tree) = get_from_to_trees(
        repo.as_ref(),
        (&from_roots, &from_heads),
        (&to_roots, &to_heads),
    )
    .await?;

    // Handle revsets here using the roots and heads computed above.
    // `DiffRenderer::show_inter_diff` compares predecessors to a single commit.
    let mut formatter = ui.stdout_formatter();
    let mut formatter = formatter.as_mut().labeled("diff");
    let from_description = concatenated_commit_descriptions(&from);
    let to_description = concatenated_commit_descriptions(&to);
    let copy_records = CopyRecords::default(); // TODO: See `DiffRenderer::show_inter_diff`.
    diff_renderer.show_text_diff(
        *formatter,
        Diff::new(DUMMY_DESCRIPTION_PATH, DUMMY_DESCRIPTION_PATH),
        Diff::new(&from_description, &to_description),
    )?;
    diff_renderer
        .show_diff_trees(
            ui,
            *formatter,
            Diff::new(&from_tree, &to_tree),
            matcher.as_ref(),
            &copy_records,
            ui.term_width(),
        )
        .await?;

    Ok(())
}

/// Get all trees for the interdiff range, for [`print_unmatched_explicit_paths`].
async fn get_all_trees(
    repo: &dyn Repo,
    (from_roots, from_heads): (&[Commit], &[Commit]),
    (to_roots, to_heads): (&[Commit], &[Commit]),
) -> BackendResult<Vec<MergedTree>> {
    // We check the parent commits to account for deleted files.
    let mut trees = vec![];
    for commit in itertools::chain(from_roots, to_roots) {
        trees.push(commit.parent_tree(repo.index()).await?);
    }
    for commit in itertools::chain(from_heads, to_heads) {
        trees.push(commit.tree());
    }
    Ok(trees)
}

/// Get the trees to diff for an interdiff command.
async fn get_from_to_trees(
    repo: &dyn Repo,
    (from_roots, from_heads): (&[Commit], &[Commit]),
    (to_roots, to_heads): (&[Commit], &[Commit]),
) -> BackendResult<(MergedTree, MergedTree)> {
    let from_parents = root_parents(from_roots).await?;
    let to_parents = root_parents(to_roots).await?;

    let from_parent_tree = merge_commit_trees(repo, &from_parents).await?;
    let to_parent_tree = merge_commit_trees(repo, &to_parents).await?;

    let from_tree = merge_commit_trees(repo, from_heads).await?;
    let to_tree = merge_commit_trees(repo, to_heads).await?;

    // Note: We're careful to rebase the _combined_ diff in order to avoid double-counting changes
    // shared by both sides.
    //
    // See `test_interdiff_revset_with_shared_changes_compared_to_itself`.
    let from_tree = MergedTree::merge(Merge::from_diffs(
        (
            to_parent_tree,
            format!("{} (new parents)", conflict_label_for_commits(&to_parents)),
        ),
        [Diff::new(
            (
                from_parent_tree,
                format!(
                    "{} (original parents)",
                    conflict_label_for_commits(&from_parents)
                ),
            ),
            (
                from_tree,
                format!(
                    "{} (original revision)",
                    conflict_label_for_commits(from_heads)
                ),
            ),
        )],
    ))
    .await?;

    Ok((from_tree, to_tree))
}

/// Get the ordered set of parents of a set of commits.
async fn root_parents(commits: &[Commit]) -> BackendResult<Vec<Commit>> {
    Ok(try_join_all(commits.iter().map(|commit| commit.parents()))
        .await?
        .into_iter()
        .flatten()
        .collect::<IndexSet<_>>()
        .into_iter()
        .collect_vec())
}

async fn resolve_revset(
    ui: &mut Ui,
    workspace_command: &WorkspaceCommandHelper,
    arg: Option<&[RevisionArg]>,
) -> Result<Vec<Commit>, CommandError> {
    let arg = arg.unwrap_or(std::slice::from_ref(&RevisionArg::AT));
    let evaluator = workspace_command.parse_union_revsets(ui, arg)?;
    check_diff_revset_has_no_gaps(workspace_command, evaluator.expression()).await?;
    let commits: Vec<Commit> = evaluator.evaluate_to_commits()?.try_collect().await?;
    if commits.is_empty() {
        return Err(user_error(format!(
            "Revset `{}` didn't resolve to any revisions",
            itertools::join(arg, " | ")
        )));
    }
    Ok(commits)
}

/// Concatenates the descriptions of `commits` for rendering a description diff.
fn concatenated_commit_descriptions(commits: &[Commit]) -> Merge<String> {
    // TODO: Concatenated descriptions are not necessarily ideal here; they won't
    // show reordered commits nicely. However, once we implement color-moved
    // (#76), that should get us most of the value.
    //
    // See: https://github.com/jj-vcs/jj/issues/76
    Merge::resolved(commits.iter().map(|c| c.description()).join("\n"))
}

/// Partitions `commits` into the roots and heads of the set: the roots are
/// the commits which have no parents in the set, and the heads are the
/// commits which are not a parent of any commit in the set.
fn roots_and_heads(commits: &[Commit]) -> (Vec<Commit>, Vec<Commit>) {
    let ids: HashSet<&CommitId> = commits.iter().map(|c| c.id()).collect();
    let parent_ids: HashSet<&CommitId> = commits.iter().flat_map(|c| c.parent_ids()).collect();
    let roots = commits
        .iter()
        .filter(|c| !c.parent_ids().iter().any(|id| ids.contains(id)))
        .cloned()
        .collect();
    let heads = commits
        .iter()
        .filter(|c| !parent_ids.contains(c.id()))
        .cloned()
        .collect();
    (roots, heads)
}
