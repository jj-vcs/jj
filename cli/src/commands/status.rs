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

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;

use itertools::Itertools as _;
use jj_lib::copies::CopyRecords;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::working_copy::UntrackedReason;
use tracing::instrument;

use crate::cli_util::print_conflicted_paths;
use crate::cli_util::print_snapshot_stats;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::diff_util::get_copy_records;
use crate::diff_util::DiffFormat;
use crate::formatter::Formatter;
use crate::ui::Ui;

/// Show high-level repo status
///
/// This includes:
///
///  * The working copy commit and its parents, and a summary of the changes in
///    the working copy (compared to the merged parents)
///  * Conflicts in the working copy
///  * [Conflicted bookmarks]
///
/// [Conflicted bookmarks]:
///     https://jj-vcs.github.io/jj/latest/bookmarks/#conflicts
#[derive(clap::Args, Clone, Debug)]
#[command(visible_alias = "st")]
pub(crate) struct StatusArgs {
    /// Restrict the status display to these paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_status(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &StatusArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats) = command.workspace_helper_with_stats(ui)?;
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
    let matcher = workspace_command
        .parse_file_patterns(ui, &args.paths)?
        .to_matcher();
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();

    if let Some(wc_commit) = &maybe_wc_commit {
        let parent_tree = wc_commit.parent_tree(repo.as_ref())?;
        let tree = wc_commit.tree()?;

        let wc_has_changes = tree.id() != parent_tree.id();
        let wc_has_untracked = !snapshot_stats.untracked_paths.is_empty();
        if !wc_has_changes && !wc_has_untracked {
            writeln!(formatter, "The working copy has no changes.")?;
        } else {
            if wc_has_changes {
                writeln!(formatter, "Working copy changes:")?;
                let mut copy_records = CopyRecords::default();
                for parent in wc_commit.parent_ids() {
                    let records = get_copy_records(repo.store(), parent, wc_commit.id(), &matcher)?;
                    copy_records.add_records(records)?;
                }
                let diff_renderer = workspace_command.diff_renderer(vec![DiffFormat::Summary]);
                let width = ui.term_width();
                diff_renderer.show_diff(
                    ui,
                    formatter,
                    &parent_tree,
                    &tree,
                    &matcher,
                    &copy_records,
                    width,
                )?;
            }

            if wc_has_untracked {
                writeln!(formatter, "Untracked paths:")?;
                print_collapsed_untracked_files(
                    formatter,
                    &snapshot_stats.untracked_paths,
                    &tree,
                    workspace_command.path_converter(),
                )?;
            }
        }

        let template = workspace_command.commit_summary_template();
        write!(formatter, "Working copy  (@) : ")?;
        template.format(wc_commit, formatter)?;
        writeln!(formatter)?;
        for parent in wc_commit.parents() {
            let parent = parent?;
            //                "Working copy  (@) : "
            write!(formatter, "Parent commit (@-): ")?;
            template.format(&parent, formatter)?;
            writeln!(formatter)?;
        }

        if wc_commit.has_conflict()? {
            // TODO: Conflicts should also be filtered by the `matcher`. See the related
            // TODO on `MergedTree::conflicts()`.
            let conflicts = wc_commit.tree()?.conflicts().collect_vec();
            writeln!(
                formatter.labeled("warning").with_heading("Warning: "),
                "There are unresolved conflicts at these paths:"
            )?;
            print_conflicted_paths(conflicts, formatter, &workspace_command)?;

            let wc_revset = RevsetExpression::commit(wc_commit.id().clone());

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
                .try_collect()?;

            workspace_command.report_repo_conflicts(formatter, repo, ancestors_conflicts)?;
        } else {
            for parent in wc_commit.parents() {
                let parent = parent?;
                if parent.has_conflict()? {
                    writeln!(
                        formatter.labeled("hint").with_heading("Hint: "),
                        "Conflict in parent commit has been resolved in working copy"
                    )?;
                    break;
                }
            }
        }
    } else {
        writeln!(formatter, "No working copy")?;
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
            "Use `jj bookmark list` to see details. Use `jj git fetch` to resolve."
        )?;
    }

    Ok(())
}

fn print_collapsed_untracked_files(
    formatter: &mut dyn Formatter,
    untracked_paths: &BTreeMap<RepoPathBuf, UntrackedReason>,
    tree: &MergedTree,
    path_converter: &RepoPathUiConverter,
) -> io::Result<()> {
    formatter.with_label("diff", |formatter| {
        let tracked = tree
            .entries()
            .map(|entry| entry.0.into_internal_string())
            .collect::<BTreeSet<String>>();

        // TODO: This loop can be improved with BTreeSet cursors once that's stable,
        // would remove the need for the whole `skip_prefixed_by` thing and turn it
        // into a BTree lookup.
        let mut skip_prefixed_by_dir = None;
        for original_path in untracked_paths.keys() {
            if skip_prefixed_by_dir
                .as_ref()
                .is_some_and(|p| original_path.as_internal_file_string().starts_with(p))
            {
                continue;
            }

            let mut path = original_path.as_ref();
            let mut path_is_dir = false;
            while let Some(parent_dir) = path.parent().filter(|p| !p.is_root()) {
                let internal_dir_string = parent_dir.to_internal_dir_string();
                if tracked
                    .range(internal_dir_string.clone()..)
                    .next()
                    .is_none_or(|p| !p.starts_with(&internal_dir_string))
                {
                    path = parent_dir;
                    path_is_dir = true;
                    skip_prefixed_by_dir = Some(internal_dir_string);
                } else {
                    break;
                }
            }

            let ui_path = path_converter.format_file_path(path);
            writeln!(
                formatter.labeled("untracked"),
                "? {ui_path}{}",
                if path_is_dir {
                    std::path::MAIN_SEPARATOR_STR
                } else {
                    ""
                }
            )?;
        }

        io::Result::Ok(())
    })
}
