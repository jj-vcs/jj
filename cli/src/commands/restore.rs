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

use std::io::Write as _;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use indoc::formatdoc;
use itertools::Itertools as _;
use jj_lib::merge::Diff;
use jj_lib::object_id::ObjectId as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Restore paths from another revision
///
/// That means that the paths get the same content in the destination (`--into`)
/// as they had in the source (`--from`). This is typically used for undoing
/// changes to some paths in the working copy (`jj restore <paths>`).
///
/// If only one of `--from` or `--into` is specified, the other one defaults to
/// the working copy.
///
/// When neither `--from` nor `--into` is specified, the command restores into
/// the working copy from its parent(s). `jj restore` without arguments is
/// similar to `jj abandon`, except that it leaves an empty revision with its
/// description and other metadata preserved.
///
/// See `jj diffedit` if you'd like to restore portions of files rather than
/// entire files.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct RestoreArgs {
    /// Restore only these paths (instead of all paths)
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::modified_changes_in_or_range_files))]
    paths: Vec<String>,

    /// Revision to restore from (source)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Option<RevisionArg>,

    /// Revision to restore into (destination)
    #[arg(long, short = 't', visible_alias = "to", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    into: Option<RevisionArg>,

    /// Undo the changes in a revision as compared to the merge of its parents.
    ///
    /// This undoes the changes that can be seen with `jj diff -r REVSET`. If
    /// `REVSET` only has a single parent, this option is equivalent to `jj
    ///  restore --into REVSET --from REVSET-`.
    ///
    /// The default behavior of `jj restore` is equivalent to `jj restore
    /// --changes-in @`.
    #[arg(long, short, value_name = "REVSET", conflicts_with_all = ["into", "from"])]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    changes_in: Option<RevisionArg>,

    /// Prints an error. DO NOT USE.
    ///
    /// If we followed the pattern of `jj diff` and `jj diffedit`, we would use
    /// `--revision` instead of `--changes-in` However, that would make it
    /// likely that someone unfamiliar with this pattern would use `-r` when
    /// they wanted `--from`. This would make a different revision empty, and
    /// the user might not even realize something went wrong.
    #[arg(long, short, hide = true)]
    revision: Option<RevisionArg>,

    /// Interactively choose which parts to restore
    #[arg(long, short)]
    interactive: bool,

    /// Specify diff editor to be used (implies --interactive)
    #[arg(long, value_name = "NAME")]
    #[arg(add = ArgValueCandidates::new(complete::diff_editors))]
    tool: Option<String>,

    /// Preserve the content (not the diff) when rebasing descendants
    #[arg(long)]
    restore_descendants: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_restore(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RestoreArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let (from_commits, from_tree, to_commit);
    if args.revision.is_some() {
        return Err(
            user_error("`jj restore` does not have a `--revision`/`-r` option.")
                .hinted("To modify the current revision, use `--from`.")
                .hinted(
                    "To undo changes in a revision compared to its parents, use `--changes-in`.",
                ),
        );
    }
    if args.from.is_some() || args.into.is_some() {
        to_commit = workspace_command
            .resolve_single_rev(ui, args.into.as_ref().unwrap_or(&RevisionArg::AT))?;
        let from_commit = workspace_command
            .resolve_single_rev(ui, args.from.as_ref().unwrap_or(&RevisionArg::AT))?;
        from_tree = from_commit.tree();
        from_commits = vec![from_commit];
    } else {
        to_commit = workspace_command
            .resolve_single_rev(ui, args.changes_in.as_ref().unwrap_or(&RevisionArg::AT))?;
        from_tree = to_commit.parent_tree(workspace_command.repo().as_ref())?;
        from_commits = to_commit.parents().try_collect()?;
    }
    workspace_command.check_rewritable([to_commit.id()])?;

    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();
    let diff_selector =
        workspace_command.diff_selector(ui, args.tool.as_deref(), args.interactive)?;
    let to_tree = to_commit.tree();
    let format_instructions = || {
        formatdoc! {"
            You are restoring changes from: {from_commits}
            to commit: {to_commit}

            The diff initially shows all changes restored. Adjust the right side until it
            shows the contents you want for the destination commit.
            ",
            from_commits = from_commits
                .iter()
                .map(|commit| workspace_command.format_commit_summary(commit))
                //      "You are restoring changes from: "
                .join("\n                                "),
            to_commit = workspace_command.format_commit_summary(&to_commit),
        }
    };
    let new_tree = diff_selector.select(
        Diff::new(&to_tree, &from_tree),
        &matcher,
        format_instructions,
    )?;

    print_unmatched_explicit_paths(
        ui,
        &workspace_command,
        &fileset_expression,
        [&to_tree, &from_tree],
    )?;

    if new_tree.tree_ids() == to_commit.tree_ids() {
        writeln!(ui.status(), "Nothing changed.")?;
    } else {
        let mut tx = workspace_command.start_transaction();
        tx.repo_mut()
            .rewrite_commit(&to_commit)
            .set_tree(new_tree)
            .write()?;
        // rebase_descendants early; otherwise the new commit would always have
        // a conflicted change id at this point.
        let (num_rebased, extra_msg) = if args.restore_descendants {
            (
                tx.repo_mut().reparent_descendants()?,
                " (while preserving their content)",
            )
        } else {
            (tx.repo_mut().rebase_descendants()?, "")
        };
        if let Some(mut formatter) = ui.status_formatter()
            && num_rebased > 0
        {
            writeln!(
                formatter,
                "Rebased {num_rebased} descendant commits{extra_msg}"
            )?;
        }
        tx.finish(ui, format!("restore into commit {}", to_commit.id().hex()))?;
    }
    Ok(())
}
