use std::collections::HashSet;
use std::io;

use bstr::BString;
use bstr::ByteVec as _;
use indoc::indoc;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts;
use jj_lib::files;
use jj_lib::files::MergeResult;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::repo_path::RepoPath;
use pollster::FutureExt as _;
use thiserror::Error;

use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

#[derive(Debug, Error)]
pub enum SelectToolError {
    #[error(
        "The ':select' merge tool can only be used for 2-sided conflicts unless every term has a \
         conflict label"
    )]
    ManySidesWithNoLabel,
    #[error("The selected side is not present in any matching conflicted files")]
    SelectedTermNotPresent,
    #[error("Conflict resolution canceled")]
    PromptQuit,
    #[error(transparent)]
    BackendError(#[from] jj_lib::backend::BackendError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn select_term(
    ui: &Ui,
    tree: &MergedTree,
    repo_paths: &[&RepoPath],
) -> Result<MergedTree, SelectToolError> {
    let labels = tree.labels_by_term("");
    // If the conflict has more than 2 sides and it doesn't have labels for every
    // term, it would be confusing to allow the user to select a side, since
    // conflict simplification could cause "side #2" at the root to become "side #1"
    // within a file.
    if labels.num_sides() > 2 && labels.iter().any(|label| label.is_empty()) {
        return Err(SelectToolError::ManySidesWithNoLabel);
    }

    let conflicts = find_conflicts_for_select_term(tree, repo_paths)?;
    // Some terms from the conflict may not appear in any files. This can happen in
    // the user has chosen to only resolve a subset of conflicted files, or if the
    // same content is present in two conflict terms. To prevent confusion, we add a
    // comment next to terms that aren't present in any tree.
    let terms_present_in_conflicts: HashSet<usize> = conflicts
        .iter()
        .flat_map(|conflict| conflict.simplified_mapping.iter().copied())
        .collect();

    let Some(selected) = prompt_conflict_term(ui, &labels, &terms_present_in_conflicts)? else {
        return Err(SelectToolError::PromptQuit);
    };

    let (new_tree, stats) = select_term_from_merged_tree(tree, &conflicts, selected)?;
    if stats.resolved == 0 {
        return Err(SelectToolError::SelectedTermNotPresent);
    }
    if stats.skipped > 0 {
        writeln!(
            ui.warning_default(),
            "Skipped resolving conflicts in {} files where the selected side was not present.",
            stats.skipped
        )?;
        writeln!(
            ui.hint_default(),
            "Try selecting a different side for the remaining files."
        )?;
    }
    Ok(new_tree)
}

#[derive(Debug)]
struct SelectTermConflict<'a> {
    path: &'a RepoPath,
    unsimplified: MergedTreeValue,
    simplified_mapping: Vec<usize>,
}

fn find_conflicts_for_select_term<'a>(
    tree: &MergedTree,
    repo_paths: &[&'a RepoPath],
) -> BackendResult<Vec<SelectTermConflict<'a>>> {
    repo_paths
        .iter()
        .map(|&path| {
            let unsimplified = tree.path_value(path)?;
            let simplified_mapping = unsimplified.get_simplified_mapping();
            Ok(SelectTermConflict {
                path,
                unsimplified,
                simplified_mapping,
            })
        })
        .try_collect()
}

fn prompt_conflict_term(
    ui: &Ui,
    labels: &Merge<&str>,
    terms_present_in_conflicts: &HashSet<usize>,
) -> io::Result<Option<usize>> {
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

    let mut has_missing_terms = false;

    writeln!(formatter, "\nSides:")?;
    for add_index in 0..num_sides {
        let is_missing = !terms_present_in_conflicts.contains(&(add_index * 2));
        if is_missing {
            formatter.push_label("missing");
        }

        let label = labels.get_add(add_index).copied().unwrap_or_default();
        write!(formatter.labeled("side"), "{:>4}", add_index + 1)?;
        write!(formatter, ": ")?;
        if label.is_empty() {
            write!(formatter, "side #{}", add_index + 1)?;
        } else {
            write!(formatter, "{label}")?;
        }
        if is_missing {
            has_missing_terms = true;
            write!(formatter, " [not present in files]")?;
            formatter.pop_label();
        }
        writeln!(formatter)?;
    }

    writeln!(formatter, "\nBases:")?;
    for remove_index in 0..num_bases {
        let is_missing = !terms_present_in_conflicts.contains(&(remove_index * 2 + 1));
        if is_missing {
            formatter.push_label("missing");
        }

        let label = labels.get_remove(remove_index).copied().unwrap_or_default();
        write!(
            formatter.labeled("base"),
            "{:>4}",
            format!("b{}", remove_index + 1)
        )?;
        write!(formatter, ": ")?;
        if label.is_empty() {
            write!(formatter, "base")?;
        } else {
            write!(formatter, "{label}")?;
        }
        if is_missing {
            has_missing_terms = true;
            write!(formatter, " [not present in files]")?;
            formatter.pop_label();
        }
        writeln!(formatter)?;
    }
    writeln!(formatter)?;

    if has_missing_terms {
        writeln!(
            ui.hint_default(),
            indoc! {"
                Some terms of this conflict are not present in any of the conflicted
                files. This happens because `jj` simplifies conflicts before materializing them
                in files, so any unnecessary terms are omitted.
            "}
        )?;
    }

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

#[derive(Debug, Copy, Clone, Default)]
struct SelectTermStats {
    skipped: usize,
    resolved: usize,
}

fn select_term_from_merged_tree(
    tree: &MergedTree,
    conflicts: &[SelectTermConflict],
    selected_index: usize,
) -> BackendResult<(MergedTree, SelectTermStats)> {
    let store = tree.store().clone();
    let mut stats = SelectTermStats::default();
    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    for conflict in conflicts {
        // We need to simplify conflicts before resolving to ensure that we keep any
        // resolved hunks for file merges.
        let simplified_conflict = conflict
            .unsimplified
            .apply_simplified_mapping(&conflict.simplified_mapping);
        // If the selected term isn't present in this conflict after simplification,
        // we skip resolving this conflict. This prevents unintuitive resolutions where
        // the selected term's content wasn't present in the materialized file.
        let Some(selected_index) = conflict
            .simplified_mapping
            .iter()
            .position(|&i| i == selected_index)
        else {
            stats.skipped += 1;
            continue;
        };

        // If the selected term is absent, delete the path from the tree.
        let Some(selected_term) = &simplified_conflict.as_slice()[selected_index] else {
            tree_builder.set_or_remove(conflict.path.to_owned(), Merge::absent());
            stats.resolved += 1;
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
            simplified_conflict.to_file_merge(),
            simplified_conflict.to_executable_merge(),
        )
        else {
            // If the conflict contains any non-files, return the selected term without
            // trying to merge anything.
            tree_builder.set_or_remove(
                conflict.path.to_owned(),
                Merge::normal(selected_term.clone()),
            );
            stats.resolved += 1;
            continue;
        };

        // Merge the file contents and executable bits before selecting the term.
        let contents =
            conflicts::extract_as_single_hunk(&file_ids, &store, conflict.path).block_on()?;
        let executable =
            conflicts::resolve_file_executable(&executable_bits).unwrap_or(*executable);

        // Collect only the selected term of each merged hunk. We still need to perform
        // the merge even though we're selecting a single side because we want to keep
        // any resolved hunks.
        let selected_contents = match files::merge_hunks(&contents, store.merge_options()) {
            MergeResult::Resolved(resolved) => resolved,
            MergeResult::Conflict(hunks) => {
                let mut result = BString::default();
                for hunk in hunks {
                    if let Some(resolved) = hunk.as_resolved() {
                        result.push_str(resolved);
                    } else {
                        result.push_str(&hunk.as_slice()[selected_index]);
                    }
                }
                result
            }
        };

        let new_file_id = store
            .write_file(conflict.path, &mut &selected_contents[..])
            .block_on()?;

        tree_builder.set_or_remove(
            conflict.path.to_owned(),
            Merge::normal(TreeValue::File {
                id: new_file_id,
                executable,
                copy_id: copy_id.clone(),
            }),
        );
        stats.resolved += 1;
    }
    let resolved_tree = tree_builder.write_tree().block_on()?;
    Ok((resolved_tree, stats))
}
