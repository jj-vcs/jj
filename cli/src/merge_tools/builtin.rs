use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CopyId;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::MIN_CONFLICT_MARKER_LEN;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::conflicts::materialized_diff_stream;
use jj_lib::copies::CopiesTreeDiffEntry;
use jj_lib::copies::CopyRecords;
use jj_lib::diff::ContentDiff;
use jj_lib::diff::DiffHunkKind;
use jj_lib::files;
use jj_lib::files::MergeResult;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::store::Store;
use jj_lib::tree_merge::MergeOptions;
use pollster::FutureExt as _;
use thiserror::Error;

use super::MergeToolFile;

#[derive(Debug, Error)]
pub enum BuiltinToolError {
    #[error("Failed to record changes")]
    Record(#[from] scm_record::RecordError),
    #[error("Failed to decode UTF-8 text for item {item} (this should not happen)")]
    DecodeUtf8 {
        source: std::str::Utf8Error,
        item: &'static str,
    },
    #[error("Rendering {item} {id} is unimplemented for the builtin difftool/mergetool")]
    Unimplemented { item: &'static str, id: String },
    #[error("Backend error")]
    BackendError(#[from] jj_lib::backend::BackendError),
}

#[derive(Clone, Debug)]
enum FileContents {
    Absent,
    Text {
        contents: String,
        hash: Option<String>,
        num_bytes: u64,
    },
    Binary {
        hash: Option<String>,
        num_bytes: u64,
    },
}

impl FileContents {
    fn describe(&self) -> Option<String> {
        match self {
            Self::Absent => None,
            Self::Text {
                contents: _,
                hash,
                num_bytes,
            }
            | Self::Binary { hash, num_bytes } => match hash {
                Some(hash) => Some(format!("{hash} ({num_bytes}B)")),
                None => Some(format!("({num_bytes}B)")),
            },
        }
    }
}

/// Information about a file that was read from disk. Note that the file may not
/// have existed, in which case its contents will be marked as absent.
#[derive(Clone, Debug)]
pub struct FileInfo {
    file_mode: scm_record::FileMode,
    contents: FileContents,
}

/// File modes according to the Git file mode conventions. used for display
/// purposes and equality comparison.
///
/// TODO: let `scm-record` accept strings instead of numbers for file modes? Or
/// figure out some other way to represent file mode changes in a jj-compatible
/// manner?
mod mode {
    pub const ABSENT: scm_record::FileMode = scm_record::FileMode::Absent;
    pub const NORMAL: scm_record::FileMode = scm_record::FileMode::Unix(0o100644);
    pub const EXECUTABLE: scm_record::FileMode = scm_record::FileMode::Unix(0o100755);
    pub const SYMLINK: scm_record::FileMode = scm_record::FileMode::Unix(0o120000);
}

fn buf_to_file_contents(hash: Option<String>, buf: Vec<u8>) -> FileContents {
    let num_bytes: u64 = buf.len().try_into().unwrap();
    let text = if buf.contains(&0) {
        None
    } else {
        String::from_utf8(buf).ok()
    };
    match text {
        Some(text) => FileContents::Text {
            contents: text,
            hash,
            num_bytes,
        },
        None => FileContents::Binary { hash, num_bytes },
    }
}

fn read_file_contents(
    materialized_value: MaterializedTreeValue,
    path: &RepoPath,
    materialize_options: &ConflictMaterializeOptions,
) -> Result<FileInfo, BuiltinToolError> {
    match materialized_value {
        MaterializedTreeValue::Absent => Ok(FileInfo {
            file_mode: mode::ABSENT,
            contents: FileContents::Absent,
        }),
        MaterializedTreeValue::AccessDenied(err) => Ok(FileInfo {
            file_mode: mode::NORMAL,
            contents: FileContents::Text {
                contents: format!("Access denied: {err}"),
                hash: None,
                num_bytes: 0,
            },
        }),

        MaterializedTreeValue::File(mut file) => {
            let buf = file.read_all(path).block_on()?;
            let file_mode = if file.executable {
                mode::EXECUTABLE
            } else {
                mode::NORMAL
            };
            let contents = buf_to_file_contents(Some(file.id.hex()), buf);
            Ok(FileInfo {
                file_mode,
                contents,
            })
        }

        MaterializedTreeValue::Symlink { id, target } => {
            let file_mode = mode::SYMLINK;
            let num_bytes = target.len().try_into().unwrap();
            Ok(FileInfo {
                file_mode,
                contents: FileContents::Text {
                    contents: target,
                    hash: Some(id.hex()),
                    num_bytes,
                },
            })
        }

        MaterializedTreeValue::Tree(tree_id) => {
            unreachable!("list of changed files included a tree: {tree_id:?}");
        }
        MaterializedTreeValue::GitSubmodule(id) => Err(BuiltinToolError::Unimplemented {
            item: "git submodule",
            id: id.hex(),
        }),
        MaterializedTreeValue::FileConflict(file) => {
            // Since scm_record doesn't support diffs of conflicts, file
            // conflicts are compared in materialized form. The UI would look
            // scary, but it can at least allow squashing resolved hunks.
            let buf = materialize_merge_result_to_bytes(
                &file.contents,
                &file.labels,
                materialize_options,
            )
            .into();
            // TODO: Render the ID somehow?
            let contents = buf_to_file_contents(None, buf);
            Ok(FileInfo {
                file_mode: mode::NORMAL,
                contents,
            })
        }
        MaterializedTreeValue::OtherConflict { id } => {
            // TODO: Non-file conflict shouldn't be rendered as a normal file
            // TODO: Render the ID somehow?
            let contents = buf_to_file_contents(None, id.describe().into_bytes());
            Ok(FileInfo {
                file_mode: mode::NORMAL,
                contents,
            })
        }
    }
}

fn make_section_changed_lines(
    contents: &str,
    change_type: scm_record::ChangeType,
) -> Vec<scm_record::SectionChangedLine<'static>> {
    contents
        .split_inclusive('\n')
        .map(|line| scm_record::SectionChangedLine {
            is_checked: false,
            change_type,
            line: Cow::Owned(line.to_owned()),
        })
        .collect()
}

fn make_diff_sections(
    left_contents: &str,
    right_contents: &str,
) -> Result<Vec<scm_record::Section<'static>>, BuiltinToolError> {
    let diff = ContentDiff::by_line([left_contents.as_bytes(), right_contents.as_bytes()]);
    let mut sections = Vec::new();
    for hunk in diff.hunks() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                debug_assert!(hunk.contents.iter().all_equal());
                let text = hunk.contents[0];
                let text = str::from_utf8(text).map_err(|err| BuiltinToolError::DecodeUtf8 {
                    source: err,
                    item: "matching text in diff hunk",
                })?;
                sections.push(scm_record::Section::Unchanged {
                    lines: text
                        .split_inclusive('\n')
                        .map(|line| Cow::Owned(line.to_owned()))
                        .collect(),
                });
            }
            DiffHunkKind::Different => {
                let sides = &hunk.contents;
                assert_eq!(sides.len(), 2, "only two inputs were provided to the diff");
                let left_side =
                    str::from_utf8(sides[0]).map_err(|err| BuiltinToolError::DecodeUtf8 {
                        source: err,
                        item: "left side of diff hunk",
                    })?;
                let right_side =
                    str::from_utf8(sides[1]).map_err(|err| BuiltinToolError::DecodeUtf8 {
                        source: err,
                        item: "right side of diff hunk",
                    })?;
                sections.push(scm_record::Section::Changed {
                    lines: [
                        make_section_changed_lines(left_side, scm_record::ChangeType::Removed),
                        make_section_changed_lines(right_side, scm_record::ChangeType::Added),
                    ]
                    .concat(),
                });
            }
        }
    }
    Ok(sections)
}

async fn make_diff_files(
    store: &Arc<Store>,
    trees: Diff<&MergedTree>,
    tree_diff: BoxStream<'_, CopiesTreeDiffEntry>,
    marker_style: ConflictMarkerStyle,
) -> Result<(Vec<RepoPathBuf>, Vec<scm_record::File<'static>>), BuiltinToolError> {
    let materialize_options = ConflictMaterializeOptions {
        marker_style,
        marker_len: None,
        merge: store.merge_options().clone(),
    };
    let conflict_labels = trees.map(MergedTree::labels);
    let mut diff_stream = materialized_diff_stream(store, tree_diff, conflict_labels);
    let mut changed_files = Vec::new();
    let mut files = Vec::new();
    while let Some(entry) = diff_stream.next().await {
        let left_path = entry.path.source();
        let right_path = entry.path.target();
        let values = entry.values?;
        let left_info = read_file_contents(values.before, left_path, &materialize_options)?;
        let right_info = read_file_contents(values.after, right_path, &materialize_options)?;
        let mut sections = Vec::new();

        if left_info.file_mode != right_info.file_mode {
            sections.push(scm_record::Section::FileMode {
                is_checked: false,
                mode: right_info.file_mode,
            });
        }

        match (left_info.contents, right_info.contents) {
            (FileContents::Absent, FileContents::Absent) => {}
            // In this context, `Absent` means the file doesn't exist. If it only
            // exists on one side, we will render a mode change section above.
            // The next two patterns are to avoid also rendering an empty
            // changed lines section that clutters the UI.
            (
                FileContents::Absent,
                FileContents::Text {
                    contents: _,
                    hash: _,
                    num_bytes: 0,
                },
            ) => {}
            (
                FileContents::Text {
                    contents: _,
                    hash: _,
                    num_bytes: 0,
                },
                FileContents::Absent,
            ) => {}
            (
                FileContents::Absent,
                FileContents::Text {
                    contents,
                    hash: _,
                    num_bytes: _,
                },
            ) => sections.push(scm_record::Section::Changed {
                lines: make_section_changed_lines(&contents, scm_record::ChangeType::Added),
            }),

            (
                FileContents::Text {
                    contents,
                    hash: _,
                    num_bytes: _,
                },
                FileContents::Absent,
            ) => sections.push(scm_record::Section::Changed {
                lines: make_section_changed_lines(&contents, scm_record::ChangeType::Removed),
            }),

            (
                FileContents::Text {
                    contents: old_contents,
                    hash: _,
                    num_bytes: _,
                },
                FileContents::Text {
                    contents: new_contents,
                    hash: _,
                    num_bytes: _,
                },
            ) => {
                sections.extend(make_diff_sections(&old_contents, &new_contents)?);
            }

            (
                FileContents::Binary {
                    hash: Some(left_hash),
                    ..
                },
                FileContents::Binary {
                    hash: Some(right_hash),
                    ..
                },
            ) if left_hash == right_hash => {
                // Binary file contents have not changed.
            }
            (left, right @ FileContents::Binary { .. })
            | (left @ FileContents::Binary { .. }, right) => {
                sections.push(scm_record::Section::Binary {
                    is_checked: false,
                    old_description: left.describe().map(Cow::Owned),
                    new_description: right.describe().map(Cow::Owned),
                });
            }
        }

        files.push(scm_record::File {
            old_path: None,
            // Path for displaying purposes, not for file access.
            path: Cow::Owned(right_path.to_fs_path_unchecked(Path::new(""))),
            file_mode: left_info.file_mode,
            sections,
        });
        changed_files.push(entry.path.target);
    }
    Ok((changed_files, files))
}

fn apply_diff_builtin(
    store: &Arc<Store>,
    left_tree: &MergedTree,
    right_tree: &MergedTree,
    changed_files: Vec<RepoPathBuf>,
    files: &[scm_record::File],
) -> BackendResult<MergedTree> {
    // Start with the right tree to match external tool behavior.
    // This ensures unmatched paths keep their values from the right tree.
    let mut tree_builder = MergedTreeBuilder::new(right_tree.clone());

    // First, revert all changed files to their left versions
    for path in &changed_files {
        let left_value = left_tree.path_value(path)?;
        tree_builder.set_or_remove(path.clone(), left_value);
    }

    // Then apply only the selected changes
    apply_changes(
        &mut tree_builder,
        changed_files,
        files,
        |path| left_tree.path_value(path),
        |path| right_tree.path_value(path),
        |path, contents, executable, copy_id| {
            let old_value = left_tree.path_value(path)?;
            let new_value = if old_value.is_resolved() {
                let id = store.write_file(path, &mut &contents[..]).block_on()?;
                Merge::normal(TreeValue::File {
                    id,
                    executable,
                    copy_id,
                })
            } else if let Some(old_file_ids) = old_value.to_file_merge() {
                // TODO: should error out if conflicts couldn't be parsed?
                let new_file_ids = conflicts::update_from_content(
                    &old_file_ids,
                    store,
                    path,
                    contents,
                    MIN_CONFLICT_MARKER_LEN, // TODO: use the materialization parameter
                )
                .block_on()?;
                match new_file_ids.into_resolved() {
                    Ok(id) => Merge::resolved(id.map(|id| TreeValue::File {
                        id,
                        executable,
                        copy_id: CopyId::placeholder(),
                    })),
                    Err(file_ids) => old_value.with_new_file_ids(&file_ids),
                }
            } else {
                panic!("unexpected content change at {path:?}: {old_value:?}");
            };
            Ok(new_value)
        },
    )?;
    tree_builder.write_tree()
}

fn apply_changes(
    tree_builder: &mut MergedTreeBuilder,
    changed_files: Vec<RepoPathBuf>,
    files: &[scm_record::File],
    select_left: impl Fn(&RepoPath) -> BackendResult<MergedTreeValue>,
    select_right: impl Fn(&RepoPath) -> BackendResult<MergedTreeValue>,
    write_file: impl Fn(&RepoPath, &[u8], bool, CopyId) -> BackendResult<MergedTreeValue>,
) -> BackendResult<()> {
    assert_eq!(
        changed_files.len(),
        files.len(),
        "result had a different number of files"
    );
    // TODO: Write files concurrently
    for (path, file) in changed_files.into_iter().zip(files) {
        let file_mode_change_selected = file
            .sections
            .iter()
            .find_map(|sec| match sec {
                scm_record::Section::FileMode { is_checked, .. } => Some(*is_checked),
                _ => None,
            })
            .unwrap_or(false);

        let (
            scm_record::SelectedChanges {
                contents,
                file_mode,
            },
            _unselected,
        ) = file.get_selected_contents();

        if file_mode == mode::ABSENT {
            // The file is not present in the selected changes.
            // Either a file mode change was selected to delete an existing file, so we
            // should remove it from the tree,
            if file_mode_change_selected {
                tree_builder.set_or_remove(path, Merge::absent());
            }
            // or the file's creation has been split out of the change, in which case we
            // don't need to change the tree.
            // In either case, we're done with this file afterwards.
            continue;
        }

        let executable = file_mode == mode::EXECUTABLE;
        match contents {
            scm_record::SelectedContents::Unchanged => {
                if file_mode_change_selected {
                    // File contents haven't changed, but file mode needs to be updated on the tree.
                    let value = override_file_executable_bit(select_left(&path)?, executable);
                    tree_builder.set_or_remove(path, value);
                } else {
                    // Neither file mode, nor contents changed => Do nothing.
                }
            }
            scm_record::SelectedContents::Binary {
                old_description: _,
                new_description: Some(_),
            } => {
                let value = override_file_executable_bit(select_right(&path)?, executable);
                tree_builder.set_or_remove(path, value);
            }
            scm_record::SelectedContents::Binary {
                old_description: _,
                new_description: None,
            } => {
                // File contents emptied out, but file mode is not absent => write empty file.
                let copy_id = CopyId::placeholder();
                let value = write_file(&path, &[], executable, copy_id)?;
                tree_builder.set_or_remove(path, value);
            }
            scm_record::SelectedContents::Text { contents } => {
                let copy_id = CopyId::placeholder();
                let value = write_file(&path, contents.as_bytes(), executable, copy_id)?;
                tree_builder.set_or_remove(path, value);
            }
        }
    }
    Ok(())
}

fn override_file_executable_bit(
    mut merged_tree_value: MergedTreeValue,
    new_executable_bit: bool,
) -> MergedTreeValue {
    for tree_value in merged_tree_value.iter_mut().flatten() {
        let TreeValue::File { executable, .. } = tree_value else {
            panic!("incompatible update: expected a TreeValue::File, got {tree_value:?}");
        };
        *executable = new_executable_bit;
    }
    merged_tree_value
}

pub fn edit_diff_builtin(
    trees: Diff<&MergedTree>,
    matcher: &dyn Matcher,
    conflict_marker_style: ConflictMarkerStyle,
) -> Result<MergedTree, BuiltinToolError> {
    let store = trees.before.store().clone();
    // TODO: handle copy tracking
    let copy_records = CopyRecords::default();
    let tree_diff = trees
        .before
        .diff_stream_with_copies(trees.after, matcher, &copy_records);
    let (changed_files, files) =
        make_diff_files(&store, trees, tree_diff, conflict_marker_style).block_on()?;
    let mut input = scm_record::helpers::CrosstermInput;
    let recorder = scm_record::Recorder::new(
        scm_record::RecordState {
            is_read_only: false,
            files,
            commits: Default::default(),
        },
        &mut input,
    );
    let result = recorder.run().map_err(BuiltinToolError::Record)?;
    apply_diff_builtin(
        &store,
        trees.before,
        trees.after,
        changed_files,
        &result.files,
    )
    .map_err(BuiltinToolError::BackendError)
}

fn make_merge_sections(
    merge_result: MergeResult,
) -> Result<Vec<scm_record::Section<'static>>, BuiltinToolError> {
    let mut sections = Vec::new();
    match merge_result {
        MergeResult::Resolved(buf) => {
            let contents = buf_to_file_contents(None, buf.into());
            let section = match contents {
                FileContents::Absent => None,
                FileContents::Text {
                    contents,
                    hash: _,
                    num_bytes: _,
                } => Some(scm_record::Section::Unchanged {
                    lines: contents
                        .split_inclusive('\n')
                        .map(|line| Cow::Owned(line.to_owned()))
                        .collect(),
                }),
                FileContents::Binary { .. } => Some(scm_record::Section::Binary {
                    // TODO: Perhaps, this should be an "unchanged" section?
                    is_checked: false,
                    old_description: None,
                    new_description: contents.describe().map(Cow::Owned),
                }),
            };
            if let Some(section) = section {
                sections.push(section);
            }
        }
        MergeResult::Conflict(hunks) => {
            for hunk in hunks {
                let section = match hunk.into_resolved() {
                    Ok(contents) => {
                        let contents = str::from_utf8(&contents).map_err(|err| {
                            BuiltinToolError::DecodeUtf8 {
                                source: err,
                                item: "unchanged hunk",
                            }
                        })?;
                        scm_record::Section::Unchanged {
                            lines: contents
                                .split_inclusive('\n')
                                .map(|line| Cow::Owned(line.to_owned()))
                                .collect(),
                        }
                    }
                    Err(merge) => {
                        let lines: Vec<scm_record::SectionChangedLine> = merge
                            .iter()
                            .zip(
                                [
                                    scm_record::ChangeType::Added,
                                    scm_record::ChangeType::Removed,
                                ]
                                .into_iter()
                                .cycle(),
                            )
                            .map(|(contents, change_type)| -> Result<_, BuiltinToolError> {
                                let contents = str::from_utf8(contents).map_err(|err| {
                                    BuiltinToolError::DecodeUtf8 {
                                        source: err,
                                        item: "conflicting hunk",
                                    }
                                })?;
                                let changed_lines =
                                    make_section_changed_lines(contents, change_type);
                                Ok(changed_lines)
                            })
                            .flatten_ok()
                            .try_collect()?;
                        scm_record::Section::Changed { lines }
                    }
                };
                sections.push(section);
            }
        }
    }
    Ok(sections)
}

fn make_merge_file(
    merge_tool_file: &MergeToolFile,
    options: &MergeOptions,
) -> Result<scm_record::File<'static>, BuiltinToolError> {
    let file = &merge_tool_file.file;
    let file_mode = if file.executable.expect("should have been resolved") {
        mode::EXECUTABLE
    } else {
        mode::NORMAL
    };
    // TODO: Maybe we should test binary contents here, and generate per-file
    // Binary section to select either "our" or "their" file.
    let merge_result = files::merge_hunks(&file.contents, options);
    let sections = make_merge_sections(merge_result)?;
    Ok(scm_record::File {
        old_path: None,
        // Path for displaying purposes, not for file access.
        path: Cow::Owned(
            merge_tool_file
                .repo_path
                .to_fs_path_unchecked(Path::new("")),
        ),
        file_mode,
        sections,
    })
}

pub fn edit_merge_builtin(
    tree: &MergedTree,
    merge_tool_files: &[MergeToolFile],
) -> Result<MergedTree, BuiltinToolError> {
    let store = tree.store();
    let mut input = scm_record::helpers::CrosstermInput;
    let recorder = scm_record::Recorder::new(
        scm_record::RecordState {
            is_read_only: false,
            files: merge_tool_files
                .iter()
                .map(|f| make_merge_file(f, store.merge_options()))
                .try_collect()?,
            commits: Default::default(),
        },
        &mut input,
    );
    let state = recorder.run()?;

    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    apply_changes(
        &mut tree_builder,
        merge_tool_files
            .iter()
            .map(|file| file.repo_path.clone())
            .collect_vec(),
        &state.files,
        |path| tree.path_value(path),
        // FIXME: It doesn't make sense to select a new value from the source tree.
        // Presently, `select_right` is never actually called, since it is used to select binary
        // sections, but `make_merge_file` does not produce `Binary` sections for conflicted files.
        // This needs to be revisited when the UI becomes capable of representing binary conflicts.
        |path| tree.path_value(path),
        |path, contents, executable, copy_id| {
            let id = store.write_file(path, &mut &contents[..]).block_on()?;
            Ok(Merge::normal(TreeValue::File {
                id,
                executable,
                copy_id,
            }))
        },
    )?;
    Ok(tree_builder.write_tree()?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use jj_lib::backend::FileId;
    use jj_lib::conflicts::extract_as_single_hunk;
    use jj_lib::matchers::EverythingMatcher;
    use jj_lib::matchers::FilesMatcher;
    use jj_lib::repo::Repo as _;
    use proptest::prelude::*;
    use proptest_state_machine::ReferenceStateMachine;
    use proptest_state_machine::StateMachineTest;
    use proptest_state_machine::prop_state_machine;
    use testutils::TestRepo;
    use testutils::assert_tree_eq;
    use testutils::dump_tree;
    use testutils::proptest::Transition;
    use testutils::proptest::WorkingCopyReferenceStateMachine;
    use testutils::repo_path;
    use testutils::repo_path_component;

    use super::*;

    fn make_diff(
        store: &Arc<Store>,
        left_tree: &MergedTree,
        right_tree: &MergedTree,
    ) -> (Vec<RepoPathBuf>, Vec<scm_record::File<'static>>) {
        make_diff_with_matcher(store, left_tree, right_tree, &EverythingMatcher)
    }

    fn make_diff_with_matcher(
        store: &Arc<Store>,
        left_tree: &MergedTree,
        right_tree: &MergedTree,
        matcher: &dyn Matcher,
    ) -> (Vec<RepoPathBuf>, Vec<scm_record::File<'static>>) {
        let copy_records = CopyRecords::default();
        let tree_diff = left_tree.diff_stream_with_copies(right_tree, matcher, &copy_records);
        make_diff_files(
            store,
            Diff::new(left_tree, right_tree),
            tree_diff,
            ConflictMarkerStyle::Diff,
        )
        .block_on()
        .unwrap()
    }

    fn apply_diff(
        store: &Arc<Store>,
        left_tree: &MergedTree,
        right_tree: &MergedTree,
        changed_files: &[RepoPathBuf],
        files: &[scm_record::File],
    ) -> MergedTree {
        apply_diff_builtin(store, left_tree, right_tree, changed_files.to_vec(), files).unwrap()
    }

    #[test]
    fn test_edit_diff_builtin() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let unchanged = repo_path("unchanged");
        let changed_path = repo_path("changed");
        let added_path = repo_path("added");
        let left_tree = testutils::create_tree(
            &test_repo.repo,
            &[
                (unchanged, "unchanged\n"),
                (changed_path, "line1\nline2\nline3\n"),
            ],
        );
        let right_tree = testutils::create_tree(
            &test_repo.repo,
            &[
                (unchanged, "unchanged\n"),
                (changed_path, "line1\nchanged1\nchanged2\nline3\nadded1\n"),
                (added_path, "added\n"),
            ],
        );

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "added",
            "changed",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "added",
                file_mode: Absent,
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33188,
                        ),
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "added\n",
                            },
                        ],
                    },
                ],
            },
            File {
                old_path: None,
                path: "changed",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Unchanged {
                        lines: [
                            "line1\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "line2\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "changed1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "changed2\n",
                            },
                        ],
                    },
                    Unchanged {
                        lines: [
                            "line3\n",
                        ],
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "added1\n",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);

        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_add_empty_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let added_empty_file_path = repo_path("empty_file");
        let left_tree = testutils::create_tree(&test_repo.repo, &[]);
        let right_tree = testutils::create_tree(&test_repo.repo, &[(added_empty_file_path, "")]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "empty_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "empty_file",
                file_mode: Absent,
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33188,
                        ),
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_add_executable_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let added_executable_file_path = repo_path("executable_file");
        let left_tree = testutils::create_tree(&test_repo.repo, &[]);
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(added_executable_file_path, "executable")
                .executable(true);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "executable_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "executable_file",
                file_mode: Absent,
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33261,
                        ),
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "executable",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_empty_file_mode_change() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let empty_file_path = repo_path("empty_file");
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(empty_file_path, vec![]).executable(false);
        });
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(empty_file_path, vec![]).executable(true);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "empty_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "empty_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33261,
                        ),
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_text_file_mode_change() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let text_file_path = repo_path("text_file");
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(text_file_path, "text").executable(false);
        });
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(text_file_path, "text").executable(true);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "text_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "text_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33261,
                        ),
                    },
                    Unchanged {
                        lines: [
                            "text",
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_binary_file_mode_change() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let binary_file_path = repo_path("binary_file");
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(binary_file_path, vec![0xff, 0x00])
                .executable(false);
        });
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(binary_file_path, vec![0xff, 0x00])
                .executable(true);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "binary_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "binary_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33261,
                        ),
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_change_binary_file_with_unselected_file_mode_change() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let binary_file_path = repo_path("binary_file");
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(binary_file_path, vec![0xff, 0x00])
                .executable(false);
        });
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(binary_file_path, vec![0xff, 0x01])
                .executable(true);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "binary_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "binary_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Unix(
                            33261,
                        ),
                    },
                    Binary {
                        is_checked: false,
                        old_description: Some(
                            "fb296c879f1852c0dca0 (2B)",
                        ),
                        new_description: Some(
                            "cc429d26cbaec338223b (2B)",
                        ),
                    },
                ],
            },
        ]
        "#);

        // Select only the binary change
        let mut files = files;
        for file in &mut files {
            for section in &mut file.sections {
                if let scm_record::Section::Binary { is_checked, .. } = section {
                    *is_checked = true;
                }
            }
        }

        let expected_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder
                .file(binary_file_path, vec![0xff, 0x01])
                .executable(false);
        });
        let actual_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(expected_tree, actual_tree);
    }

    #[test]
    fn test_edit_diff_builtin_delete_binary_file_with_unselected_file_mode_change() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let binary_file_path = repo_path("binary_file");
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(binary_file_path, vec![0xff, 0x00]);
        });
        let right_tree = testutils::create_tree(&test_repo.repo, &[]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "binary_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "binary_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Absent,
                    },
                    Binary {
                        is_checked: false,
                        old_description: Some(
                            "fb296c879f1852c0dca0 (2B)",
                        ),
                        new_description: None,
                    },
                ],
            },
        ]
        "#);

        // Select only the binary change
        let mut files = files;
        for file in &mut files {
            for section in &mut file.sections {
                if let scm_record::Section::Binary { is_checked, .. } = section {
                    *is_checked = true;
                }
            }
        }

        let expected_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(binary_file_path, vec![]);
        });
        let actual_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(expected_tree, actual_tree);
    }

    #[test]
    fn test_edit_diff_builtin_delete_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let file_path = repo_path("file_with_content");
        let left_tree = testutils::create_tree(&test_repo.repo, &[(file_path, "content\n")]);
        let right_tree = testutils::create_tree(&test_repo.repo, &[]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "file_with_content",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "file_with_content",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Absent,
                    },
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "content\n",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_delete_empty_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let added_empty_file_path = repo_path("empty_file");
        let left_tree = testutils::create_tree(&test_repo.repo, &[(added_empty_file_path, "")]);
        let right_tree = testutils::create_tree(&test_repo.repo, &[]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "empty_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "empty_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    FileMode {
                        is_checked: false,
                        mode: Absent,
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_modify_empty_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let empty_file_path = repo_path("empty_file");
        let left_tree = testutils::create_tree(&test_repo.repo, &[(empty_file_path, "")]);
        let right_tree =
            testutils::create_tree(&test_repo.repo, &[(empty_file_path, "modified\n")]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "empty_file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "empty_file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "modified\n",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_make_file_empty() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let file_path = repo_path("file_with_content");
        let left_tree = testutils::create_tree(&test_repo.repo, &[(file_path, "content\n")]);
        let right_tree = testutils::create_tree(&test_repo.repo, &[(file_path, "")]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "file_with_content",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "file_with_content",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "content\n",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_conflict_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let file_path = repo_path("file");
        let left_tree = {
            let base = testutils::create_single_tree(&test_repo.repo, &[(file_path, "")]);
            let left = testutils::create_single_tree(&test_repo.repo, &[(file_path, "1\n")]);
            let right = testutils::create_single_tree(&test_repo.repo, &[(file_path, "2\n")]);
            MergedTree::unlabeled(
                store.clone(),
                Merge::from_vec(vec![
                    left.id().clone(),
                    base.id().clone(),
                    right.id().clone(),
                ]),
            )
        };
        let right_tree = testutils::create_tree(&test_repo.repo, &[(file_path, "resolved\n")]);

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "file",
        ]
        "#);
        insta::assert_debug_snapshot!(files, @r#"
        [
            File {
                old_path: None,
                path: "file",
                file_mode: Unix(
                    33188,
                ),
                sections: [
                    Changed {
                        lines: [
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "<<<<<<< Conflict 1 of 1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "%%%%%%% Changes from base to side #1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "+1\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "+++++++ Contents of side #2\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: "2\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Removed,
                                line: ">>>>>>> Conflict 1 of 1 ends\n",
                            },
                            SectionChangedLine {
                                is_checked: false,
                                change_type: Added,
                                line: "resolved\n",
                            },
                        ],
                    },
                ],
            },
        ]
        "#);
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_replace_directory_with_file() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let folder_path = repo_path("folder");
        let file_in_folder_path = folder_path.join(repo_path_component("file_in_folder"));
        let left_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(&file_in_folder_path, vec![]);
        });
        let right_tree = testutils::create_tree_with(&test_repo.repo, |builder| {
            builder.file(folder_path, vec![]);
        });

        let (changed_files, files) = make_diff(store, &left_tree, &right_tree);
        insta::assert_debug_snapshot!(changed_files, @r#"
        [
            "folder",
            "folder/file_in_folder",
        ]
        "#);
        insta::with_settings!({filters => vec![(r"\\\\", "/")]}, {
            insta::assert_debug_snapshot!(files, @r#"
            [
                File {
                    old_path: None,
                    path: "folder",
                    file_mode: Absent,
                    sections: [
                        FileMode {
                            is_checked: false,
                            mode: Unix(
                                33188,
                            ),
                        },
                    ],
                },
                File {
                    old_path: None,
                    path: "folder/file_in_folder",
                    file_mode: Unix(
                        33188,
                    ),
                    sections: [
                        FileMode {
                            is_checked: false,
                            mode: Absent,
                        },
                    ],
                },
            ]
            "#);
        });
        let no_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        assert_tree_eq!(
            right_tree,
            all_changes_tree,
            "all-changes tree was different",
        );
    }

    #[test]
    fn test_edit_diff_builtin_with_matcher() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let matched_path = repo_path("matched");
        let unmatched_path = repo_path("unmatched");
        let left_tree = testutils::create_tree(
            &test_repo.repo,
            &[
                (matched_path, "left matched\n"),
                (unmatched_path, "left unmatched\n"),
            ],
        );
        let right_tree = testutils::create_tree(
            &test_repo.repo,
            &[
                (matched_path, "right matched\n"),
                (unmatched_path, "right unmatched\n"),
            ],
        );

        let matcher = FilesMatcher::new([matched_path]);

        let (changed_files, files) =
            make_diff_with_matcher(store, &left_tree, &right_tree, &matcher);

        assert_eq!(changed_files, vec![matched_path.to_owned()]);

        let result_tree =
            apply_diff_builtin(store, &left_tree, &right_tree, changed_files, &files).unwrap();

        assert_eq!(
            result_tree.path_value(matched_path).unwrap(),
            left_tree.path_value(matched_path).unwrap()
        );
        assert_eq!(
            result_tree.path_value(unmatched_path).unwrap(),
            right_tree.path_value(unmatched_path).unwrap()
        );
    }

    #[test]
    fn test_make_merge_sections() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();

        let path = repo_path("file");
        let base_tree = testutils::create_tree(
            &test_repo.repo,
            &[(path, "base 1\nbase 2\nbase 3\nbase 4\nbase 5\n")],
        );
        let left_tree = testutils::create_tree(
            &test_repo.repo,
            &[(path, "left 1\nbase 2\nbase 3\nbase 4\nleft 5\n")],
        );
        let right_tree = testutils::create_tree(
            &test_repo.repo,
            &[(path, "right 1\nbase 2\nbase 3\nbase 4\nright 5\n")],
        );

        fn to_file_id(tree_value: MergedTreeValue) -> Option<FileId> {
            match tree_value.into_resolved() {
                Ok(Some(TreeValue::File {
                    id,
                    executable: _,
                    copy_id: _,
                })) => Some(id.clone()),
                other => {
                    panic!("merge should have been a FileId: {other:?}")
                }
            }
        }

        let merge = Merge::from_vec(vec![
            to_file_id(left_tree.path_value(path).unwrap()),
            to_file_id(base_tree.path_value(path).unwrap()),
            to_file_id(right_tree.path_value(path).unwrap()),
        ]);
        let content = extract_as_single_hunk(&merge, store, path)
            .block_on()
            .unwrap();
        let merge_result = files::merge_hunks(&content, store.merge_options());
        let sections = make_merge_sections(merge_result).unwrap();
        insta::assert_debug_snapshot!(sections, @r#"
        [
            Changed {
                lines: [
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Added,
                        line: "left 1\n",
                    },
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Removed,
                        line: "base 1\n",
                    },
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Added,
                        line: "right 1\n",
                    },
                ],
            },
            Unchanged {
                lines: [
                    "base 2\n",
                    "base 3\n",
                    "base 4\n",
                ],
            },
            Changed {
                lines: [
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Added,
                        line: "left 5\n",
                    },
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Removed,
                        line: "base 5\n",
                    },
                    SectionChangedLine {
                        is_checked: false,
                        change_type: Added,
                        line: "right 5\n",
                    },
                ],
            },
        ]
        "#);
    }

    prop_state_machine! {
        #[test]
        fn test_edit_diff_builtin_all_or_nothing_proptest(
            sequential 1..20 => EditDiffBuiltinAllOrNothingPropTest
        );

        #[test]
        fn test_edit_diff_builtin_partial_selection_proptest(
            sequential 1..20 => EditDiffBuiltinPartialSelectionPropTest
        );
    }

    /// SUT for property-based test to check that selecting all or none of the
    /// changes in the diff between two working copy states reproduces the right
    /// or the left tree, respectively.
    struct EditDiffBuiltinAllOrNothingPropTest {
        test_repo: TestRepo,
        prev_tree: MergedTree,
    }

    impl StateMachineTest for EditDiffBuiltinAllOrNothingPropTest {
        type SystemUnderTest = Self;

        type Reference = WorkingCopyReferenceStateMachine;

        fn init_test(ref_state: &WorkingCopyReferenceStateMachine) -> Self::SystemUnderTest {
            let test_repo = TestRepo::init();
            let initial_tree = ref_state.create_tree(&test_repo.repo);
            Self {
                test_repo,
                prev_tree: initial_tree,
            }
        }

        fn apply(
            state: Self::SystemUnderTest,
            ref_state: &WorkingCopyReferenceStateMachine,
            transition: Transition,
        ) -> Self::SystemUnderTest {
            match transition {
                Transition::Commit => {
                    let prev_tree = ref_state.create_tree(&state.test_repo.repo);
                    Self {
                        test_repo: state.test_repo,
                        prev_tree,
                    }
                }

                Transition::SetDirEntry { .. } => {
                    // Do nothing; this is handled by the reference state machine.
                    state
                }
            }
        }

        fn check_invariants(
            state: &Self::SystemUnderTest,
            ref_state: &WorkingCopyReferenceStateMachine,
        ) {
            let store = state.test_repo.repo.store();
            let left_tree = &state.prev_tree;
            let right_tree = ref_state.create_tree(&state.test_repo.repo);

            let (changed_files, files) = make_diff(store, left_tree, &right_tree);
            let no_changes_tree = apply_diff(store, left_tree, &right_tree, &changed_files, &files);
            assert_tree_eq!(left_tree, no_changes_tree, "no-changes tree was different");

            let mut files = files;
            for file in &mut files {
                file.toggle_all();
            }
            let all_changes_tree =
                apply_diff(store, left_tree, &right_tree, &changed_files, &files);
            assert_tree_eq!(
                right_tree,
                all_changes_tree,
                "all-changes tree was different",
            );
        }
    }

    /// SUT for property-based test to check that after selecting some of the
    /// changes in a diff, applying the remaining changes to the intermediate
    /// tree reproduces the right tree.
    ///
    /// This "roundtrip" property only holds if none of the selected changes
    /// implicitly incurs changes that would conflict with the unselected
    /// ones. An example of this is the deletion of a non-empty file which
    /// is represented by a file mode change to `Absent` and a text or
    /// binary change deleting the contents. When only the file mode change
    /// is selected, the file is still entirely removed. scm_record should
    /// ensure that selecting the file mode change implies the content
    /// change, but we cannot rely on this.
    ///
    /// Another situation arises when a file, e.g. "a", is replaced by a
    /// directory containing another file, e.g. "a/b". Selecting the creation of
    /// "a/b" but not the deletion of "a" will still implicitly replace the file
    /// "a" directory. scm_record currently does not group or enforce selections
    /// of changes in a way to prevent this.
    ///
    /// This test does not allow selections of changes that violate the
    /// "roundtrip" property for the above reasons. Otherwise, it sources its
    /// selection from a bit mask that is part of the reference state and is
    /// subject to random generation and shrinking but otherwise does not affect
    /// the state machine's transition, nor is it affected by any of the
    /// transitions.
    struct EditDiffBuiltinPartialSelectionPropTest {
        test_repo: TestRepo,
        prev_tree: MergedTree,
        prev_file_list: BTreeSet<RepoPathBuf>,
    }

    impl StateMachineTest for EditDiffBuiltinPartialSelectionPropTest {
        type SystemUnderTest = Self;
        type Reference = WorkingCopyWithSelectionStateMachine;

        fn init_test(ref_state: &WorkingCopyWithSelectionStateMachine) -> Self::SystemUnderTest {
            let test_repo = TestRepo::init();
            let initial_tree = ref_state.working_copy.create_tree(&test_repo.repo);
            Self {
                test_repo,
                prev_tree: initial_tree,
                prev_file_list: BTreeSet::new(),
            }
        }

        fn apply(
            state: Self::SystemUnderTest,
            ref_state: &WorkingCopyWithSelectionStateMachine,
            transition: Transition,
        ) -> Self::SystemUnderTest {
            match transition {
                Transition::Commit => {
                    let prev_tree = ref_state.working_copy.create_tree(&state.test_repo.repo);
                    let prev_file_list = ref_state
                        .working_copy
                        .paths()
                        .map(ToOwned::to_owned)
                        .collect();
                    Self {
                        test_repo: state.test_repo,
                        prev_tree,
                        prev_file_list,
                    }
                }

                Transition::SetDirEntry { .. } => {
                    // Do nothing; this is handled by the reference state machine.
                    state
                }
            }
        }

        fn check_invariants(
            state: &Self::SystemUnderTest,
            ref_state: &WorkingCopyWithSelectionStateMachine,
        ) {
            let store = state.test_repo.repo.store();
            let left_tree = &state.prev_tree;
            let right_tree = ref_state.working_copy.create_tree(&state.test_repo.repo);

            let (changed_files, files) = make_diff(store, left_tree, &right_tree);

            let mut files = files;
            for (path, file) in changed_files.iter().zip(&mut files) {
                for (section, selected) in file.sections.iter_mut().zip(&ref_state.selection_mask) {
                    section.set_checked(*selected);
                }

                // Sanity checks: Does the partial selection make sense on its own?
                if let Some(scm_record::Section::FileMode { is_checked, mode }) =
                    file.sections.first()
                {
                    let did_file_exist = state.prev_file_list.contains(path);
                    let is_anything_selected = file.sections.iter().any(|sec| match sec {
                        scm_record::Section::FileMode { is_checked, .. }
                        | scm_record::Section::Binary { is_checked, .. } => *is_checked,
                        scm_record::Section::Changed { lines } => {
                            lines.iter().any(|line| line.is_checked)
                        }
                        scm_record::Section::Unchanged { .. } => false,
                    });

                    if did_file_exist && *mode == scm_record::FileMode::Absent && *is_checked {
                        // File was removed, so all sections need to be checked.
                        file.set_checked(true);
                    }
                    if !did_file_exist && is_anything_selected {
                        // File was created, so if any changes are selected, then so must the file
                        // mode change.
                        file.sections[0].set_checked(true);
                    }
                }

                if state
                    .prev_file_list
                    .iter()
                    .any(|f| f.ancestors().skip(1).contains(path.as_ref()))
                {
                    // Do not create files which would overwrite directories.
                    file.set_checked(false);
                }

                if path
                    .ancestors()
                    .skip(1)
                    .any(|dir| state.prev_file_list.contains(dir))
                {
                    // Do not create files which would create directories overwriting files.
                    file.set_checked(false);
                }
            }

            eprintln!("selected changes: {files:#?}");

            let selected_changes_tree =
                apply_diff(store, left_tree, &right_tree, &changed_files, &files);

            eprintln!(
                "selected changes intermediate tree:\n{}",
                dump_tree(&selected_changes_tree)
            );

            // Transform `files` to create the complementary set of changes:
            for file in &mut files {
                // If a file mode change was applied, update the base mode.
                if let Some(scm_record::Section::FileMode { is_checked, mode }) =
                    file.sections.first()
                    && *is_checked
                {
                    file.file_mode = *mode;
                }

                // If the file has been renamed, it's now in its new position.
                file.old_path = None;

                // Only keep sections which weren't selected previously. For text files,
                // transform additions which have already been applied into `Unchanged` hunks.
                file.sections = std::mem::take(&mut file.sections)
                    .into_iter()
                    .flat_map(|sec| {
                        use scm_record::ChangeType::*;
                        use scm_record::Section::*;
                        use scm_record::SectionChangedLine;
                        match sec {
                            Changed { lines } => lines
                                .into_iter()
                                .filter_map(|line_change| match line_change {
                                    SectionChangedLine {
                                        is_checked: true,
                                        change_type: Added,
                                        line,
                                    } => Some(Unchanged { lines: vec![line] }),
                                    SectionChangedLine {
                                        is_checked: true,
                                        change_type: Removed,
                                        line: _,
                                    } => None,
                                    SectionChangedLine {
                                        is_checked: false, ..
                                    } => Some(Changed {
                                        lines: vec![line_change],
                                    }),
                                })
                                .collect(),

                            Unchanged { .. }
                            | FileMode {
                                is_checked: false, ..
                            }
                            | Binary {
                                is_checked: false, ..
                            } => vec![sec],

                            FileMode {
                                is_checked: true, ..
                            }
                            | Binary {
                                is_checked: true, ..
                            } => {
                                vec![]
                            }
                        }
                    })
                    .collect();

                // We want to select all of the remaining changes this time.
                file.set_checked(true);
            }

            eprintln!("remaining changes: {files:#?}");

            let all_changes_tree = apply_diff(
                store,
                &selected_changes_tree,
                &right_tree,
                &changed_files,
                &files,
            );
            assert_tree_eq!(
                right_tree,
                all_changes_tree,
                "all-changes tree was different",
            );
        }
    }

    #[derive(Debug, Clone, Default)]
    struct WorkingCopyWithSelectionStateMachine {
        working_copy: WorkingCopyReferenceStateMachine,
        selection_mask: Vec<bool>,
    }

    impl ReferenceStateMachine for WorkingCopyWithSelectionStateMachine {
        type State = Self;
        type Transition = <WorkingCopyReferenceStateMachine as ReferenceStateMachine>::Transition;

        fn init_state() -> BoxedStrategy<Self::State> {
            (
                WorkingCopyReferenceStateMachine::init_state(),
                proptest::collection::vec(any::<bool>(), 20),
            )
                .prop_map(|(working_copy, selection_mask)| Self {
                    working_copy,
                    selection_mask,
                })
                .boxed()
        }

        fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
            WorkingCopyReferenceStateMachine::transitions(&state.working_copy)
        }

        fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
            state.working_copy =
                WorkingCopyReferenceStateMachine::apply(state.working_copy, transition);
            state
        }
    }
}
