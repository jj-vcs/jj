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

use bstr::BString;
use bstr::ByteVec as _;
use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts;
use jj_lib::files;
use jj_lib::files::MergeResult;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo_path::RepoPathBuf;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_conflicted_paths;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::complete;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Resolve conflicted files with an external merge tool
///
/// Only conflicts that can be resolved with a 3-way merge are supported. See
/// docs for merge tool configuration instructions. External merge tools will be
/// invoked for each conflicted file one-by-one until all conflicts are
/// resolved. To stop resolving conflicts, exit the merge tool without making
/// any changes.
///
/// Note that conflicts can also be resolved without using this command. You may
/// edit the conflict markers in the conflicted file directly with a text
/// editor.
//  TODOs:
//   - `jj resolve --editor` to resolve a conflict in the default text editor. Should work for
//     conflicts with 3+ adds. Useful to resolve conflicts in a commit other than the current one.
//   - A way to help split commits with conflicts that are too complicated (more than two sides)
//     into commits with simpler conflicts. In case of a tree with many merges, we could for example
//     point to existing commits with simpler conflicts where resolving those conflicts would help
//     simplify the present one.
//   - Maybe we should split this into subcommands instead of using flags to select different
//     behaviors?
#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("action")))]
pub(crate) struct ResolveArgs {
    #[arg(
        long, short,
        default_value = "@",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_mutable_conflicts),
    )]
    revision: RevisionArg,
    /// Instead of resolving conflicts, list all the conflicts
    // TODO: Also have a `--summary` option. `--list` currently acts like
    // `diff --summary`, but should be more verbose.
    #[arg(long, short, group = "action")]
    list: bool,
    /// Specify 3-way merge tool to be used
    #[arg(
        long,
        group = "action",
        value_name = "NAME",
        add = ArgValueCandidates::new(complete::merge_editors),
    )]
    tool: Option<String>,
    /// Interactively select a side of the conflict to keep
    ///
    /// All other conflict sides will be discarded, but resolved files and
    /// resolved sections of conflicted files will not be modified.
    #[arg(long, group = "action")]
    select: bool,
    /// Only resolve conflicts in these paths. You can use the `--list` argument
    /// to find paths to use here.
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::revision_conflicted_files),
    )]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_resolve(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ResolveArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();
    let commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let tree = commit.tree();
    let conflicts = tree.conflicts_matching(&matcher).collect_vec();

    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, [&tree])?;

    if conflicts.is_empty() {
        return Err(cli_error(if args.paths.is_empty() {
            "No conflicts found at this revision"
        } else {
            "No conflicts found at the given path(s)"
        }));
    }
    if args.list {
        return print_conflicted_paths(
            conflicts,
            ui.stdout_formatter().as_mut(),
            &workspace_command,
        );
    };

    let repo_paths = conflicts
        .iter()
        .map(|(path, _)| path.as_ref())
        .collect_vec();
    workspace_command.check_rewritable([commit.id()])?;
    let (new_tree, partial_resolution_error) = if args.select {
        let labels = tree.labels_by_term("");
        // If the conflict has more than 2 sides and it doesn't have labels for every
        // term, it would be confusing to allow the user to select a side, since
        // conflict simplification could cause "side #2" at the root to become "side #1"
        // within a file.
        if labels.num_sides() > 2 && labels.iter().any(|label| label.is_empty()) {
            return Err(cli_error(
                "`jj resolve --select` can only be used for 2-sided conflicts unless every term \
                 has a conflict label",
            ));
        }

        let Some(selected) = prompt_conflict_term(ui, &labels)? else {
            return Ok(());
        };
        drop(labels);
        let new_tree = select_term_from_merged_tree(tree, conflicts, selected)?;
        (new_tree, None)
    } else {
        let merge_editor = workspace_command.merge_editor(ui, args.tool.as_deref())?;
        merge_editor.edit_files(ui, &tree, &repo_paths)?
    };
    let mut tx = workspace_command.start_transaction();
    let new_commit = tx
        .repo_mut()
        .rewrite_commit(&commit)
        .set_tree(new_tree)
        .write()?;
    tx.finish(
        ui,
        format!("Resolve conflicts in commit {}", commit.id().hex()),
    )?;

    // Print conflicts that are still present after resolution if the workspace
    // working copy is not at the commit. Otherwise, the conflicting paths will
    // be printed by the `tx.finish()` instead.
    if workspace_command.get_wc_commit_id() != Some(new_commit.id())
        && let Some(mut formatter) = ui.status_formatter()
        && new_commit.has_conflict()
    {
        let new_tree = new_commit.tree();
        let new_conflicts = new_tree.conflicts().collect_vec();
        writeln!(
            formatter.labeled("warning").with_heading("Warning: "),
            "After this operation, some files at this revision still have conflicts:"
        )?;
        print_conflicted_paths(new_conflicts, formatter.as_mut(), &workspace_command)?;
    }

    if let Some(err) = partial_resolution_error {
        return Err(err.into());
    }
    Ok(())
}

fn prompt_conflict_term(ui: &Ui, labels: &Merge<&str>) -> Result<Option<usize>, CommandError> {
    let num_sides = labels.num_sides();
    let num_bases = num_sides - 1;

    let mut formatter = ui.stderr_formatter().into_labeled("resolve_select");
    write!(formatter, "Select a side (")?;
    write!(formatter.labeled("side"), "1")?;
    write!(formatter, "-")?;
    write!(formatter.labeled("side"), "{num_sides}")?;
    write!(formatter, ") or a base (")?;
    write!(formatter.labeled("base"), "b1")?;
    if num_bases > 1 {
        write!(formatter, "-")?;
        write!(formatter.labeled("base"), "b{num_bases}")?;
    }
    writeln!(formatter, ") to keep:")?;

    // Example: "1", "b1", "2", "b2", "3", and "q" for quit
    let choices = (1..=num_bases)
        .flat_map(|i| [i.to_string(), format!("b{i}")])
        .chain([num_sides.to_string(), "q".to_owned()])
        .collect_vec();

    writeln!(formatter, "\nSides:")?;
    for add_index in 0..num_sides {
        let label = labels.get_add(add_index).copied().unwrap_or_default();
        write!(formatter.labeled("side"), "{:>4}", add_index + 1)?;
        write!(formatter, ": ")?;
        if label.is_empty() {
            writeln!(formatter, "side #{}", add_index + 1)?;
        } else {
            writeln!(formatter, "{label}")?;
        }
    }

    writeln!(formatter, "\nBases:")?;
    for remove_index in 0..num_bases {
        let label = labels.get_remove(remove_index).copied().unwrap_or_default();
        write!(
            formatter.labeled("base"),
            "{:>4}",
            format!("b{}", remove_index + 1)
        )?;
        write!(formatter, ": ")?;
        if label.is_empty() {
            writeln!(formatter, "base")?;
        } else {
            writeln!(formatter, "{label}")?;
        }
    }
    writeln!(formatter)?;

    let selected = ui.prompt_choice(
        "Enter a side or base number (or \"q\" to quit)",
        &choices,
        None,
    )?;
    if selected < labels.as_slice().len() {
        Ok(Some(selected))
    } else {
        Ok(None)
    }
}

fn select_term_from_merged_tree(
    tree: MergedTree,
    conflicts: Vec<(RepoPathBuf, BackendResult<MergedTreeValue>)>,
    selected: usize,
) -> Result<MergedTree, CommandError> {
    let store = tree.store().clone();
    let mut tree_builder = MergedTreeBuilder::new(tree);
    for (path, conflict) in conflicts {
        let conflict = conflict?;
        // If the selected term is absent, delete the path from the tree.
        let Some(selected_term) = &conflict.as_slice()[selected] else {
            tree_builder.set_or_remove(path, Merge::absent());
            continue;
        };

        let (
            TreeValue::File {
                id: _,
                executable,
                copy_id,
            },
            Some(file_ids),
            Some(executable_bits),
        ) = (
            selected_term,
            conflict.to_file_merge(),
            conflict.to_executable_merge(),
        )
        else {
            // If the conflict contains any non-files, return the selected term without
            // trying to merge anything.
            tree_builder.set_or_remove(path, Merge::normal(selected_term.clone()));
            continue;
        };

        // Merge the file contents and executable bits before selecting the term.
        let contents = conflicts::extract_as_single_hunk(&file_ids, &store, &path).block_on()?;
        let executable =
            conflicts::resolve_file_executable(&executable_bits).unwrap_or(*executable);

        // Collect only the selected term of each merged hunk.
        let selected_contents = match files::merge_hunks(&contents, store.merge_options()) {
            MergeResult::Resolved(resolved) => resolved,
            MergeResult::Conflict(hunks) => {
                let mut result = BString::default();
                for hunk in hunks {
                    if let Some(resolved) = hunk.as_resolved() {
                        result.push_str(resolved);
                    } else {
                        result.push_str(&hunk.as_slice()[selected]);
                    }
                }
                result
            }
        };

        let new_file_id = store
            .write_file(&path, &mut &selected_contents[..])
            .block_on()?;

        tree_builder.set_or_remove(
            path,
            Merge::normal(TreeValue::File {
                id: new_file_id,
                executable,
                copy_id: copy_id.clone(),
            }),
        );
    }
    let resolved_tree = tree_builder.write_tree()?;
    Ok(resolved_tree)
}
