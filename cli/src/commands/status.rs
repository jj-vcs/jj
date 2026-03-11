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

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::copies::CopyRecords;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo;
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::working_copy::SnapshotStats;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::print_conflicted_paths;
use crate::cli_util::print_snapshot_stats;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::cli_util::visit_collapsed_untracked_files;
use crate::command_error::CommandError;
use crate::diff_util::DiffFormat;
use crate::diff_util::get_copy_records;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Show high-level repo status [default alias: st]
///
/// This includes:
///
/// * The working copy commit and its parents, and a summary of the changes in
///   the working copy (compared to the merged parents)
///
/// * Conflicts in the working copy
///
/// * [Conflicted bookmarks]
///
/// Note: You can use `jj diff --summary -r <rev>` to see the changed files for
/// a specific revision.
///
/// [Conflicted bookmarks]:
///     https://docs.jj-vcs.dev/latest/bookmarks/#conflicts
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct StatusArgs {
    /// Restrict the status display to these paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_status(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &StatusArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats) = command.workspace_helper_with_stats(ui).await?;
    print_snapshot_stats(
        ui,
        &snapshot_stats,
        workspace_command.env().path_converter(),
    )?;
    let repo = workspace_command.repo();
    let maybe_wc_commit = workspace_command
        .get_wc_commit_id()
        .map(|id| repo.store().get_commit(id))
        .transpose()?;
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();

    if let Some(wc_commit) = &maybe_wc_commit {
        let status = collect_working_copy_status(repo.as_ref(), wc_commit, snapshot_stats).await?;
        print_unmatched_explicit_paths(
            ui,
            &workspace_command,
            &fileset_expression,
            [&status.tree],
        )?;

        if !status.has_any_tracked_changes() && !status.has_any_untracked_paths() {
            writeln!(formatter, "The working copy has no changes.")?;
        } else {
            if status.has_any_tracked_changes() {
                let mut copy_records = CopyRecords::default();
                for parent in &status.parents {
                    let records =
                        get_copy_records(repo.store(), parent.id(), status.commit.id(), &matcher)
                            .await?;
                    copy_records.add_records(records);
                }
                let diff_renderer = workspace_command.diff_renderer(vec![DiffFormat::Summary]);
                let width = ui.term_width();
                let mut diff_output = vec![];
                diff_renderer
                    .show_diff(
                        ui,
                        ui.new_formatter(&mut diff_output).as_mut(),
                        Diff::new(&status.parent_tree, &status.tree),
                        &matcher,
                        &copy_records,
                        width,
                    )
                    .await?;
                if !diff_output.is_empty() {
                    writeln!(formatter, "Working copy changes:")?;
                    formatter.raw()?.write_all(&diff_output)?;
                }
            }

            let mut matching_untracked_paths = status.untracked_paths_matching(&matcher).peekable();
            if matching_untracked_paths.peek().is_some() {
                writeln!(formatter, "Untracked paths:")?;
                visit_collapsed_untracked_files(
                    matching_untracked_paths,
                    &status.tree,
                    |path, is_dir| {
                        let ui_path = workspace_command.path_converter().format_file_path(path);
                        writeln!(
                            formatter.labeled("diff").labeled("untracked"),
                            "? {ui_path}{}",
                            if is_dir {
                                std::path::MAIN_SEPARATOR_STR
                            } else {
                                ""
                            }
                        )?;
                        Ok(())
                    },
                )
                .await?;
            }
        }

        let template = workspace_command.commit_summary_template();
        write!(formatter, "Working copy  (@) : ")?;
        template.format(&status.commit, formatter)?;
        writeln!(formatter)?;
        for parent in &status.parents {
            //                "Working copy  (@) : "
            write!(formatter, "Parent commit (@-): ")?;
            template.format(parent, formatter)?;
            writeln!(formatter)?;
        }

        if status.commit.has_conflict() {
            let conflicts = status.tree.conflicts_matching(&matcher).collect_vec();
            writeln!(
                formatter.labeled("warning").with_heading("Warning: "),
                "There are unresolved conflicts at these paths:"
            )?;
            print_conflicted_paths(conflicts, formatter, &workspace_command)?;

            let wc_revset = RevsetExpression::commit(status.commit.id().clone());

            // Ancestors with conflicts, excluding the current working copy commit.
            let ancestors_conflicts: Vec<_> = workspace_command
                .attach_revset_evaluator(
                    wc_revset
                        .parents()
                        .ancestors()
                        .filtered(RevsetFilterPredicate::HasConflict)
                        .minus(&workspace_command.env().immutable_expression()),
                )
                .evaluate_to_commit_ids()?
                .try_collect()
                .await?;

            workspace_command
                .report_repo_conflicts(formatter, repo, ancestors_conflicts)
                .await?;
        } else {
            for parent in &status.parents {
                if parent.has_conflict() {
                    writeln!(
                        formatter.labeled("hint").with_heading("Hint: "),
                        "Conflict in parent commit has been resolved in working copy."
                    )?;
                    break;
                }
            }
        }
    } else {
        writeln!(formatter, "No working copy.")?;
    }

    let conflicted_local_bookmarks = repo
        .view()
        .local_bookmarks()
        .filter(|(_, target)| target.has_conflict())
        .map(|(bookmark_name, _)| bookmark_name)
        .collect_vec();
    let conflicted_remote_bookmarks = repo
        .view()
        .all_remote_bookmarks()
        .filter(|(_, remote_ref)| remote_ref.target.has_conflict())
        .map(|(symbol, _)| symbol)
        .collect_vec();
    if !conflicted_local_bookmarks.is_empty() {
        writeln!(
            formatter.labeled("warning").with_heading("Warning: "),
            "These bookmarks have conflicts:"
        )?;
        for name in conflicted_local_bookmarks {
            write!(formatter, "  ")?;
            write!(formatter.labeled("bookmark"), "{}", name.as_symbol())?;
            writeln!(formatter)?;
        }
        writeln!(
            formatter.labeled("hint").with_heading("Hint: "),
            "Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to \
             resolve."
        )?;
    }
    if !conflicted_remote_bookmarks.is_empty() {
        writeln!(
            formatter.labeled("warning").with_heading("Warning: "),
            "These remote bookmarks have conflicts:"
        )?;
        for symbol in conflicted_remote_bookmarks {
            write!(formatter, "  ")?;
            write!(formatter.labeled("bookmark"), "{symbol}")?;
            writeln!(formatter)?;
        }
        writeln!(
            formatter.labeled("hint").with_heading("Hint: "),
            "Use `jj bookmark list` to see details. Resolve by fetching an updated bookmark from \
             the remote."
        )?;
    }

    Ok(())
}

struct WorkingCopyStatus {
    commit: Commit,
    parents: Vec<Commit>,
    parent_tree: MergedTree,
    tree: MergedTree,
    snapshot_stats: SnapshotStats,
}

impl WorkingCopyStatus {
    fn has_any_tracked_changes(&self) -> bool {
        self.tree.tree_ids() != self.parent_tree.tree_ids()
    }

    fn has_any_untracked_paths(&self) -> bool {
        !self.snapshot_stats.untracked_paths.is_empty()
    }

    fn untracked_paths_matching(&self, matcher: &dyn Matcher) -> impl Iterator<Item = &RepoPath> {
        self.snapshot_stats
            .untracked_paths
            .keys()
            .filter(|path| matcher.matches(path))
            .map(|path| path.as_ref())
    }
}

async fn collect_working_copy_status(
    repo: &dyn Repo,
    commit: &Commit,
    snapshot_stats: SnapshotStats,
) -> Result<WorkingCopyStatus, CommandError> {
    let commit = commit.clone();
    let parents = commit.parents().await?;
    let parent_tree = commit.parent_tree(repo).await?;
    let tree = commit.tree();

    Ok(WorkingCopyStatus {
        commit,
        parents,
        parent_tree,
        tree,
        snapshot_stats,
    })
}
