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

use std::collections::BTreeMap;
use std::io::Write as _;

use clap_complete::ArgValueCompleter;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::commit::Commit;
use jj_lib::matchers::PrefixMatcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::complete;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Move or rename a file or directory in revisions and their descendants
///
/// `ORIG_NAME` is moved to `NEW_NAME` in each of the given revisions and all
/// their descendants. Apart from the move, the content of each revision is
/// preserved: revisions are reparented onto their rewritten parents, not
/// rebased, so the move never introduces conflicts.
///
/// If `NEW_NAME` already exists in a revision, or one of its parent paths is
/// not a directory, the existing entry is overwritten and a warning is
/// printed. If `ORIG_NAME` doesn't exist in a revision, the revision's content
/// is left unchanged and a warning is printed.
///
/// Example usage:
///
/// ```shell
/// jj file move -s xyz src/old_name.rs src/new_name.rs
/// ```
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileMoveArgs {
    /// Move the path in the specified revision(s) and all their descendants
    #[arg(long, short, default_value = "@", value_name = "REVSETS")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    source: Vec<RevisionArg>,

    /// The file or directory to move
    #[arg(value_name = "ORIG_NAME", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    orig_name: String,

    /// The new path of the file or directory
    #[arg(value_name = "NEW_NAME", value_hint = clap::ValueHint::AnyPath)]
    new_name: String,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_move(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileMoveArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let orig_path = workspace_command.parse_file_path(&args.orig_name)?;
    let new_path = workspace_command.parse_file_path(&args.new_name)?;
    let orig_ui_path = workspace_command.format_file_path(&orig_path);
    let new_ui_path = workspace_command.format_file_path(&new_path);

    let target_expr = workspace_command
        .parse_union_revsets(ui, &args.source)?
        .resolve()?
        .descendants();
    workspace_command
        .check_rewritable_expr(&target_expr)
        .await?;
    if orig_path == new_path {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let commit_ids: Vec<CommitId> = target_expr
        .evaluate(workspace_command.repo().as_ref())?
        .stream()
        .try_collect()
        .await?;

    let mut tx = workspace_command.start_transaction();
    let mut num_moved = 0;
    // Rewritten commits by the existing path that was overwritten in them.
    let mut overwritten_commits: BTreeMap<RepoPathBuf, Vec<Commit>> = BTreeMap::new();
    // Commits (rewritten if their parents changed) without `orig_path`.
    let mut missing_commits = Vec::new();
    // Every commit in `commit_ids` is visited since the set is closed under
    // descendants.
    tx.repo_mut()
        .transform_descendants(commit_ids, async |rewriter| {
            let old_tree = rewriter.old_commit().tree();
            match move_path(&old_tree, &orig_path, &new_path).await? {
                Some((new_tree, overwritten_path)) => {
                    let new_commit = rewriter.reparent().set_tree(new_tree).write().await?;
                    num_moved += 1;
                    if let Some(path) = overwritten_path {
                        overwritten_commits
                            .entry(path)
                            .or_default()
                            .push(new_commit);
                    }
                }
                None if rewriter.parents_changed() => {
                    missing_commits.push(rewriter.reparent().write().await?);
                }
                None => {
                    missing_commits.push(rewriter.old_commit().clone());
                }
            }
            Ok(())
        })
        .await?;

    if !missing_commits.is_empty() {
        let mut formatter = ui.stderr_formatter();
        writeln!(
            formatter.labeled("warning").with_heading("Warning: "),
            "'{orig_ui_path}' does not exist in these commits:"
        )?;
        print_updated_commits(
            formatter.as_mut(),
            &tx.commit_summary_template(),
            &missing_commits,
        )?;
    }
    for (path, commits) in &overwritten_commits {
        let ui_path = tx.base_workspace_helper().format_file_path(path);
        let mut formatter = ui.stderr_formatter();
        writeln!(
            formatter.labeled("warning").with_heading("Warning: "),
            "Overwrote existing '{ui_path}' in these commits:"
        )?;
        print_updated_commits(formatter.as_mut(), &tx.commit_summary_template(), commits)?;
    }
    if !overwritten_commits.is_empty() {
        writeln!(
            ui.hint_default(),
            "If this was not intentional, run `jj undo` to restore the previous state."
        )?;
    }
    if num_moved > 0 {
        writeln!(
            ui.status(),
            "Moved '{orig_ui_path}' to '{new_ui_path}' in {num_moved} commits."
        )?;
    }
    tx.finish(
        ui,
        format!(
            "move {} to {} in {num_moved} commits",
            orig_path.as_internal_file_string(),
            new_path.as_internal_file_string()
        ),
    )
    .await?;
    Ok(())
}

/// Moves all entries at or below `orig_path` to `new_path`, replacing anything
/// else at or below `new_path`.
///
/// Returns `None` if `orig_path` doesn't exist in `tree`. Otherwise, returns
/// the new tree and the existing path that was overwritten, if any.
async fn move_path(
    tree: &MergedTree,
    orig_path: &RepoPath,
    new_path: &RepoPath,
) -> BackendResult<Option<(MergedTree, Option<RepoPathBuf>)>> {
    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    // All entries are removed before any is added at its new path, since a new
    // path may coincide with an old one when one of `orig_path` and `new_path`
    // contains the other (e.g. `dir/a` -> `dir/sub/a` and `dir/sub/a` ->
    // `dir/sub/sub/a` when moving `dir` to `dir/sub`).
    let moved_entries: Vec<_> = tree
        .entries_matching(&PrefixMatcher::new([orig_path]))
        .map(|(path, value)| -> BackendResult<_> {
            let value = value?;
            let mut dest_path = new_path.to_owned();
            dest_path.extend(
                path.strip_prefix(orig_path)
                    .expect("entry should be below the prefix it matched")
                    .components(),
            );
            tree_builder.set_or_remove(path, Merge::absent());
            Ok((dest_path, value))
        })
        .try_collect()?;
    if moved_entries.is_empty() {
        return Ok(None);
    }

    // Entries below `orig_path` are being moved, not overwritten.
    let mut overwritten_path = None;
    for (path, _value) in tree.entries_matching(&PrefixMatcher::new([new_path])) {
        if !path.starts_with(orig_path) {
            overwritten_path = Some(new_path);
        }
        tree_builder.set_or_remove(path, Merge::absent());
    }
    // A parent path that isn't a directory (e.g. `foo` when moving to
    // `foo/bar`) is replaced by a directory.
    for ancestor in new_path.ancestors().skip(1) {
        if ancestor.is_root() {
            break;
        }
        if ancestor.starts_with(orig_path) {
            continue;
        }
        let value = tree.path_value(ancestor).await?;
        if !value.is_absent() && !value.is_tree() {
            overwritten_path = Some(ancestor);
        }
    }
    for (dest_path, value) in moved_entries {
        tree_builder.set_or_remove(dest_path, value);
    }
    let new_tree = tree_builder.write_tree().await?;
    Ok(Some((new_tree, overwritten_path.map(RepoPath::to_owned))))
}
