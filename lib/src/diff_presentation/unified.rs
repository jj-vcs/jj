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

pub fn unified_diff_hunks(
    contents: Diff<&BStr>,
    context: usize,
    options: LineCompareMode,
) -> Vec<UnifiedDiffHunk<'_>> {
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

pub struct CombinedDiffHunk<'content> {
    pub parent_line_ranges: Vec<Range<usize>>,
    pub result_line_range: Range<usize>,
    pub lines: Vec<(Vec<DiffLineType>, DiffTokenVec<'content>)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombinedDiffMode {
    Cc,
    Combined,
}

struct PairwiseDiffLines<'content> {
    result_lines: Vec<(DiffLineType, DiffTokenVec<'content>)>,
    removed_lines: Vec<Vec<DiffTokenVec<'content>>>,
}

struct CombinedDiffLine<'content> {
    parent_line_types: Vec<DiffLineType>,
    parent_has_line: Vec<bool>,
    result_has_line: bool,
    tokens: DiffTokenVec<'content>,
}

fn pairwise_diff_lines<'content>(
    parent: &'content BStr,
    result: &'content BStr,
    compare_mode: &LineCompareMode,
) -> PairwiseDiffLines<'content> {
    let mut result_lines = vec![];
    let mut removed_lines: Vec<Vec<DiffTokenVec<'content>>> = vec![vec![]];
    let diff = diff_by_line([parent, result], compare_mode);
    for hunk in diff.hunks() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                let [_, right] = hunk.contents[..].try_into().unwrap();
                for line in right.split_inclusive(|b| *b == b'\n') {
                    let tokens = vec![(DiffTokenType::Matching, line)];
                    result_lines.push((DiffLineType::Context, tokens));
                }
            }
            DiffHunkKind::Different => {
                let lines = unzip_diff_hunks_to_lines(ContentDiff::by_word(hunk.contents).hunks());
                let result_index = result_lines.len();
                if removed_lines.len() <= result_index {
                    removed_lines.resize_with(result_index + 1, Vec::new);
                }
                removed_lines[result_index].extend(lines.before);
                result_lines.extend(
                    lines
                        .after
                        .into_iter()
                        .map(|line| (DiffLineType::Added, line)),
                );
            }
        }
    }
    removed_lines.resize_with(result_lines.len() + 1, Vec::new);
    PairwiseDiffLines {
        result_lines,
        removed_lines,
    }
}

pub fn combined_diff_hunks<'content>(
    parents: &'content [FileContent<BString>],
    result: &'content FileContent<BString>,
    context: usize,
    compare_mode: LineCompareMode,
    mode: CombinedDiffMode,
) -> Vec<CombinedDiffHunk<'content>> {
    fn tokens_to_bytes(tokens: &DiffTokenVec<'_>) -> Vec<u8> {
        let total_len: usize = tokens.iter().map(|(_, content)| content.len()).sum();
        let mut bytes = Vec::with_capacity(total_len);
        for (_, content) in tokens {
            bytes.extend_from_slice(content);
        }
        bytes
    }

    fn tokens_eq_bytes(tokens: &DiffTokenVec<'_>, expected: &[u8]) -> bool {
        let mut offset = 0;
        for (_, content) in tokens {
            let next_offset = offset + content.len();
            if next_offset > expected.len() {
                return false;
            }
            let expected_slice = &expected[offset..next_offset];
            if expected_slice != *content {
                return false;
            }
            offset = next_offset;
        }
        offset == expected.len()
    }

    if parents.is_empty() {
        return vec![];
    }

    let result_contents: &BStr = result.contents.as_ref();
    let mut parent_diffs: Vec<PairwiseDiffLines<'content>> = parents
        .iter()
        .map(|parent| pairwise_diff_lines(parent.contents.as_ref(), result_contents, &compare_mode))
        .collect();
    let result_line_count = parent_diffs[0].result_lines.len();
    for diff in &parent_diffs[1..] {
        debug_assert_eq!(diff.result_lines.len(), result_line_count);
    }

    for diff in &mut parent_diffs {
        for removed_lines in &mut diff.removed_lines {
            removed_lines.reverse();
        }
    }

    let num_parents = parents.len();
    // Merge per-parent diffs into a single ordered line stream.
    let mut combined_lines = Vec::new();
    for result_index in 0..=result_line_count {
        loop {
            let mut first_parent_index = None;
            let mut expected_tokens = None;
            for (parent_index, parent_diff) in parent_diffs.iter().enumerate() {
                if let Some(tokens) = parent_diff.removed_lines[result_index].last() {
                    first_parent_index = Some(parent_index);
                    expected_tokens = Some(tokens.clone());
                    break;
                }
            }
            let Some(first_parent_index) = first_parent_index else {
                break;
            };
            let expected_tokens =
                expected_tokens.expect("parent with removed line must have tokens");
            let expected_bytes = tokens_to_bytes(&expected_tokens);

            let mut parent_line_types = vec![DiffLineType::Context; num_parents];
            let mut parent_has_line = vec![false; num_parents];
            parent_line_types[first_parent_index] = DiffLineType::Removed;
            parent_has_line[first_parent_index] = true;
            for (parent_index, parent_diff) in
                parent_diffs.iter().enumerate().skip(first_parent_index + 1)
            {
                if let Some(tokens) = parent_diff.removed_lines[result_index].last()
                    && tokens_eq_bytes(tokens, &expected_bytes)
                {
                    parent_line_types[parent_index] = DiffLineType::Removed;
                    parent_has_line[parent_index] = true;
                }
            }

            let tokens = parent_diffs[first_parent_index].removed_lines[result_index]
                .pop()
                .expect("removed line should be present");
            debug_assert!(tokens_eq_bytes(&tokens, &expected_bytes));
            for (parent_index, parent_diff) in parent_diffs.iter_mut().enumerate() {
                if parent_index == first_parent_index {
                    continue;
                }
                if parent_line_types[parent_index] == DiffLineType::Removed {
                    let popped = parent_diff.removed_lines[result_index]
                        .pop()
                        .expect("removed line should be present");
                    debug_assert!(tokens_eq_bytes(&popped, &expected_bytes));
                }
            }
            combined_lines.push(CombinedDiffLine {
                parent_line_types,
                parent_has_line,
                result_has_line: false,
                tokens,
            });
        }

        if result_index == result_line_count {
            break;
        }

        let mut parent_line_types = Vec::with_capacity(num_parents);
        let mut parent_has_line = Vec::with_capacity(num_parents);
        let mut tokens = parent_diffs[0].result_lines[result_index].1.clone();
        let mut tokens_from_added =
            parent_diffs[0].result_lines[result_index].0 == DiffLineType::Added;
        for parent_diff in &parent_diffs {
            let (line_type, line_tokens) = &parent_diff.result_lines[result_index];
            parent_line_types.push(*line_type);
            parent_has_line.push(*line_type == DiffLineType::Context);
            if !tokens_from_added && *line_type == DiffLineType::Added {
                tokens = line_tokens.clone();
                tokens_from_added = true;
            }
        }
        combined_lines.push(CombinedDiffLine {
            parent_line_types,
            parent_has_line,
            result_has_line: true,
            tokens,
        });
    }

    // Track line counts to compute per-parent/result hunk ranges.
    let mut result_prefix = vec![0; combined_lines.len() + 1];
    let mut parent_prefix = vec![vec![0; combined_lines.len() + 1]; num_parents];
    for (index, line) in combined_lines.iter().enumerate() {
        result_prefix[index + 1] = result_prefix[index] + if line.result_has_line { 1 } else { 0 };
        for (parent_index, parent_line_prefix) in parent_prefix.iter_mut().enumerate() {
            parent_line_prefix[index + 1] = parent_line_prefix[index]
                + if line.parent_has_line[parent_index] {
                    1
                } else {
                    0
                };
        }
    }

    // Slice the combined line stream into hunks with context.
    let mut hunk_ranges = vec![];
    let mut current_start = None;
    let mut last_change = 0;
    for (index, line) in combined_lines.iter().enumerate() {
        let changed = line
            .parent_line_types
            .iter()
            .any(|line_type| *line_type != DiffLineType::Context);
        if changed {
            if current_start.is_none() {
                current_start = Some(index.saturating_sub(context));
            }
            last_change = index;
        } else if let Some(start) = current_start
            && index > last_change + context
        {
            let end = (last_change + context + 1).min(combined_lines.len());
            hunk_ranges.push(start..end);
            current_start = None;
        }
    }
    if let Some(start) = current_start {
        let end = (last_change + context + 1).min(combined_lines.len());
        hunk_ranges.push(start..end);
    }

    let mut hunks = vec![];
    for range in hunk_ranges {
        let parent_line_ranges = parent_prefix
            .iter()
            .map(|parent_line_prefix| {
                parent_line_prefix[range.start]..parent_line_prefix[range.end]
            })
            .collect();
        let result_line_range = result_prefix[range.start]..result_prefix[range.end];
        let lines = combined_lines[range.start..range.end]
            .iter()
            .map(|line| (line.parent_line_types.clone(), line.tokens.clone()))
            .collect();
        hunks.push(CombinedDiffHunk {
            parent_line_ranges,
            result_line_range,
            lines,
        });
    }
    if mode == CombinedDiffMode::Cc {
        hunks
            .into_iter()
            .filter(|hunk| {
                let num_parents = hunk.parent_line_ranges.len();
                (0..num_parents).all(|parent_index| {
                    let parent_matches_result = hunk.lines.iter().all(|(line_types, _)| {
                        let is_removed_line = line_types.contains(&DiffLineType::Removed);
                        let parent_type = line_types[parent_index];
                        if is_removed_line {
                            parent_type != DiffLineType::Removed
                        } else {
                            parent_type == DiffLineType::Context
                        }
                    });
                    !parent_matches_result
                })
            })
            .collect()
    } else {
        hunks
    }
}
