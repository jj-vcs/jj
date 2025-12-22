// Copyright 2025 The Jujutsu Authors
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

//! Utilities to compute unified (Git-style) diffs of 2 sides

use std::ops::Range;

use bstr::BStr;
use bstr::BString;
use thiserror::Error;

use super::DiffTokenType;
use super::DiffTokenVec;
use super::FileContent;
use super::LineCompareMode;
use super::diff_by_line;
use super::file_content_for_diff;
use super::unzip_diff_hunks_to_lines;
use crate::backend::BackendError;
use crate::conflicts::ConflictMaterializeOptions;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialize_merge_result_to_bytes;
use crate::diff::ContentDiff;
use crate::diff::DiffHunkKind;
use crate::merge::Diff;
use crate::object_id::ObjectId as _;
use crate::repo_path::RepoPath;

#[derive(Clone, Debug)]
pub struct GitDiffPart {
    /// Octal mode string or `None` if the file is absent.
    pub mode: Option<&'static str>,
    pub hash: String,
    pub content: FileContent<BString>,
}

#[derive(Debug, Error)]
pub enum UnifiedDiffError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("Access denied to {path}")]
    AccessDenied {
        path: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

pub fn git_diff_part(
    path: &RepoPath,
    value: MaterializedTreeValue,
    materialize_options: &ConflictMaterializeOptions,
) -> Result<GitDiffPart, UnifiedDiffError> {
    const DUMMY_HASH: &str = "0000000000";
    let mode;
    let mut hash;
    let content;
    match value {
        MaterializedTreeValue::Absent => {
            return Ok(GitDiffPart {
                mode: None,
                hash: DUMMY_HASH.to_owned(),
                content: FileContent {
                    is_binary: false,
                    contents: BString::default(),
                },
            });
        }
        MaterializedTreeValue::AccessDenied(err) => {
            return Err(UnifiedDiffError::AccessDenied {
                path: path.as_internal_file_string().to_owned(),
                source: err,
            });
        }
        MaterializedTreeValue::File(mut file) => {
            mode = if file.executable { "100755" } else { "100644" };
            hash = file.id.hex();
            content = file_content_for_diff(path, &mut file, |content| content)?;
        }
        MaterializedTreeValue::Symlink { id, target } => {
            mode = "120000";
            hash = id.hex();
            content = FileContent {
                // Unix file paths can't contain null bytes.
                is_binary: false,
                contents: target.into(),
            };
        }
        MaterializedTreeValue::GitSubmodule(id) => {
            // TODO: What should we actually do here?
            mode = "040000";
            hash = id.hex();
            content = FileContent {
                is_binary: false,
                contents: BString::default(),
            };
        }
        MaterializedTreeValue::FileConflict(file) => {
            mode = match file.executable {
                Some(true) => "100755",
                Some(false) | None => "100644",
            };
            hash = DUMMY_HASH.to_owned();
            content = FileContent {
                is_binary: false, // TODO: are we sure this is never binary?
                contents: materialize_merge_result_to_bytes(
                    &file.contents,
                    &file.labels,
                    materialize_options,
                ),
            };
        }
        MaterializedTreeValue::OtherConflict { id, labels } => {
            mode = "100644";
            hash = DUMMY_HASH.to_owned();
            content = FileContent {
                is_binary: false,
                contents: id.describe(&labels).into(),
            };
        }
        MaterializedTreeValue::Tree(_) => {
            panic!("Unexpected tree in diff at path {path:?}");
        }
    }
    hash.truncate(10);
    Ok(GitDiffPart {
        mode: Some(mode),
        hash,
        content,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineType {
    Context,
    Removed,
    Added,
}

pub struct UnifiedDiffHunk<'content> {
    pub left_line_range: Range<usize>,
    pub right_line_range: Range<usize>,
    pub lines: Vec<(DiffLineType, DiffTokenVec<'content>)>,
}

impl<'content> UnifiedDiffHunk<'content> {
    fn extend_context_lines(&mut self, lines: impl IntoIterator<Item = &'content [u8]>) {
        let old_len = self.lines.len();
        self.lines.extend(lines.into_iter().map(|line| {
            let tokens = vec![(DiffTokenType::Matching, line)];
            (DiffLineType::Context, tokens)
        }));
        self.left_line_range.end += self.lines.len() - old_len;
        self.right_line_range.end += self.lines.len() - old_len;
    }

    fn extend_removed_lines(&mut self, lines: impl IntoIterator<Item = DiffTokenVec<'content>>) {
        let old_len = self.lines.len();
        self.lines
            .extend(lines.into_iter().map(|line| (DiffLineType::Removed, line)));
        self.left_line_range.end += self.lines.len() - old_len;
    }

    fn extend_added_lines(&mut self, lines: impl IntoIterator<Item = DiffTokenVec<'content>>) {
        let old_len = self.lines.len();
        self.lines
            .extend(lines.into_iter().map(|line| (DiffLineType::Added, line)));
        self.right_line_range.end += self.lines.len() - old_len;
    }
}

pub fn unified_diff_hunks<'content>(
    contents: Diff<&'content BStr>,
    context: usize,
    options: LineCompareMode,
) -> Vec<UnifiedDiffHunk<'content>> {
    let mut hunks = vec![];
    let mut current_hunk = UnifiedDiffHunk {
        left_line_range: 0..0,
        right_line_range: 0..0,
        lines: vec![],
    };
    let diff = diff_by_line(contents.into_array(), &options);
    let mut diff_hunks = diff.hunks().peekable();
    while let Some(hunk) = diff_hunks.next() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                // Just use the right (i.e. new) content. We could count the
                // number of skipped lines separately, but the number of the
                // context lines should match the displayed content.
                let [_, right] = hunk.contents[..].try_into().unwrap();
                let mut lines = right.split_inclusive(|b| *b == b'\n').fuse();
                if !current_hunk.lines.is_empty() {
                    // The previous hunk line should be either removed/added.
                    current_hunk.extend_context_lines(lines.by_ref().take(context));
                }
                let before_lines = if diff_hunks.peek().is_some() {
                    lines.by_ref().rev().take(context).collect()
                } else {
                    vec![] // No more hunks
                };
                let num_skip_lines = lines.count();
                if num_skip_lines > 0 {
                    let left_start = current_hunk.left_line_range.end + num_skip_lines;
                    let right_start = current_hunk.right_line_range.end + num_skip_lines;
                    if !current_hunk.lines.is_empty() {
                        hunks.push(current_hunk);
                    }
                    current_hunk = UnifiedDiffHunk {
                        left_line_range: left_start..left_start,
                        right_line_range: right_start..right_start,
                        lines: vec![],
                    };
                }
                // The next hunk should be of DiffHunk::Different type if any.
                current_hunk.extend_context_lines(before_lines.into_iter().rev());
            }
            DiffHunkKind::Different => {
                let lines = unzip_diff_hunks_to_lines(ContentDiff::by_word(hunk.contents).hunks());
                current_hunk.extend_removed_lines(lines.before);
                current_hunk.extend_added_lines(lines.after);
            }
        }
    }
    if !current_hunk.lines.is_empty() {
        hunks.push(current_hunk);
    }
    hunks
}
