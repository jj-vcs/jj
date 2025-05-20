use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use futures::stream::BoxStream;
use futures::StreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::MergedTreeId;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::conflicts::materialized_diff_stream;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::MIN_CONFLICT_MARKER_LEN;
use jj_lib::copies::CopiesTreeDiffEntry;
use jj_lib::copies::CopyRecords;
use jj_lib::diff::Diff;
use jj_lib::diff::DiffHunkKind;
use jj_lib::files;
use jj_lib::files::MergeResult;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::store::Store;
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
            FileContents::Absent => None,
            FileContents::Text {
                contents: _,
                hash,
                num_bytes,
            }
            | FileContents::Binary { hash, num_bytes } => match hash {
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
    conflict_marker_style: ConflictMarkerStyle,
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
            let buf =
                materialize_merge_result_to_bytes(&file.contents, conflict_marker_style).into();
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
    let diff = Diff::by_line([left_contents.as_bytes(), right_contents.as_bytes()]);
    let mut sections = Vec::new();
    for hunk in diff.hunks() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                debug_assert!(hunk.contents.iter().all_equal());
                let text = hunk.contents[0];
                let text =
                    std::str::from_utf8(text).map_err(|err| BuiltinToolError::DecodeUtf8 {
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
                    std::str::from_utf8(sides[0]).map_err(|err| BuiltinToolError::DecodeUtf8 {
                        source: err,
                        item: "left side of diff hunk",
                    })?;
                let right_side =
                    std::str::from_utf8(sides[1]).map_err(|err| BuiltinToolError::DecodeUtf8 {
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
    tree_diff: BoxStream<'_, CopiesTreeDiffEntry>,
    conflict_marker_style: ConflictMarkerStyle,
) -> Result<(Vec<RepoPathBuf>, Vec<scm_record::File<'static>>), BuiltinToolError> {
    let mut diff_stream = materialized_diff_stream(store, tree_diff);
    let mut changed_files = Vec::new();
    let mut files = Vec::new();
    while let Some(entry) = diff_stream.next().await {
        let left_path = entry.path.source();
        let right_path = entry.path.target();
        let (left_value, right_value) = entry.values?;
        let left_info = read_file_contents(left_value, left_path, conflict_marker_style)?;
        let right_info = read_file_contents(right_value, right_path, conflict_marker_style)?;
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
    conflict_marker_style: ConflictMarkerStyle,
) -> BackendResult<MergedTreeId> {
    let mut tree_builder = MergedTreeBuilder::new(left_tree.id().clone());
    apply_changes(
        &mut tree_builder,
        changed_files,
        files,
        |path| left_tree.path_value(path),
        |path| right_tree.path_value(path),
        |path, contents, executable| {
            let old_value = left_tree.path_value(path)?;
            let new_value = if old_value.is_resolved() {
                let id = store.write_file(path, &mut &contents[..]).block_on()?;
                Merge::normal(TreeValue::File { id, executable })
            } else if let Some(old_file_ids) = old_value.to_file_merge() {
                // TODO: should error out if conflicts couldn't be parsed?
                let new_file_ids = conflicts::update_from_content(
                    &old_file_ids,
                    store,
                    path,
                    contents,
                    conflict_marker_style,
                    MIN_CONFLICT_MARKER_LEN, // TODO: use the materialization parameter
                )
                .block_on()?;
                match new_file_ids.into_resolved() {
                    Ok(id) => Merge::resolved(id.map(|id| TreeValue::File { id, executable })),
                    Err(file_ids) => old_value.with_new_file_ids(&file_ids),
                }
            } else {
                panic!("unexpected content change at {path:?}: {old_value:?}");
            };
            Ok(new_value)
        },
    )?;
    tree_builder.write_tree(store)
}

fn apply_changes(
    tree_builder: &mut MergedTreeBuilder,
    changed_files: Vec<RepoPathBuf>,
    files: &[scm_record::File],
    select_left: impl Fn(&RepoPath) -> BackendResult<MergedTreeValue>,
    select_right: impl Fn(&RepoPath) -> BackendResult<MergedTreeValue>,
    write_file: impl Fn(&RepoPath, &[u8], bool) -> BackendResult<MergedTreeValue>,
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
                new_description: _,
            } => {
                let value = select_right(&path)?;
                tree_builder.set_or_remove(path, value);
            }
            scm_record::SelectedContents::Text { contents } => {
                let value = write_file(&path, contents.as_bytes(), executable)?;
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
    left_tree: &MergedTree,
    right_tree: &MergedTree,
    matcher: &dyn Matcher,
    conflict_marker_style: ConflictMarkerStyle,
) -> Result<MergedTreeId, BuiltinToolError> {
    let store = left_tree.store().clone();
    // TODO: handle copy tracking
    let copy_records = CopyRecords::default();
    let tree_diff = left_tree.diff_stream_with_copies(right_tree, matcher, &copy_records);
    let (changed_files, files) =
        make_diff_files(&store, tree_diff, conflict_marker_style).block_on()?;
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
    let tree_id = apply_diff_builtin(
        &store,
        left_tree,
        right_tree,
        changed_files,
        &result.files,
        conflict_marker_style,
    )
    .map_err(BuiltinToolError::BackendError)?;
    Ok(tree_id)
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
                        let contents = std::str::from_utf8(&contents).map_err(|err| {
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
                                let contents = std::str::from_utf8(contents).map_err(|err| {
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
) -> Result<scm_record::File<'static>, BuiltinToolError> {
    let file = &merge_tool_file.file;
    let file_mode = if file.executable.expect("should have been resolved") {
        mode::EXECUTABLE
    } else {
        mode::NORMAL
    };
    // TODO: Maybe we should test binary contents here, and generate per-file
    // Binary section to select either "our" or "their" file.
    let merge_result = files::merge_hunks(&file.contents);
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
) -> Result<MergedTreeId, BuiltinToolError> {
    let mut input = scm_record::helpers::CrosstermInput;
    let recorder = scm_record::Recorder::new(
        scm_record::RecordState {
            is_read_only: false,
            files: merge_tool_files.iter().map(make_merge_file).try_collect()?,
            commits: Default::default(),
        },
        &mut input,
    );
    let state = recorder.run()?;

    let store = tree.store();
    let mut tree_builder = MergedTreeBuilder::new(tree.id().clone());
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
        |path, contents, executable| {
            let id = store.write_file(path, &mut &contents[..]).block_on()?;
            Ok(Merge::normal(TreeValue::File { id, executable }))
        },
    )?;
    Ok(tree_builder.write_tree(store)?)
}

#[cfg(test)]
mod tests {
    use jj_lib::backend::FileId;
    use jj_lib::conflicts::extract_as_single_hunk;
    use jj_lib::matchers::EverythingMatcher;
    use jj_lib::merge::MergedTreeValue;
    use jj_lib::repo::Repo as _;
    use testutils::dump_tree;
    use testutils::repo_path;
    use testutils::repo_path_component;
    use testutils::TestRepo;

    use super::*;

    fn make_diff(
        store: &Arc<Store>,
        left_tree: &MergedTree,
        right_tree: &MergedTree,
    ) -> (Vec<RepoPathBuf>, Vec<scm_record::File<'static>>) {
        let copy_records = CopyRecords::default();
        let tree_diff =
            left_tree.diff_stream_with_copies(right_tree, &EverythingMatcher, &copy_records);
        make_diff_files(store, tree_diff, ConflictMarkerStyle::Diff)
            .block_on()
            .unwrap()
    }

    fn apply_diff(
        store: &Arc<Store>,
        left_tree: &MergedTree,
        right_tree: &MergedTree,
        changed_files: &[RepoPathBuf],
        files: &[scm_record::File],
    ) -> MergedTreeId {
        apply_diff_builtin(
            store,
            left_tree,
            right_tree,
            changed_files.to_vec(),
            files,
            ConflictMarkerStyle::Diff,
        )
        .unwrap()
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

        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        insta::assert_debug_snapshot!(files, @r###"
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
        "###);
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            left_tree.id(),
            no_changes_tree.id(),
            "no-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &left_tree.id()),
            dump_tree(store, &no_changes_tree.id()),
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            right_tree.id(),
            all_changes_tree.id(),
            "all-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &right_tree.id()),
            dump_tree(store, &all_changes_tree.id()),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            left_tree.id(),
            no_changes_tree.id(),
            "no-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &left_tree.id()),
            dump_tree(store, &no_changes_tree.id()),
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            right_tree.id(),
            all_changes_tree.id(),
            "all-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &right_tree.id()),
            dump_tree(store, &all_changes_tree.id()),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            left_tree.id(),
            no_changes_tree.id(),
            "no-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &left_tree.id()),
            dump_tree(store, &no_changes_tree.id()),
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            right_tree.id(),
            all_changes_tree.id(),
            "all-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &right_tree.id()),
            dump_tree(store, &all_changes_tree.id()),
        );
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
        insta::assert_debug_snapshot!(files, @r###"
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
        "###);
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
        insta::assert_debug_snapshot!(files, @r###"
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
        "###);
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
            MergedTree::new(Merge::from_vec(vec![left, base, right]))
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
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            no_changes_tree.id(),
            left_tree.id(),
            "no-changes tree was different",
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            all_changes_tree.id(),
            right_tree.id(),
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
            "folder/file_in_folder",
            "folder",
        ]
        "#);
        insta::with_settings!({filters => vec![(r"\\\\", "/")]}, {
            insta::assert_debug_snapshot!(files, @r#"
            [
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
            ]
            "#);
        });
        let no_changes_tree_id = apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let no_changes_tree = store.get_root_tree(&no_changes_tree_id).unwrap();
        assert_eq!(
            left_tree.id(),
            no_changes_tree.id(),
            "no-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &left_tree.id()),
            dump_tree(store, &no_changes_tree.id()),
        );

        let mut files = files;
        for file in &mut files {
            file.toggle_all();
        }
        let all_changes_tree_id =
            apply_diff(store, &left_tree, &right_tree, &changed_files, &files);
        let all_changes_tree = store.get_root_tree(&all_changes_tree_id).unwrap();
        assert_eq!(
            right_tree.id(),
            all_changes_tree.id(),
            "all-changes tree was different:\nexpected tree:\n{}\nactual tree:\n{}",
            dump_tree(store, &right_tree.id()),
            dump_tree(store, &all_changes_tree.id()),
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
                Ok(Some(TreeValue::File { id, executable: _ })) => Some(id.clone()),
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
        let merge_result = files::merge_hunks(&content);
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
}
