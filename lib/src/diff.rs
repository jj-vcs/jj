// Copyright 2021 The Jujutsu Authors
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

#![allow(missing_docs)]

use std::cmp::max;
use std::cmp::min;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::iter;
use std::ops::Range;
use std::slice;

use bstr::BStr;
use itertools::Itertools;

pub fn find_line_ranges(text: &[u8]) -> Vec<Range<usize>> {
    text.split_inclusive(|b| *b == b'\n')
        .scan(0, |total, line| {
            let start = *total;
            *total += line.len();
            Some(start..*total)
        })
        .collect()
}

fn is_word_byte(b: u8) -> bool {
    // TODO: Make this configurable (probably higher up in the call stack)
    matches!(
        b,
        // Count 0x80..0xff as word bytes so multi-byte UTF-8 chars are
        // treated as a single unit.
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'\x80'..=b'\xff'
    )
}

pub fn find_word_ranges(text: &[u8]) -> Vec<Range<usize>> {
    let mut word_ranges = vec![];
    let mut word_start_pos = 0;
    let mut in_word = false;
    for (i, b) in text.iter().enumerate() {
        if in_word && !is_word_byte(*b) {
            in_word = false;
            word_ranges.push(word_start_pos..i);
            word_start_pos = i;
        } else if !in_word && is_word_byte(*b) {
            in_word = true;
            word_start_pos = i;
        }
    }
    if in_word && word_start_pos < text.len() {
        word_ranges.push(word_start_pos..text.len());
    }
    word_ranges
}

pub fn find_nonword_ranges(text: &[u8]) -> Vec<Range<usize>> {
    text.iter()
        .positions(|b| !is_word_byte(*b))
        .map(|i| i..i + 1)
        .collect()
}

/// Index in a list of word (or token) ranges.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct WordPosition(usize);

#[derive(Clone, Debug)]
struct DiffSource<'input, 'aux> {
    text: &'input BStr,
    ranges: &'aux [Range<usize>],
}

impl<'input, 'aux> DiffSource<'input, 'aux> {
    fn new<T: AsRef<[u8]> + ?Sized>(text: &'input T, ranges: &'aux [Range<usize>]) -> Self {
        DiffSource {
            text: BStr::new(text),
            ranges,
        }
    }

    fn narrowed(&self, positions: Range<WordPosition>) -> Self {
        DiffSource {
            text: self.text,
            ranges: &self.ranges[positions.start.0..positions.end.0],
        }
    }

    fn range_at(&self, position: WordPosition) -> Range<usize> {
        self.ranges[position.0].clone()
    }
}

struct Histogram<'a> {
    word_to_positions: HashMap<&'a BStr, Vec<WordPosition>>,
    count_to_words: BTreeMap<usize, Vec<&'a BStr>>,
}

impl Histogram<'_> {
    fn calculate<'a>(source: &DiffSource<'a, '_>, max_occurrences: usize) -> Histogram<'a> {
        let mut word_to_positions: HashMap<&BStr, Vec<WordPosition>> = HashMap::new();
        for (i, range) in source.ranges.iter().enumerate() {
            let word = &source.text[range.clone()];
            let positions = word_to_positions.entry(word).or_default();
            // Allow one more than max_occurrences, so we can later skip those with more
            // than max_occurrences
            if positions.len() <= max_occurrences {
                positions.push(WordPosition(i));
            }
        }
        let mut count_to_words: BTreeMap<usize, Vec<&BStr>> = BTreeMap::new();
        for (word, ranges) in &word_to_positions {
            count_to_words.entry(ranges.len()).or_default().push(word);
        }
        Histogram {
            word_to_positions,
            count_to_words,
        }
    }
}

/// Finds the LCS given a array where the value of `input[i]` indicates that
/// the position of element `i` in the right array is at position `input[i]` in
/// the left array.
///
/// For example (some have multiple valid outputs):
///
/// [0,1,2] => [(0,0),(1,1),(2,2)]
/// [2,1,0] => [(0,2)]
/// [0,1,4,2,3,5,6] => [(0,0),(1,1),(2,3),(3,4),(5,5),(6,6)]
/// [0,1,4,3,2,5,6] => [(0,0),(1,1),(4,2),(5,5),(6,6)]
fn find_lcs(input: &[usize]) -> Vec<(usize, usize)> {
    if input.is_empty() {
        return vec![];
    }

    let mut chain = vec![(0, 0, 0); input.len()];
    let mut global_longest = 0;
    let mut global_longest_right_pos = 0;
    for (right_pos, &left_pos) in input.iter().enumerate() {
        let mut longest_from_here = 1;
        let mut previous_right_pos = usize::MAX;
        for i in (0..right_pos).rev() {
            let (previous_len, previous_left_pos, _) = chain[i];
            if previous_left_pos < left_pos {
                let len = previous_len + 1;
                if len > longest_from_here {
                    longest_from_here = len;
                    previous_right_pos = i;
                    if len > global_longest {
                        global_longest = len;
                        global_longest_right_pos = right_pos;
                        // If this is the longest chain globally so far, we cannot find a
                        // longer one by using a previous value, so break early.
                        break;
                    }
                }
            }
        }
        chain[right_pos] = (longest_from_here, left_pos, previous_right_pos);
    }

    let mut result = vec![];
    let mut right_pos = global_longest_right_pos;
    loop {
        let (_, left_pos, previous_right_pos) = chain[right_pos];
        result.push((left_pos, right_pos));
        if previous_right_pos == usize::MAX {
            break;
        }
        right_pos = previous_right_pos;
    }
    result.reverse();

    result
}

/// Finds unchanged ranges among the ones given as arguments. The data between
/// those ranges is ignored.
fn unchanged_ranges(left: &DiffSource, right: &DiffSource) -> Vec<(Range<usize>, Range<usize>)> {
    if left.ranges.is_empty() || right.ranges.is_empty() {
        return vec![];
    }

    // Prioritize LCS-based algorithm than leading/trailing matches
    let result = unchanged_ranges_lcs(left, right);
    if !result.is_empty() {
        return result;
    }

    // Trim leading common ranges (i.e. grow previous unchanged region)
    let common_leading_len = iter::zip(left.ranges, right.ranges)
        .take_while(|&(l, r)| left.text[l.clone()] == right.text[r.clone()])
        .count();
    let (left_leading_ranges, left_ranges) = left.ranges.split_at(common_leading_len);
    let (right_leading_ranges, right_ranges) = right.ranges.split_at(common_leading_len);

    // Trim trailing common ranges (i.e. grow next unchanged region)
    let common_trailing_len = iter::zip(left_ranges.iter().rev(), right_ranges.iter().rev())
        .take_while(|&(l, r)| left.text[l.clone()] == right.text[r.clone()])
        .count();
    let left_trailing_ranges = &left_ranges[(left_ranges.len() - common_trailing_len)..];
    let right_trailing_ranges = &right_ranges[(right_ranges.len() - common_trailing_len)..];

    itertools::chain(
        iter::zip(
            left_leading_ranges.iter().cloned(),
            right_leading_ranges.iter().cloned(),
        ),
        iter::zip(
            left_trailing_ranges.iter().cloned(),
            right_trailing_ranges.iter().cloned(),
        ),
    )
    .collect()
}

fn unchanged_ranges_lcs(
    left: &DiffSource,
    right: &DiffSource,
) -> Vec<(Range<usize>, Range<usize>)> {
    let max_occurrences = 100;
    let left_histogram = Histogram::calculate(left, max_occurrences);
    if *left_histogram.count_to_words.keys().next().unwrap() > max_occurrences {
        // If there are very many occurrences of all words, then we just give up.
        return vec![];
    }
    let right_histogram = Histogram::calculate(right, max_occurrences);
    // Look for words with few occurrences in `left` (could equally well have picked
    // `right`?). If any of them also occur in `right`, then we add the words to
    // the LCS.
    let Some(uncommon_shared_words) = left_histogram
        .count_to_words
        .iter()
        .map(|(left_count, left_words)| -> Vec<&BStr> {
            left_words
                .iter()
                .copied()
                .filter(|left_word| {
                    let right_count = right_histogram
                        .word_to_positions
                        .get(left_word)
                        .map_or(0, |right_positions| right_positions.len());
                    *left_count == right_count
                })
                .collect()
        })
        .find(|words| !words.is_empty())
    else {
        return vec![];
    };

    // [(index into ranges, serial to identify {word, occurrence #})]
    let (mut left_positions, mut right_positions): (Vec<_>, Vec<_>) = uncommon_shared_words
        .iter()
        .flat_map(|word| {
            let left_occurrences = &left_histogram.word_to_positions[word];
            let right_occurrences = &right_histogram.word_to_positions[word];
            assert_eq!(left_occurrences.len(), right_occurrences.len());
            iter::zip(left_occurrences, right_occurrences)
        })
        .enumerate()
        .map(|(serial, (&left_pos, &right_pos))| ((left_pos, serial), (right_pos, serial)))
        .unzip();
    left_positions.sort_unstable_by_key(|&(pos, _serial)| pos);
    right_positions.sort_unstable_by_key(|&(pos, _serial)| pos);
    let left_index_by_right_index: Vec<usize> = {
        let mut left_index_map = vec![0; left_positions.len()];
        for (i, &(_pos, serial)) in left_positions.iter().enumerate() {
            left_index_map[serial] = i;
        }
        right_positions
            .iter()
            .map(|&(_pos, serial)| left_index_map[serial])
            .collect()
    };

    let lcs = find_lcs(&left_index_by_right_index);

    // Produce output ranges, recursing into the modified areas between the elements
    // in the LCS.
    let mut result = vec![];
    let mut previous_left_position = WordPosition(0);
    let mut previous_right_position = WordPosition(0);
    for (left_index, right_index) in lcs {
        let (left_position, _) = left_positions[left_index];
        let (right_position, _) = right_positions[right_index];
        let skipped_left_positions = previous_left_position..left_position;
        let skipped_right_positions = previous_right_position..right_position;
        if !skipped_left_positions.is_empty() || !skipped_right_positions.is_empty() {
            for unchanged_nested_range in unchanged_ranges(
                &left.narrowed(skipped_left_positions.clone()),
                &right.narrowed(skipped_right_positions.clone()),
            ) {
                result.push(unchanged_nested_range);
            }
        }
        result.push((left.range_at(left_position), right.range_at(right_position)));
        previous_left_position = WordPosition(left_position.0 + 1);
        previous_right_position = WordPosition(right_position.0 + 1);
    }
    // Also recurse into range at end (after common ranges).
    let skipped_left_positions = previous_left_position..WordPosition(left.ranges.len());
    let skipped_right_positions = previous_right_position..WordPosition(right.ranges.len());
    if !skipped_left_positions.is_empty() || !skipped_right_positions.is_empty() {
        for unchanged_nested_range in unchanged_ranges(
            &left.narrowed(skipped_left_positions),
            &right.narrowed(skipped_right_positions),
        ) {
            result.push(unchanged_nested_range);
        }
    }

    result
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct UnchangedRange {
    base_range: Range<usize>,
    offsets: Vec<isize>,
}

impl UnchangedRange {
    fn start(&self, side: usize) -> usize {
        self.base_range
            .start
            .wrapping_add(self.offsets[side] as usize)
    }

    fn end(&self, side: usize) -> usize {
        self.base_range
            .end
            .wrapping_add(self.offsets[side] as usize)
    }
}

impl PartialOrd for UnchangedRange {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for UnchangedRange {
    fn cmp(&self, other: &Self) -> Ordering {
        self.base_range
            .start
            .cmp(&other.base_range.start)
            .then_with(|| self.base_range.end.cmp(&other.base_range.end))
    }
}

/// Takes any number of inputs and finds regions that are them same between all
/// of them.
#[derive(Clone, Debug)]
pub struct Diff<'input> {
    base_input: &'input BStr,
    other_inputs: Vec<&'input BStr>,
    // The key is a range in the base input. The value is the start of each non-base region
    // relative to the base region's start. By making them relative, they don't need to change
    // when the base range changes.
    unchanged_regions: Vec<UnchangedRange>,
}

/// Takes the current regions and intersects it with the new unchanged ranges
/// from a 2-way diff. The result is a map of unchanged regions with one more
/// offset in the map's values.
fn intersect_regions(
    current_ranges: Vec<UnchangedRange>,
    new_unchanged_ranges: &[(Range<usize>, Range<usize>)],
) -> Vec<UnchangedRange> {
    let mut result = vec![];
    let mut current_ranges_iter = current_ranges.into_iter().peekable();
    for (new_base_range, other_range) in new_unchanged_ranges.iter() {
        assert_eq!(new_base_range.len(), other_range.len());
        while let Some(UnchangedRange {
            base_range,
            offsets,
        }) = current_ranges_iter.peek()
        {
            // No need to look further if we're past the new range.
            if base_range.start >= new_base_range.end {
                break;
            }
            // Discard any current unchanged regions that don't match between the base and
            // the new input.
            if base_range.end <= new_base_range.start {
                current_ranges_iter.next();
                continue;
            }
            let new_start = max(base_range.start, new_base_range.start);
            let new_end = min(base_range.end, new_base_range.end);
            let mut new_offsets = offsets.clone();
            new_offsets.push(other_range.start.wrapping_sub(new_base_range.start) as isize);
            result.push(UnchangedRange {
                base_range: new_start..new_end,
                offsets: new_offsets,
            });
            if base_range.end >= new_base_range.end {
                // Break without consuming the item; there may be other new ranges that overlap
                // with it.
                break;
            }
            current_ranges_iter.next();
        }
    }
    result
}

impl<'input> Diff<'input> {
    pub fn for_tokenizer<T: AsRef<[u8]> + ?Sized + 'input>(
        inputs: impl IntoIterator<Item = &'input T>,
        tokenizer: impl Fn(&[u8]) -> Vec<Range<usize>>,
    ) -> Self {
        let mut inputs = inputs.into_iter().map(BStr::new);
        let base_input = inputs.next().expect("inputs must not be empty");
        let other_inputs = inputs.collect_vec();
        // First tokenize each input
        let base_token_ranges: Vec<Range<usize>>;
        let other_token_ranges: Vec<Vec<Range<usize>>>;
        // No need to tokenize if one of the inputs is empty. Non-empty inputs
        // are all different.
        if base_input.is_empty() || other_inputs.iter().any(|input| input.is_empty()) {
            base_token_ranges = vec![];
            other_token_ranges = iter::repeat(vec![]).take(other_inputs.len()).collect();
        } else {
            base_token_ranges = tokenizer(base_input);
            other_token_ranges = other_inputs
                .iter()
                .map(|other_input| tokenizer(other_input))
                .collect();
        }
        Self::with_inputs_and_token_ranges(
            base_input,
            other_inputs,
            &base_token_ranges,
            &other_token_ranges,
        )
    }

    fn with_inputs_and_token_ranges(
        base_input: &'input BStr,
        other_inputs: Vec<&'input BStr>,
        base_token_ranges: &[Range<usize>],
        other_token_ranges: &[Vec<Range<usize>>],
    ) -> Self {
        assert_eq!(other_inputs.len(), other_token_ranges.len());
        // Look for unchanged regions. Initially consider the whole range of the base
        // input as unchanged (compared to itself). Then diff each other input
        // against the base. Intersect the previously found ranges with the
        // unchanged ranges in the diff.
        let base_source = DiffSource::new(base_input, base_token_ranges);
        let mut unchanged_regions = vec![UnchangedRange {
            base_range: 0..base_input.len(),
            offsets: vec![],
        }];
        for (other_input, other_token_ranges) in iter::zip(&other_inputs, other_token_ranges) {
            let other_source = DiffSource::new(other_input, other_token_ranges);
            let unchanged_diff_ranges = unchanged_ranges(&base_source, &other_source);
            unchanged_regions = intersect_regions(unchanged_regions, &unchanged_diff_ranges);
        }
        // Add an empty range at the end to make life easier for hunks().
        let offsets = other_inputs
            .iter()
            .map(|input| input.len().wrapping_sub(base_input.len()) as isize)
            .collect_vec();
        unchanged_regions.push(UnchangedRange {
            base_range: base_input.len()..base_input.len(),
            offsets,
        });

        let mut diff = Self {
            base_input,
            other_inputs,
            unchanged_regions,
        };
        diff.compact_unchanged_regions();
        diff
    }

    pub fn unrefined<T: AsRef<[u8]> + ?Sized + 'input>(
        inputs: impl IntoIterator<Item = &'input T>,
    ) -> Self {
        Diff::for_tokenizer(inputs, |_| vec![])
    }

    /// Compares `inputs` line by line.
    pub fn by_line<T: AsRef<[u8]> + ?Sized + 'input>(
        inputs: impl IntoIterator<Item = &'input T>,
    ) -> Self {
        Diff::for_tokenizer(inputs, find_line_ranges)
    }

    /// Compares `inputs` word by word.
    ///
    /// The `inputs` is usually a changed hunk (e.g. a `DiffHunk::Different`)
    /// that was the output from a line-by-line diff.
    pub fn by_word<T: AsRef<[u8]> + ?Sized + 'input>(
        inputs: impl IntoIterator<Item = &'input T>,
    ) -> Self {
        let mut diff = Diff::for_tokenizer(inputs, find_word_ranges);
        diff.refine_changed_regions(find_nonword_ranges);
        diff
    }

    pub fn hunks<'diff>(&'diff self) -> DiffHunkIterator<'diff, 'input> {
        let previous_offsets = vec![0; self.other_inputs.len()];
        DiffHunkIterator {
            diff: self,
            previous: UnchangedRange {
                base_range: 0..0,
                offsets: previous_offsets,
            },
            unchanged_emitted: true,
            unchanged_iter: self.unchanged_regions.iter(),
        }
    }

    /// Uses the given tokenizer to split the changed regions into smaller
    /// regions. Then tries to finds unchanged regions among them.
    pub fn refine_changed_regions(&mut self, tokenizer: impl Fn(&[u8]) -> Vec<Range<usize>>) {
        let mut previous = UnchangedRange {
            base_range: 0..0,
            offsets: vec![0; self.other_inputs.len()],
        };
        let mut new_unchanged_ranges = vec![];
        for current in self.unchanged_regions.iter() {
            // For the changed region between the previous region and the current one,
            // create a new Diff instance. Then adjust the start positions and
            // offsets to be valid in the context of the larger Diff instance
            // (`self`).
            let mut slices =
                vec![&self.base_input[previous.base_range.end..current.base_range.start]];
            for i in 0..current.offsets.len() {
                let changed_range = previous.end(i)..current.start(i);
                slices.push(&self.other_inputs[i][changed_range]);
            }

            let refined_diff = Diff::for_tokenizer(slices, &tokenizer);

            for UnchangedRange {
                base_range,
                offsets,
            } in refined_diff.unchanged_regions
            {
                let new_base_start = base_range.start + previous.base_range.end;
                let new_base_end = base_range.end + previous.base_range.end;
                let offsets = iter::zip(offsets, &previous.offsets)
                    .map(|(refi, prev)| refi + prev)
                    .collect_vec();
                new_unchanged_ranges.push(UnchangedRange {
                    base_range: new_base_start..new_base_end,
                    offsets,
                });
            }
            previous = current.clone();
        }
        self.unchanged_regions = self
            .unchanged_regions
            .iter()
            .cloned()
            .merge(new_unchanged_ranges)
            .collect_vec();
        self.compact_unchanged_regions();
    }

    fn compact_unchanged_regions(&mut self) {
        let mut compacted = vec![];
        let mut maybe_previous: Option<UnchangedRange> = None;
        for current in self.unchanged_regions.iter() {
            if let Some(previous) = maybe_previous {
                if previous.base_range.end == current.base_range.start
                    && previous.offsets == *current.offsets
                {
                    maybe_previous = Some(UnchangedRange {
                        base_range: previous.base_range.start..current.base_range.end,
                        offsets: current.offsets.clone(),
                    });
                    continue;
                }
                compacted.push(previous);
            }
            maybe_previous = Some(current.clone());
        }
        if let Some(previous) = maybe_previous {
            compacted.push(previous);
        }
        self.unchanged_regions = compacted;
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum DiffHunk<'input> {
    Matching(&'input BStr),
    Different(Vec<&'input BStr>),
}

impl<'input> DiffHunk<'input> {
    pub fn matching<T: AsRef<[u8]> + ?Sized>(content: &'input T) -> Self {
        DiffHunk::Matching(BStr::new(content))
    }

    pub fn different<T: AsRef<[u8]> + ?Sized + 'input>(
        contents: impl IntoIterator<Item = &'input T>,
    ) -> Self {
        DiffHunk::Different(contents.into_iter().map(BStr::new).collect())
    }
}

pub struct DiffHunkIterator<'diff, 'input> {
    diff: &'diff Diff<'input>,
    previous: UnchangedRange,
    unchanged_emitted: bool,
    unchanged_iter: slice::Iter<'diff, UnchangedRange>,
}

impl<'diff, 'input> Iterator for DiffHunkIterator<'diff, 'input> {
    type Item = DiffHunk<'input>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.unchanged_emitted {
                self.unchanged_emitted = true;
                if !self.previous.base_range.is_empty() {
                    return Some(DiffHunk::Matching(
                        &self.diff.base_input[self.previous.base_range.clone()],
                    ));
                }
            }
            if let Some(current) = self.unchanged_iter.next() {
                let mut slices = vec![
                    &self.diff.base_input[self.previous.base_range.end..current.base_range.start],
                ];
                for (i, input) in self.diff.other_inputs.iter().enumerate() {
                    slices.push(&input[self.previous.end(i)..current.start(i)]);
                }
                self.previous = current.clone();
                self.unchanged_emitted = false;
                if slices.iter().any(|slice| !slice.is_empty()) {
                    return Some(DiffHunk::Different(slices));
                }
            } else {
                break;
            }
        }
        None
    }
}

/// Diffs slices of bytes. The returned diff hunks may be any length (may
/// span many lines or may be only part of a line). This currently uses
/// Histogram diff (or maybe something similar; I'm not sure I understood the
/// algorithm correctly). It first diffs lines in the input and then refines
/// the changed ranges at the word level.
pub fn diff<'a, T: AsRef<[u8]> + ?Sized + 'a>(
    inputs: impl IntoIterator<Item = &'a T>,
) -> Vec<DiffHunk<'a>> {
    let mut diff = Diff::for_tokenizer(inputs, find_line_ranges);
    diff.refine_changed_regions(find_word_ranges);
    diff.refine_changed_regions(find_nonword_ranges);
    diff.hunks().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Extracted to a function because type inference is ambiguous due to
    // `impl PartialEq<aho_corasick::util::search::Span> for std::ops::Range<usize>`
    fn no_ranges() -> Vec<Range<usize>> {
        vec![]
    }

    #[test]
    fn test_find_line_ranges_empty() {
        assert_eq!(find_line_ranges(b""), no_ranges());
    }

    #[test]
    fn test_find_line_ranges_blank_line() {
        assert_eq!(find_line_ranges(b"\n"), vec![0..1]);
    }

    #[test]
    fn test_find_line_ranges_missing_newline_at_eof() {
        assert_eq!(find_line_ranges(b"foo"), vec![0..3]);
    }

    #[test]
    fn test_find_line_ranges_multiple_lines() {
        assert_eq!(find_line_ranges(b"a\nbb\nccc\n"), vec![0..2, 2..5, 5..9]);
    }

    #[test]
    fn test_find_word_ranges_empty() {
        assert_eq!(find_word_ranges(b""), no_ranges());
    }

    #[test]
    fn test_find_word_ranges_single_word() {
        assert_eq!(find_word_ranges(b"Abc"), vec![0..3]);
    }

    #[test]
    fn test_find_word_ranges_no_word() {
        assert_eq!(find_word_ranges(b"+-*/"), no_ranges());
    }

    #[test]
    fn test_find_word_ranges_word_then_non_word() {
        assert_eq!(find_word_ranges(b"Abc   "), vec![0..3]);
    }

    #[test]
    fn test_find_word_ranges_non_word_then_word() {
        assert_eq!(find_word_ranges(b"   Abc"), vec![3..6]);
    }

    #[test]
    fn test_find_word_ranges_multibyte() {
        assert_eq!(find_word_ranges("⊢".as_bytes()), vec![0..3])
    }

    #[test]
    fn test_find_lcs_empty() {
        let empty: Vec<(usize, usize)> = vec![];
        assert_eq!(find_lcs(&[]), empty);
    }

    #[test]
    fn test_find_lcs_single_element() {
        assert_eq!(find_lcs(&[0]), vec![(0, 0)]);
    }

    #[test]
    fn test_find_lcs_in_order() {
        assert_eq!(find_lcs(&[0, 1, 2]), vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn test_find_lcs_reverse_order() {
        assert_eq!(find_lcs(&[2, 1, 0]), vec![(2, 0)]);
    }

    #[test]
    fn test_find_lcs_two_swapped() {
        assert_eq!(
            find_lcs(&[0, 1, 4, 3, 2, 5, 6]),
            vec![(0, 0), (1, 1), (2, 4), (5, 5), (6, 6)]
        );
    }

    #[test]
    fn test_find_lcs_element_moved_earlier() {
        assert_eq!(
            find_lcs(&[0, 1, 4, 2, 3, 5, 6]),
            vec![(0, 0), (1, 1), (2, 3), (3, 4), (5, 5), (6, 6)]
        );
    }

    #[test]
    fn test_find_lcs_element_moved_later() {
        assert_eq!(
            find_lcs(&[0, 1, 3, 4, 2, 5, 6]),
            vec![(0, 0), (1, 1), (3, 2), (4, 3), (5, 5), (6, 6)]
        );
    }

    #[test]
    fn test_find_lcs_interleaved_longest_chains() {
        assert_eq!(
            find_lcs(&[0, 4, 2, 9, 6, 5, 1, 3, 7, 8]),
            vec![(0, 0), (1, 6), (3, 7), (7, 8), (8, 9)]
        );
    }

    #[test]
    fn test_find_word_ranges_many_words() {
        assert_eq!(
            find_word_ranges(b"fn find_words(text: &[u8])"),
            vec![0..2, 3..13, 14..18, 22..24]
        );
    }

    #[test]
    fn test_unchanged_ranges_insert_in_middle() {
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a b b c", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a b X b c", &[0..1, 2..3, 4..5, 6..7, 8..9]),
            ),
            vec![(0..1, 0..1), (2..3, 2..3), (4..5, 6..7), (6..7, 8..9)]
        );
    }

    #[test]
    fn test_unchanged_ranges_non_unique_removed() {
        // We used to consider the first two "a" in the first input to match the two
        // "a"s in the second input. We no longer do.
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a b a c", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(0..1, 0..1)]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"b a c a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(6..7, 6..7)]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"b a a c", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a b c a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(0..1, 0..1), (6..7, 6..7)]
        );
    }

    #[test]
    fn test_unchanged_ranges_non_unique_added() {
        // We used to consider the first two "a" in the first input to match the two
        // "a"s in the second input. We no longer do.
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a b a c", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(0..1, 0..1)]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"b a c a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(6..7, 6..7)]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"b a a c", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![]
        );
        assert_eq!(
            unchanged_ranges(
                &DiffSource::new(b"a b c a", &[0..1, 2..3, 4..5, 6..7]),
                &DiffSource::new(b"a a a a", &[0..1, 2..3, 4..5, 6..7]),
            ),
            vec![(0..1, 0..1), (6..7, 6..7)]
        );
    }

    #[test]
    fn test_intersect_regions_existing_empty() {
        let actual = intersect_regions(vec![], &[(20..25, 55..60)]);
        let expected = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_intersect_regions_new_ranges_within_existing() {
        let actual = intersect_regions(
            vec![UnchangedRange {
                base_range: 20..70,
                offsets: vec![3],
            }],
            &[(25..30, 35..40), (40..50, 40..50)],
        );
        let expected = vec![
            UnchangedRange {
                base_range: 25..30,
                offsets: vec![3, 10],
            },
            UnchangedRange {
                base_range: 40..50,
                offsets: vec![3, 0],
            },
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_intersect_regions_partial_overlap() {
        let actual = intersect_regions(
            vec![UnchangedRange {
                base_range: 20..50,
                offsets: vec![-3],
            }],
            &[(15..25, 5..15), (45..60, 55..70)],
        );
        let expected = vec![
            UnchangedRange {
                base_range: 20..25,
                offsets: vec![-3, -10],
            },
            UnchangedRange {
                base_range: 45..50,
                offsets: vec![-3, 10],
            },
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_intersect_regions_new_range_overlaps_multiple_existing() {
        let actual = intersect_regions(
            vec![
                UnchangedRange {
                    base_range: 20..50,
                    offsets: vec![3, -8],
                },
                UnchangedRange {
                    base_range: 70..80,
                    offsets: vec![7, 1],
                },
            ],
            &[(10..100, 5..95)],
        );
        let expected = vec![
            UnchangedRange {
                base_range: 20..50,
                offsets: vec![3, -8, -5],
            },
            UnchangedRange {
                base_range: 70..80,
                offsets: vec![7, 1, -5],
            },
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_diff_single_input() {
        assert_eq!(diff(["abc"]), vec![DiffHunk::matching("abc")]);
    }

    #[test]
    fn test_diff_some_empty_inputs() {
        // All empty
        assert_eq!(diff([""]), vec![]);
        assert_eq!(diff(["", ""]), vec![]);
        assert_eq!(diff(["", "", ""]), vec![]);

        // One empty
        assert_eq!(diff(["a b", ""]), vec![DiffHunk::different(["a b", ""])]);
        assert_eq!(diff(["", "a b"]), vec![DiffHunk::different(["", "a b"])]);

        // One empty, two match
        assert_eq!(
            diff(["a b", "", "a b"]),
            vec![DiffHunk::different(["a b", "", "a b"])]
        );
        assert_eq!(
            diff(["", "a b", "a b"]),
            vec![DiffHunk::different(["", "a b", "a b"])]
        );

        // Two empty, one differs
        assert_eq!(
            diff(["a b", "", ""]),
            vec![DiffHunk::different(["a b", "", ""])]
        );
        assert_eq!(
            diff(["", "a b", ""]),
            vec![DiffHunk::different(["", "a b", ""])]
        );
    }

    #[test]
    fn test_diff_two_inputs_one_different() {
        assert_eq!(
            diff(["a b c", "a X c"]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["b", "X"]),
                DiffHunk::matching(" c"),
            ]
        );
    }

    #[test]
    fn test_diff_multiple_inputs_one_different() {
        assert_eq!(
            diff(["a b c", "a X c", "a b c"]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["b", "X", "b"]),
                DiffHunk::matching(" c"),
            ]
        );
    }

    #[test]
    fn test_diff_multiple_inputs_all_different() {
        assert_eq!(
            diff(["a b c", "a X c", "a c X"]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["b ", "X ", ""]),
                DiffHunk::matching("c"),
                DiffHunk::different(["", "", " X"]),
            ]
        );
    }

    #[test]
    fn test_diff_for_tokenizer_compacted() {
        // Tests that unchanged regions are compacted when using for_tokenizer()
        let diff = Diff::for_tokenizer(
            ["a\nb\nc\nd\ne\nf\ng", "a\nb\nc\nX\ne\nf\ng"],
            find_line_ranges,
        );
        assert_eq!(
            diff.hunks().collect_vec(),
            vec![
                DiffHunk::matching("a\nb\nc\n"),
                DiffHunk::different(["d\n", "X\n"]),
                DiffHunk::matching("e\nf\ng"),
            ]
        );
    }

    #[test]
    fn test_diff_nothing_in_common() {
        assert_eq!(
            diff(["aaa", "bb"]),
            vec![DiffHunk::different(["aaa", "bb"])]
        );
    }

    #[test]
    fn test_diff_insert_in_middle() {
        assert_eq!(
            diff(["a z", "a S z"]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["", "S "]),
                DiffHunk::matching("z"),
            ]
        );
    }

    #[test]
    fn test_diff_no_unique_middle_flips() {
        assert_eq!(
            diff(["a R R S S z", "a S S R R z"]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["R R ", ""]),
                DiffHunk::matching("S S "),
                DiffHunk::different(["", "R R "]),
                DiffHunk::matching("z")
            ],
        );
    }

    #[test]
    fn test_diff_recursion_needed() {
        assert_eq!(
            diff([
                "a q x q y q z q b q y q x q c",
                "a r r x q y z q b y q x r r c",
            ]),
            vec![
                DiffHunk::matching("a "),
                DiffHunk::different(["q", "r"]),
                DiffHunk::matching(" "),
                DiffHunk::different(["", "r "]),
                DiffHunk::matching("x q y "),
                DiffHunk::different(["q ", ""]),
                DiffHunk::matching("z q b "),
                DiffHunk::different(["q ", ""]),
                DiffHunk::matching("y q x "),
                DiffHunk::different(["q", "r"]),
                DiffHunk::matching(" "),
                DiffHunk::different(["", "r "]),
                DiffHunk::matching("c"),
            ]
        );
    }

    #[test]
    fn test_diff_real_case_write_fmt() {
        // This is from src/ui.rs in commit f44d246e3f88 in this repo. It highlights the
        // need for recursion into the range at the end: after splitting at "Arguments"
        // and "formatter", the region at the end has the unique words "write_fmt"
        // and "fmt", but we forgot to recurse into that region, so we ended up
        // saying that "write_fmt(fmt).unwrap()" was replaced by b"write_fmt(fmt)".
        #[rustfmt::skip]
        assert_eq!(
            diff([
                "    pub fn write_fmt(&mut self, fmt: fmt::Arguments<\'_>) {\n        self.styler().write_fmt(fmt).unwrap()\n",
                "    pub fn write_fmt(&mut self, fmt: fmt::Arguments<\'_>) -> io::Result<()> {\n        self.styler().write_fmt(fmt)\n"
            ]),
            vec![
                DiffHunk::matching("    pub fn write_fmt(&mut self, fmt: fmt::Arguments<\'_>) "),
                DiffHunk::different(["", "-> io::Result<()> "]),
                DiffHunk::matching("{\n        self.styler().write_fmt(fmt)"),
                DiffHunk::different([".unwrap()", ""]),
                DiffHunk::matching("\n")
            ]
        );
    }

    #[test]
    fn test_diff_real_case_gitgit_read_tree_c() {
        // This is the diff from commit e497ea2a9b in the git.git repo
        #[rustfmt::skip]
        assert_eq!(
            diff([
                r##"/*
 * GIT - The information manager from hell
 *
 * Copyright (C) Linus Torvalds, 2005
 */
#include "#cache.h"

static int unpack(unsigned char *sha1)
{
	void *buffer;
	unsigned long size;
	char type[20];

	buffer = read_sha1_file(sha1, type, &size);
	if (!buffer)
		usage("unable to read sha1 file");
	if (strcmp(type, "tree"))
		usage("expected a 'tree' node");
	while (size) {
		int len = strlen(buffer)+1;
		unsigned char *sha1 = buffer + len;
		char *path = strchr(buffer, ' ')+1;
		unsigned int mode;
		if (size < len + 20 || sscanf(buffer, "%o", &mode) != 1)
			usage("corrupt 'tree' file");
		buffer = sha1 + 20;
		size -= len + 20;
		printf("%o %s (%s)\n", mode, path, sha1_to_hex(sha1));
	}
	return 0;
}

int main(int argc, char **argv)
{
	int fd;
	unsigned char sha1[20];

	if (argc != 2)
		usage("read-tree <key>");
	if (get_sha1_hex(argv[1], sha1) < 0)
		usage("read-tree <key>");
	sha1_file_directory = getenv(DB_ENVIRONMENT);
	if (!sha1_file_directory)
		sha1_file_directory = DEFAULT_DB_ENVIRONMENT;
	if (unpack(sha1) < 0)
		usage("unpack failed");
	return 0;
}
"##,
                r##"/*
 * GIT - The information manager from hell
 *
 * Copyright (C) Linus Torvalds, 2005
 */
#include "#cache.h"

static void create_directories(const char *path)
{
	int len = strlen(path);
	char *buf = malloc(len + 1);
	const char *slash = path;

	while ((slash = strchr(slash+1, '/')) != NULL) {
		len = slash - path;
		memcpy(buf, path, len);
		buf[len] = 0;
		mkdir(buf, 0700);
	}
}

static int create_file(const char *path)
{
	int fd = open(path, O_WRONLY | O_TRUNC | O_CREAT, 0600);
	if (fd < 0) {
		if (errno == ENOENT) {
			create_directories(path);
			fd = open(path, O_WRONLY | O_TRUNC | O_CREAT, 0600);
		}
	}
	return fd;
}

static int unpack(unsigned char *sha1)
{
	void *buffer;
	unsigned long size;
	char type[20];

	buffer = read_sha1_file(sha1, type, &size);
	if (!buffer)
		usage("unable to read sha1 file");
	if (strcmp(type, "tree"))
		usage("expected a 'tree' node");
	while (size) {
		int len = strlen(buffer)+1;
		unsigned char *sha1 = buffer + len;
		char *path = strchr(buffer, ' ')+1;
		char *data;
		unsigned long filesize;
		unsigned int mode;
		int fd;

		if (size < len + 20 || sscanf(buffer, "%o", &mode) != 1)
			usage("corrupt 'tree' file");
		buffer = sha1 + 20;
		size -= len + 20;
		data = read_sha1_file(sha1, type, &filesize);
		if (!data || strcmp(type, "blob"))
			usage("tree file refers to bad file data");
		fd = create_file(path);
		if (fd < 0)
			usage("unable to create file");
		if (write(fd, data, filesize) != filesize)
			usage("unable to write file");
		fchmod(fd, mode);
		close(fd);
		free(data);
	}
	return 0;
}

int main(int argc, char **argv)
{
	int fd;
	unsigned char sha1[20];

	if (argc != 2)
		usage("read-tree <key>");
	if (get_sha1_hex(argv[1], sha1) < 0)
		usage("read-tree <key>");
	sha1_file_directory = getenv(DB_ENVIRONMENT);
	if (!sha1_file_directory)
		sha1_file_directory = DEFAULT_DB_ENVIRONMENT;
	if (unpack(sha1) < 0)
		usage("unpack failed");
	return 0;
}
"##,
            ]),
            vec![
               DiffHunk::matching("/*\n * GIT - The information manager from hell\n *\n * Copyright (C) Linus Torvalds, 2005\n */\n#include \"#cache.h\"\n\n"),
               DiffHunk::different(["", "static void create_directories(const char *path)\n{\n\tint len = strlen(path);\n\tchar *buf = malloc(len + 1);\n\tconst char *slash = path;\n\n\twhile ((slash = strchr(slash+1, \'/\')) != NULL) {\n\t\tlen = slash - path;\n\t\tmemcpy(buf, path, len);\n\t\tbuf[len] = 0;\n\t\tmkdir(buf, 0700);\n\t}\n}\n\nstatic int create_file(const char *path)\n{\n\tint fd = open(path, O_WRONLY | O_TRUNC | O_CREAT, 0600);\n\tif (fd < 0) {\n\t\tif (errno == ENOENT) {\n\t\t\tcreate_directories(path);\n\t\t\tfd = open(path, O_WRONLY | O_TRUNC | O_CREAT, 0600);\n\t\t}\n\t}\n\treturn fd;\n}\n\n"]),
               DiffHunk::matching("static int unpack(unsigned char *sha1)\n{\n\tvoid *buffer;\n\tunsigned long size;\n\tchar type[20];\n\n\tbuffer = read_sha1_file(sha1, type, &size);\n\tif (!buffer)\n\t\tusage(\"unable to read sha1 file\");\n\tif (strcmp(type, \"tree\"))\n\t\tusage(\"expected a \'tree\' node\");\n\twhile (size) {\n\t\tint len = strlen(buffer)+1;\n\t\tunsigned char *sha1 = buffer + len;\n\t\tchar *path = strchr(buffer, \' \')+1;\n"),
               DiffHunk::different(["", "\t\tchar *data;\n\t\tunsigned long filesize;\n"]),
               DiffHunk::matching("\t\tunsigned int mode;\n"),
               DiffHunk::different(["", "\t\tint fd;\n\n"]),
               DiffHunk::matching("\t\tif (size < len + 20 || sscanf(buffer, \"%o\", &mode) != 1)\n\t\t\tusage(\"corrupt \'tree\' file\");\n\t\tbuffer = sha1 + 20;\n\t\tsize -= len + 20;\n\t\t"),
               DiffHunk::different(["printf(\"%o %s (%s)\\n\", mode, path,", "data ="]),
               DiffHunk::matching(" "),
               DiffHunk::different(["sha1_to_hex", "read_sha1_file"]),
               DiffHunk::matching("(sha1"),
               DiffHunk::different([")", ", type, &filesize);\n\t\tif (!data || strcmp(type, \"blob\"))\n\t\t\tusage(\"tree file refers to bad file data\");\n\t\tfd = create_file(path);\n\t\tif (fd < 0)\n\t\t\tusage(\"unable to create file\");\n\t\tif (write(fd, data, filesize) != filesize)\n\t\t\tusage(\"unable to write file\");\n\t\tfchmod(fd, mode);\n\t\tclose(fd);\n\t\tfree(data"]),
               DiffHunk::matching(");\n\t}\n\treturn 0;\n}\n\nint main(int argc, char **argv)\n{\n\tint fd;\n\tunsigned char sha1[20];\n\n\tif (argc != 2)\n\t\tusage(\"read-tree <key>\");\n\tif (get_sha1_hex(argv[1], sha1) < 0)\n\t\tusage(\"read-tree <key>\");\n\tsha1_file_directory = getenv(DB_ENVIRONMENT);\n\tif (!sha1_file_directory)\n\t\tsha1_file_directory = DEFAULT_DB_ENVIRONMENT;\n\tif (unpack(sha1) < 0)\n\t\tusage(\"unpack failed\");\n\treturn 0;\n}\n"),
            ]
        );
    }
}
