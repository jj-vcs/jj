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

use clap::ArgGroup;
use clap_complete::ArgValueCompleter;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::commit::conflict_label_for_commits;
use jj_lib::copies::CopyRecords;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::revset::RevsetExpression;
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
    let (from, from_roots, from_heads) =
        resolve_revset(ui, &workspace_command, args.from.as_deref()).await?;
    let (to, to_roots, to_heads) =
        resolve_revset(ui, &workspace_command, args.to.as_deref()).await?;
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let (from_tree, to_tree) = get_from_to_trees(
        &workspace_command,
        (&from_roots, &from_heads),
        (&to_roots, &to_heads),
    )
    .await?;

    print_unmatched_explicit_paths(
        ui,
        &workspace_command,
        &fileset_expression,
        [&from_tree, &to_tree],
    )
    .await?;

    let diff_renderer = workspace_command.diff_renderer_for(&args.format)?;
    ui.request_pager();

    // Handle revsets here using the roots and heads computed above.
    // `DiffRenderer::show_inter_diff` compares predecessors to a single commit.
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();
    let from_description = concatenated_commit_descriptions(&from);
    let to_description = concatenated_commit_descriptions(&to);
    let copy_records = CopyRecords::default(); // TODO: See `DiffRenderer::show_inter_diff`.
    diff_renderer.show_text_diff(
        formatter,
        Diff::new(DUMMY_DESCRIPTION_PATH, DUMMY_DESCRIPTION_PATH),
        Diff::new(&from_description, &to_description),
    )?;
    diff_renderer
        .show_diff(
            ui,
            formatter,
            Diff::new(&from_tree, &to_tree),
            matcher.as_ref(),
            &copy_records,
            ui.term_width(),
        )
        .await?;

    Ok(())
}

/// Rebases the combined change from `from_roots` to `from_heads` onto the parents of `to_roots`,
/// and returns the trees to compare.
///
/// Unlike `rebase_to_dest_parent()`, this rebases the combined diff rather than each commit's diff
/// separately.
async fn get_from_to_trees(
    workspace_command: &WorkspaceCommandHelper,
    (from_roots, from_heads): (&[Commit], &[Commit]),
    (to_roots, to_heads): (&[Commit], &[Commit]),
) -> Result<(MergedTree, MergedTree), CommandError> {
    let repo = workspace_command.repo().as_ref();
    let from_parents = parents(workspace_command, from_roots).await?;
    let to_parents = parents(workspace_command, to_roots).await?;

    let from_parent_tree = merge_commit_trees(repo, &from_parents).await?;
    let to_parent_tree = merge_commit_trees(repo, &to_parents).await?;

    let from_tree = merge_commit_trees(repo, from_heads).await?;
    let to_tree = merge_commit_trees(repo, to_heads).await?;

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

/// Get the parents of a set of commits in reverse topological order.
async fn parents(
    workspace_command: &WorkspaceCommandHelper,
    commits: &[Commit],
) -> Result<Vec<Commit>, CommandError> {
    let expression =
        RevsetExpression::commits(commits.iter().map(|commit| commit.id().clone()).collect());
    Ok(workspace_command
        .attach_revset_evaluator(expression.parents())
        .evaluate_to_commits()?
        .try_collect()
        .await?)
}

/// Resolve a revset as commits, and then also return the `roots()` and `heads()` of those commits.
async fn resolve_revset(
    ui: &mut Ui,
    workspace_command: &WorkspaceCommandHelper,
    arg: Option<&[RevisionArg]>,
) -> Result<(Vec<Commit>, Vec<Commit>, Vec<Commit>), CommandError> {
    // TODO: There's a lot of duplicated work here; we evaluate the revset four times (one
    // for the commits, again for `check_diff_revset_has_no_gaps`, and then again for the roots and
    // heads.
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
    let roots = workspace_command
        .attach_revset_evaluator(evaluator.expression().roots())
        .evaluate_to_commits()?
        .try_collect()
        .await?;
    let heads = workspace_command
        .attach_revset_evaluator(evaluator.expression().heads())
        .evaluate_to_commits()?
        .try_collect()
        .await?;
    Ok((commits, roots, heads))
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
