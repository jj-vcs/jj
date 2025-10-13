// Copyright 2023 The Jujutsu Authors
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

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Debug;
use std::fs::File;
use std::io;
use std::io::Read;
use std::iter;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use itertools::Itertools as _;
use smallvec::smallvec;
use thiserror::Error;

use super::changed_path::CompositeChangedPathIndex;
use super::composite::AsCompositeIndex;
use super::composite::ChangeIdIndexImpl;
use super::composite::CommitIndexSegment;
use super::composite::CommitIndexSegmentId;
use super::composite::CompositeCommitIndex;
use super::composite::CompositeIndex;
use super::entry::GlobalCommitPosition;
use super::entry::LocalCommitPosition;
use super::entry::SmallGlobalCommitPositionsVec;
use super::entry::SmallLocalCommitPositionsVec;
use super::mutable::DefaultMutableIndex;
use super::revset_engine;
use super::revset_engine::RevsetImpl;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::graph::GraphNode;
use crate::index::ChangeIdIndex;
use crate::index::Index;
use crate::index::IndexError;
use crate::index::MutableIndex;
use crate::index::ReadonlyIndex;
use crate::object_id::HexPrefix;
use crate::object_id::ObjectId;
use crate::object_id::PrefixResolution;
use crate::repo_path::RepoPathBuf;
use crate::revset::ResolvedExpression;
use crate::revset::Revset;
use crate::revset::RevsetEvaluationError;
use crate::store::Store;

/// Error while loading index segment file.
#[derive(Debug, Error)]
pub enum ReadonlyIndexLoadError {
    #[error("Unexpected {kind} index version")]
    UnexpectedVersion {
        /// Index type.
        kind: &'static str,
        found_version: u32,
        expected_version: u32,
    },
    #[error("Failed to load {kind} index file '{name}'")]
    Other {
        /// Index type.
        kind: &'static str,
        /// Index file name.
        name: String,
        /// Underlying error.
        #[source]
        error: io::Error,
    },
}

impl ReadonlyIndexLoadError {
    pub(super) fn invalid_data(
        kind: &'static str,
        name: impl Into<String>,
        error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self::from_io_err(
            kind,
            name,
            io::Error::new(io::ErrorKind::InvalidData, error),
        )
    }

    pub(super) fn from_io_err(
        kind: &'static str,
        name: impl Into<String>,
        error: io::Error,
    ) -> Self {
        Self::Other {
            kind,
            name: name.into(),
            error,
        }
    }

    /// Returns true if the underlying error suggests data corruption.
    pub(super) fn is_corrupt_or_not_found(&self) -> bool {
        match self {
            Self::UnexpectedVersion { .. } => true,
            Self::Other { error, .. } => {
                // If the parent file name field is corrupt, the file wouldn't be found.
                // And there's no need to distinguish it from an empty file.
                matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::InvalidData
                        | io::ErrorKind::UnexpectedEof
                )
            }
        }
    }
}

/// Current format version of the commit index segment file.
pub(super) const COMMIT_INDEX_SEGMENT_FILE_FORMAT_VERSION: u32 = 6;

/// If set, the value is stored in the overflow table.
pub(super) const OVERFLOW_FLAG: u32 = 0x8000_0000;

/// Global index position of parent entry, or overflow pointer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ParentIndexPosition(u32);

impl ParentIndexPosition {
    fn as_inlined(self) -> Option<GlobalCommitPosition> {
        (self.0 & OVERFLOW_FLAG == 0).then_some(GlobalCommitPosition(self.0))
    }

    fn as_overflow(self) -> Option<u32> {
        (self.0 & OVERFLOW_FLAG != 0).then_some(!self.0)
    }
}

/// Local position of entry pointed by change id, or overflow pointer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ChangeLocalPosition(u32);

impl ChangeLocalPosition {
    fn as_inlined(self) -> Option<LocalCommitPosition> {
        (self.0 & OVERFLOW_FLAG == 0).then_some(LocalCommitPosition(self.0))
    }

    fn as_overflow(self) -> Option<u32> {
        (self.0 & OVERFLOW_FLAG != 0).then_some(!self.0)
    }
}

/// Lengths of fields to be serialized.
#[derive(Clone, Copy, Debug)]
pub(super) struct FieldLengths {
    pub commit_id: usize,
    pub change_id: usize,
}

struct CommitGraphEntry<'a> {
    data: &'a [u8],
}

// TODO: Add pointers to ancestors further back, like a skip list. Clear the
// lowest set bit to determine which generation number the pointers point to.
impl CommitGraphEntry<'_> {
    fn size(commit_id_length: usize) -> usize {
        16 + commit_id_length
    }

    fn generation_number(&self) -> u32 {
        u32::from_le_bytes(self.data[0..4].try_into().unwrap())
    }

    fn parent1_pos_or_overflow_pos(&self) -> ParentIndexPosition {
        ParentIndexPosition(u32::from_le_bytes(self.data[4..8].try_into().unwrap()))
    }

    fn parent2_pos_or_overflow_len(&self) -> ParentIndexPosition {
        ParentIndexPosition(u32::from_le_bytes(self.data[8..12].try_into().unwrap()))
    }

    fn change_id_lookup_pos(&self) -> u32 {
        u32::from_le_bytes(self.data[12..16].try_into().unwrap())
    }

    fn commit_id(&self) -> CommitId {
        CommitId::from_bytes(self.commit_id_bytes())
    }

    // might be better to add borrowed version of CommitId
    fn commit_id_bytes(&self) -> &[u8] {
        &self.data[16..]
    }
}

/// Commit index segment backed by immutable file.
///
/// File format:
/// ```text
/// u32: file format version
/// u32: parent segment file name length (0 means root)
/// <length number of bytes>: parent segment file name
///
/// u32: number of local commit entries
/// u32: number of local change ids
/// u32: number of overflow parent entries
/// u32: number of overflow change id positions
/// for each entry, in some topological order with parents first:
///   u32: generation number
///   if number of parents <= 2:
///     u32: (< 0x8000_0000) global index position for parent 1
///          (==0xffff_ffff) no parent 1
///     u32: (< 0x8000_0000) global index position for parent 2
///          (==0xffff_ffff) no parent 2
///   else:
///     u32: (>=0x8000_0000) position in the overflow table, bit-negated
///     u32: (>=0x8000_0000) number of parents (in the overflow table), bit-negated
///   u32: change id position in the sorted change ids table
///   <commit id length number of bytes>: commit id
/// for each entry, sorted by commit id:
///   u32: local position in the graph entries table
/// for each entry, sorted by change id:
///   <change id length number of bytes>: change id
/// for each entry, sorted by change id:
///   if number of associated commits == 1:
///     u32: (< 0x8000_0000) local position in the graph entries table
///   else:
///     u32: (>=0x8000_0000) position in the overflow table, bit-negated
/// for each overflow parent:
///   u32: global index position
/// for each overflow change id entry:
///   u32: local position in the graph entries table
/// ```
///
/// Note that u32 fields are 4-byte aligned so long as the parent file name
/// (which is hexadecimal hash) and commit/change ids aren't of exotic length.
// TODO: replace the table by a trie so we don't have to repeat the full commit
//       ids
// TODO: add a fanout table like git's commit graph has?
pub(super) struct ReadonlyCommitIndexSegment {
    parent_file: Option<Arc<Self>>,
    num_parent_commits: u32,
    id: CommitIndexSegmentId,
    field_lengths: FieldLengths,
    // Number of commits not counting the parent file
    num_local_commits: u32,
    num_local_change_ids: u32,
    num_change_overflow_entries: u32,
    // Base data offsets in bytes:
    commit_lookup_base: usize,
    change_id_table_base: usize,
    change_pos_table_base: usize,
    parent_overflow_base: usize,
    change_overflow_base: usize,
    data: Vec<u8>,
}

impl Debug for ReadonlyCommitIndexSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("ReadonlyCommitIndexSegment")
            .field("id", &self.id)
            .field("parent_file", &self.parent_file)
            .finish_non_exhaustive()
    }
}

impl ReadonlyCommitIndexSegment {
    /// Loads both parent segments and local entries from the given file `name`.
    pub(super) fn load(
        dir: &Path,
        id: CommitIndexSegmentId,
        lengths: FieldLengths,
    ) -> Result<Arc<Self>, ReadonlyIndexLoadError> {
        let mut file = File::open(dir.join(id.hex()))
            .map_err(|err| ReadonlyIndexLoadError::from_io_err("commit", id.hex(), err))?;
        Self::load_from(&mut file, dir, id, lengths)
    }

    /// Loads both parent segments and local entries from the given `file`.
    pub(super) fn load_from(
        file: &mut dyn Read,
        dir: &Path,
        id: CommitIndexSegmentId,
        lengths: FieldLengths,
    ) -> Result<Arc<Self>, ReadonlyIndexLoadError> {
        let from_io_err = |err| ReadonlyIndexLoadError::from_io_err("commit", id.hex(), err);
        let read_u32 = |file: &mut dyn Read| {
            let mut buf = [0; 4];
            file.read_exact(&mut buf).map_err(from_io_err)?;
            Ok(u32::from_le_bytes(buf))
        };
        let format_version = read_u32(file)?;
        if format_version != COMMIT_INDEX_SEGMENT_FILE_FORMAT_VERSION {
            return Err(ReadonlyIndexLoadError::UnexpectedVersion {
                kind: "commit",
                found_version: format_version,
                expected_version: COMMIT_INDEX_SEGMENT_FILE_FORMAT_VERSION,
            });
        }
        let parent_filename_len = read_u32(file)?;
        let maybe_parent_file = if parent_filename_len > 0 {
            let mut parent_filename_bytes = vec![0; parent_filename_len as usize];
            file.read_exact(&mut parent_filename_bytes)
                .map_err(from_io_err)?;
            let parent_file_id = CommitIndexSegmentId::try_from_hex(parent_filename_bytes)
                .ok_or_else(|| {
                    ReadonlyIndexLoadError::invalid_data(
                        "commit",
                        id.hex(),
                        "parent file name is not valid hex",
                    )
                })?;
            let parent_file = Self::load(dir, parent_file_id, lengths)?;
            Some(parent_file)
        } else {
            None
        };
        Self::load_with_parent_file(file, id, maybe_parent_file, lengths)
    }

    /// Loads local entries from the given `file`, returns new segment linked to
    /// the given `parent_file`.
    pub(super) fn load_with_parent_file(
        file: &mut dyn Read,
        id: CommitIndexSegmentId,
        parent_file: Option<Arc<Self>>,
        lengths: FieldLengths,
    ) -> Result<Arc<Self>, ReadonlyIndexLoadError> {
        let from_io_err = |err| ReadonlyIndexLoadError::from_io_err("commit", id.hex(), err);
        let read_u32 = |file: &mut dyn Read| {
            let mut buf = [0; 4];
            file.read_exact(&mut buf).map_err(from_io_err)?;
            Ok(u32::from_le_bytes(buf))
        };
        let num_parent_commits = parent_file
            .as_ref()
            .map_or(0, |segment| segment.as_composite().num_commits());
        let num_local_commits = read_u32(file)?;
        let num_local_change_ids = read_u32(file)?;
        let num_parent_overflow_entries = read_u32(file)?;
        let num_change_overflow_entries = read_u32(file)?;
        let mut data = vec![];
        file.read_to_end(&mut data).map_err(from_io_err)?;

        let commit_graph_entry_size = CommitGraphEntry::size(lengths.commit_id);
        let graph_size = (num_local_commits as usize) * commit_graph_entry_size;
        let commit_lookup_size = (num_local_commits as usize) * 4;
        let change_id_table_size = (num_local_change_ids as usize) * lengths.change_id;
        let change_pos_table_size = (num_local_change_ids as usize) * 4;
        let parent_overflow_size = (num_parent_overflow_entries as usize) * 4;
        let change_overflow_size = (num_change_overflow_entries as usize) * 4;

        let graph_base = 0;
        let commit_lookup_base = graph_base + graph_size;
        let change_id_table_base = commit_lookup_base + commit_lookup_size;
        let change_pos_table_base = change_id_table_base + change_id_table_size;
        let parent_overflow_base = change_pos_table_base + change_pos_table_size;
        let change_overflow_base = parent_overflow_base + parent_overflow_size;
        let expected_size = change_overflow_base + change_overflow_size;

        if data.len() != expected_size {
            return Err(ReadonlyIndexLoadError::invalid_data(
                "commit",
                id.hex(),
                "unexpected data length",
            ));
        }

        Ok(Arc::new(Self {
            parent_file,
            num_parent_commits,
            id,
            field_lengths: lengths,
            num_local_commits,
            num_local_change_ids,
            num_change_overflow_entries,
            commit_lookup_base,
            change_id_table_base,
            change_pos_table_base,
            parent_overflow_base,
            change_overflow_base,
            data,
        }))
    }

    pub(super) fn as_composite(&self) -> &CompositeCommitIndex {
        CompositeCommitIndex::new(self)
    }

    pub(super) fn id(&self) -> &CommitIndexSegmentId {
        &self.id
    }

    pub(super) fn field_lengths(&self) -> FieldLengths {
        self.field_lengths
    }

    fn graph_entry(&self, local_pos: LocalCommitPosition) -> CommitGraphEntry<'_> {
        let table = &self.data[..self.commit_lookup_base];
        let entry_size = CommitGraphEntry::size(self.field_lengths.commit_id);
        let offset = (local_pos.0 as usize) * entry_size;
        CommitGraphEntry {
            data: &table[offset..][..entry_size],
        }
    }

    fn commit_lookup_pos(&self, lookup_pos: u32) -> LocalCommitPosition {
        let table = &self.data[self.commit_lookup_base..self.change_id_table_base];
        let offset = (lookup_pos as usize) * 4;
        LocalCommitPosition(u32::from_le_bytes(table[offset..][..4].try_into().unwrap()))
    }

    fn change_lookup_id(&self, lookup_pos: u32) -> ChangeId {
        ChangeId::from_bytes(self.change_lookup_id_bytes(lookup_pos))
    }

    // might be better to add borrowed version of ChangeId
    fn change_lookup_id_bytes(&self, lookup_pos: u32) -> &[u8] {
        let table = &self.data[self.change_id_table_base..self.change_pos_table_base];
        let offset = (lookup_pos as usize) * self.field_lengths.change_id;
        &table[offset..][..self.field_lengths.change_id]
    }

    fn change_lookup_pos(&self, lookup_pos: u32) -> ChangeLocalPosition {
        let table = &self.data[self.change_pos_table_base..self.parent_overflow_base];
        let offset = (lookup_pos as usize) * 4;
        ChangeLocalPosition(u32::from_le_bytes(table[offset..][..4].try_into().unwrap()))
    }

    fn overflow_parents(
        &self,
        overflow_pos: u32,
        num_parents: u32,
    ) -> SmallGlobalCommitPositionsVec {
        let table = &self.data[self.parent_overflow_base..self.change_overflow_base];
        let offset = (overflow_pos as usize) * 4;
        let size = (num_parents as usize) * 4;
        let (chunks, _remainder) = table[offset..][..size].as_chunks();
        chunks
            .iter()
            .map(|&chunk: &[u8; 4]| GlobalCommitPosition(u32::from_le_bytes(chunk)))
            .collect()
    }

    /// Scans graph entry positions stored in the overflow change ids table.
    fn overflow_changes_from(
        &self,
        overflow_pos: u32,
    ) -> impl Iterator<Item = LocalCommitPosition> + use<'_> {
        let table = &self.data[self.change_overflow_base..];
        let offset = (overflow_pos as usize) * 4;
        let (chunks, _remainder) = table[offset..].as_chunks();
        chunks
            .iter()
            .map(|&chunk: &[u8; 4]| LocalCommitPosition(u32::from_le_bytes(chunk)))
    }

    /// Binary searches commit id by `prefix`. Returns the lookup position.
    fn commit_id_byte_prefix_to_lookup_pos(&self, prefix: &[u8]) -> PositionLookupResult {
        binary_search_pos_by(self.num_local_commits, |pos| {
            let local_pos = self.commit_lookup_pos(pos);
            let entry = self.graph_entry(local_pos);
            entry.commit_id_bytes().cmp(prefix)
        })
    }

    /// Binary searches change id by `prefix`. Returns the lookup position.
    fn change_id_byte_prefix_to_lookup_pos(&self, prefix: &[u8]) -> PositionLookupResult {
        binary_search_pos_by(self.num_local_change_ids, |pos| {
            let change_id_bytes = self.change_lookup_id_bytes(pos);
            change_id_bytes.cmp(prefix)
        })
    }
}

impl CommitIndexSegment for ReadonlyCommitIndexSegment {
    fn num_parent_commits(&self) -> u32 {
        self.num_parent_commits
    }

    fn num_local_commits(&self) -> u32 {
        self.num_local_commits
    }

    fn parent_file(&self) -> Option<&Arc<ReadonlyCommitIndexSegment>> {
        self.parent_file.as_ref()
    }

    fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<LocalCommitPosition> {
        self.commit_id_byte_prefix_to_lookup_pos(commit_id.as_bytes())
            .ok()
            .map(|pos| self.commit_lookup_pos(pos))
    }

    fn resolve_neighbor_commit_ids(
        &self,
        commit_id: &CommitId,
    ) -> (Option<CommitId>, Option<CommitId>) {
        self.commit_id_byte_prefix_to_lookup_pos(commit_id.as_bytes())
            .map_neighbors(|pos| {
                let local_pos = self.commit_lookup_pos(pos);
                let entry = self.graph_entry(local_pos);
                entry.commit_id()
            })
    }

    fn resolve_commit_id_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        self.commit_id_byte_prefix_to_lookup_pos(prefix.min_prefix_bytes())
            .prefix_matches(prefix, |pos| {
                let local_pos = self.commit_lookup_pos(pos);
                let entry = self.graph_entry(local_pos);
                entry.commit_id()
            })
            .map(|(id, _)| id)
    }

    fn resolve_neighbor_change_ids(
        &self,
        change_id: &ChangeId,
    ) -> (Option<ChangeId>, Option<ChangeId>) {
        self.change_id_byte_prefix_to_lookup_pos(change_id.as_bytes())
            .map_neighbors(|pos| self.change_lookup_id(pos))
    }

    fn resolve_change_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> PrefixResolution<(ChangeId, SmallLocalCommitPositionsVec)> {
        self.change_id_byte_prefix_to_lookup_pos(prefix.min_prefix_bytes())
            .prefix_matches(prefix, |pos| self.change_lookup_id(pos))
            .map(|(id, lookup_pos)| {
                let change_pos = self.change_lookup_pos(lookup_pos);
                if let Some(local_pos) = change_pos.as_inlined() {
                    (id, smallvec![local_pos])
                } else {
                    let overflow_pos = change_pos.as_overflow().unwrap();
                    // Collect commits having the same change id. For cache
                    // locality, it might be better to look for the next few
                    // change id positions to determine the size.
                    let positions: SmallLocalCommitPositionsVec = self
                        .overflow_changes_from(overflow_pos)
                        .take_while(|&local_pos| {
                            let entry = self.graph_entry(local_pos);
                            entry.change_id_lookup_pos() == lookup_pos
                        })
                        .collect();
                    debug_assert_eq!(
                        overflow_pos + u32::try_from(positions.len()).unwrap(),
                        (lookup_pos + 1..self.num_local_change_ids)
                            .find_map(|lookup_pos| self.change_lookup_pos(lookup_pos).as_overflow())
                            .unwrap_or(self.num_change_overflow_entries),
                        "all overflow positions to the next change id should be collected"
                    );
                    (id, positions)
                }
            })
    }

    fn generation_number(&self, local_pos: LocalCommitPosition) -> u32 {
        self.graph_entry(local_pos).generation_number()
    }

    fn commit_id(&self, local_pos: LocalCommitPosition) -> CommitId {
        self.graph_entry(local_pos).commit_id()
    }

    fn change_id(&self, local_pos: LocalCommitPosition) -> ChangeId {
        let entry = self.graph_entry(local_pos);
        self.change_lookup_id(entry.change_id_lookup_pos())
    }

    fn num_parents(&self, local_pos: LocalCommitPosition) -> u32 {
        let graph_entry = self.graph_entry(local_pos);
        let pos1_or_overflow_pos = graph_entry.parent1_pos_or_overflow_pos();
        let pos2_or_overflow_len = graph_entry.parent2_pos_or_overflow_len();
        let inlined_len1 = pos1_or_overflow_pos.as_inlined().is_some() as u32;
        let inlined_len2 = pos2_or_overflow_len.as_inlined().is_some() as u32;
        let overflow_len = pos2_or_overflow_len.as_overflow().unwrap_or(0);
        inlined_len1 + inlined_len2 + overflow_len
    }

    fn parent_positions(&self, local_pos: LocalCommitPosition) -> SmallGlobalCommitPositionsVec {
        let graph_entry = self.graph_entry(local_pos);
        let pos1_or_overflow_pos = graph_entry.parent1_pos_or_overflow_pos();
        let pos2_or_overflow_len = graph_entry.parent2_pos_or_overflow_len();
        if let Some(pos1) = pos1_or_overflow_pos.as_inlined() {
            if let Some(pos2) = pos2_or_overflow_len.as_inlined() {
                smallvec![pos1, pos2]
            } else {
                smallvec![pos1]
            }
        } else {
            let overflow_pos = pos1_or_overflow_pos.as_overflow().unwrap();
            let num_parents = pos2_or_overflow_len.as_overflow().unwrap();
            self.overflow_parents(overflow_pos, num_parents)
        }
    }
}

/// Commit index backend which stores data on local disk.
#[derive(Clone, Debug)]
pub struct DefaultReadonlyIndex(CompositeIndex);

impl DefaultReadonlyIndex {
    pub(super) fn from_segment(
        commits: Arc<ReadonlyCommitIndexSegment>,
        changed_paths: CompositeChangedPathIndex,
    ) -> Self {
        Self(CompositeIndex::from_readonly(commits, changed_paths))
    }

    pub(super) fn readonly_commits(&self) -> &Arc<ReadonlyCommitIndexSegment> {
        self.0.readonly_commits().expect("must have readonly")
    }

    pub(super) fn changed_paths(&self) -> &CompositeChangedPathIndex {
        self.0.changed_paths()
    }

    /// Returns the number of all indexed commits.
    pub fn num_commits(&self) -> u32 {
        self.0.commits().num_commits()
    }

    /// Collects statistics of indexed commits and segments.
    pub fn stats(&self) -> IndexStats {
        let commits = self.readonly_commits();
        let num_commits = commits.as_composite().num_commits();
        let mut num_merges = 0;
        let mut max_generation_number = 0;
        let mut change_ids = HashSet::new();
        for pos in (0..num_commits).map(GlobalCommitPosition) {
            let entry = commits.as_composite().entry_by_pos(pos);
            max_generation_number = max_generation_number.max(entry.generation_number());
            if entry.num_parents() > 1 {
                num_merges += 1;
            }
            change_ids.insert(entry.change_id());
        }
        let num_heads = u32::try_from(commits.as_composite().all_heads_pos().count()).unwrap();
        let mut commit_levels = iter::successors(Some(commits), |segment| segment.parent_file())
            .map(|segment| CommitIndexLevelStats {
                num_commits: segment.num_local_commits(),
                name: segment.id().hex(),
            })
            .collect_vec();
        commit_levels.reverse();

        let changed_paths = self.changed_paths();
        let changed_path_commits_range = changed_paths
            .start_commit_pos()
            .map(|GlobalCommitPosition(start)| start..(start + changed_paths.num_commits()));
        let changed_path_levels = changed_paths
            .readonly_segments()
            .iter()
            .map(|segment| ChangedPathIndexLevelStats {
                num_commits: segment.num_local_commits(),
                num_changed_paths: segment.num_changed_paths(),
                num_paths: segment.num_paths(),
                name: segment.id().hex(),
            })
            .collect_vec();

        IndexStats {
            num_commits,
            num_merges,
            max_generation_number,
            num_heads,
            num_changes: change_ids.len().try_into().unwrap(),
            commit_levels,
            changed_path_commits_range,
            changed_path_levels,
        }
    }

    /// Looks up generation of the specified commit.
    pub fn generation_number(&self, commit_id: &CommitId) -> Option<u32> {
        let entry = self.0.commits().entry_by_id(commit_id)?;
        Some(entry.generation_number())
    }

    #[doc(hidden)] // for tests
    pub fn evaluate_revset_impl(
        &self,
        expression: &ResolvedExpression,
        store: &Arc<Store>,
    ) -> Result<DefaultReadonlyIndexRevset, RevsetEvaluationError> {
        let inner = revset_engine::evaluate(expression, store, self.clone())?;
        Ok(DefaultReadonlyIndexRevset { inner })
    }

    pub(super) fn start_modification(&self) -> DefaultMutableIndex {
        DefaultMutableIndex::incremental(self)
    }
}

impl AsCompositeIndex for DefaultReadonlyIndex {
    fn as_composite(&self) -> &CompositeIndex {
        &self.0
    }
}

impl Index for DefaultReadonlyIndex {
    fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        self.0.shortest_unique_commit_id_prefix_len(commit_id)
    }

    fn resolve_commit_id_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        self.0.resolve_commit_id_prefix(prefix)
    }

    fn has_id(&self, commit_id: &CommitId) -> bool {
        self.0.has_id(commit_id)
    }

    fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        self.0.is_ancestor(ancestor_id, descendant_id)
    }

    fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        self.0.common_ancestors(set1, set2)
    }

    fn all_heads_for_gc(&self) -> Result<Box<dyn Iterator<Item = CommitId> + '_>, IndexError> {
        self.0.all_heads_for_gc()
    }

    fn heads(
        &self,
        candidates: &mut dyn Iterator<Item = &CommitId>,
    ) -> Result<Vec<CommitId>, IndexError> {
        self.0.heads(candidates)
    }

    fn changed_paths_in_commit(
        &self,
        commit_id: &CommitId,
    ) -> Result<Option<Box<dyn Iterator<Item = RepoPathBuf> + '_>>, IndexError> {
        self.0.changed_paths_in_commit(commit_id)
    }

    fn evaluate_revset(
        &self,
        expression: &ResolvedExpression,
        store: &Arc<Store>,
    ) -> Result<Box<dyn Revset + '_>, RevsetEvaluationError> {
        self.0.evaluate_revset(expression, store)
    }
}

impl ReadonlyIndex for DefaultReadonlyIndex {
    fn as_index(&self) -> &dyn Index {
        self
    }

    fn change_id_index(
        &self,
        heads: &mut dyn Iterator<Item = &CommitId>,
    ) -> Box<dyn ChangeIdIndex> {
        Box::new(ChangeIdIndexImpl::new(self.clone(), heads))
    }

    fn start_modification(&self) -> Box<dyn MutableIndex> {
        Box::new(Self::start_modification(self))
    }
}

#[derive(Debug)]
#[doc(hidden)] // for tests
pub struct DefaultReadonlyIndexRevset {
    inner: RevsetImpl<DefaultReadonlyIndex>,
}

impl DefaultReadonlyIndexRevset {
    pub fn iter_graph_impl(
        &self,
        skip_transitive_edges: bool,
    ) -> impl Iterator<Item = Result<GraphNode<CommitId>, RevsetEvaluationError>> {
        self.inner.iter_graph_impl(skip_transitive_edges)
    }

    pub fn into_inner(self) -> Box<dyn Revset> {
        Box::new(self.inner)
    }
}

#[derive(Clone, Debug)]
pub struct IndexStats {
    pub num_commits: u32,
    pub num_merges: u32,
    pub max_generation_number: u32,
    pub num_heads: u32,
    pub num_changes: u32,
    pub commit_levels: Vec<CommitIndexLevelStats>,
    pub changed_path_commits_range: Option<Range<u32>>,
    pub changed_path_levels: Vec<ChangedPathIndexLevelStats>,
}

#[derive(Clone, Debug)]
pub struct CommitIndexLevelStats {
    pub num_commits: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ChangedPathIndexLevelStats {
    /// Number of commits.
    pub num_commits: u32,
    /// Sum of number of per-commit changed paths.
    pub num_changed_paths: u32,
    /// Number of unique paths.
    pub num_paths: u32,
    /// Index file name.
    pub name: String,
}

/// Binary search result in a sorted lookup table.
#[derive(Clone, Copy, Debug)]
struct PositionLookupResult {
    /// `Ok` means the element is found at the position. `Err` contains the
    /// position where the element could be inserted.
    result: Result<u32, u32>,
    size: u32,
}

impl PositionLookupResult {
    /// Returns position of the element if exactly matched.
    fn ok(self) -> Option<u32> {
        self.result.ok()
    }

    /// Returns `(previous, next)` positions of the matching element or
    /// boundary.
    fn neighbors(self) -> (Option<u32>, Option<u32>) {
        match self.result {
            Ok(pos) => (pos.checked_sub(1), (pos + 1..self.size).next()),
            Err(pos) => (pos.checked_sub(1), (pos..self.size).next()),
        }
    }

    /// Looks up `(previous, next)` elements by the given function.
    fn map_neighbors<T>(self, mut lookup: impl FnMut(u32) -> T) -> (Option<T>, Option<T>) {
        let (prev_pos, next_pos) = self.neighbors();
        (prev_pos.map(&mut lookup), next_pos.map(&mut lookup))
    }

    /// Looks up matching elements from the current position, returns one if
    /// the given `prefix` unambiguously matches.
    fn prefix_matches<T: ObjectId>(
        self,
        prefix: &HexPrefix,
        lookup: impl FnMut(u32) -> T,
    ) -> PrefixResolution<(T, u32)> {
        let lookup_pos = self.result.unwrap_or_else(|pos| pos);
        let mut matches = (lookup_pos..self.size)
            .map(lookup)
            .take_while(|id| prefix.matches(id))
            .fuse();
        match (matches.next(), matches.next()) {
            (Some(id), None) => PrefixResolution::SingleMatch((id, lookup_pos)),
            (Some(_), Some(_)) => PrefixResolution::AmbiguousMatch,
            (None, _) => PrefixResolution::NoMatch,
        }
    }
}

/// Binary searches u32 position with the given comparison function.
fn binary_search_pos_by(size: u32, mut f: impl FnMut(u32) -> Ordering) -> PositionLookupResult {
    let mut low = 0;
    let mut high = size;
    while low < high {
        let mid = (low + high) / 2;
        let cmp = f(mid);
        // According to Rust std lib, this produces cmov instructions.
        // https://github.com/rust-lang/rust/blob/1.76.0/library/core/src/slice/mod.rs#L2845-L2855
        low = if cmp == Ordering::Less { mid + 1 } else { low };
        high = if cmp == Ordering::Greater { mid } else { high };
        if cmp == Ordering::Equal {
            let result = Ok(mid);
            return PositionLookupResult { result, size };
        }
    }
    let result = Err(low);
    PositionLookupResult { result, size }
}
