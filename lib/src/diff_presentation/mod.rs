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

//! Utilities to present file diffs to the user

#![expect(missing_docs)]

use std::borrow::Borrow;
use std::mem;

use bstr::BString;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendResult;
use crate::conflicts::MaterializedFileValue;
use crate::diff::CompareBytesExactly;
use crate::diff::CompareBytesIgnoreAllWhitespace;
use crate::diff::CompareBytesIgnoreWhitespaceAmount;
use crate::diff::ContentDiff;
use crate::diff::DiffHunk;
use crate::diff::DiffHunkKind;
use crate::diff::find_line_ranges;
use crate::merge::Diff;
use crate::repo_path::RepoPath;

pub mod unified;
// TODO: colored_diffs utils should also be moved from `jj_cli::diff_utils` to
// here.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffTokenType {
    Matching,
    Different,
}

type DiffTokenVec<'content> = Vec<(DiffTokenType, &'content [u8])>;

#[derive(Clone, Debug)]
pub struct FileContent<T> {
    /// false if this file is likely text; true if it is likely binary.
    pub is_binary: bool,
    pub contents: T,
}

pub fn file_content_for_diff<T>(
    path: &RepoPath,
    file: &mut MaterializedFileValue,
    map_resolved: impl FnOnce(BString) -> T,
) -> BackendResult<FileContent<T>> {
    // If this is a binary file, don't show the full contents.
    // Determine whether it's binary by whether the first 8k bytes contain a null
    // character; this is the same heuristic used by git as of writing: https://github.com/git/git/blob/eea0e59ffbed6e33d171ace5be13cde9faa41639/xdiff-interface.c#L192-L198
    const PEEK_SIZE: usize = 8000;
    // TODO: currently we look at the whole file, even though for binary files we
    // only need to know the file size. To change that we'd have to extend all
    // the data backends to support getting the length.
    let contents = BString::new(file.read_all(path).block_on()?);
    let start = &contents[..PEEK_SIZE.min(contents.len())];
    Ok(FileContent {
        is_binary: start.contains(&b'\0'),
        contents: map_resolved(contents),
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LineCompareMode {
    /// Compares lines literally.
    #[default]
    Exact,
    /// Compares lines ignoring any whitespace occurrences.
    IgnoreAllSpace,
    /// Compares lines ignoring changes in whitespace amount.
    IgnoreSpaceChange,
}

pub fn diff_by_line<'input, T: AsRef<[u8]> + ?Sized + 'input>(
    inputs: impl IntoIterator<Item = &'input T>,
    options: &LineCompareMode,
) -> ContentDiff<'input> {
    // TODO: If we add --ignore-blank-lines, its tokenizer will have to attach
    // blank lines to the preceding range. Maybe it can also be implemented as a
    // post-process (similar to refine_changed_regions()) that expands unchanged
    // regions across blank lines.
    match options {
        LineCompareMode::Exact => {
            ContentDiff::for_tokenizer(inputs, find_line_ranges, CompareBytesExactly)
        }
        LineCompareMode::IgnoreAllSpace => {
            ContentDiff::for_tokenizer(inputs, find_line_ranges, CompareBytesIgnoreAllWhitespace)
        }
        LineCompareMode::IgnoreSpaceChange => {
            ContentDiff::for_tokenizer(inputs, find_line_ranges, CompareBytesIgnoreWhitespaceAmount)
        }
    }
}

/// Splits `[left, right]` hunk pairs into `[left_lines, right_lines]`.
pub fn unzip_diff_hunks_to_lines<'content, I>(diff_hunks: I) -> Diff<Vec<DiffTokenVec<'content>>>
where
    I: IntoIterator,
    I::Item: Borrow<DiffHunk<'content>>,
{
    let mut left_lines: Vec<DiffTokenVec<'content>> = vec![];
    let mut right_lines: Vec<DiffTokenVec<'content>> = vec![];
    let mut left_tokens: DiffTokenVec<'content> = vec![];
    let mut right_tokens: DiffTokenVec<'content> = vec![];

    for hunk in diff_hunks {
        let hunk = hunk.borrow();
        match hunk.kind {
            DiffHunkKind::Matching => {
                // TODO: add support for unmatched contexts
                debug_assert!(hunk.contents.iter().all_equal());
                for token in hunk.contents[0].split_inclusive(|b| *b == b'\n') {
                    left_tokens.push((DiffTokenType::Matching, token));
                    right_tokens.push((DiffTokenType::Matching, token));
                    if token.ends_with(b"\n") {
                        left_lines.push(mem::take(&mut left_tokens));
                        right_lines.push(mem::take(&mut right_tokens));
                    }
                }
            }
            DiffHunkKind::Different => {
                let [left, right] = hunk.contents[..]
                    .try_into()
                    .expect("hunk should have exactly two inputs");
                for token in left.split_inclusive(|b| *b == b'\n') {
                    left_tokens.push((DiffTokenType::Different, token));
                    if token.ends_with(b"\n") {
                        left_lines.push(mem::take(&mut left_tokens));
                    }
                }
                for token in right.split_inclusive(|b| *b == b'\n') {
                    right_tokens.push((DiffTokenType::Different, token));
                    if token.ends_with(b"\n") {
                        right_lines.push(mem::take(&mut right_tokens));
                    }
                }
            }
        }
    }

    if !left_tokens.is_empty() {
        left_lines.push(left_tokens);
    }
    if !right_tokens.is_empty() {
        right_lines.push(right_tokens);
    }
    Diff::new(left_lines, right_lines)
}
