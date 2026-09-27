// Copyright 2024 The Jujutsu Authors
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

//! Algorithm to split changes in a single source commit into its most relevant
//! ancestors, 'absorbing' them away.

use std::cmp;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use bstr::BString;
use futures::StreamExt as _;
use itertools::Itertools as _;
use thiserror::Error;

use crate::annotate::FileAnnotator;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::backend::TreeValue;
use crate::commit::Commit;
use crate::commit::conflict_label_for_commits;
use crate::conflicts::MaterializedFileValue;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialized_diff_stream;
use crate::copies::CopyRecords;
use crate::diff::ContentDiff;
use crate::diff::DiffHunkKind;
use crate::matchers::Matcher;
use crate::merge::Diff;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::merged_tree_builder::MergedTreeBuilder;
use crate::repo::MutableRepo;
use crate::repo::Repo;
use crate::repo_path::RepoPathBuf;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::RevsetEvaluationError;

/// The source commit to absorb into its ancestry.
#[derive(Clone, Debug)]
pub struct AbsorbSource {
    commit: Commit,
    parents: Vec<Commit>,
    parent_tree: MergedTree,
    source_tree: MergedTree,
}

impl AbsorbSource {
    /// Create an absorb source from a single commit.
    pub async fn from_commit(repo: &dyn Repo, commit: Commit) -> BackendResult<Self> {
        let parent_tree = commit.parent_tree(repo).await?;
        let source_tree = commit.tree();
        Self::from_tree(commit, source_tree, parent_tree).await
    }

    /// Create an absorb source from a commit and a tree derived from it.
    pub async fn from_tree(
        commit: Commit,
        source_tree: MergedTree,
        parent_tree: MergedTree,
    ) -> BackendResult<Self> {
        let parents = commit.parents().await?;
        Ok(Self {
            commit,
            parents,
            parent_tree,
            source_tree,
        })
    }
}

/// Error splitting an absorb source into modified ancestry trees.
#[derive(Debug, Error)]
pub enum AbsorbError {
    /// Error while contacting the Backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// Error resolving commit ancestry.
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
}

/// A hunk of the source revision which could not be absorbed into any
/// destination commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnabsorbedHunk {
    /// Path of the file the hunk belongs to.
    pub path: RepoPathBuf,
    /// 1-based range of the lines the hunk covers in the source revision file.
    /// The end is exclusive. For a deletion, this is the line the removed
    /// content was on, which can be past the end of the file.
    pub lines: Range<usize>,
    /// Reason the hunk could not be absorbed.
    pub reason: UnabsorbedReason,
}

/// Reason why a hunk of the source revision could not be absorbed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnabsorbedReason {
    /// The destination of the changed lines could not be determined, typically
    /// because some of the lines were modified by a commit outside the
    /// destination revisions.
    NoDestination,
    /// The hunk is an insertion between the lines of several commits, so it can
    /// be absorbed into any of them.
    ///
    /// The candidates are listed in the source order.
    Ambiguous(Vec<CommitId>),
    /// The hunk modifies the lines of several commits, so it can't be absorbed
    /// into a single destination.
    ///
    /// The candidates are listed in the source order.
    SpansMultipleCommits(Vec<CommitId>),
}

/// An absorb 'plan' indicating which commits should be modified and what they
/// should be modified to.
#[derive(Default)]
pub struct SelectedTrees {
    /// Commits to be modified, to be passed to `absorb_hunks`.
    pub target_commits: HashMap<CommitId, MergedTreeBuilder>,
    /// Paths that were not absorbed for various error reasons.
    pub skipped_paths: Vec<(RepoPathBuf, String)>,
    /// Hunks that were left in the source revision, in file order.
    pub unabsorbed_hunks: Vec<UnabsorbedHunk>,
}

/// Builds trees to be merged into destination commits by splitting source
/// changes based on file annotation.
pub async fn split_hunks_to_trees(
    repo: &dyn Repo,
    source: &AbsorbSource,
    destinations: &Arc<ResolvedRevsetExpression>,
    matcher: &dyn Matcher,
) -> Result<SelectedTrees, AbsorbError> {
    let mut selected_trees = SelectedTrees::default();

    let left_tree = &source.parent_tree;
    let right_tree = &source.source_tree;
    // TODO: enable copy tracking if we add support for annotate and merge
    let copy_records = CopyRecords::default();
    let tree_diff = left_tree.diff_stream_with_copies(right_tree, matcher, &copy_records);
    let mut diff_stream = materialized_diff_stream(
        repo.store(),
        tree_diff,
        Diff::new(left_tree.labels(), right_tree.labels()),
    );
    while let Some(entry) = diff_stream.next().await {
        let left_path = entry.path.source();
        let right_path = entry.path.target();
        let values = entry.values?;
        let (left_text, executable, copy_id) = match to_file_value(values.before) {
            Ok(Some(mut value)) => (
                value.read_all(left_path).await?,
                value.executable,
                value.copy_id,
            ),
            // New file should have no destinations
            Ok(None) => continue,
            Err(reason) => {
                selected_trees
                    .skipped_paths
                    .push((left_path.to_owned(), reason));
                continue;
            }
        };
        let (right_text, deleted) = match to_file_value(values.after) {
            Ok(Some(mut value)) => (value.read_all(right_path).await?, false),
            Ok(None) => (vec![], true),
            Err(reason) => {
                selected_trees
                    .skipped_paths
                    .push((right_path.to_owned(), reason));
                continue;
            }
        };

        // Compute annotation of parent (= left) content to map right hunks
        let mut annotator =
            FileAnnotator::with_file_content(source.commit.id(), left_path, left_text.clone());
        annotator.compute(repo, destinations).await?;
        let annotation = annotator.to_annotation();
        let annotation_ranges = annotation
            .compact_line_ranges()
            .filter_map(|(commit_id, range)| Some((commit_id.ok()?, range)))
            .collect_vec();
        let diff = ContentDiff::by_line([&left_text, &right_text]);
        let split_hunks = split_file_hunks(&annotation_ranges, &diff);
        let right_path = right_path.to_owned();
        selected_trees
            .unabsorbed_hunks
            .extend(
                split_hunks
                    .unabsorbed
                    .into_iter()
                    .map(|(range, reason)| UnabsorbedHunk {
                        path: right_path.clone(),
                        lines: line_range(&right_text, &range),
                        reason,
                    }),
            );
        let selected_ranges = split_hunks.selected;
        // Build trees containing parent (= left) contents + selected hunks
        for (&commit_id, ranges) in &selected_ranges {
            let tree_builder = selected_trees
                .target_commits
                .entry(commit_id.clone())
                .or_insert_with(|| MergedTreeBuilder::new(left_tree.clone()));
            let new_text = combine_texts(&left_text, &right_text, ranges);
            // Since changes to be absorbed are represented as diffs relative to
            // the source parent, we can propagate file deletion only if the
            // whole file content is deleted at a single destination commit.
            let new_tree_value = if new_text.is_empty() && deleted {
                Merge::absent()
            } else {
                let id = repo
                    .store()
                    .write_file(left_path, &mut new_text.as_slice())
                    .await?;
                Merge::normal(TreeValue::File {
                    id,
                    executable,
                    copy_id: copy_id.clone(),
                })
            };
            tree_builder.set_or_remove(left_path.to_owned(), new_tree_value);
        }
    }

    Ok(selected_trees)
}

type SelectedRange = (Range<usize>, Range<usize>);

/// Hunks of the source file split by destination commit.
struct SplitFileHunks<'a> {
    /// Ranges of the source (= right) text to be absorbed into each commit.
    selected: HashMap<&'a CommitId, Vec<SelectedRange>>,
    /// Hunks which couldn't be mapped to a destination commit, and the source
    /// range each of them covers.
    unabsorbed: Vec<(Range<usize>, UnabsorbedReason)>,
}

/// Maps `diff` hunks to commits based on the left `annotation_ranges`. The
/// `annotation_ranges` should be compacted.
fn split_file_hunks<'a>(
    annotation_ranges: &[(&'a CommitId, Range<usize>)],
    diff: &ContentDiff,
) -> SplitFileHunks<'a> {
    debug_assert!(annotation_ranges.iter().all(|(_, range)| !range.is_empty()));
    let mut selected_ranges: HashMap<&CommitId, Vec<_>> = HashMap::new();
    let mut unabsorbed = Vec::new();
    let mut diff_hunk_ranges = diff
        .hunk_ranges()
        .filter(|hunk| hunk.kind == DiffHunkKind::Different);
    let mut remaining_ranges = annotation_ranges;
    while !remaining_ranges.is_empty() {
        let Some(hunk) = diff_hunk_ranges.next() else {
            break;
        };
        let [left_range, right_range]: &[_; 2] = hunk.ranges[..].try_into().unwrap();
        assert!(!left_range.is_empty() || !right_range.is_empty());
        if right_range.is_empty() {
            // If the hunk is pure deletion, it can be mapped to multiple
            // overlapped annotation ranges unambiguously.
            let skip = remaining_ranges
                .iter()
                .take_while(|(_, range)| range.end <= left_range.start)
                .count();
            remaining_ranges = &remaining_ranges[skip..];
            let pre_overlap = remaining_ranges
                .iter()
                .take_while(|(_, range)| range.end < left_range.end)
                .count();
            let maybe_overlapped_ranges = remaining_ranges.get(..pre_overlap + 1);
            remaining_ranges = &remaining_ranges[pre_overlap..];
            let Some(overlapped_ranges) = maybe_overlapped_ranges else {
                unabsorbed.push((right_range.clone(), UnabsorbedReason::NoDestination));
                continue;
            };
            // Ensure that the ranges are contiguous and include the start.
            let all_covered = overlapped_ranges
                .iter()
                .try_fold(left_range.start, |prev_end, (_, cur)| {
                    (cur.start <= prev_end).then_some(cur.end)
                })
                .inspect(|&last_end| assert!(left_range.end <= last_end))
                .is_some();
            if all_covered {
                for (commit_id, cur_range) in overlapped_ranges {
                    let start = cmp::max(cur_range.start, left_range.start);
                    let end = cmp::min(cur_range.end, left_range.end);
                    assert!(start < end);
                    let selected = selected_ranges.entry(commit_id).or_default();
                    selected.push((start..end, right_range.clone()));
                }
            } else {
                // A deletion is mapped to all the overlapped ranges, so it can
                // only be rejected if some of the deleted lines are not
                // attributed to any destination.
                unabsorbed.push((right_range.clone(), UnabsorbedReason::NoDestination));
            }
        } else {
            // In other cases, the hunk should be included in an annotation
            // range to map it unambiguously. Skip any pre-overlapped ranges.
            let skip = remaining_ranges
                .iter()
                .take_while(|(_, range)| range.end < left_range.end)
                .count();
            remaining_ranges = &remaining_ranges[skip..];
            let Some((commit_id, cur_range)) = remaining_ranges.first() else {
                unabsorbed.push((right_range.clone(), UnabsorbedReason::NoDestination));
                continue;
            };
            let contained = cur_range.start <= left_range.start && left_range.end <= cur_range.end;
            // A pure insertion at the boundary of two annotation ranges can be
            // mapped to either of them, which is ambiguous.
            let ambiguous_with = remaining_ranges.get(1).filter(|(_, next_range)| {
                cur_range.end == left_range.start && next_range.start == left_range.end
            });
            if contained && ambiguous_with.is_none() {
                let selected = selected_ranges.entry(commit_id).or_default();
                selected.push((left_range.clone(), right_range.clone()));
            } else if let Some((next_commit_id, _)) = ambiguous_with {
                unabsorbed.push((
                    right_range.clone(),
                    UnabsorbedReason::Ambiguous(vec![
                        (*commit_id).clone(),
                        (*next_commit_id).clone(),
                    ]),
                ));
            } else {
                let candidates = overlapping_commits(annotation_ranges, left_range);
                let reason = match candidates.as_slice() {
                    // The hunk covers lines which were not modified by any
                    // destination, so there's no candidate to choose from.
                    [] | [_] => UnabsorbedReason::NoDestination,
                    _ => UnabsorbedReason::SpansMultipleCommits(candidates),
                };
                unabsorbed.push((right_range.clone(), reason));
            }
        }
    }
    // Hunks after the last annotation range can't be mapped to any commit.
    for hunk in diff_hunk_ranges {
        let [_, right_range]: &[_; 2] = hunk.ranges[..].try_into().unwrap();
        unabsorbed.push((right_range.clone(), UnabsorbedReason::NoDestination));
    }
    SplitFileHunks {
        selected: selected_ranges,
        unabsorbed,
    }
}

/// Commits of the annotation ranges overlapping the given range of the left (=
/// parent) text, in source order. An empty `range` denotes an insertion point
/// between lines. A commit annotating several ranges is listed once.
fn overlapping_commits(
    annotation_ranges: &[(&CommitId, Range<usize>)],
    range: &Range<usize>,
) -> Vec<CommitId> {
    annotation_ranges
        .iter()
        .filter(|(_, annotated)| {
            if range.is_empty() {
                annotated.start <= range.start && range.start <= annotated.end
            } else {
                annotated.start < range.end && range.start < annotated.end
            }
        })
        .map(|(commit_id, _)| (*commit_id).clone())
        .unique()
        .collect()
}

/// Converts a range of source file bytes to a 1-based range of line numbers.
///
/// An empty `range` denotes a deletion or an insertion point, and is mapped to
/// the single line it lands on.
fn line_range(text: &[u8], range: &Range<usize>) -> Range<usize> {
    let start = line_number_at(text, range.start);
    let end = if range.is_empty() {
        start
    } else {
        // The range may include the trailing newline of the last line.
        line_number_at(text, range.end - 1)
    };
    start..end + 1
}

fn line_number_at(text: &[u8], offset: usize) -> usize {
    1 + memchr::memchr_iter(b'\n', &text[..offset]).count()
}

/// Constructs new text by replacing `text1` range with `text2` range for each
/// selected `(range1, range2)` pairs.
fn combine_texts(text1: &[u8], text2: &[u8], selected_ranges: &[SelectedRange]) -> BString {
    itertools::chain!(
        [(0..0, 0..0)],
        selected_ranges.iter().cloned(),
        [(text1.len()..text1.len(), text2.len()..text2.len())],
    )
    .tuple_windows()
    // Copy unchanged hunk from text1 and current hunk from text2
    .map(|((prev1, _), (cur1, cur2))| (prev1.end..cur1.start, cur2))
    .flat_map(|(range1, range2)| [&text1[range1], &text2[range2]])
    .collect()
}

/// Describes changes made by [`absorb_hunks()`].
#[derive(Clone, Debug)]
pub struct AbsorbStats {
    /// Rewritten source commit which the absorbed hunks were removed, or `None`
    /// if the source commit was abandoned or no hunks were moved.
    pub rewritten_source: Option<Commit>,
    /// Rewritten commits which the source hunks were absorbed into, in forward
    /// topological order.
    pub rewritten_destinations: Vec<Commit>,
    /// Number of descendant commits which were rebased. The number of rewritten
    /// destination commits are not included.
    pub num_rebased: usize,
}

/// Merges selected trees into the specified commits. Abandons the source commit
/// if it becomes discardable.
pub async fn absorb_hunks(
    repo: &mut MutableRepo,
    source: &AbsorbSource,
    mut selected_trees: HashMap<CommitId, MergedTreeBuilder>,
) -> BackendResult<AbsorbStats> {
    let mut rewritten_source = None;
    let mut rewritten_destinations = Vec::new();
    let mut num_rebased = 0;
    let parents_label = conflict_label_for_commits(&source.parents);
    let source_commit_label = source.commit.conflict_label();
    // Rewrite commits in topological order so that descendant commits wouldn't
    // be rewritten multiple times.
    repo.transform_descendants(selected_trees.keys().cloned().collect(), async |rewriter| {
        // Remove selected hunks from the source commit by reparent()
        if rewriter.old_commit().id() == source.commit.id() {
            let commit_builder = rewriter.reparent();
            if commit_builder.is_discardable().await? {
                commit_builder.abandon();
            } else {
                rewritten_source = Some(commit_builder.write().await?);
                num_rebased += 1;
            }
            return Ok(());
        }
        let Some(tree_builder) = selected_trees.remove(rewriter.old_commit().id()) else {
            rewriter.rebase().await?.write().await?;
            num_rebased += 1;
            return Ok(());
        };
        // Merge hunks between source parent tree and selected tree
        let selected_tree = tree_builder.write_tree().await?;
        let destination_label = rewriter.old_commit().conflict_label();
        let commit_builder = rewriter.rebase().await?;
        let destination_tree = commit_builder.tree();
        let new_tree = MergedTree::merge(Merge::from_vec(vec![
            (
                destination_tree,
                format!("{destination_label} (absorb destination)"),
            ),
            (
                source.parent_tree.clone(),
                format!("{parents_label} (parents of absorbed revision)"),
            ),
            (
                selected_tree,
                format!("absorbed changes (from {source_commit_label})"),
            ),
        ]))
        .await?;
        let mut predecessors = commit_builder.predecessors().to_vec();
        predecessors.push(source.commit.id().clone());
        let new_commit = commit_builder
            .set_tree(new_tree)
            .set_predecessors(predecessors)
            .write()
            .await?;
        rewritten_destinations.push(new_commit);
        Ok(())
    })
    .await?;
    Ok(AbsorbStats {
        rewritten_source,
        rewritten_destinations,
        num_rebased,
    })
}

fn to_file_value(value: MaterializedTreeValue) -> Result<Option<MaterializedFileValue>, String> {
    match value {
        MaterializedTreeValue::Absent => Ok(None), // New or deleted file
        MaterializedTreeValue::AccessDenied(err) => Err(format!("Access is denied: {err}")),
        MaterializedTreeValue::File(file) => Ok(Some(file)),
        MaterializedTreeValue::Symlink { .. } => Err("Is a symlink".into()),
        MaterializedTreeValue::FileConflict(_) | MaterializedTreeValue::OtherConflict { .. } => {
            Err("Is a conflict".into())
        }
        MaterializedTreeValue::GitSubmodule(_) => Err("Is a Git submodule".into()),
        MaterializedTreeValue::Tree(_) => panic!("diff should not contain trees"),
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;

    use super::*;

    #[test]
    fn test_split_file_hunks_empty_or_single_line() {
        let commit_id1 = &CommitId::from_hex("111111");

        // unchanged
        assert_eq!(
            split_file_hunks(&[], &ContentDiff::by_line(["", ""])).selected,
            hashmap! {}
        );

        // insert single line
        assert_eq!(
            split_file_hunks(&[], &ContentDiff::by_line(["", "2X\n"])).selected,
            hashmap! {}
        );
        // delete single line
        assert_eq!(
            split_file_hunks(&[(commit_id1, 0..3)], &ContentDiff::by_line(["1a\n", ""])).selected,
            hashmap! { commit_id1 => vec![(0..3, 0..0)] }
        );
        // modify single line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..3)],
                &ContentDiff::by_line(["1a\n", "1AA\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..3, 0..4)] }
        );
    }

    #[test]
    fn test_split_file_hunks_single_range() {
        let commit_id1 = &CommitId::from_hex("111111");

        // insert first, middle, and last lines
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6)],
                &ContentDiff::by_line(["1a\n1b\n", "1X\n1a\n1Y\n1b\n1Z\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..0, 0..3), (3..3, 6..9), (6..6, 12..15)],
            }
        );
        // delete first, middle, and last lines
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..15)],
                &ContentDiff::by_line(["1a\n1b\n1c\n1d\n1e\n1f\n", "1b\n1d\n1f\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..0), (6..9, 3..3), (12..15, 6..6)],
            }
        );
        // modify non-contiguous lines
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..12)],
                &ContentDiff::by_line(["1a\n1b\n1c\n1d\n", "1A\n1b\n1C\n1d\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..3, 0..3), (6..9, 6..9)] }
        );
    }

    #[test]
    fn test_split_file_hunks_contiguous_ranges_insert() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // insert first line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1X\n1a\n1b\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..0, 0..3)] }
        );
        // insert middle line to first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1X\n1b\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..3, 3..6)] }
        );
        // insert middle line between ranges (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n3X\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // insert middle line to second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2a\n2X\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..9, 9..12)] }
        );
        // insert last line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2a\n2b\n2X\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(12..12, 12..15)] }
        );
    }

    #[test]
    fn test_split_file_hunks_contiguous_ranges_delete() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // delete first line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1b\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..3, 0..0)] }
        );
        // delete middle line from first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..3)] }
        );
        // delete middle line from second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(6..9, 6..6)] }
        );
        // delete last line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2a\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..12, 9..9)] }
        );
        // delete first and last lines
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1b\n2a\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..0)],
                commit_id2 => vec![(9..12, 6..6)],
            }
        );

        // delete across ranges (split first annotation range)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(3..6, 3..3)],
                commit_id2 => vec![(6..12, 3..3)],
            }
        );
        // delete middle lines across ranges (split both annotation ranges)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n2b\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(3..6, 3..3)],
                commit_id2 => vec![(6..9, 3..3)],
            }
        );
        // delete across ranges (split second annotation range)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "2b\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..6, 0..0)],
                commit_id2 => vec![(6..9, 0..0)],
            }
        );

        // delete all
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", ""])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..6, 0..0)],
                commit_id2 => vec![(6..12, 0..0)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_contiguous_ranges_modify() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // modify first line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n1b\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..3, 0..3)] }
        );
        // modify middle line of first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1B\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..6)] }
        );
        // modify middle lines of both ranges (ambiguous)
        // ('hg absorb' accepts this)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1B\n2A\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // modify middle line of second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2A\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(6..9, 6..9)] }
        );
        // modify last line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2a\n2B\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..12, 9..12)] }
        );
        // modify first and last lines
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n1b\n2a\n2B\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..3)],
                commit_id2 => vec![(9..12, 9..12)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_contiguous_ranges_modify_insert() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // modify first range, insert adjacent middle line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n1B\n1X\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..6, 0..9)] }
        );
        // modify second range, insert adjacent middle line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2X\n2A\n2B\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(6..12, 6..15)] }
        );
        // modify second range, insert last line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2A\n2B\n2X\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(6..12, 6..15)] }
        );
        // modify first and last lines (unambiguous), insert middle line between
        // ranges (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n1b\n3X\n2a\n2B\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..3)],
                commit_id2 => vec![(9..12, 12..15)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_contiguous_ranges_modify_delete() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // modify first line, delete adjacent middle line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..6, 0..3)] }
        );
        // modify last line, delete adjacent middle line
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n2B\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(6..12, 6..9)] }
        );
        // modify first and last lines, delete middle line from first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n2a\n2B\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..6, 0..3)],
                commit_id2 => vec![(9..12, 6..9)],
            }
        );
        // modify first and last lines, delete middle line from second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1A\n1b\n2B\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..3)],
                commit_id2 => vec![(6..12, 6..9)],
            }
        );
        // modify middle line, delete adjacent middle line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1B\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_insert() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // insert middle line to first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n1X\n0a\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(6..6, 6..9)] }
        );
        // insert middle line to second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0a\n2X\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..9, 9..12)] }
        );
        // insert middle lines to both ranges
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n1X\n0a\n2X\n2a\n2b\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(6..6, 6..9)],
                commit_id2 => vec![(9..9, 12..15)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_insert_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // insert middle line to first range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n1X\n0A\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // insert middle line to second range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0A\n2X\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // insert middle lines to both ranges, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n1X\n0A\n2X\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_delete() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // delete middle line from first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n0a\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..3)] }
        );
        // delete middle line from second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..12, 9..9)] }
        );
        // delete middle lines from both ranges
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n0a\n2b\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(3..6, 3..3)],
                commit_id2 => vec![(9..12, 6..6)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_delete_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // delete middle line from first range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n0A\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete middle line from second range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0A\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete middle lines from both ranges, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n0A\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_delete_delete_masked() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // 'hg absorb' accepts these, but it seems better to reject them as
        // ambiguous. Masked lines cannot be deleted.

        // delete middle line from first range, delete masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete middle line from second range, delete masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete middle lines from both ranges, delete masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_modify() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // modify middle line of first range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1B\n0a\n2a\n2b\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..6)] }
        );
        // modify middle line of second range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0a\n2A\n2b\n"])
            )
            .selected,
            hashmap! { commit_id2 => vec![(9..12, 9..12)] }
        );
        // modify middle lines of both ranges
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1B\n0a\n2A\n2b\n"])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(3..6, 3..6)],
                commit_id2 => vec![(9..12, 9..12)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_ranges_modify_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // modify middle line of first range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1B\n0A\n2a\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // modify middle line of second range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1b\n0A\n2A\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
        // modify middle lines to both ranges, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), /* 6..9, */ (commit_id2, 9..15)],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n1B\n0A\n2A\n2b\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_insert() {
        let commit_id1 = &CommitId::from_hex("111111");

        // insert middle line to range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n1b\n1X\n0a\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(6..6, 6..9)] }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_insert_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");

        // insert middle line to range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n1b\n1X\n0A\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_delete() {
        let commit_id1 = &CommitId::from_hex("111111");

        // delete middle line from range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n0a\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..3)] }
        );
        // delete all lines from range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "0a\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(0..6, 0..0)] }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_delete_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");

        // delete middle line from range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n0A\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete all lines from range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "0A\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_delete_delete_masked() {
        let commit_id1 = &CommitId::from_hex("111111");

        // 'hg absorb' accepts these, but it seems better to reject them as
        // ambiguous. Masked lines cannot be deleted.

        // delete middle line from range, delete masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n"])
            )
            .selected,
            hashmap! {}
        );
        // delete all lines from range, delete masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", ""])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_modify() {
        let commit_id1 = &CommitId::from_hex("111111");

        // modify middle line of range
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n1B\n0a\n"])
            )
            .selected,
            hashmap! { commit_id1 => vec![(3..6, 3..6)] }
        );
    }

    #[test]
    fn test_split_file_hunks_non_contiguous_tail_range_modify_modify_masked() {
        let commit_id1 = &CommitId::from_hex("111111");

        // modify middle line of range, modify masked line (ambiguous)
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* , 6..9 */],
                &ContentDiff::by_line(["1a\n1b\n0a\n", "1a\n1B\n0A\n"])
            )
            .selected,
            hashmap! {}
        );
    }

    #[test]
    fn test_split_file_hunks_multiple_edits() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");
        let commit_id3 = &CommitId::from_hex("333333");

        assert_eq!(
            split_file_hunks(
                &[
                    (commit_id1, 0..3),   // 1a       => 1A
                    (commit_id2, 3..6),   // 2a       => 2a
                    (commit_id1, 6..15),  // 1b 1c 1d => 1B 1d
                    (commit_id3, 15..21), // 3a 3b    => 3X 3A 3b 3Y
                ],
                &ContentDiff::by_line([
                    "1a\n2a\n1b\n1c\n1d\n3a\n3b\n",
                    "1A\n2a\n1B\n1d\n3X\n3A\n3b\n3Y\n"
                ])
            )
            .selected,
            hashmap! {
                commit_id1 => vec![(0..3, 0..3), (6..12, 6..9)],
                commit_id3 => vec![(15..18, 12..18), (21..21, 21..24)],
            }
        );
    }

    #[test]
    fn test_split_file_hunks_unabsorbed_no_destination() {
        let commit_id1 = &CommitId::from_hex("111111");

        // The file has no annotated lines, e.g. it was last modified by commits
        // outside the destination revisions.
        assert_eq!(
            split_file_hunks(&[], &ContentDiff::by_line(["1a\n", "1A\n"])).unabsorbed,
            vec![(0..3, UnabsorbedReason::NoDestination)]
        );

        // The hunk is after the last annotation range.
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..3)],
                &ContentDiff::by_line(["1a\n1b\n", "1a\n1B\n"])
            )
            .unabsorbed,
            vec![(3..6, UnabsorbedReason::NoDestination)]
        );

        // The hunk overlaps an annotation range but also covers lines which
        // aren't part of it.
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 3..6)],
                &ContentDiff::by_line(["1a\n1b\n1c\n", "1A\n1B\n1C\n"])
            )
            .unabsorbed,
            vec![(0..9, UnabsorbedReason::NoDestination)]
        );

        // The hunk after the annotation ranges have been exhausted by a
        // deletion.
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..3)],
                &ContentDiff::by_line(["1a\n1b\n1c\n0a\n1d\n", "1a\n0a\n1D\n"])
            )
            .unabsorbed,
            vec![
                (3..3, UnabsorbedReason::NoDestination),
                (6..9, UnabsorbedReason::NoDestination),
            ]
        );

        // A deletion of lines which aren't all attributed to a destination
        // can't be mapped to any commit.
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6) /* 6..9, */,],
                &ContentDiff::by_line(["1a\n1b\n0a\n2a\n2b\n", "1a\n2b\n"])
            )
            .unabsorbed,
            vec![(3..3, UnabsorbedReason::NoDestination)]
        );
    }

    #[test]
    fn test_split_file_hunks_partially_absorbed() {
        let commit_id1 = &CommitId::from_hex("111111");

        // The first line is attributed to the commit 1, but the last line isn't
        // attributed to any destination.
        let hunks = split_file_hunks(
            &[(commit_id1, 0..3)],
            &ContentDiff::by_line(["1a\n1b\n1c\n", "1A\n1b\n1C\n"]),
        );
        assert_eq!(
            hunks.selected,
            hashmap! { commit_id1 => vec![(0..3, 0..3)] }
        );
        assert_eq!(
            hunks.unabsorbed,
            vec![(6..9, UnabsorbedReason::NoDestination)]
        );
    }

    #[test]
    fn test_split_file_hunks_unabsorbed_ambiguous() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");

        // Insertion between the lines of two annotation ranges
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1b\n3X\n2a\n2b\n"])
            )
            .unabsorbed,
            vec![(
                6..9,
                UnabsorbedReason::Ambiguous(vec![commit_id1.clone(), commit_id2.clone()])
            )]
        );
    }

    #[test]
    fn test_split_file_hunks_unabsorbed_spans_multiple_commits() {
        let commit_id1 = &CommitId::from_hex("111111");
        let commit_id2 = &CommitId::from_hex("222222");
        let spans = |commit_ids: Vec<CommitId>| UnabsorbedReason::SpansMultipleCommits(commit_ids);
        let both = || vec![commit_id1.clone(), commit_id2.clone()];

        // Modification of lines from two adjacent annotation ranges
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..6), (commit_id2, 6..12)],
                &ContentDiff::by_line(["1a\n1b\n2a\n2b\n", "1a\n1B\n2A\n2b\n"])
            )
            .unabsorbed,
            vec![(3..9, spans(both()))]
        );

        // Modification of lines from two annotation ranges separated by lines
        // which aren't attributed to any destination
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..3), /* 3..6, */ (commit_id2, 6..9)],
                &ContentDiff::by_line(["1a\n0a\n0b\n", "1A\n0A\n0B\n"])
            )
            .unabsorbed,
            vec![(0..9, spans(both()))]
        );

        // A commit annotating several of the overlapped ranges is only listed
        // once
        assert_eq!(
            split_file_hunks(
                &[(commit_id1, 0..3), (commit_id2, 3..6), (commit_id1, 6..15)],
                &ContentDiff::by_line(["1a\n2a\n1b\n1c\n1d\n", "1A\n2A\n1B\n1c\n1d\n"])
            )
            .unabsorbed,
            vec![(0..9, spans(both()))]
        );
    }

    #[test]
    fn test_line_range() {
        let text = b"1a\n1b\n1c\n";

        // Modification of the second line
        assert_eq!(line_range(text, &(3..6)), 2..3);
        // Modification of the first and last lines
        assert_eq!(line_range(text, &(0..3)), 1..2);
        assert_eq!(line_range(text, &(6..9)), 3..4);
        // Insertion between lines
        assert_eq!(line_range(text, &(6..6)), 3..4);
        // Insertion at the end of the file
        assert_eq!(line_range(text, &(9..9)), 4..5);

        // Deletion of the second line
        assert_eq!(line_range(b"1a\n1b\n1c\n", &(3..3)), 2..3);
        // Deletion of the last line, which was on the line past the end of the
        // resulting file
        assert_eq!(line_range(b"1a\n", &(3..3)), 2..3);

        // File without a trailing newline
        assert_eq!(line_range(b"1a", &(0..2)), 1..2);
        // Empty file
        assert_eq!(line_range(b"", &(0..0)), 1..2);
    }

    #[test]
    fn test_combine_texts() {
        assert_eq!(combine_texts(b"", b"", &[]), "");
        assert_eq!(combine_texts(b"foo", b"bar", &[]), "foo");
        assert_eq!(combine_texts(b"foo", b"bar", &[(0..3, 0..3)]), "bar");

        assert_eq!(
            combine_texts(
                b"1a\n2a\n1b\n1c\n1d\n3a\n3b\n",
                b"1A\n2a\n1B\n1d\n3X\n3A\n3b\n3Y\n",
                &[(0..3, 0..3), (6..12, 6..9)]
            ),
            "1A\n2a\n1B\n1d\n3a\n3b\n"
        );
        assert_eq!(
            combine_texts(
                b"1a\n2a\n1b\n1c\n1d\n3a\n3b\n",
                b"1A\n2a\n1B\n1d\n3X\n3A\n3b\n3Y\n",
                &[(15..18, 12..18), (21..21, 21..24)]
            ),
            "1a\n2a\n1b\n1c\n1d\n3X\n3A\n3b\n3Y\n"
        );
    }
}
