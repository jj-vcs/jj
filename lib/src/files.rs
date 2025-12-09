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

#![expect(missing_docs)]

use std::borrow::Borrow;
use std::collections::VecDeque;
use std::iter;
use std::mem;

use bstr::BStr;
use bstr::BString;
use either::Either;
use itertools::Itertools as _;

use crate::diff::ContentDiff;
use crate::diff::DiffHunk;
use crate::diff::DiffHunkKind;
use crate::merge::Merge;
use crate::merge::SameChange;
use crate::tree_merge::MergeOptions;

/// A diff line which may contain small hunks originating from both sides.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct DiffLine<'a> {
    pub line_number: DiffLineNumber,
    pub hunks: Vec<(DiffLineHunkSide, &'a BStr)>,
}

impl DiffLine<'_> {
    pub fn has_left_content(&self) -> bool {
        self.hunks
            .iter()
            .any(|&(side, _)| side != DiffLineHunkSide::Right)
    }

    pub fn has_right_content(&self) -> bool {
        self.hunks
            .iter()
            .any(|&(side, _)| side != DiffLineHunkSide::Left)
    }

    pub fn is_unmodified(&self) -> bool {
        self.hunks
            .iter()
            .all(|&(side, _)| side == DiffLineHunkSide::Both)
    }

    fn take(&mut self) -> Self {
        Self {
            line_number: self.line_number,
            hunks: mem::take(&mut self.hunks),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffLineNumber {
    pub left: u32,
    pub right: u32,
}

/// Which side a `DiffLine` hunk belongs to?
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineHunkSide {
    Both,
    Left,
    Right,
}

pub struct DiffLineIterator<'a, I> {
    diff_hunks: iter::Fuse<I>,
    current_line: DiffLine<'a>,
    queued_lines: VecDeque<DiffLine<'a>>,
}

impl<'a, I> DiffLineIterator<'a, I>
where
    I: Iterator,
    I::Item: Borrow<DiffHunk<'a>>,
{
    /// Iterates `diff_hunks` by line. Each hunk should have exactly two inputs.
    pub fn new(diff_hunks: I) -> Self {
        let line_number = DiffLineNumber { left: 1, right: 1 };
        Self::with_line_number(diff_hunks, line_number)
    }

    /// Iterates `diff_hunks` by line. Each hunk should have exactly two inputs.
    /// Hunk's line numbers start from the given `line_number`.
    pub fn with_line_number(diff_hunks: I, line_number: DiffLineNumber) -> Self {
        let current_line = DiffLine {
            line_number,
            hunks: vec![],
        };
        Self {
            diff_hunks: diff_hunks.fuse(),
            current_line,
            queued_lines: VecDeque::new(),
        }
    }
}

impl<I> DiffLineIterator<'_, I> {
    /// Returns line number of the next hunk. After all hunks are consumed, this
    /// returns the next line number if the last hunk ends with newline.
    pub fn next_line_number(&self) -> DiffLineNumber {
        let next_line = self.queued_lines.front().unwrap_or(&self.current_line);
        next_line.line_number
    }
}

impl<'a, I> Iterator for DiffLineIterator<'a, I>
where
    I: Iterator,
    I::Item: Borrow<DiffHunk<'a>>,
{
    type Item = DiffLine<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // TODO: Should we attempt to interpret as utf-8 and otherwise break only at
        // newlines?
        while self.queued_lines.is_empty() {
            let Some(hunk) = self.diff_hunks.next() else {
                break;
            };
            let hunk = hunk.borrow();
            match hunk.kind {
                DiffHunkKind::Matching => {
                    // TODO: add support for unmatched contexts?
                    debug_assert!(hunk.contents.iter().all_equal());
                    let text = hunk.contents[0];
                    let lines = text.split_inclusive(|b| *b == b'\n').map(BStr::new);
                    for line in lines {
                        self.current_line.hunks.push((DiffLineHunkSide::Both, line));
                        if line.ends_with(b"\n") {
                            self.queued_lines.push_back(self.current_line.take());
                            self.current_line.line_number.left += 1;
                            self.current_line.line_number.right += 1;
                        }
                    }
                }
                DiffHunkKind::Different => {
                    let [left_text, right_text] = hunk.contents[..]
                        .try_into()
                        .expect("hunk should have exactly two inputs");
                    let left_lines = left_text.split_inclusive(|b| *b == b'\n').map(BStr::new);
                    for left_line in left_lines {
                        self.current_line
                            .hunks
                            .push((DiffLineHunkSide::Left, left_line));
                        if left_line.ends_with(b"\n") {
                            self.queued_lines.push_back(self.current_line.take());
                            self.current_line.line_number.left += 1;
                        }
                    }
                    let mut right_lines =
                        right_text.split_inclusive(|b| *b == b'\n').map(BStr::new);
                    // Omit blank right line if matching hunk of the same line
                    // number has already been queued. Here we only need to
                    // check the first queued line since the other lines should
                    // be created in the left_lines loop above.
                    if right_text.starts_with(b"\n")
                        && self.current_line.hunks.is_empty()
                        && self
                            .queued_lines
                            .front()
                            .is_some_and(|queued| queued.has_right_content())
                    {
                        let blank_line = right_lines.next().unwrap();
                        assert_eq!(blank_line, b"\n");
                        self.current_line.line_number.right += 1;
                    }
                    for right_line in right_lines {
                        self.current_line
                            .hunks
                            .push((DiffLineHunkSide::Right, right_line));
                        if right_line.ends_with(b"\n") {
                            self.queued_lines.push_back(self.current_line.take());
                            self.current_line.line_number.right += 1;
                        }
                    }
                }
            }
        }

        if let Some(line) = self.queued_lines.pop_front() {
            return Some(line);
        }

        if !self.current_line.hunks.is_empty() {
            return Some(self.current_line.take());
        }

        None
    }
}

/// Diff hunk that may be unresolved conflicts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictDiffHunk<'input> {
    pub kind: DiffHunkKind,
    pub lefts: Merge<&'input BStr>,
    pub rights: Merge<&'input BStr>,
}

/// Iterator adaptor that translates non-conflict hunks to resolved `Merge`.
///
/// Trivial conflicts in the diff inputs should have been resolved by caller.
pub fn conflict_diff_hunks<'input, I>(
    diff_hunks: I,
    num_lefts: usize,
) -> impl Iterator<Item = ConflictDiffHunk<'input>>
where
    I: IntoIterator,
    I::Item: Borrow<DiffHunk<'input>>,
{
    fn to_merge<'input>(contents: &[&'input BStr]) -> Merge<&'input BStr> {
        // Not using trivial_merge() so that the original content can be
        // reproduced by concatenating hunks.
        if contents.iter().all_equal() {
            Merge::resolved(contents[0])
        } else {
            Merge::from_vec(contents)
        }
    }

    diff_hunks.into_iter().map(move |hunk| {
        let hunk = hunk.borrow();
        let (lefts, rights) = hunk.contents.split_at(num_lefts);
        if let ([left], [right]) = (lefts, rights) {
            // Non-conflicting diff shouldn't have identical contents
            ConflictDiffHunk {
                kind: hunk.kind,
                lefts: Merge::resolved(left),
                rights: Merge::resolved(right),
            }
        } else {
            let lefts = to_merge(lefts);
            let rights = to_merge(rights);
            let kind = match hunk.kind {
                DiffHunkKind::Matching => DiffHunkKind::Matching,
                DiffHunkKind::Different if lefts == rights => DiffHunkKind::Matching,
                DiffHunkKind::Different => DiffHunkKind::Different,
            };
            ConflictDiffHunk {
                kind,
                lefts,
                rights,
            }
        }
    })
}

/// Granularity of hunks when merging files.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileMergeHunkLevel {
    /// Splits into line hunks.
    Line,
    /// Splits into word hunks.
    Word,
}

/// Merge result in either fully-resolved or conflicts form, akin to
/// `Result<BString, Vec<Merge<BString>>>`.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum MergeResult {
    /// Resolved content if inputs can be merged successfully.
    Resolved(BString),
    /// List of partially-resolved hunks if some of them cannot be merged.
    Conflict(Vec<Merge<BString>>),
}

/// Splits `inputs` into hunks, resolves trivial merge conflicts for each.
///
/// Returns either fully-resolved content or list of partially-resolved hunks.
pub fn merge_hunks<T: AsRef<[u8]>>(inputs: &Merge<T>, options: &MergeOptions) -> MergeResult {
    merge_inner(inputs, options)
}

/// Splits `inputs` into hunks, resolves trivial merge conflicts for each, then
/// concatenates the outcome back to single `Merge` object.
///
/// The returned merge object is either fully resolved or conflict having the
/// same number of terms as the `inputs`.
pub fn merge<T: AsRef<[u8]>>(inputs: &Merge<T>, options: &MergeOptions) -> Merge<BString> {
    merge_inner(inputs, options)
}

/// Splits `inputs` into hunks, attempts to resolve trivial merge conflicts for
/// each.
///
/// If all input hunks can be merged successfully, returns the merged content.
pub fn try_merge<T: AsRef<[u8]>>(inputs: &Merge<T>, options: &MergeOptions) -> Option<BString> {
    merge_inner(inputs, options)
}

fn merge_inner<'input, T, B>(inputs: &'input Merge<T>, options: &MergeOptions) -> B
where
    T: AsRef<[u8]>,
    B: FromMergeHunks<'input>,
{
    // TODO: Using the first remove as base (first in the inputs) is how it's
    // usually done for 3-way conflicts. Are there better heuristics when there are
    // more than 3 parts?
    let num_diffs = inputs.removes().len();
    let diff = ContentDiff::by_line(inputs.removes().chain(inputs.adds()));
    let hunks = resolve_diff_hunks(&diff, num_diffs, options.same_change);
    match options.hunk_level {
        FileMergeHunkLevel::Line => B::from_hunks(hunks.map(MergeHunk::Borrowed)),
        FileMergeHunkLevel::Word => {
            B::from_hunks(hunks.map(|h| merge_hunk_by_word(h, options.same_change)))
        }
    }
}

fn merge_hunk_by_word(inputs: Merge<&BStr>, same_change: SameChange) -> MergeHunk<'_> {
    if inputs.is_resolved() {
        return MergeHunk::Borrowed(inputs);
    }
    let num_diffs = inputs.removes().len();
    let diff = ContentDiff::by_word(inputs.removes().chain(inputs.adds()));
    let hunks = resolve_diff_hunks(&diff, num_diffs, same_change);
    // We could instead use collect_merged() to return partially-merged hunk.
    // This would be more consistent with the line-based merge function, but
    // might produce surprising results. Partially-merged conflicts would be
    // hard to review because they would have mixed contexts.
    if let Some(content) = collect_resolved(hunks.map(MergeHunk::Borrowed)) {
        MergeHunk::Owned(Merge::resolved(content))
    } else {
        drop(diff);
        MergeHunk::Borrowed(inputs)
    }
}

/// `Cow`-like type over `Merge<T>`.
#[derive(Clone, Debug)]
enum MergeHunk<'input> {
    Borrowed(Merge<&'input BStr>),
    Owned(Merge<BString>),
}

impl MergeHunk<'_> {
    fn len(&self) -> usize {
        match self {
            MergeHunk::Borrowed(merge) => merge.as_slice().len(),
            MergeHunk::Owned(merge) => merge.as_slice().len(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &BStr> {
        match self {
            MergeHunk::Borrowed(merge) => Either::Left(merge.iter().copied()),
            MergeHunk::Owned(merge) => Either::Right(merge.iter().map(Borrow::borrow)),
        }
    }

    fn as_resolved(&self) -> Option<&BStr> {
        match self {
            MergeHunk::Borrowed(merge) => merge.as_resolved().copied(),
            MergeHunk::Owned(merge) => merge.as_resolved().map(Borrow::borrow),
        }
    }

    fn into_owned(self) -> Merge<BString> {
        match self {
            MergeHunk::Borrowed(merge) => merge.map(|&s| s.to_owned()),
            MergeHunk::Owned(merge) => merge,
        }
    }
}

/// `FromIterator` for merge result.
trait FromMergeHunks<'input>: Sized {
    fn from_hunks<I: IntoIterator<Item = MergeHunk<'input>>>(hunks: I) -> Self;
}

impl<'input> FromMergeHunks<'input> for MergeResult {
    fn from_hunks<I: IntoIterator<Item = MergeHunk<'input>>>(hunks: I) -> Self {
        collect_hunks(hunks)
    }
}

impl<'input> FromMergeHunks<'input> for Merge<BString> {
    fn from_hunks<I: IntoIterator<Item = MergeHunk<'input>>>(hunks: I) -> Self {
        collect_merged(hunks)
    }
}

impl<'input> FromMergeHunks<'input> for Option<BString> {
    fn from_hunks<I: IntoIterator<Item = MergeHunk<'input>>>(hunks: I) -> Self {
        collect_resolved(hunks)
    }
}

/// Collects merged hunks into either fully-resolved content or list of
/// partially-resolved hunks.
fn collect_hunks<'input>(hunks: impl IntoIterator<Item = MergeHunk<'input>>) -> MergeResult {
    let mut resolved_hunk = BString::new(vec![]);
    let mut merge_hunks: Vec<Merge<BString>> = vec![];
    for hunk in hunks {
        if let Some(content) = hunk.as_resolved() {
            resolved_hunk.extend_from_slice(content);
        } else {
            if !resolved_hunk.is_empty() {
                merge_hunks.push(Merge::resolved(resolved_hunk));
                resolved_hunk = BString::new(vec![]);
            }
            merge_hunks.push(hunk.into_owned());
        }
    }

    if merge_hunks.is_empty() {
        MergeResult::Resolved(resolved_hunk)
    } else {
        if !resolved_hunk.is_empty() {
            merge_hunks.push(Merge::resolved(resolved_hunk));
        }
        MergeResult::Conflict(merge_hunks)
    }
}

/// Collects merged hunks back to single `Merge` object, duplicating resolved
/// hunks to all positive and negative terms.
fn collect_merged<'input>(hunks: impl IntoIterator<Item = MergeHunk<'input>>) -> Merge<BString> {
    let mut maybe_resolved = Merge::resolved(BString::default());
    for hunk in hunks {
        if let Some(content) = hunk.as_resolved() {
            for buf in &mut maybe_resolved {
                buf.extend_from_slice(content);
            }
        } else {
            maybe_resolved = match maybe_resolved.into_resolved() {
                Ok(content) => Merge::from_vec(vec![content; hunk.len()]),
                Err(conflict) => conflict,
            };
            assert_eq!(maybe_resolved.as_slice().len(), hunk.len());
            for (buf, s) in iter::zip(&mut maybe_resolved, hunk.iter()) {
                buf.extend_from_slice(s);
            }
        }
    }
    maybe_resolved
}

/// Collects resolved merge hunks. Short-circuits on unresolved hunk.
fn collect_resolved<'input>(hunks: impl IntoIterator<Item = MergeHunk<'input>>) -> Option<BString> {
    let mut resolved_content = BString::default();
    for hunk in hunks {
        resolved_content.extend_from_slice(hunk.as_resolved()?);
    }
    Some(resolved_content)
}

/// Iterator that attempts to resolve trivial merge conflict for each hunk.
fn resolve_diff_hunks<'input>(
    diff: &ContentDiff<'input>,
    num_diffs: usize,
    same_change: SameChange,
) -> impl Iterator<Item = Merge<&'input BStr>> {
    diff.hunks().map(move |diff_hunk| match diff_hunk.kind {
        DiffHunkKind::Matching => {
            debug_assert!(diff_hunk.contents.iter().all_equal());
            Merge::resolved(diff_hunk.contents[0])
        }
        DiffHunkKind::Different => {
            let merge = Merge::from_removes_adds(
                diff_hunk.contents[..num_diffs].iter().copied(),
                diff_hunk.contents[num_diffs..].iter().copied(),
            );
            match merge.resolve_trivial(same_change) {
                Some(&content) => Merge::resolved(content),
                None => merge,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    fn conflict<const N: usize>(values: [&[u8]; N]) -> Merge<BString> {
        Merge::from_vec(values.map(hunk).to_vec())
    }

    fn resolved(value: &[u8]) -> Merge<BString> {
        Merge::resolved(hunk(value))
    }

    fn hunk(data: &[u8]) -> BString {
        data.into()
    }

    #[test]
    fn test_diff_line_iterator_line_numbers() {
        let mut line_iter = DiffLineIterator::with_line_number(
            [DiffHunk::different(["a\nb", "c\nd\n"])].into_iter(),
            DiffLineNumber { left: 1, right: 10 },
        );
        // Nothing queued
        assert_eq!(
            line_iter.next_line_number(),
            DiffLineNumber { left: 1, right: 10 }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 10 },
                hunks: vec![(DiffLineHunkSide::Left, "a\n".as_ref())],
            }
        );
        // Multiple lines queued
        assert_eq!(
            line_iter.next_line_number(),
            DiffLineNumber { left: 2, right: 10 }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 10 },
                hunks: vec![
                    (DiffLineHunkSide::Left, "b".as_ref()),
                    (DiffLineHunkSide::Right, "c\n".as_ref()),
                ],
            }
        );
        // Single line queued
        assert_eq!(
            line_iter.next_line_number(),
            DiffLineNumber { left: 2, right: 11 }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 11 },
                hunks: vec![(DiffLineHunkSide::Right, "d\n".as_ref())],
            }
        );
        // No more lines: left remains 2 as it lacks newline
        assert_eq!(
            line_iter.next_line_number(),
            DiffLineNumber { left: 2, right: 12 }
        );
        assert!(line_iter.next().is_none());
        assert_eq!(
            line_iter.next_line_number(),
            DiffLineNumber { left: 2, right: 12 }
        );
    }

    #[test]
    fn test_diff_line_iterator_blank_right_line_single_left() {
        let mut line_iter = DiffLineIterator::new(
            [
                DiffHunk::matching(["a"].repeat(2)),
                DiffHunk::different(["x\n", "\ny\n"]),
            ]
            .into_iter(),
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 1 },
                hunks: vec![
                    (DiffLineHunkSide::Both, "a".as_ref()),
                    (DiffLineHunkSide::Left, "x\n".as_ref()),
                ],
            }
        );
        // "\n" (line_number.right = 1) can be omitted because the previous diff
        // line has a right content.
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 2 },
                hunks: vec![(DiffLineHunkSide::Right, "y\n".as_ref())],
            }
        );
    }

    #[test]
    fn test_diff_line_iterator_blank_right_line_multiple_lefts() {
        let mut line_iter = DiffLineIterator::new(
            [
                DiffHunk::matching(["a"].repeat(2)),
                DiffHunk::different(["x\n\n", "\ny\n"]),
            ]
            .into_iter(),
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 1 },
                hunks: vec![
                    (DiffLineHunkSide::Both, "a".as_ref()),
                    (DiffLineHunkSide::Left, "x\n".as_ref()),
                ],
            }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 1 },
                hunks: vec![(DiffLineHunkSide::Left, "\n".as_ref())],
            }
        );
        // "\n" (line_number.right = 1) can still be omitted because one of the
        // preceding diff line has a right content.
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 3, right: 2 },
                hunks: vec![(DiffLineHunkSide::Right, "y\n".as_ref())],
            }
        );
    }

    #[test]
    fn test_diff_line_iterator_blank_right_line_after_non_empty_left() {
        let mut line_iter = DiffLineIterator::new(
            [
                DiffHunk::matching(["a"].repeat(2)),
                DiffHunk::different(["x\nz", "\ny\n"]),
            ]
            .into_iter(),
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 1 },
                hunks: vec![
                    (DiffLineHunkSide::Both, "a".as_ref()),
                    (DiffLineHunkSide::Left, "x\n".as_ref()),
                ],
            }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 1 },
                hunks: vec![
                    (DiffLineHunkSide::Left, "z".as_ref()),
                    (DiffLineHunkSide::Right, "\n".as_ref()),
                ],
            }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 2, right: 2 },
                hunks: vec![(DiffLineHunkSide::Right, "y\n".as_ref())],
            }
        );
    }

    #[test]
    fn test_diff_line_iterator_blank_right_line_without_preceding_lines() {
        let mut line_iter = DiffLineIterator::new([DiffHunk::different(["", "\ny\n"])].into_iter());
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 1 },
                hunks: vec![(DiffLineHunkSide::Right, "\n".as_ref())],
            }
        );
        assert_eq!(
            line_iter.next().unwrap(),
            DiffLine {
                line_number: DiffLineNumber { left: 1, right: 2 },
                hunks: vec![(DiffLineHunkSide::Right, "y\n".as_ref())],
            }
        );
    }

    #[test]
    fn test_conflict_diff_hunks_no_conflicts() {
        let diff_hunks = [
            DiffHunk::matching(["a\n"].repeat(2)),
            DiffHunk::different(["b\n", "c\n"]),
        ];
        let num_lefts = 1;
        insta::assert_debug_snapshot!(
            conflict_diff_hunks(&diff_hunks, num_lefts).collect_vec(), @r#"
        [
            ConflictDiffHunk {
                kind: Matching,
                lefts: Resolved(
                    "a\n",
                ),
                rights: Resolved(
                    "a\n",
                ),
            },
            ConflictDiffHunk {
                kind: Different,
                lefts: Resolved(
                    "b\n",
                ),
                rights: Resolved(
                    "c\n",
                ),
            },
        ]
        "#);
    }

    #[test]
    fn test_conflict_diff_hunks_simple_conflicts() {
        let diff_hunks = [
            // conflict hunk
            DiffHunk::different(["a\n", "X\n", "b\n", "c\n"]),
            DiffHunk::matching(["d\n"].repeat(4)),
            // non-conflict hunk
            DiffHunk::different(["e\n", "e\n", "e\n", "f\n"]),
        ];
        let num_lefts = 3;
        insta::assert_debug_snapshot!(
            conflict_diff_hunks(&diff_hunks, num_lefts).collect_vec(), @r#"
        [
            ConflictDiffHunk {
                kind: Different,
                lefts: Conflicted(
                    [
                        "a\n",
                        "X\n",
                        "b\n",
                    ],
                ),
                rights: Resolved(
                    "c\n",
                ),
            },
            ConflictDiffHunk {
                kind: Matching,
                lefts: Resolved(
                    "d\n",
                ),
                rights: Resolved(
                    "d\n",
                ),
            },
            ConflictDiffHunk {
                kind: Different,
                lefts: Resolved(
                    "e\n",
                ),
                rights: Resolved(
                    "f\n",
                ),
            },
        ]
        "#);
    }

    #[test]
    fn test_conflict_diff_hunks_matching_conflicts() {
        let diff_hunks = [
            // matching conflict hunk
            DiffHunk::different(["a\n", "X\n", "b\n", "a\n", "X\n", "b\n"]),
            DiffHunk::matching(["c\n"].repeat(6)),
        ];
        let num_lefts = 3;
        insta::assert_debug_snapshot!(
            conflict_diff_hunks(&diff_hunks, num_lefts).collect_vec(), @r#"
        [
            ConflictDiffHunk {
                kind: Matching,
                lefts: Conflicted(
                    [
                        "a\n",
                        "X\n",
                        "b\n",
                    ],
                ),
                rights: Conflicted(
                    [
                        "a\n",
                        "X\n",
                        "b\n",
                    ],
                ),
            },
            ConflictDiffHunk {
                kind: Matching,
                lefts: Resolved(
                    "c\n",
                ),
                rights: Resolved(
                    "c\n",
                ),
            },
        ]
        "#);
    }

    #[test]
    fn test_conflict_diff_hunks_no_trivial_resolution() {
        let diff_hunks = [DiffHunk::different(["a", "b", "a", "a"])];
        let num_lefts = 1;
        insta::assert_debug_snapshot!(
            conflict_diff_hunks(&diff_hunks, num_lefts).collect_vec(), @r#"
        [
            ConflictDiffHunk {
                kind: Different,
                lefts: Resolved(
                    "a",
                ),
                rights: Conflicted(
                    [
                        "b",
                        "a",
                        "a",
                    ],
                ),
            },
        ]
        "#);
        let num_lefts = 3;
        insta::assert_debug_snapshot!(
            conflict_diff_hunks(&diff_hunks, num_lefts).collect_vec(), @r#"
        [
            ConflictDiffHunk {
                kind: Different,
                lefts: Conflicted(
                    [
                        "a",
                        "b",
                        "a",
                    ],
                ),
                rights: Resolved(
                    "a",
                ),
            },
        ]
        "#);
    }

    #[test]
    fn test_merge_single_hunk() {
        let options = MergeOptions {
            hunk_level: FileMergeHunkLevel::Line,
            same_change: SameChange::Accept,
        };
        let merge_hunks = |inputs: &_| merge_hunks(inputs, &options);
        // Unchanged and empty on all sides
        assert_eq!(
            merge_hunks(&conflict([b"", b"", b""])),
            MergeResult::Resolved(hunk(b""))
        );
        // Unchanged on all sides
        assert_eq!(
            merge_hunks(&conflict([b"a", b"a", b"a"])),
            MergeResult::Resolved(hunk(b"a"))
        );
        // One side removed, one side unchanged
        assert_eq!(
            merge_hunks(&conflict([b"", b"a\n", b"a\n"])),
            MergeResult::Resolved(hunk(b""))
        );
        // One side unchanged, one side removed
        assert_eq!(
            merge_hunks(&conflict([b"a\n", b"a\n", b""])),
            MergeResult::Resolved(hunk(b""))
        );
        // Both sides removed same line
        assert_eq!(
            merge_hunks(&conflict([b"", b"a\n", b""])),
            MergeResult::Resolved(hunk(b""))
        );
        // One side modified, one side unchanged
        assert_eq!(
            merge_hunks(&conflict([b"a b", b"a", b"a"])),
            MergeResult::Resolved(hunk(b"a b"))
        );
        // One side unchanged, one side modified
        assert_eq!(
            merge_hunks(&conflict([b"a", b"a", b"a b"])),
            MergeResult::Resolved(hunk(b"a b"))
        );
        // All sides added same content
        assert_eq!(
            merge_hunks(&conflict([b"a\n", b"", b"a\n", b"", b"a\n"])),
            MergeResult::Resolved(hunk(b"a\n"))
        );
        // One side modified, two sides added
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"b", b"", b"b"])),
            MergeResult::Conflict(vec![conflict([b"b", b"a", b"b", b"", b"b"])])
        );
        // All sides removed same content
        assert_eq!(
            merge_hunks(&conflict([b"", b"a\n", b"", b"a\n", b"", b"a\n", b""])),
            MergeResult::Resolved(hunk(b""))
        );
        // One side modified, two sides removed
        assert_eq!(
            merge_hunks(&conflict([b"b\n", b"a\n", b"", b"a\n", b""])),
            MergeResult::Conflict(vec![conflict([b"b\n", b"a\n", b"", b"a\n", b""])])
        );
        // Three sides made the same change
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"b", b"a", b"b"])),
            MergeResult::Resolved(hunk(b"b"))
        );
        // One side removed, one side modified
        assert_eq!(
            merge_hunks(&conflict([b"", b"a\n", b"b\n"])),
            MergeResult::Conflict(vec![conflict([b"", b"a\n", b"b\n"])])
        );
        // One side modified, one side removed
        assert_eq!(
            merge_hunks(&conflict([b"b\n", b"a\n", b""])),
            MergeResult::Conflict(vec![conflict([b"b\n", b"a\n", b""])])
        );
        // Two sides modified in different ways
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"c"])),
            MergeResult::Conflict(vec![conflict([b"b", b"a", b"c"])])
        );
        // Two of three sides don't change, third side changes
        assert_eq!(
            merge_hunks(&conflict([b"a", b"a", b"", b"a", b"a"])),
            MergeResult::Resolved(hunk(b""))
        );
        // One side unchanged, two other sides make the same change
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"a", b"a", b"b"])),
            MergeResult::Resolved(hunk(b"b"))
        );
        // One side unchanged, two other sides make the different change
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"a", b"a", b"c"])),
            MergeResult::Conflict(vec![conflict([b"b", b"a", b"a", b"a", b"c"])])
        );
        // Merge of an unresolved conflict and another branch, where the other branch
        // undid the change from one of the inputs to the unresolved conflict in the
        // first.
        assert_eq!(
            merge_hunks(&conflict([b"b", b"a", b"a", b"b", b"c"])),
            MergeResult::Resolved(hunk(b"c"))
        );
        // Merge of an unresolved conflict and another branch.
        assert_eq!(
            merge_hunks(&conflict([b"c", b"a", b"d", b"b", b"e"])),
            MergeResult::Conflict(vec![conflict([b"c", b"a", b"d", b"b", b"e"])])
        );
        // Two sides made the same change, third side made a different change
        assert_eq!(
            merge_hunks(&conflict([b"c", b"a", b"c", b"b", b"c"])),
            MergeResult::Conflict(vec![conflict([b"c", b"a", b"c", b"b", b"c"])])
        );
    }

    #[test]
    fn test_merge_multi_hunk() {
        let options = MergeOptions {
            hunk_level: FileMergeHunkLevel::Line,
            same_change: SameChange::Accept,
        };
        let merge_hunks = |inputs: &_| merge_hunks(inputs, &options);
        let merge = |inputs: &_| merge(inputs, &options);
        let try_merge = |inputs: &_| try_merge(inputs, &options);
        // Two sides left one line unchanged, and added conflicting additional lines
        let inputs = conflict([b"a\nb\n", b"a\n", b"a\nc\n"]);
        assert_eq!(
            merge_hunks(&inputs),
            MergeResult::Conflict(vec![resolved(b"a\n"), conflict([b"b\n", b"", b"c\n"])])
        );
        assert_eq!(merge(&inputs), conflict([b"a\nb\n", b"a\n", b"a\nc\n"]));
        assert_eq!(try_merge(&inputs), None);

        // Two sides changed different lines: no conflict
        let inputs = conflict([b"a2\nb\nc\n", b"a\nb\nc\n", b"a\nb\nc2\n"]);
        assert_eq!(
            merge_hunks(&inputs),
            MergeResult::Resolved(hunk(b"a2\nb\nc2\n"))
        );
        assert_eq!(merge(&inputs), resolved(b"a2\nb\nc2\n"));
        assert_eq!(try_merge(&inputs), Some(hunk(b"a2\nb\nc2\n")));

        // Conflict with non-conflicting lines around
        let inputs = conflict([b"a\nb1\nc\n", b"a\nb\nc\n", b"a\nb2\nc\n"]);
        assert_eq!(
            merge_hunks(&inputs),
            MergeResult::Conflict(vec![
                resolved(b"a\n"),
                conflict([b"b1\n", b"b\n", b"b2\n"]),
                resolved(b"c\n"),
            ])
        );
        assert_eq!(
            merge(&inputs),
            conflict([b"a\nb1\nc\n", b"a\nb\nc\n", b"a\nb2\nc\n"])
        );
        assert_eq!(try_merge(&inputs), None);

        // Two conflict hunks, one can be resolved
        let inputs = conflict([b"a\nb\nc\n", b"a1\nb\nc\n", b"a2\nb\nc2\n"]);
        assert_eq!(
            merge_hunks(&inputs),
            MergeResult::Conflict(vec![
                conflict([b"a\n", b"a1\n", b"a2\n"]),
                resolved(b"b\nc2\n"),
            ])
        );
        assert_eq!(
            merge(&inputs),
            conflict([b"a\nb\nc2\n", b"a1\nb\nc2\n", b"a2\nb\nc2\n"])
        );
        assert_eq!(try_merge(&inputs), None);

        // One side changes a line and adds a block after. The other side just adds the
        // same block. You might expect the last block would be deduplicated. However,
        // the changes in the first side can be parsed as follows:
        // ```
        //  a {
        // -    p
        // +    q
        // +}
        // +
        // +b {
        // +    x
        //  }
        // ```
        // Therefore, the first side modifies the block `a { .. }`, and the second side
        // adds `b { .. }`. Git and Mercurial both duplicate the block in the result.
        let base = indoc! {b"
            a {
                p
            }
        "};
        let left = indoc! {b"
            a {
                q
            }

            b {
                x
            }
        "};
        let right = indoc! {b"
            a {
                p
            }

            b {
                x
            }
        "};
        let merged = indoc! {b"
            a {
                q
            }

            b {
                x
            }

            b {
                x
            }
        "};
        assert_eq!(merge(&conflict([left, base, right])), resolved(merged));
    }

    #[test]
    fn test_merge_hunk_by_word() {
        let options = MergeOptions {
            hunk_level: FileMergeHunkLevel::Word,
            same_change: SameChange::Accept,
        };
        let merge = |inputs: &_| merge(inputs, &options);
        // No context line in between, but "\n" is a context word
        assert_eq!(
            merge(&conflict([b"c\nb\n", b"a\nb\n", b"a\nd\n"])),
            resolved(b"c\nd\n")
        );
        // Both sides added to different positions
        assert_eq!(merge(&conflict([b"a b", b"a", b"c a"])), resolved(b"c a b"));
        // Both sides added to the same position: can't resolve word-level
        // conflicts and the whole line should be left as a conflict
        assert_eq!(
            merge(&conflict([b"a b", b"a", b"a c"])),
            conflict([b"a b", b"a", b"a c"])
        );
        // One side added, both sides added to the same position: the former
        // word-level conflict could be resolved, but we preserve the original
        // content in that case
        assert_eq!(
            merge(&conflict([b"a b", b"a", b"x a c"])),
            conflict([b"a b", b"a", b"x a c"])
        );
    }
}
