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

//! Index of per-commit changed paths.

use std::collections::HashMap;
use std::fmt;
use std::fmt::Debug;
use std::fs::File;
use std::io::Read;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use blake2::Blake2b512;
use digest::Digest as _;
use either::Either;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use tempfile::NamedTempFile;

use super::entry::GlobalCommitPosition;
use super::readonly::ReadonlyIndexLoadError;
use crate::backend::BackendResult;
use crate::commit::Commit;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::persist_content_addressed_temp_file;
use crate::index::Index;
use crate::matchers::EverythingMatcher;
use crate::object_id::ObjectId as _;
use crate::object_id::id_type;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::rewrite::merge_commit_trees_no_resolve_without_repo;
use crate::tree_merge::resolve_file_values;

/// Current format version of the changed-path index segment file.
const FILE_FORMAT_VERSION: u32 = 0;

id_type!(pub(super) ChangedPathIndexSegmentId { hex() });

/// Commit position within a changed-path index segment.
///
/// This may be different from `LocalCommitPosition`, which is a position
/// relative to the start of the commit index segment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CommitPosition(u32);

/// Path position within a changed-path index segment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PathPosition(u32);

/// Changed-path index segment backed by immutable file.
///
/// File format:
/// ```text
/// u32: file format version
///
/// u32: number of (local) commit entries
/// u32: number of changed path entries
/// u32: number of path entries
/// u32: number of bytes of path entries
///
/// for each commit, in commit-index order
///   u32: position in changed-path table
/// u32: number of changed-path entries (as sentinel)
/// for each commit, in commit-index order
///   for each changed path, sorted by path
///     u32: lookup position of path
///
/// for each path, sorted by path
///   u32: byte offset in sorted paths table
/// u32: number of bytes of path entries (as sentinel)
/// for each path, sorted by path
///   <arbitrary length of bytes>: path
/// ```
///
/// * The parent segment id isn't stored in a segment file. This allows us to
///   insert parents without rewriting the descendant segments.
/// * Paths table isn't compacted across segments to keep the implementation
///   simple. There isn't a strong reason to map paths to globally-unique
///   integers. This also means changed-path positions are sorted by both path
///   texts and integers.
/// * Changed-path positions are sorted by paths so that we can binary-search
///   entries by exact path or path prefix if needed.
/// * Path components aren't split nor compressed so we can borrow `&RepoPath`
///   from the index data.
///
/// Ideas for future improvements:
/// * Multi-level index based on the paths? Since indexing is slow, it might
///   make sense to split index files based on path depths.
/// * Shared paths table to save disk space?
pub(super) struct ReadonlyChangedPathIndexSegment {
    id: ChangedPathIndexSegmentId,
    num_local_commits: u32,
    num_changed_paths: u32,
    num_paths: u32,
    // Base data offsets in bytes:
    commit_lookup_base: usize,
    changed_path_lookup_base: usize,
    path_lookup_base: usize,
    path_bytes_base: usize,
    data: Vec<u8>,
}

impl Debug for ReadonlyChangedPathIndexSegment {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("ReadonlyChangedPathIndexSegment")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl ReadonlyChangedPathIndexSegment {
    pub(super) fn load(
        dir: &Path,
        id: ChangedPathIndexSegmentId,
    ) -> Result<Arc<Self>, ReadonlyIndexLoadError> {
        let mut file = File::open(dir.join(id.hex()))
            .map_err(|err| ReadonlyIndexLoadError::from_io_err("changed-path", id.hex(), err))?;
        Self::load_from(&mut file, id)
    }

    pub(super) fn load_from(
        file: &mut dyn Read,
        id: ChangedPathIndexSegmentId,
    ) -> Result<Arc<Self>, ReadonlyIndexLoadError> {
        let from_io_err = |err| ReadonlyIndexLoadError::from_io_err("changed-path", id.hex(), err);
        let read_u32 = |file: &mut dyn Read| {
            let mut buf = [0; 4];
            file.read_exact(&mut buf).map_err(from_io_err)?;
            Ok(u32::from_le_bytes(buf))
        };

        let format_version = read_u32(file)?;
        if format_version != FILE_FORMAT_VERSION {
            return Err(ReadonlyIndexLoadError::UnexpectedVersion {
                kind: "changed-path",
                found_version: format_version,
                expected_version: FILE_FORMAT_VERSION,
            });
        }

        let num_local_commits = read_u32(file)?;
        let num_changed_paths = read_u32(file)?;
        let num_paths = read_u32(file)?;
        let num_path_bytes = read_u32(file)?;
        let mut data = vec![];
        file.read_to_end(&mut data).map_err(from_io_err)?;

        let commit_lookup_size = (num_local_commits as usize + 1) * 4;
        let changed_path_lookup_size = (num_changed_paths as usize) * 4;
        let path_lookup_size = (num_paths as usize + 1) * 4;

        let commit_lookup_base = 0;
        let changed_path_lookup_base = commit_lookup_base + commit_lookup_size;
        let path_lookup_base = changed_path_lookup_base + changed_path_lookup_size;
        let path_bytes_base = path_lookup_base + path_lookup_size;
        let expected_size = path_bytes_base + (num_path_bytes as usize);

        if data.len() != expected_size {
            return Err(ReadonlyIndexLoadError::invalid_data(
                "changed-path",
                id.hex(),
                "unexpected data length",
            ));
        }

        Ok(Arc::new(Self {
            id,
            num_local_commits,
            num_changed_paths,
            num_paths,
            commit_lookup_base,
            changed_path_lookup_base,
            path_lookup_base,
            path_bytes_base,
            data,
        }))
    }

    pub(super) fn id(&self) -> &ChangedPathIndexSegmentId {
        &self.id
    }

    pub(super) fn num_local_commits(&self) -> u32 {
        self.num_local_commits
    }

    pub(super) fn num_changed_paths(&self) -> u32 {
        self.num_changed_paths
    }

    pub(super) fn num_paths(&self) -> u32 {
        self.num_paths
    }

    fn changed_paths(&self, pos: CommitPosition) -> impl ExactSizeIterator<Item = &RepoPath> {
        let table = self.changed_paths_table(pos);
        let (chunks, _remainder) = table.as_chunks();
        chunks
            .iter()
            .map(|&chunk: &[u8; 4]| PathPosition(u32::from_le_bytes(chunk)))
            .map(|pos| self.path(pos))
    }

    fn changed_paths_table(&self, pos: CommitPosition) -> &[u8] {
        let table = &self.data[self.commit_lookup_base..self.changed_path_lookup_base];
        let offset = pos.0 as usize * 4;
        let start = u32::from_le_bytes(table[offset..][0..4].try_into().unwrap());
        let end = u32::from_le_bytes(table[offset..][4..8].try_into().unwrap());

        let table = &self.data[self.changed_path_lookup_base..self.path_lookup_base];
        &table[(start as usize) * 4..(end as usize) * 4]
    }

    fn path(&self, pos: PathPosition) -> &RepoPath {
        let bytes = self.path_bytes(pos);
        RepoPath::from_internal_string(
            str::from_utf8(bytes).expect("indexed path should be valid utf-8"),
        )
        .expect("indexed path should be valid")
    }

    fn path_bytes(&self, pos: PathPosition) -> &[u8] {
        let table = &self.data[self.path_lookup_base..self.path_bytes_base];
        let offset = pos.0 as usize * 4;
        let start = u32::from_le_bytes(table[offset..][0..4].try_into().unwrap());
        let end = u32::from_le_bytes(table[offset..][4..8].try_into().unwrap());

        let bytes = &self.data[self.path_bytes_base..];
        &bytes[start as usize..end as usize]
    }

    #[cfg(test)]
    fn paths(&self) -> impl ExactSizeIterator<Item = &RepoPath> {
        (0..self.num_paths).map(|pos| self.path(PathPosition(pos)))
    }
}

/// Changed-path index segment which is not serialized to file.
#[derive(Clone)]
pub(super) struct MutableChangedPathIndexSegment {
    entries: Vec<Vec<RepoPathBuf>>,
}

impl Debug for MutableChangedPathIndexSegment {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("MutableChangedPathIndexSegment")
            .finish_non_exhaustive()
    }
}

impl MutableChangedPathIndexSegment {
    pub(super) fn empty() -> Self {
        Self { entries: vec![] }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn num_local_commits(&self) -> u32 {
        self.entries.len().try_into().unwrap()
    }

    fn changed_paths(&self, pos: CommitPosition) -> impl ExactSizeIterator<Item = &RepoPath> {
        self.entries[pos.0 as usize].iter().map(AsRef::as_ref)
    }

    fn add_changed_paths(&mut self, paths: Vec<RepoPathBuf>) {
        debug_assert!(paths.is_sorted_by(|a, b| a < b));
        self.entries.push(paths);
    }

    fn extend_from_readonly_segment(&mut self, other_segment: &ReadonlyChangedPathIndexSegment) {
        self.entries
            .reserve(usize::try_from(other_segment.num_local_commits()).unwrap());
        for pos in (0..other_segment.num_local_commits()).map(CommitPosition) {
            let paths = other_segment
                .changed_paths(pos)
                .map(|path| path.to_owned())
                .collect();
            self.add_changed_paths(paths);
        }
    }

    fn extend_from_mutable_segment(&mut self, other_segment: Self) {
        self.entries.extend(other_segment.entries);
    }

    fn serialize_into(&self, buf: &mut Vec<u8>) {
        let mut paths = self.entries.iter().flatten().unique().collect_vec();
        paths.sort_unstable();
        let path_pos_map: HashMap<_, _> = paths
            .iter()
            .enumerate()
            .map(|(i, &path)| (path, PathPosition(u32::try_from(i).unwrap())))
            .collect();

        buf.extend(FILE_FORMAT_VERSION.to_le_bytes());
        let num_commits = u32::try_from(self.entries.len()).unwrap();
        let num_paths = u32::try_from(paths.len()).unwrap();
        buf.extend(num_commits.to_le_bytes());
        let num_changed_paths_offset = buf.len();
        buf.extend(0_u32.to_le_bytes());
        buf.extend(num_paths.to_le_bytes());
        let num_path_bytes_offset = buf.len();
        buf.extend(0_u32.to_le_bytes());

        let mut num_changed_paths: u32 = 0;
        for paths in &self.entries {
            buf.extend(num_changed_paths.to_le_bytes());
            num_changed_paths += u32::try_from(paths.len()).unwrap();
        }
        buf.extend(num_changed_paths.to_le_bytes()); // sentinel
        buf[num_changed_paths_offset..][..4].copy_from_slice(&num_changed_paths.to_le_bytes());

        for path in self.entries.iter().flatten() {
            let PathPosition(pos) = path_pos_map[path];
            buf.extend(pos.to_le_bytes());
        }

        let mut num_path_bytes: u32 = 0;
        for &path in &paths {
            buf.extend(num_path_bytes.to_le_bytes());
            num_path_bytes += u32::try_from(path.as_internal_file_string().len()).unwrap();
        }
        buf.extend(num_path_bytes.to_le_bytes()); // sentinel
        buf[num_path_bytes_offset..][..4].copy_from_slice(&num_path_bytes.to_le_bytes());

        for &path in &paths {
            buf.extend(path.as_internal_file_string().as_bytes());
        }
    }

    pub(super) fn save_in(
        &self,
        dir: &Path,
    ) -> Result<Arc<ReadonlyChangedPathIndexSegment>, PathError> {
        let mut buf = Vec::new();
        self.serialize_into(&mut buf);
        let mut hasher = Blake2b512::new();
        hasher.update(&buf);

        let file_id = ChangedPathIndexSegmentId::from_bytes(&hasher.finalize());
        let file_path = dir.join(file_id.hex());
        let mut file = NamedTempFile::new_in(dir).context(dir)?;
        file.as_file_mut().write_all(&buf).context(file.path())?;
        persist_content_addressed_temp_file(file, &file_path).context(&file_path)?;

        let segment = ReadonlyChangedPathIndexSegment::load_from(&mut &buf[..], file_id)
            .expect("in-memory index data should be valid and readable");
        Ok(segment)
    }
}

/// Index of per-commit changed paths.
#[derive(Clone, Debug)]
pub(super) struct CompositeChangedPathIndex {
    start_commit_pos: Option<GlobalCommitPosition>,
    num_commits: u32, // cache
    readonly_segments: Vec<Arc<ReadonlyChangedPathIndexSegment>>,
    mutable_segment: Option<Box<MutableChangedPathIndexSegment>>,
}

impl CompositeChangedPathIndex {
    /// Creates empty changed-path index which cannot store entries. In other
    /// words, the changed-path index is disabled.
    pub(super) fn null() -> Self {
        Self {
            start_commit_pos: None,
            num_commits: 0,
            readonly_segments: vec![],
            mutable_segment: None,
        }
    }

    /// Creates empty changed-path index which will store entries from
    /// `start_commit_pos`.
    pub(super) fn empty(start_commit_pos: GlobalCommitPosition) -> Self {
        Self {
            start_commit_pos: Some(start_commit_pos),
            num_commits: 0,
            readonly_segments: vec![],
            mutable_segment: None,
        }
    }

    pub(super) fn load(
        dir: &Path,
        start_commit_pos: GlobalCommitPosition,
        ids: &[ChangedPathIndexSegmentId],
    ) -> Result<Self, ReadonlyIndexLoadError> {
        let readonly_segments: Vec<_> = ids
            .iter()
            .map(|id| ReadonlyChangedPathIndexSegment::load(dir, id.clone()))
            .try_collect()?;
        let num_commits = readonly_segments
            .iter()
            .map(|segment| segment.num_local_commits())
            .sum();
        Ok(Self {
            start_commit_pos: Some(start_commit_pos),
            num_commits,
            readonly_segments,
            mutable_segment: None,
        })
    }

    /// Adds mutable segment if needed.
    pub(super) fn make_mutable(&mut self) {
        if self.start_commit_pos.is_none() || self.mutable_segment.is_some() {
            return;
        }
        self.mutable_segment = Some(Box::new(MutableChangedPathIndexSegment::empty()));
    }

    /// Position of the first indexed (or to-be-indexed) commit.
    pub(super) fn start_commit_pos(&self) -> Option<GlobalCommitPosition> {
        self.start_commit_pos
    }

    /// New commit index position which can be added to this index.
    pub(super) fn next_mutable_commit_pos(&self) -> Option<GlobalCommitPosition> {
        if self.mutable_segment.is_some() {
            self.start_commit_pos
                .map(|GlobalCommitPosition(start)| GlobalCommitPosition(start + self.num_commits))
        } else {
            None
        }
    }

    pub(super) fn num_commits(&self) -> u32 {
        self.num_commits
    }

    pub(super) fn readonly_segments(&self) -> &[Arc<ReadonlyChangedPathIndexSegment>] {
        &self.readonly_segments
    }

    /// Appends segments from the `other` index. This and the other index should
    /// be contiguous.
    pub(super) fn append_segments(&mut self, other: &Self) {
        assert!(self.mutable_segment.is_none());
        let GlobalCommitPosition(self_start_pos) =
            self.start_commit_pos.expect("should have start pos");
        let Some(GlobalCommitPosition(other_start_pos)) = other.start_commit_pos else {
            return;
        };
        assert_eq!(self_start_pos + self.num_commits, other_start_pos);
        self.readonly_segments
            .extend_from_slice(&other.readonly_segments);
        self.mutable_segment = other.mutable_segment.clone();
        self.num_commits += other.num_commits;
    }

    /// Maps `global_pos` to segment and segment-local position.
    fn find_segment(
        &self,
        global_pos: GlobalCommitPosition,
    ) -> Option<(
        CommitPosition,
        Either<&ReadonlyChangedPathIndexSegment, &MutableChangedPathIndexSegment>,
    )> {
        let mut local_pos = u32::checked_sub(global_pos.0, self.start_commit_pos?.0)?;
        for segment in &self.readonly_segments {
            local_pos = match u32::checked_sub(local_pos, segment.num_local_commits()) {
                Some(next_local_pos) => next_local_pos,
                None => return Some((CommitPosition(local_pos), Either::Left(segment))),
            };
        }
        let segment = self.mutable_segment.as_deref()?;
        (local_pos < segment.num_local_commits())
            .then_some((CommitPosition(local_pos), Either::Right(segment)))
    }

    /// Returns iterator over paths changed at the specified commit. The paths
    /// are sorted. Returns `None` if the commit wasn't indexed.
    pub(super) fn changed_paths(
        &self,
        global_pos: GlobalCommitPosition,
    ) -> Option<impl ExactSizeIterator<Item = &RepoPath>> {
        let (local_pos, segment) = self.find_segment(global_pos)?;
        Some(segment.map_either(
            |x| x.changed_paths(local_pos),
            |x| x.changed_paths(local_pos),
        ))
    }

    /// Adds changed paths of the next commit.
    ///
    /// The input `paths` must be sorted.
    ///
    /// Caller must ensure that the commit matches `next_mutable_commit_pos()`.
    /// Panics if this index isn't mutable (i.e. `next_mutable_commit_pos()` is
    /// `None`.)
    pub(super) fn add_changed_paths(&mut self, paths: Vec<RepoPathBuf>) {
        let segment = self
            .mutable_segment
            .as_deref_mut()
            .expect("should have mutable");
        segment.add_changed_paths(paths);
        self.num_commits += 1;
    }

    /// Squashes parent segments if the mutable segment has more than half the
    /// commits of its parent segment. This is done recursively, so the stack of
    /// index segments has O(log n) files.
    pub(super) fn maybe_squash_with_ancestors(&mut self) {
        let Some(mutable_segment) = self.mutable_segment.as_deref() else {
            return;
        };
        let mut num_new_commits = mutable_segment.num_local_commits();
        let mut squash_start = self.readonly_segments.len();
        for segment in self.readonly_segments.iter().rev() {
            // TODO: We should probably also squash if the parent segment has
            // less than N commits, regardless of how many (few) are in
            // `mutable_segment`.
            if 2 * num_new_commits < segment.num_local_commits() {
                break;
            }
            num_new_commits += segment.num_local_commits();
            squash_start -= 1;
        }
        if squash_start == self.readonly_segments.len() {
            return;
        }
        let mut squashed_segment = Box::new(MutableChangedPathIndexSegment::empty());
        for segment in self.readonly_segments.drain(squash_start..) {
            squashed_segment.extend_from_readonly_segment(&segment);
        }
        squashed_segment.extend_from_mutable_segment(*self.mutable_segment.take().unwrap());
        self.mutable_segment = Some(squashed_segment);
    }

    /// Writes mutable segment if exists, turns it into readonly segment.
    pub(super) fn save_in(&mut self, dir: &Path) -> Result<(), PathError> {
        let Some(segment) = self.mutable_segment.take() else {
            return Ok(());
        };
        if segment.is_empty() {
            return Ok(());
        };
        let segment = segment.save_in(dir)?;
        self.readonly_segments.push(segment);
        Ok(())
    }
}

/// Calculates the parent tree of the given `commit`, and builds a sorted list
/// of changed paths compared to the parent tree.
pub(super) async fn collect_changed_paths(
    index: &dyn Index,
    commit: &Commit,
) -> BackendResult<Vec<RepoPathBuf>> {
    let parents: Vec<_> = commit.parents_async().await?;
    if let [p] = parents.as_slice()
        && commit.tree_id() == p.tree_id()
    {
        return Ok(vec![]);
    }
    // Don't resolve the entire tree. It's cheaper to resolve each conflict file
    // even if we have to visit all files.
    tracing::trace!(?commit, parents_count = parents.len(), "calculating diffs");
    let store = commit.store();
    let from_tree = merge_commit_trees_no_resolve_without_repo(store, index, &parents).await?;
    let to_tree = commit.tree();
    let tree_diff = from_tree.diff_stream(&to_tree, &EverythingMatcher);
    let paths = tree_diff
        .map(|entry| entry.values.map(|values| (entry.path, values)))
        .try_filter_map(async |(path, mut diff)| {
            diff.before = resolve_file_values(store, &path, diff.before).await?;
            Ok(diff.is_changed().then_some(path))
        })
        .try_collect()
        .await?;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;
    use crate::tests::new_temp_dir;

    fn repo_path(value: &str) -> &RepoPath {
        RepoPath::from_internal_string(value).unwrap()
    }

    fn repo_path_buf(value: impl Into<String>) -> RepoPathBuf {
        RepoPathBuf::from_internal_string(value).unwrap()
    }

    fn collect_changed_paths(
        index: &CompositeChangedPathIndex,
        pos: GlobalCommitPosition,
    ) -> Option<Vec<&RepoPath>> {
        Some(index.changed_paths(pos)?.collect())
    }

    #[test]
    fn test_composite_null() {
        let mut index = CompositeChangedPathIndex::null();
        assert_eq!(index.start_commit_pos(), None);
        assert_eq!(index.next_mutable_commit_pos(), None);
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);

        // No entries can be added to "null" index
        index.make_mutable();
        assert!(index.mutable_segment.is_none());
        assert_eq!(index.num_commits(), 0);
    }

    #[test]
    fn test_composite_empty() {
        let temp_dir = new_temp_dir();
        let mut index = CompositeChangedPathIndex::empty(GlobalCommitPosition(0));
        assert_eq!(index.start_commit_pos(), Some(GlobalCommitPosition(0)));
        assert_eq!(index.next_mutable_commit_pos(), None);
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);

        index.make_mutable();
        assert!(index.mutable_segment.is_some());
        assert_eq!(
            index.next_mutable_commit_pos(),
            Some(GlobalCommitPosition(0))
        );
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);

        // Empty segment shouldn't be saved on disk
        index.save_in(temp_dir.path()).unwrap();
        assert!(index.mutable_segment.is_none());
        assert!(index.readonly_segments.is_empty());
        assert_eq!(index.start_commit_pos(), Some(GlobalCommitPosition(0)));
        assert_eq!(index.next_mutable_commit_pos(), None);
        assert_eq!(index.num_commits(), 0);
    }

    #[test_case(false, false; "mutable")]
    #[test_case(true, false; "readonly")]
    #[test_case(true, true; "readonly, reloaded")]
    fn test_composite_some_commits(on_disk: bool, reload: bool) {
        let temp_dir = new_temp_dir();
        let start_commit_pos = GlobalCommitPosition(1);
        let mut index = CompositeChangedPathIndex::empty(start_commit_pos);
        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("foo")]);
        index.add_changed_paths(vec![]);
        index.add_changed_paths(vec![repo_path_buf("bar/baz"), repo_path_buf("foo")]);
        index.add_changed_paths(vec![]);
        assert_eq!(
            index.next_mutable_commit_pos(),
            Some(GlobalCommitPosition(5))
        );
        assert_eq!(index.num_commits(), 4);
        if on_disk {
            index.save_in(temp_dir.path()).unwrap();
            assert!(index.mutable_segment.is_none());
            assert_eq!(index.readonly_segments.len(), 1);
            assert_eq!(index.next_mutable_commit_pos(), None);
            assert_eq!(index.num_commits(), 4);
        }
        if reload {
            let ids = index
                .readonly_segments()
                .iter()
                .map(|segment| segment.id().clone())
                .collect_vec();
            index =
                CompositeChangedPathIndex::load(temp_dir.path(), start_commit_pos, &ids).unwrap();
        }
        if let [segment] = &*index.readonly_segments {
            assert_eq!(segment.num_local_commits(), 4);
            assert_eq!(segment.num_changed_paths(), 3);
            assert_eq!(segment.num_paths(), 2);
            assert_eq!(
                segment.paths().collect_vec(),
                [repo_path("bar/baz"), repo_path("foo")]
            );
        }

        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(1)),
            Some(vec![repo_path("foo")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(2)),
            Some(vec![])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(3)),
            Some(vec![repo_path("bar/baz"), repo_path("foo")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(4)),
            Some(vec![])
        );
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(5)), None);
    }

    #[test]
    fn test_composite_empty_commits() {
        let temp_dir = new_temp_dir();
        let mut index = CompositeChangedPathIndex::empty(GlobalCommitPosition(0));
        index.make_mutable();
        // An empty commits table can be serialized/deserialized if forced
        let segment = index.mutable_segment.take().unwrap();
        let segment = segment.save_in(temp_dir.path()).unwrap();
        index.readonly_segments.push(segment);
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);
    }

    #[test]
    fn test_composite_empty_changed_paths() {
        let temp_dir = new_temp_dir();
        let mut index = CompositeChangedPathIndex::empty(GlobalCommitPosition(0));
        index.make_mutable();
        index.add_changed_paths(vec![]);
        // An empty paths table can be serialized/deserialized
        assert_eq!(index.num_commits(), 1);
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(0)),
            Some(vec![])
        );
    }

    #[test_case(false; "with mutable")]
    #[test_case(true; "fully readonly")]
    fn test_composite_segmented(on_disk: bool) {
        let temp_dir = new_temp_dir();
        let mut index = CompositeChangedPathIndex::empty(GlobalCommitPosition(1));
        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("b")]);
        index.save_in(temp_dir.path()).unwrap();
        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("c")]);
        index.add_changed_paths(vec![repo_path_buf("a/b"), repo_path_buf("b")]);
        index.save_in(temp_dir.path()).unwrap();
        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("d")]);
        index.add_changed_paths(vec![repo_path_buf("a/c"), repo_path_buf("c")]);
        if on_disk {
            index.save_in(temp_dir.path()).unwrap();
            assert!(index.mutable_segment.is_none());
            assert_eq!(index.readonly_segments.len(), 3);
            assert_eq!(index.next_mutable_commit_pos(), None);
        } else {
            assert_eq!(index.readonly_segments.len(), 2);
            assert!(index.mutable_segment.is_some());
        }
        assert_eq!(index.num_commits(), 5);

        assert_eq!(
            index.readonly_segments[0].paths().collect_vec(),
            [repo_path("b")]
        );
        assert_eq!(
            index.readonly_segments[1].paths().collect_vec(),
            [repo_path("a/b"), repo_path("b"), repo_path("c")]
        );
        if on_disk {
            assert_eq!(
                index.readonly_segments[2].paths().collect_vec(),
                [repo_path("a/c"), repo_path("c"), repo_path("d")]
            );
        }

        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(0)), None);
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(1)),
            Some(vec![repo_path("b")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(2)),
            Some(vec![repo_path("c")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(3)),
            Some(vec![repo_path("a/b"), repo_path("b")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(4)),
            Some(vec![repo_path("d")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(5)),
            Some(vec![repo_path("a/c"), repo_path("c")])
        );
        assert_eq!(collect_changed_paths(&index, GlobalCommitPosition(6)), None);
    }

    #[test]
    fn test_composite_squash_segments() {
        let temp_dir = new_temp_dir();
        let mut index = CompositeChangedPathIndex::empty(GlobalCommitPosition(0));
        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("0")]);
        index.maybe_squash_with_ancestors();
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(index.readonly_segments.len(), 1);
        assert_eq!(index.readonly_segments[0].num_local_commits(), 1);

        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("1")]);
        index.maybe_squash_with_ancestors();
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(index.readonly_segments.len(), 1);
        assert_eq!(index.readonly_segments[0].num_local_commits(), 2);

        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("2")]);
        index.maybe_squash_with_ancestors();
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(index.readonly_segments.len(), 1);
        assert_eq!(index.readonly_segments[0].num_local_commits(), 3);

        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("3")]);
        index.maybe_squash_with_ancestors();
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(index.readonly_segments.len(), 2);
        assert_eq!(index.readonly_segments[0].num_local_commits(), 3);
        assert_eq!(index.readonly_segments[1].num_local_commits(), 1);

        index.make_mutable();
        index.add_changed_paths(vec![repo_path_buf("4")]);
        index.add_changed_paths(vec![repo_path_buf("5")]);
        index.maybe_squash_with_ancestors();
        index.save_in(temp_dir.path()).unwrap();
        assert_eq!(index.readonly_segments.len(), 1);
        assert_eq!(index.readonly_segments[0].num_local_commits(), 6);

        // Squashed segments should preserve the original entries.
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(0)),
            Some(vec![repo_path("0")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(1)),
            Some(vec![repo_path("1")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(2)),
            Some(vec![repo_path("2")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(3)),
            Some(vec![repo_path("3")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(4)),
            Some(vec![repo_path("4")])
        );
        assert_eq!(
            collect_changed_paths(&index, GlobalCommitPosition(5)),
            Some(vec![repo_path("5")])
        );
    }
}
