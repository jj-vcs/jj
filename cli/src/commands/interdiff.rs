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
use jj_lib::commit::Commit;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::check_diff_revset_has_no_gaps;
use crate::diff_util::roots_and_heads;
use crate::ui::Ui;

/// Show differences between the diffs of two revisions
///
/// This is like running `jj diff -r` on each change, then comparing those
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
/// first. For example, if revisions A..B were rebased and modified to C..D,
/// then `jj interdiff --from A..B --to C..D` shows how the changes evolved.
/// Multiple heads and/or roots are supported, but gaps in the revsets are not
/// supported (e.g. `--from 'A|C'` in a linear chain A..C).
///
/// To see the changes throughout the whole evolution of a change instead of
/// between just two revisions, use `jj evolog -p` instead.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("to_diff").args(&["from", "to"]).multiple(true).required(true)))]
#[command(mut_arg("ignore_all_space", |a| a.short('w')))]
#[command(mut_arg("ignore_space_change", |a| a.short('b')))]
pub(crate) struct InterdiffArgs {
    /// The first revision(s) to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Option<RevisionArg>,

    /// The second revision(s) to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    to: Option<RevisionArg>,

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
    let resolve_revset = async |arg: &Option<RevisionArg>| -> Result<Vec<Commit>, CommandError> {
        let arg = arg.as_ref().unwrap_or(&RevisionArg::AT);
        let evaluator = workspace_command.parse_revset(ui, arg)?;
        check_diff_revset_has_no_gaps(&workspace_command, evaluator.expression()).await?;
        let commits: Vec<Commit> = evaluator.evaluate_to_commits()?.try_collect().await?;
        if commits.is_empty() {
            return Err(user_error(format!(
                "Revset `{arg}` didn't resolve to any revisions"
            )));
        }
        Ok(commits)
    };
    let from = resolve_revset(&args.from).await?;
    let to = resolve_revset(&args.to).await?;
    let repo = workspace_command.repo();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let (from_roots, from_heads) = roots_and_heads(&from);
    let (to_roots, to_heads) = roots_and_heads(&to);
    // We check the parent commits to account for deleted files.
    let mut trees = vec![];
    for commit in itertools::chain(&from_roots, &to_roots) {
        trees.push(commit.parent_tree(repo.as_ref()).await?);
    }
    for commit in itertools::chain(&from_heads, &to_heads) {
        trees.push(commit.tree());
    }
    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, &trees)?;

    let diff_renderer = workspace_command.diff_renderer_for(&args.format)?;
    ui.request_pager();
    diff_renderer
        .show_inter_diff(
            ui,
            ui.stdout_formatter().as_mut(),
            &from,
            &to,
            matcher.as_ref(),
            ui.term_width(),
        )
        .await?;
    Ok(())
}
