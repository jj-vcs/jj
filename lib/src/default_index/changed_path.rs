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
use std::str;
use std::sync::Arc;

use blake2::Blake2b512;
use digest::Digest as _;
use either::Either;
use itertools::Itertools as _;
use tempfile::NamedTempFile;

use super::entry::GlobalCommitPosition;
use super::readonly::ReadonlyIndexLoadError;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::persist_content_addressed_temp_file;
use crate::object_id::ObjectId as _;
use crate::object_id::id_type;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
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

    #[cfg_attr(not(test), expect(dead_code))] // TODO
    pub(super) fn num_changed_paths(&self) -> u32 {
        self.num_changed_paths
    }

    #[cfg_attr(not(test), expect(dead_code))] // TODO
    pub(super) fn num_paths(&self) -> u32 {
        self.num_paths
    }

    fn changed_paths(&self, pos: CommitPosition) -> impl ExactSizeIterator<Item = &RepoPath> {
        let table = self.changed_paths_table(pos);
        table
            .chunks_exact(4)
            .map(|x| PathPosition(u32::from_le_bytes(x.try_into().unwrap())))
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
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
    #[cfg_attr(not(test), expect(dead_code))] // TODO
    pub(super) fn next_mutable_commit_pos(&self) -> Option<GlobalCommitPosition> {
        if self.mutable_segment.is_some() {
            self.start_commit_pos
                .map(|GlobalCommitPosition(start)| GlobalCommitPosition(start + self.num_commits))
        } else {
            None
        }
    }

    #[cfg_attr(not(test), expect(dead_code))] // TODO
    pub(super) fn num_commits(&self) -> u32 {
        self.num_commits
    }

    pub(super) fn readonly_segments(&self) -> &[Arc<ReadonlyChangedPathIndexSegment>] {
        &self.readonly_segments
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
    #[cfg_attr(not(test), expect(dead_code))] // TODO
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
    #[cfg_attr(not(test), expect(dead_code))] // TODO
    pub(super) fn add_changed_paths(&mut self, paths: Vec<RepoPathBuf>) {
        let segment = self
            .mutable_segment
            .as_deref_mut()
            .expect("should have mutable");
        segment.add_changed_paths(paths);
        self.num_commits += 1;
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
}
