use std::io;

use bstr::BString;
use bstr::ByteVec as _;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts;
use jj_lib::files;
use jj_lib::files::MergeResult;
use jj_lib::merge::Merge;
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

    let Some(selected) = prompt_conflict_term(ui, &labels)? else {
        return Err(SelectToolError::PromptQuit);
    };

    let (new_tree, stats) = select_term_from_merged_tree(tree, repo_paths, selected)?;
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

fn prompt_conflict_term(ui: &Ui, labels: &Merge<&str>) -> io::Result<Option<usize>> {
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

#[derive(Debug, Copy, Clone, Default)]
struct SelectTermStats {
    skipped: usize,
}

fn select_term_from_merged_tree(
    tree: &MergedTree,
    repo_paths: &[&RepoPath],
    selected: usize,
) -> BackendResult<(MergedTree, SelectTermStats)> {
    let store = tree.store().clone();
    let mut stats = SelectTermStats::default();
    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    for &path in repo_paths {
        let unsimplified_conflict = tree.path_value(path)?;
        let simplified_mapping = unsimplified_conflict.get_simplified_mapping();
        let conflict = unsimplified_conflict.simplify();
        // If the selected term isn't present in this conflict after simplification,
        // leave the path conflicted. This prevents unintuitive resolutions where the
        // selected term's content wasn't present in the materialized file, and it
        // allows us to accept resolved hunks from the simplified file content when
        // merging file conflicts.
        let Some(selected) = simplified_mapping.iter().position(|&i| i == selected) else {
            stats.skipped += 1;
            continue;
        };

        // If the selected term is absent, delete the path from the tree.
        let Some(selected_term) = &conflict.as_slice()[selected] else {
            tree_builder.set_or_remove(path.to_owned(), Merge::absent());
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
            tree_builder.set_or_remove(path.to_owned(), Merge::normal(selected_term.clone()));
            continue;
        };

        // Merge the file contents and executable bits before selecting the term.
        let contents = conflicts::extract_as_single_hunk(&file_ids, &store, path).block_on()?;
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
            .write_file(path, &mut &selected_contents[..])
            .block_on()?;

        tree_builder.set_or_remove(
            path.to_owned(),
            Merge::normal(TreeValue::File {
                id: new_file_id,
                executable,
                copy_id: copy_id.clone(),
            }),
        );
    }
    let resolved_tree = tree_builder.write_tree()?;
    Ok((resolved_tree, stats))
}
