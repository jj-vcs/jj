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

//! Methods that allow annotation (attribution and blame) for a file in a
//! repository.
//!
//! TODO: Add support for different blame layers with a trait in the future.
//! Like commit metadata and more.

use std::collections::HashMap;
use std::collections::hash_map;
use std::iter;
use std::ops::Range;
use std::sync::Arc;

use bstr::BStr;
use bstr::BString;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::conflicts::ConflictMarkerStyle;
use crate::conflicts::ConflictMaterializeOptions;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialize_merge_result_to_bytes;
use crate::conflicts::materialize_tree_value;
use crate::diff::ContentDiff;
use crate::diff::DiffHunkKind;
use crate::files::FileMergeHunkLevel;
use crate::fileset::FilesetExpression;
use crate::graph::GraphEdge;
use crate::merge::SameChange;
use crate::merged_tree::MergedTree;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetFilterPredicate;
use crate::store::Store;
use crate::tree_merge::MergeOptions;

/// Annotation results for a specific file
#[derive(Clone, Debug)]
pub struct FileAnnotation {
    line_map: OriginalLineMap,
    text: BString,
}

impl FileAnnotation {
    /// Returns iterator over `(line_origin, line)`s.
    ///
    /// For each line, `Ok(line_origin)` returns information about the
    /// originator commit of the line. If no originator commit was found
    /// within the domain, `Err(line_origin)` should be set. It points to the
    /// commit outside of the domain where the search stopped.
    ///
    /// The `line` includes newline character.
    pub fn line_origins(&self) -> impl Iterator<Item = (Result<&LineOrigin, &LineOrigin>, &BStr)> {
        itertools::zip_eq(&self.line_map, self.text.split_inclusive(|b| *b == b'\n'))
            .map(|(line_origin, line)| (line_origin.as_ref(), line.as_ref()))
    }

    /// Returns iterator over `(commit_id, line)`s.
    ///
    /// For each line, `Ok(commit_id)` points to the originator commit of the
    /// line. If no originator commit was found within the domain,
    /// `Err(commit_id)` should be set. It points to the commit outside of the
    /// domain where the search stopped.
    ///
    /// The `line` includes newline character.
    pub fn lines(&self) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, &BStr)> {
        itertools::zip_eq(
            self.commit_ids(),
            self.text
                .split_inclusive(|b| *b == b'\n')
                .map(AsRef::as_ref),
        )
    }

    /// Returns iterator over `(commit_id, line_range)`s.
    ///
    /// See [`Self::lines()`] for `commit_id`s.
    ///
    /// The `line_range` is a slice range in the file `text`. Consecutive ranges
    /// having the same `commit_id` are not compacted.
    pub fn line_ranges(
        &self,
    ) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, Range<usize>)> {
        let ranges = self
            .text
            .split_inclusive(|b| *b == b'\n')
            .scan(0, |total, line| {
                let start = *total;
                *total += line.len();
                Some(start..*total)
            });
        itertools::zip_eq(self.commit_ids(), ranges)
    }

    /// Returns iterator over compacted `(commit_id, line_range)`s.
    ///
    /// Consecutive ranges having the same `commit_id` are merged into one.
    pub fn compact_line_ranges(
        &self,
    ) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, Range<usize>)> {
        let mut ranges = self.line_ranges();
        let mut acc = ranges.next();
        iter::from_fn(move || {
            let (acc_commit_id, acc_range) = acc.as_mut()?;
            for (cur_commit_id, cur_range) in ranges.by_ref() {
                if *acc_commit_id == cur_commit_id {
                    acc_range.end = cur_range.end;
                } else {
                    return acc.replace((cur_commit_id, cur_range));
                }
            }
            acc.take()
        })
    }

    /// File content at the starting commit.
    pub fn text(&self) -> &BStr {
        self.text.as_ref()
    }

    fn commit_ids(&self) -> impl Iterator<Item = Result<&CommitId, &CommitId>> {
        self.line_map.iter().map(|line_origin| {
            line_origin
                .as_ref()
                .map(|origin| &origin.commit_id)
                .map_err(|origin| &origin.commit_id)
        })
    }
}

/// Annotation process for a specific file.
#[derive(Clone, Debug)]
pub struct FileAnnotator {
    // If we add copy-tracing support, file_path might be tracked by state.
    file_path: RepoPathBuf,
    starting_text: BString,
    state: AnnotationState,
}

impl FileAnnotator {
    /// Initializes annotator for a specific file in the `starting_commit`.
    ///
    /// If the file is not found, the result would be empty.
    pub fn from_commit(starting_commit: &Commit, file_path: &RepoPath) -> BackendResult<Self> {
        let source = Source::load(starting_commit, file_path)?;
        Ok(Self::with_source(starting_commit.id(), file_path, source))
    }

    /// Initializes annotator for a specific file path starting with the given
    /// content.
    ///
    /// The file content at the `starting_commit` is set to `starting_text`.
    /// This is typically one of the file contents in the conflict or
    /// merged-parent tree.
    pub fn with_file_content(
        starting_commit_id: &CommitId,
        file_path: &RepoPath,
        starting_text: impl Into<Vec<u8>>,
    ) -> Self {
        let source = Source::new(BString::new(starting_text.into()));
        Self::with_source(starting_commit_id, file_path, source)
    }

    fn with_source(
        starting_commit_id: &CommitId,
        file_path: &RepoPath,
        mut source: Source,
    ) -> Self {
        source.fill_line_map();
        let starting_text = source.text.clone();
        let state = AnnotationState {
            original_line_map: (0..source.line_map.len())
                .map(|line_number| {
                    Err(LineOrigin {
                        commit_id: starting_commit_id.clone(),
                        line_number,
                    })
                })
                .collect(),
            commit_source_map: HashMap::from([(starting_commit_id.clone(), source)]),
            num_unresolved_roots: 0,
        };
        Self {
            file_path: file_path.to_owned(),
            starting_text,
            state,
        }
    }

    /// Computes line-by-line annotation within the `domain`.
    ///
    /// The `domain` expression narrows the range of ancestors to search. It
    /// will be intersected as `domain & ::pending_commits & files(file_path)`.
    /// The `pending_commits` is assumed to be included in the `domain`.
    pub fn compute(
        &mut self,
        repo: &dyn Repo,
        domain: &Arc<ResolvedRevsetExpression>,
    ) -> Result<(), RevsetEvaluationError> {
        process_commits(repo, &mut self.state, domain, &self.file_path)
    }

    /// Remaining commit ids to visit from.
    pub fn pending_commits(&self) -> impl Iterator<Item = &CommitId> {
        self.state.commit_source_map.keys()
    }

    /// Returns the current state as line-oriented annotation.
    pub fn to_annotation(&self) -> FileAnnotation {
        // Just clone the line map. We might want to change the underlying data
        // model something akin to interleaved delta in order to get annotation
        // at a certain ancestor commit without recomputing.
        FileAnnotation {
            line_map: self.state.original_line_map.clone(),
            text: self.starting_text.clone(),
        }
    }
}

/// Intermediate state of file annotation.
#[derive(Clone, Debug)]
struct AnnotationState {
    original_line_map: OriginalLineMap,
    /// Commits to file line mappings and contents.
    commit_source_map: HashMap<CommitId, Source>,
    /// Number of unresolved root commits in `commit_source_map`.
    num_unresolved_roots: usize,
}

/// Line mapping and file content at a certain commit.
#[derive(Clone, Debug)]
struct Source {
    /// Mapping of line numbers in the file at the current commit to the
    /// starting file, sorted by the line numbers at the current commit.
    line_map: Vec<(usize, usize)>,
    /// File content at the current commit.
    text: BString,
}

impl Source {
    fn new(text: BString) -> Self {
        Self {
            line_map: Vec::new(),
            text,
        }
    }

    fn load(commit: &Commit, file_path: &RepoPath) -> Result<Self, BackendError> {
        let tree = commit.tree();
        let text = get_file_contents(commit.store(), file_path, &tree).block_on()?;
        Ok(Self::new(text))
    }

    fn fill_line_map(&mut self) {
        let lines = self.text.split_inclusive(|b| *b == b'\n');
        self.line_map = lines.enumerate().map(|(i, _)| (i, i)).collect();
    }
}

/// List of origins for each line, indexed by line numbers in the
/// starting file.
type OriginalLineMap = Vec<Result<LineOrigin, LineOrigin>>;

/// Information about the origin of an annotated line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineOrigin {
    /// Commit ID where the line was introduced.
    pub commit_id: CommitId,
    /// 0-based line number of the line in the origin commit.
    pub line_number: usize,
}

/// Starting from the source commits, compute changes at that commit relative to
/// its direct parents, updating the mappings as we go.
fn process_commits(
    repo: &dyn Repo,
    state: &mut AnnotationState,
    domain: &Arc<ResolvedRevsetExpression>,
    file_name: &RepoPath,
) -> Result<(), RevsetEvaluationError> {
    let predicate = RevsetFilterPredicate::File(FilesetExpression::file_path(file_name.to_owned()));
    // TODO: If the domain isn't a contiguous range, changes masked out by it
    // might not be caught by the closest ancestor revision. For example,
    // domain=merges() would pick up almost nothing because merge revisions
    // are usually empty. Perhaps, we want to query `files(file_path,
    // within_sub_graph=domain)`, not `domain & files(file_path)`.
    let heads = RevsetExpression::commits(state.commit_source_map.keys().cloned().collect());
    let revset = heads
        .union(&domain.intersection(&heads.ancestors()).filtered(predicate))
        .evaluate(repo)?;

    state.num_unresolved_roots = 0;
    for node in revset.iter_graph() {
        let (commit_id, edge_list) = node?;
        process_commit(repo, file_name, state, &commit_id, &edge_list)?;
        if state.commit_source_map.len() == state.num_unresolved_roots {
            // No more lines to propagate to ancestors.
            break;
        }
    }
    Ok(())
}

/// For a given commit, for each parent, we compare the version in the parent
/// tree with the current version, updating the mappings for any lines in
/// common. If the parent doesn't have the file, we skip it.
fn process_commit(
    repo: &dyn Repo,
    file_name: &RepoPath,
    state: &mut AnnotationState,
    current_commit_id: &CommitId,
    edges: &[GraphEdge<CommitId>],
) -> Result<(), BackendError> {
    let Some(mut current_source) = state.commit_source_map.remove(current_commit_id) else {
        return Ok(());
    };

    for parent_edge in edges {
        let parent_commit_id = &parent_edge.target;
        let parent_source = match state.commit_source_map.entry(parent_commit_id.clone()) {
            hash_map::Entry::Occupied(entry) => entry.into_mut(),
            hash_map::Entry::Vacant(entry) => {
                let commit = repo.store().get_commit(entry.key())?;
                entry.insert(Source::load(&commit, file_name)?)
            }
        };

        // For two versions of the same file, for all the lines in common,
        // overwrite the new mapping in the results for the new commit. Let's
        // say I have a file in commit A and commit B. We know that according to
        // local line_map, in commit A, line 3 corresponds to line 7 of the
        // starting file. Now, line 3 in Commit A corresponds to line 6 in
        // commit B. Then, we update local line_map to say that "Commit B line 6
        // goes to line 7 of the starting file". We repeat this for all lines in
        // common in the two commits.
        let mut current_lines = current_source.line_map.iter().copied().peekable();
        let mut new_current_line_map = Vec::new();
        let mut new_parent_line_map = Vec::new();
        copy_same_lines_with(
            &current_source.text,
            &parent_source.text,
            |current_start, parent_start, count| {
                new_current_line_map
                    .extend(current_lines.peeking_take_while(|&(cur, _)| cur < current_start));
                while let Some((current, starting)) =
                    current_lines.next_if(|&(cur, _)| cur < current_start + count)
                {
                    let parent = parent_start + (current - current_start);
                    new_parent_line_map.push((parent, starting));
                }
            },
        );
        new_current_line_map.extend(current_lines);
        current_source.line_map = new_current_line_map;
        parent_source.line_map = if parent_source.line_map.is_empty() {
            new_parent_line_map
        } else {
            itertools::merge(parent_source.line_map.iter().copied(), new_parent_line_map).collect()
        };
        if parent_source.line_map.is_empty() {
            state.commit_source_map.remove(parent_commit_id);
        } else if parent_edge.is_missing() {
            // If an omitted parent had the file, leave these lines unresolved.
            // The origin of the unresolved lines is represented as
            // Err(LineOrigin { parent_commit_id, parent_line_number }).
            for &(parent_line_number, starting_line_number) in &parent_source.line_map {
                state.original_line_map[starting_line_number] = Err(LineOrigin {
                    commit_id: parent_commit_id.clone(),
                    line_number: parent_line_number,
                });
            }
            state.num_unresolved_roots += 1;
        }
    }

    // Once we've looked at all parents of a commit, any leftover lines must be
    // original to the current commit, so we save this information in
    // original_line_map.
    for (current_line_number, starting_line_number) in current_source.line_map {
        state.original_line_map[starting_line_number] = Ok(LineOrigin {
            commit_id: current_commit_id.clone(),
            line_number: current_line_number,
        });
    }

    Ok(())
}

/// For two files, calls `copy(current_start, parent_start, count)` for each
/// range of contiguous lines in common (e.g. line 8-10 maps to line 9-11.)
fn copy_same_lines_with(
    current_contents: &[u8],
    parent_contents: &[u8],
    mut copy: impl FnMut(usize, usize, usize),
) {
    let diff = ContentDiff::by_line([current_contents, parent_contents]);
    let mut current_line_counter: usize = 0;
    let mut parent_line_counter: usize = 0;
    for hunk in diff.hunks() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                let count = hunk.contents[0].split_inclusive(|b| *b == b'\n').count();
                copy(current_line_counter, parent_line_counter, count);
                current_line_counter += count;
                parent_line_counter += count;
            }
            DiffHunkKind::Different => {
                let current_output = hunk.contents[0];
                let parent_output = hunk.contents[1];
                current_line_counter += current_output.split_inclusive(|b| *b == b'\n').count();
                parent_line_counter += parent_output.split_inclusive(|b| *b == b'\n').count();
            }
        }
    }
}

async fn get_file_contents(
    store: &Store,
    path: &RepoPath,
    tree: &MergedTree,
) -> Result<BString, BackendError> {
    let file_value = tree.path_value_async(path).await?;
    let effective_file_value = materialize_tree_value(store, path, file_value).await?;
    match effective_file_value {
        MaterializedTreeValue::File(mut file) => Ok(file.read_all(path).await?.into()),
        MaterializedTreeValue::FileConflict(file) => {
            // TODO: track line origins without materializing
            let options = ConflictMaterializeOptions {
                marker_style: ConflictMarkerStyle::Diff,
                marker_len: None,
                merge: MergeOptions {
                    hunk_level: FileMergeHunkLevel::Line,
                    same_change: SameChange::Accept,
                },
            };
            Ok(materialize_merge_result_to_bytes(&file.contents, &options))
        }
        _ => Ok(BString::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line_origin(commit_id: &CommitId, line_number: usize) -> LineOrigin {
        LineOrigin {
            commit_id: commit_id.clone(),
            line_number,
        }
    }

    #[test]
    fn test_lines_iterator_empty() {
        let annotation = FileAnnotation {
            line_map: vec![],
            text: "".into(),
        };
        assert_eq!(annotation.line_origins().collect_vec(), vec![]);
        assert_eq!(annotation.lines().collect_vec(), vec![]);
        assert_eq!(annotation.line_ranges().collect_vec(), vec![]);
        assert_eq!(annotation.compact_line_ranges().collect_vec(), vec![]);
    }

    #[test]
    fn test_lines_iterator_with_content() {
        let commit_id1 = CommitId::from_hex("111111");
        let commit_id2 = CommitId::from_hex("222222");
        let commit_id3 = CommitId::from_hex("333333");
        let annotation = FileAnnotation {
            line_map: vec![
                Ok(make_line_origin(&commit_id1, 0)),
                Ok(make_line_origin(&commit_id2, 1)),
                Ok(make_line_origin(&commit_id3, 2)),
            ],
            text: "foo\n\nbar\n".into(),
        };
        assert_eq!(
            annotation.line_origins().collect_vec(),
            vec![
                (Ok(&make_line_origin(&commit_id1, 0)), "foo\n".as_ref()),
                (Ok(&make_line_origin(&commit_id2, 1)), "\n".as_ref()),
                (Ok(&make_line_origin(&commit_id3, 2)), "bar\n".as_ref()),
            ]
        );
        assert_eq!(
            annotation.lines().collect_vec(),
            vec![
                (Ok(&commit_id1), "foo\n".as_ref()),
                (Ok(&commit_id2), "\n".as_ref()),
                (Ok(&commit_id3), "bar\n".as_ref()),
            ]
        );
        assert_eq!(
            annotation.line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..4),
                (Ok(&commit_id2), 4..5),
                (Ok(&commit_id3), 5..9),
            ]
        );
        assert_eq!(
            annotation.compact_line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..4),
                (Ok(&commit_id2), 4..5),
                (Ok(&commit_id3), 5..9),
            ]
        );
    }

    #[test]
    fn test_lines_iterator_compaction() {
        let commit_id1 = CommitId::from_hex("111111");
        let commit_id2 = CommitId::from_hex("222222");
        let commit_id3 = CommitId::from_hex("333333");
        let annotation = FileAnnotation {
            line_map: vec![
                Ok(make_line_origin(&commit_id1, 0)),
                Ok(make_line_origin(&commit_id1, 1)),
                Ok(make_line_origin(&commit_id2, 2)),
                Ok(make_line_origin(&commit_id1, 3)),
                Ok(make_line_origin(&commit_id3, 4)),
                Ok(make_line_origin(&commit_id3, 5)),
                Ok(make_line_origin(&commit_id3, 6)),
            ],
            text: "\n".repeat(7).into(),
        };
        assert_eq!(
            annotation.compact_line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..2),
                (Ok(&commit_id2), 2..3),
                (Ok(&commit_id1), 3..4),
                (Ok(&commit_id3), 4..7),
            ]
        );
    }
}
