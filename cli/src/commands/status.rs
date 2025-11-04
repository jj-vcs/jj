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

use itertools::Itertools as _;
use jj_lib::copies::CopyRecords;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::print_conflicted_paths;
use crate::cli_util::print_snapshot_stats;
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
/// [Conflicted bookmarks]:
///     https://jj-vcs.github.io/jj/latest/bookmarks/#conflicts
#[derive(clap::Args, Clone, Debug)]
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
        let tree = wc_commit.tree();

        let wc_has_changes = tree.tree_ids() != parent_tree.tree_ids();
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
                diff_renderer
                    .show_diff(
                        ui,
                        formatter,
                        [&parent_tree, &tree],
                        &matcher,
                        &copy_records,
                        width,
                    )
                    .block_on()?;
            }

            if wc_has_untracked {
                writeln!(formatter, "Untracked paths:")?;
                visit_collapsed_untracked_files(
                    snapshot_stats.untracked_paths.keys(),
                    tree,
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
                .block_on()?;
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

        if wc_commit.has_conflict() {
            // TODO: Conflicts should also be filtered by the `matcher`. See the related
            // TODO on `MergedTree::conflicts()`.
            let conflicts = wc_commit.tree().conflicts().collect_vec();
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
                if parent.has_conflict() {
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

async fn visit_collapsed_untracked_files(
    untracked_paths: impl IntoIterator<Item = impl AsRef<RepoPath>>,
    tree: MergedTree,
    mut on_path: impl FnMut(&RepoPath, bool) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let trees = tree.trees()?;
    let mut stack = vec![trees];

    // TODO: This loop can be improved with BTreeMap cursors once that's stable,
    // would remove the need for the whole `skip_prefixed_by` thing and turn it
    // into a B-tree lookup.
    let mut skip_prefixed_by_dir: Option<RepoPathBuf> = None;
    'untracked: for path in untracked_paths {
        let path = path.as_ref();
        if skip_prefixed_by_dir
            .as_ref()
            .is_some_and(|p| path.starts_with(p))
        {
            continue;
        } else {
            skip_prefixed_by_dir = None;
        }

        let mut it = path.components().dropping_back(1);
        let first_mismatch = it.by_ref().enumerate().find(|(i, component)| {
            stack.get(i + 1).is_none_or(|tree| {
                tree.dir()
                    .components()
                    .next_back()
                    .expect("should always have at least one element (the root)")
                    != *component
            })
        });

        if let Some((i, component)) = first_mismatch {
            stack.truncate(i + 1);
            for component in std::iter::once(component).chain(it) {
                let parent = stack
                    .last()
                    .expect("should always have at least one element (the root)");

                if let Some(subtree) = parent.sub_tree(component).await? {
                    stack.push(subtree);
                } else {
                    let dir = parent.dir().join(component);

                    on_path(&dir, true)?;
                    skip_prefixed_by_dir = Some(dir);

                    continue 'untracked;
                }
            }
        }

        on_path(path, false)?;
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use testutils::TestRepo;
    use testutils::TestTreeBuilder;
    use testutils::repo_path;

    use super::*;

    fn collect_collapsed_untracked_files_string(
        untracked_paths: &[&RepoPath],
        tree: MergedTree,
    ) -> String {
        let mut result = String::new();
        visit_collapsed_untracked_files(untracked_paths, tree, |path, is_dir| {
            result.push_str("? ");
            if is_dir {
                result.push_str(&path.to_internal_dir_string());
            } else {
                result.push_str(path.as_internal_file_string());
            }
            result.push('\n');
            Ok(())
        })
        .block_on()
        .unwrap();
        result
    }

    #[test]
    fn test_collapsed_untracked_files() {
        let repo = TestRepo::init();

        let tracked = {
            let mut builder = TestTreeBuilder::new(repo.repo.store().clone());

            builder.file(repo_path("top_level_file"), "");
            // ? "untracked_top_level_file"
            // ? "dir"
            // ? "dir2/c"
            builder.file(repo_path("dir2/d"), "");
            // ? "dir3/partially_tracked/e"
            builder.file(repo_path("dir3/partially_tracked/f"), "");
            // ? "dir3/fully_untracked/"
            builder.file(repo_path("dir3/j"), "");
            // ? "dir3/k"

            builder.write_merged_tree()
        };
        let untracked = &[
            repo_path("untracked_top_level_file"),
            repo_path("dir/a"),
            repo_path("dir/b"),
            repo_path("dir2/c"),
            repo_path("dir3/partially_tracked/e"),
            repo_path("dir3/fully_untracked/g"),
            repo_path("dir3/fully_untracked/h"),
            repo_path("dir3/k"),
        ];

        insta::assert_snapshot!(
            collect_collapsed_untracked_files_string(untracked, tracked),
            @r"
        ? untracked_top_level_file
        ? dir/
        ? dir2/c
        ? dir3/partially_tracked/e
        ? dir3/fully_untracked/
        ? dir3/k
        "
        );
    }
}
