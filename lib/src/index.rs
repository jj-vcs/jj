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

use std::cmp::{max, min, Ordering};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet};
use std::fmt::{Debug, Formatter};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Cursor, Read, Write};
use std::ops::{Bound, Range};
use std::path::PathBuf;
use std::sync::Arc;
use std::{io, iter};

use blake2::{Blake2b512, Digest};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use itertools::Itertools;
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::backend::{self, ChangeId, CommitId, ObjectId};
use crate::commit::Commit;
use crate::file_util::persist_content_addressed_temp_file;
#[cfg(not(feature = "map_first_last"))]
// This import is used on Rust 1.61, but not on recent version.
// TODO: Remove it when our MSRV becomes recent enough.
#[allow(unused_imports)]
use crate::nightly_shims::BTreeSetExt;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct IndexPosition(u32);

impl IndexPosition {
    pub const MAX: Self = IndexPosition(u32::MAX);
}

#[derive(Clone, Copy)]
pub enum IndexRef<'a> {
    Readonly(&'a ReadonlyIndex),
    Mutable(&'a MutableIndex),
}

impl<'a> From<&'a ReadonlyIndex> for IndexRef<'a> {
    fn from(index: &'a ReadonlyIndex) -> Self {
        IndexRef::Readonly(index)
    }
}

impl<'a> From<&'a MutableIndex> for IndexRef<'a> {
    fn from(index: &'a MutableIndex) -> Self {
        IndexRef::Mutable(index)
    }
}

impl<'a> IndexRef<'a> {
    pub fn num_commits(&self) -> u32 {
        match self {
            IndexRef::Readonly(index) => index.num_commits(),
            IndexRef::Mutable(index) => index.num_commits(),
        }
    }

    pub fn stats(&self) -> IndexStats {
        match self {
            IndexRef::Readonly(index) => index.stats(),
            IndexRef::Mutable(index) => index.stats(),
        }
    }

    pub fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        match self {
            IndexRef::Readonly(index) => index.commit_id_to_pos(commit_id),
            IndexRef::Mutable(index) => index.commit_id_to_pos(commit_id),
        }
    }

    pub fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        match self {
            IndexRef::Readonly(index) => index.shortest_unique_commit_id_prefix_len(commit_id),
            IndexRef::Mutable(index) => index.shortest_unique_commit_id_prefix_len(commit_id),
        }
    }

    pub fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        match self {
            IndexRef::Readonly(index) => index.resolve_prefix(prefix),
            IndexRef::Mutable(index) => index.resolve_prefix(prefix),
        }
    }

    pub fn entry_by_id(&self, commit_id: &CommitId) -> Option<IndexEntry<'a>> {
        match self {
            IndexRef::Readonly(index) => index.entry_by_id(commit_id),
            IndexRef::Mutable(index) => index.entry_by_id(commit_id),
        }
    }

    pub fn entry_by_pos(&self, pos: IndexPosition) -> IndexEntry<'a> {
        match self {
            IndexRef::Readonly(index) => index.entry_by_pos(pos),
            IndexRef::Mutable(index) => index.entry_by_pos(pos),
        }
    }

    pub fn has_id(&self, commit_id: &CommitId) -> bool {
        match self {
            IndexRef::Readonly(index) => index.has_id(commit_id),
            IndexRef::Mutable(index) => index.has_id(commit_id),
        }
    }

    pub fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        match self {
            IndexRef::Readonly(index) => index.is_ancestor(ancestor_id, descendant_id),
            IndexRef::Mutable(index) => index.is_ancestor(ancestor_id, descendant_id),
        }
    }

    pub fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        match self {
            IndexRef::Readonly(index) => index.common_ancestors(set1, set2),
            IndexRef::Mutable(index) => index.common_ancestors(set1, set2),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = IndexEntry<'a>> + '_ {
        (0..self.num_commits()).map(|pos| self.entry_by_pos(IndexPosition(pos)))
    }

    pub fn walk_revs(&self, wanted: &[CommitId], unwanted: &[CommitId]) -> RevWalk<'a> {
        match self {
            IndexRef::Readonly(index) => index.walk_revs(wanted, unwanted),
            IndexRef::Mutable(index) => index.walk_revs(wanted, unwanted),
        }
    }

    pub fn heads<'candidates>(
        &self,
        candidates: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<CommitId> {
        match self {
            IndexRef::Readonly(index) => index.heads(candidates),
            IndexRef::Mutable(index) => index.heads(candidates),
        }
    }
}

struct CommitGraphEntry<'a> {
    data: &'a [u8],
    commit_id_length: usize,
    change_id_length: usize,
}

// TODO: Add pointers to ancestors further back, like a skip list. Clear the
// lowest set bit to determine which generation number the pointers point to.
impl CommitGraphEntry<'_> {
    fn size(commit_id_length: usize, change_id_length: usize) -> usize {
        20 + commit_id_length + change_id_length
    }

    fn generation_number(&self) -> u32 {
        (&self.data[4..]).read_u32::<LittleEndian>().unwrap()
    }

    fn num_parents(&self) -> u32 {
        (&self.data[8..]).read_u32::<LittleEndian>().unwrap()
    }

    fn parent1_pos(&self) -> IndexPosition {
        IndexPosition((&self.data[12..]).read_u32::<LittleEndian>().unwrap())
    }

    fn parent2_overflow_pos(&self) -> u32 {
        (&self.data[16..]).read_u32::<LittleEndian>().unwrap()
    }

    // TODO: Consider storing the change ids in a separate table. That table could
    // be sorted by change id and have the end index into a list as value. That list
    // would be the concatenation of all index positions associated with the change.
    // Possible advantages: avoids duplicating change ids; smaller main graph leads
    // to better cache locality when walking it; ability to quickly find all
    // commits associated with a change id.
    fn change_id(&self) -> ChangeId {
        ChangeId::new(self.data[20..20 + self.change_id_length].to_vec())
    }

    fn commit_id(&self) -> CommitId {
        CommitId::from_bytes(
            &self.data
                [20 + self.change_id_length..20 + self.change_id_length + self.commit_id_length],
        )
    }
}

struct CommitLookupEntry<'a> {
    data: &'a [u8],
    commit_id_length: usize,
}

impl CommitLookupEntry<'_> {
    fn size(commit_id_length: usize) -> usize {
        commit_id_length + 4
    }

    fn commit_id(&self) -> CommitId {
        CommitId::from_bytes(self.commit_id_bytes())
    }

    // might be better to add borrowed version of CommitId
    fn commit_id_bytes(&self) -> &[u8] {
        &self.data[0..self.commit_id_length]
    }

    fn pos(&self) -> IndexPosition {
        IndexPosition(
            (&self.data[self.commit_id_length..self.commit_id_length + 4])
                .read_u32::<LittleEndian>()
                .unwrap(),
        )
    }
}

#[derive(Error, Debug)]
pub enum IndexLoadError {
    #[error("Index file '{0}' is corrupt.")]
    IndexCorrupt(String),
    #[error("I/O error while loading index file: {0}")]
    IoError(#[from] io::Error),
}

// File format:
// u32: number of entries
// u32: number of parent overflow entries
// for each entry, in some topological order with parents first:
//   u32: generation number
//   u32: number of parents
//   u32: position in this table for parent 1
//   u32: position in the overflow table of parent 2
//   <hash length number of bytes>: commit id
// for each entry, sorted by commit id:
//   <hash length number of bytes>: commit id
//    u32: position in the entry table above
// TODO: add a version number
// TODO: replace the table by a trie so we don't have to repeat the full commit
//       ids
// TODO: add a fanout table like git's commit graph has?
pub struct ReadonlyIndex {
    parent_file: Option<Arc<ReadonlyIndex>>,
    num_parent_commits: u32,
    name: String,
    commit_id_length: usize,
    change_id_length: usize,
    commit_graph_entry_size: usize,
    commit_lookup_entry_size: usize,
    // Number of commits not counting the parent file
    num_local_commits: u32,
    graph: Vec<u8>,
    lookup: Vec<u8>,
    overflow_parent: Vec<u8>,
}

impl Debug for ReadonlyIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("ReadonlyIndex")
            .field("name", &self.name)
            .field("parent_file", &self.parent_file)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexPrefix {
    // For odd-length prefix, lower 4 bits of the last byte is padded with 0
    min_prefix_bytes: Vec<u8>,
    has_odd_byte: bool,
}

impl HexPrefix {
    pub fn new(prefix: &str) -> Option<HexPrefix> {
        let has_odd_byte = prefix.len() & 1 != 0;
        let min_prefix_bytes = if has_odd_byte {
            hex::decode(prefix.to_owned() + "0").ok()?
        } else {
            hex::decode(prefix).ok()?
        };
        Some(HexPrefix {
            min_prefix_bytes,
            has_odd_byte,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        HexPrefix {
            min_prefix_bytes: bytes.to_owned(),
            has_odd_byte: false,
        }
    }

    pub fn hex(&self) -> String {
        let mut hex_string = hex::encode(&self.min_prefix_bytes);
        if self.has_odd_byte {
            hex_string.pop().unwrap();
        }
        hex_string
    }

    /// Minimum bytes that would match this prefix. (e.g. "abc0" for "abc")
    ///
    /// Use this to partition a sorted slice, and test `matches(id)` from there.
    pub fn min_prefix_bytes(&self) -> &[u8] {
        &self.min_prefix_bytes
    }

    fn split_odd_byte(&self) -> (Option<u8>, &[u8]) {
        if self.has_odd_byte {
            let (&odd, prefix) = self.min_prefix_bytes.split_last().unwrap();
            (Some(odd), prefix)
        } else {
            (None, &self.min_prefix_bytes)
        }
    }

    pub fn matches<Q: ObjectId>(&self, id: &Q) -> bool {
        let id_bytes = id.as_bytes();
        let (maybe_odd, prefix) = self.split_odd_byte();
        if id_bytes.starts_with(prefix) {
            if let Some(odd) = maybe_odd {
                matches!(id_bytes.get(prefix.len()), Some(v) if v & 0xf0 == odd)
            } else {
                true
            }
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixResolution<T> {
    NoMatch,
    SingleMatch(T),
    AmbiguousMatch,
}

impl<T: Clone> PrefixResolution<T> {
    fn plus(&self, other: &PrefixResolution<T>) -> PrefixResolution<T> {
        match (self, other) {
            (PrefixResolution::NoMatch, other) => other.clone(),
            (local, PrefixResolution::NoMatch) => local.clone(),
            (PrefixResolution::AmbiguousMatch, _) => PrefixResolution::AmbiguousMatch,
            (_, PrefixResolution::AmbiguousMatch) => PrefixResolution::AmbiguousMatch,
            (PrefixResolution::SingleMatch(_), PrefixResolution::SingleMatch(_)) => {
                PrefixResolution::AmbiguousMatch
            }
        }
    }
}

#[derive(Debug)]
struct MutableGraphEntry {
    commit_id: CommitId,
    change_id: ChangeId,
    generation_number: u32,
    parent_positions: Vec<IndexPosition>,
}

pub struct MutableIndex {
    parent_file: Option<Arc<ReadonlyIndex>>,
    num_parent_commits: u32,
    commit_id_length: usize,
    change_id_length: usize,
    graph: Vec<MutableGraphEntry>,
    lookup: BTreeMap<CommitId, IndexPosition>,
}

impl MutableIndex {
    pub(crate) fn full(commit_id_length: usize, change_id_length: usize) -> Self {
        Self {
            parent_file: None,
            num_parent_commits: 0,
            commit_id_length,
            change_id_length,
            graph: vec![],
            lookup: BTreeMap::new(),
        }
    }

    pub fn incremental(parent_file: Arc<ReadonlyIndex>) -> Self {
        let num_parent_commits = parent_file.num_parent_commits + parent_file.num_local_commits;
        let commit_id_length = parent_file.commit_id_length;
        let change_id_length = parent_file.change_id_length;
        Self {
            parent_file: Some(parent_file),
            num_parent_commits,
            commit_id_length,
            change_id_length,
            graph: vec![],
            lookup: BTreeMap::new(),
        }
    }

    pub fn as_index_ref(&self) -> IndexRef {
        IndexRef::Mutable(self)
    }

    pub fn add_commit(&mut self, commit: &Commit) {
        self.add_commit_data(
            commit.id().clone(),
            commit.change_id().clone(),
            commit.parent_ids(),
        );
    }

    pub(crate) fn add_commit_data(
        &mut self,
        commit_id: CommitId,
        change_id: ChangeId,
        parent_ids: &[CommitId],
    ) {
        if self.has_id(&commit_id) {
            return;
        }
        let mut entry = MutableGraphEntry {
            commit_id,
            change_id,
            generation_number: 0,
            parent_positions: vec![],
        };
        for parent_id in parent_ids {
            let parent_entry = self
                .entry_by_id(parent_id)
                .expect("parent commit is not indexed");
            entry.generation_number = max(
                entry.generation_number,
                parent_entry.generation_number() + 1,
            );
            entry.parent_positions.push(parent_entry.pos);
        }
        self.lookup.insert(
            entry.commit_id.clone(),
            IndexPosition(self.graph.len() as u32 + self.num_parent_commits),
        );
        self.graph.push(entry);
    }

    fn add_commits_from(&mut self, other_segment: &dyn IndexSegment) {
        let other = CompositeIndex(other_segment);
        for pos in other_segment.segment_num_parent_commits()..other.num_commits() {
            let entry = other.entry_by_pos(IndexPosition(pos));
            let parent_ids = entry
                .parents()
                .iter()
                .map(|entry| entry.commit_id())
                .collect_vec();
            self.add_commit_data(entry.commit_id(), entry.change_id(), &parent_ids);
        }
    }

    pub fn merge_in(&mut self, other: &Arc<ReadonlyIndex>) {
        let mut maybe_own_ancestor = self.parent_file.clone();
        let mut maybe_other_ancestor = Some(other.clone());
        let mut files_to_add = vec![];
        loop {
            if maybe_other_ancestor.is_none() {
                break;
            }
            let other_ancestor = maybe_other_ancestor.as_ref().unwrap();
            if maybe_own_ancestor.is_none() {
                files_to_add.push(other_ancestor.clone());
                maybe_other_ancestor = other_ancestor.parent_file.clone();
                continue;
            }
            let own_ancestor = maybe_own_ancestor.as_ref().unwrap();
            if own_ancestor.name == other_ancestor.name {
                break;
            }
            if own_ancestor.num_commits() < other_ancestor.num_commits() {
                files_to_add.push(other_ancestor.clone());
                maybe_other_ancestor = other_ancestor.parent_file.clone();
            } else {
                maybe_own_ancestor = own_ancestor.parent_file.clone();
            }
        }

        for file in files_to_add.iter().rev() {
            self.add_commits_from(file.as_ref());
        }
    }

    fn serialize(self) -> Vec<u8> {
        assert_eq!(self.graph.len(), self.lookup.len());

        let num_commits = self.graph.len() as u32;

        let mut buf = vec![];

        if let Some(parent_file) = &self.parent_file {
            buf.write_u32::<LittleEndian>(parent_file.name.len() as u32)
                .unwrap();
            buf.write_all(parent_file.name.as_bytes()).unwrap();
        } else {
            buf.write_u32::<LittleEndian>(0).unwrap();
        }

        buf.write_u32::<LittleEndian>(num_commits).unwrap();
        // We'll write the actual value later
        let parent_overflow_offset = buf.len();
        buf.write_u32::<LittleEndian>(0_u32).unwrap();

        let mut parent_overflow = vec![];
        for entry in self.graph {
            let flags = 0;
            buf.write_u32::<LittleEndian>(flags).unwrap();

            buf.write_u32::<LittleEndian>(entry.generation_number)
                .unwrap();

            buf.write_u32::<LittleEndian>(entry.parent_positions.len() as u32)
                .unwrap();
            let mut parent1_pos = IndexPosition(0);
            let parent_overflow_pos = parent_overflow.len() as u32;
            for (i, parent_pos) in entry.parent_positions.iter().enumerate() {
                if i == 0 {
                    parent1_pos = *parent_pos;
                } else {
                    parent_overflow.push(*parent_pos);
                }
            }
            buf.write_u32::<LittleEndian>(parent1_pos.0).unwrap();
            buf.write_u32::<LittleEndian>(parent_overflow_pos).unwrap();

            assert_eq!(entry.change_id.as_bytes().len(), self.change_id_length);
            buf.write_all(entry.change_id.as_bytes()).unwrap();

            assert_eq!(entry.commit_id.as_bytes().len(), self.commit_id_length);
            buf.write_all(entry.commit_id.as_bytes()).unwrap();
        }

        for (commit_id, pos) in self.lookup {
            buf.write_all(commit_id.as_bytes()).unwrap();
            buf.write_u32::<LittleEndian>(pos.0).unwrap();
        }

        buf[parent_overflow_offset..parent_overflow_offset + 4]
            .as_mut()
            .write_u32::<LittleEndian>(parent_overflow.len() as u32)
            .unwrap();
        for parent_pos in parent_overflow {
            buf.write_u32::<LittleEndian>(parent_pos.0).unwrap();
        }

        buf
    }

    /// If the MutableIndex has more than half the commits of its parent
    /// ReadonlyIndex, return MutableIndex with the commits from both. This
    /// is done recursively, so the stack of index files has O(log n) files.
    fn maybe_squash_with_ancestors(self) -> MutableIndex {
        let mut num_new_commits = self.segment_num_commits();
        let mut files_to_squash = vec![];
        let mut maybe_parent_file = self.parent_file.clone();
        let mut squashed;
        loop {
            match maybe_parent_file {
                Some(parent_file) => {
                    // TODO: We should probably also squash if the parent file has less than N
                    // commits, regardless of how many (few) are in `self`.
                    if 2 * num_new_commits < parent_file.segment_num_commits() {
                        squashed = MutableIndex::incremental(parent_file);
                        break;
                    }
                    num_new_commits += parent_file.segment_num_commits();
                    files_to_squash.push(parent_file.clone());
                    maybe_parent_file = parent_file.parent_file.clone();
                }
                None => {
                    squashed = MutableIndex::full(self.commit_id_length, self.change_id_length);
                    break;
                }
            }
        }

        if files_to_squash.is_empty() {
            return self;
        }

        for parent_file in files_to_squash.iter().rev() {
            squashed.add_commits_from(parent_file.as_ref());
        }
        squashed.add_commits_from(&self);
        squashed
    }

    pub fn save_in(self, dir: PathBuf) -> io::Result<Arc<ReadonlyIndex>> {
        if self.segment_num_commits() == 0 && self.parent_file.is_some() {
            return Ok(self.parent_file.unwrap());
        }

        let commit_id_length = self.commit_id_length;
        let change_id_length = self.change_id_length;

        let buf = self.maybe_squash_with_ancestors().serialize();
        let mut hasher = Blake2b512::new();
        hasher.update(&buf);
        let index_file_id_hex = hex::encode(hasher.finalize());
        let index_file_path = dir.join(&index_file_id_hex);

        let mut temp_file = NamedTempFile::new_in(&dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(&buf)?;
        persist_content_addressed_temp_file(temp_file, index_file_path)?;

        let mut cursor = Cursor::new(&buf);
        ReadonlyIndex::load_from(
            &mut cursor,
            dir,
            index_file_id_hex,
            commit_id_length,
            change_id_length,
        )
        .map_err(|err| match err {
            IndexLoadError::IndexCorrupt(err) => {
                panic!("Just-created index file is corrupt: {err}")
            }
            IndexLoadError::IoError(err) => err,
        })
    }

    pub fn num_commits(&self) -> u32 {
        CompositeIndex(self).num_commits()
    }

    pub fn stats(&self) -> IndexStats {
        CompositeIndex(self).stats()
    }

    pub fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        CompositeIndex(self).commit_id_to_pos(commit_id)
    }

    pub fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        CompositeIndex(self).shortest_unique_commit_id_prefix_len(commit_id)
    }

    pub fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        CompositeIndex(self).resolve_prefix(prefix)
    }

    pub fn entry_by_id(&self, commit_id: &CommitId) -> Option<IndexEntry> {
        CompositeIndex(self).entry_by_id(commit_id)
    }

    pub fn entry_by_pos(&self, pos: IndexPosition) -> IndexEntry {
        CompositeIndex(self).entry_by_pos(pos)
    }

    pub fn has_id(&self, commit_id: &CommitId) -> bool {
        CompositeIndex(self).has_id(commit_id)
    }

    pub fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        CompositeIndex(self).is_ancestor(ancestor_id, descendant_id)
    }

    pub fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        CompositeIndex(self).common_ancestors(set1, set2)
    }

    pub fn walk_revs(&self, wanted: &[CommitId], unwanted: &[CommitId]) -> RevWalk {
        CompositeIndex(self).walk_revs(wanted, unwanted)
    }

    pub fn heads<'candidates>(
        &self,
        candidates: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<CommitId> {
        CompositeIndex(self).heads(candidates)
    }

    pub fn topo_order<'candidates>(
        &self,
        input: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<IndexEntry> {
        CompositeIndex(self).topo_order(input)
    }
}

trait IndexSegment {
    fn segment_num_parent_commits(&self) -> u32;

    fn segment_num_commits(&self) -> u32;

    fn segment_parent_file(&self) -> Option<&Arc<ReadonlyIndex>>;

    fn segment_name(&self) -> Option<String>;

    fn segment_commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition>;

    /// Suppose the given `commit_id` exists, returns the positions of the
    /// previous and next commit ids in lexicographical order.
    fn segment_commit_id_to_neighbor_positions(
        &self,
        commit_id: &CommitId,
    ) -> (Option<IndexPosition>, Option<IndexPosition>);

    fn segment_resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId>;

    fn segment_generation_number(&self, local_pos: u32) -> u32;

    fn segment_commit_id(&self, local_pos: u32) -> CommitId;

    fn segment_change_id(&self, local_pos: u32) -> ChangeId;

    fn segment_num_parents(&self, local_pos: u32) -> u32;

    fn segment_parent_positions(&self, local_pos: u32) -> Vec<IndexPosition>;

    fn segment_entry_by_pos(&self, pos: IndexPosition, local_pos: u32) -> IndexEntry;
}

#[derive(Clone)]
struct CompositeIndex<'a>(&'a dyn IndexSegment);

impl<'a> CompositeIndex<'a> {
    fn ancestor_files_without_local(&self) -> impl Iterator<Item = &Arc<ReadonlyIndex>> {
        let parent_file = self.0.segment_parent_file();
        iter::successors(parent_file, |file| file.segment_parent_file())
    }

    fn ancestor_index_segments(&self) -> impl Iterator<Item = &dyn IndexSegment> {
        iter::once(self.0).chain(
            self.ancestor_files_without_local()
                .map(|file| file.as_ref() as &dyn IndexSegment),
        )
    }

    pub fn num_commits(&self) -> u32 {
        self.0.segment_num_parent_commits() + self.0.segment_num_commits()
    }

    pub fn stats(&self) -> IndexStats {
        let num_commits = self.num_commits();
        let mut num_merges = 0;
        let mut max_generation_number = 0;
        let mut is_head = vec![true; num_commits as usize];
        let mut change_ids = HashSet::new();
        for pos in 0..num_commits {
            let entry = self.entry_by_pos(IndexPosition(pos));
            max_generation_number = max(max_generation_number, entry.generation_number());
            if entry.num_parents() > 1 {
                num_merges += 1;
            }
            for parent_pos in entry.parent_positions() {
                is_head[parent_pos.0 as usize] = false;
            }
            change_ids.insert(entry.change_id());
        }
        let num_heads = is_head.iter().filter(|is_head| **is_head).count() as u32;

        let mut levels = self
            .ancestor_index_segments()
            .map(|segment| IndexLevelStats {
                num_commits: segment.segment_num_commits(),
                name: segment.segment_name(),
            })
            .collect_vec();
        levels.reverse();

        IndexStats {
            num_commits,
            num_merges,
            max_generation_number,
            num_heads,
            num_changes: change_ids.len() as u32,
            levels,
        }
    }

    fn entry_by_pos(&self, pos: IndexPosition) -> IndexEntry<'a> {
        let num_parent_commits = self.0.segment_num_parent_commits();
        if pos.0 >= num_parent_commits {
            self.0.segment_entry_by_pos(pos, pos.0 - num_parent_commits)
        } else {
            let parent_file: &ReadonlyIndex = self.0.segment_parent_file().unwrap().as_ref();
            // The parent ReadonlyIndex outlives the child
            let parent_file: &'a ReadonlyIndex = unsafe { std::mem::transmute(parent_file) };

            CompositeIndex(parent_file).entry_by_pos(pos)
        }
    }

    pub fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        self.ancestor_index_segments()
            .find_map(|segment| segment.segment_commit_id_to_pos(commit_id))
    }

    /// Suppose the given `commit_id` exists, returns the minimum prefix length
    /// to disambiguate it. The length to be returned is a number of hexadecimal
    /// digits.
    ///
    /// If the given `commit_id` doesn't exist, this will return the prefix
    /// length that never matches with any commit ids.
    pub fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        let (prev_id, next_id) = self.resolve_neighbor_commit_ids(commit_id);
        itertools::chain(prev_id, next_id)
            .map(|id| backend::common_hex_len(commit_id.as_bytes(), id.as_bytes()) + 1)
            .max()
            .unwrap_or(0)
    }

    /// Suppose the given `commit_id` exists, returns the previous and next
    /// commit ids in lexicographical order.
    fn resolve_neighbor_commit_ids(
        &self,
        commit_id: &CommitId,
    ) -> (Option<CommitId>, Option<CommitId>) {
        self.ancestor_index_segments()
            .map(|segment| {
                let num_parent_commits = segment.segment_num_parent_commits();
                let to_local_pos = |pos: IndexPosition| pos.0 - num_parent_commits;
                let (prev_pos, next_pos) =
                    segment.segment_commit_id_to_neighbor_positions(commit_id);
                (
                    prev_pos.map(|p| segment.segment_commit_id(to_local_pos(p))),
                    next_pos.map(|p| segment.segment_commit_id(to_local_pos(p))),
                )
            })
            .reduce(|(acc_prev_id, acc_next_id), (prev_id, next_id)| {
                (
                    acc_prev_id.into_iter().chain(prev_id).max(),
                    acc_next_id.into_iter().chain(next_id).min(),
                )
            })
            .unwrap()
    }

    pub fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        self.ancestor_index_segments()
            .fold(PrefixResolution::NoMatch, |acc_match, segment| {
                if acc_match == PrefixResolution::AmbiguousMatch {
                    acc_match // avoid checking the parent file(s)
                } else {
                    let local_match = segment.segment_resolve_prefix(prefix);
                    acc_match.plus(&local_match)
                }
            })
    }

    pub fn entry_by_id(&self, commit_id: &CommitId) -> Option<IndexEntry<'a>> {
        self.commit_id_to_pos(commit_id)
            .map(|pos| self.entry_by_pos(pos))
    }

    pub fn has_id(&self, commit_id: &CommitId) -> bool {
        self.commit_id_to_pos(commit_id).is_some()
    }

    pub fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        let ancestor_pos = self.commit_id_to_pos(ancestor_id).unwrap();
        let descendant_pos = self.commit_id_to_pos(descendant_id).unwrap();
        self.is_ancestor_pos(ancestor_pos, descendant_pos)
    }

    fn is_ancestor_pos(&self, ancestor_pos: IndexPosition, descendant_pos: IndexPosition) -> bool {
        let ancestor_generation = self.entry_by_pos(ancestor_pos).generation_number();
        let mut work = vec![descendant_pos];
        let mut visited = HashSet::new();
        while let Some(descendant_pos) = work.pop() {
            let descendant_entry = self.entry_by_pos(descendant_pos);
            if descendant_pos == ancestor_pos {
                return true;
            }
            if !visited.insert(descendant_entry.pos) {
                continue;
            }
            if descendant_entry.generation_number() <= ancestor_generation {
                continue;
            }
            work.extend(descendant_entry.parent_positions());
        }
        false
    }

    pub fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        let pos1 = set1
            .iter()
            .map(|id| self.commit_id_to_pos(id).unwrap())
            .collect_vec();
        let pos2 = set2
            .iter()
            .map(|id| self.commit_id_to_pos(id).unwrap())
            .collect_vec();
        self.common_ancestors_pos(&pos1, &pos2)
            .iter()
            .map(|pos| self.entry_by_pos(*pos).commit_id())
            .collect()
    }

    fn common_ancestors_pos(
        &self,
        set1: &[IndexPosition],
        set2: &[IndexPosition],
    ) -> BTreeSet<IndexPosition> {
        let mut items1: BTreeSet<_> = set1
            .iter()
            .map(|pos| IndexEntryByGeneration(self.entry_by_pos(*pos)))
            .collect();
        let mut items2: BTreeSet<_> = set2
            .iter()
            .map(|pos| IndexEntryByGeneration(self.entry_by_pos(*pos)))
            .collect();

        let mut result = BTreeSet::new();
        while !(items1.is_empty() || items2.is_empty()) {
            #[allow(unstable_name_collisions)]
            let entry1 = items1.last().unwrap();
            #[allow(unstable_name_collisions)]
            let entry2 = items2.last().unwrap();
            match entry1.cmp(entry2) {
                Ordering::Greater => {
                    #[allow(unstable_name_collisions)]
                    let entry1 = items1.pop_last().unwrap();
                    for parent_entry in entry1.0.parents() {
                        items1.insert(IndexEntryByGeneration(parent_entry));
                    }
                }
                Ordering::Less => {
                    #[allow(unstable_name_collisions)]
                    let entry2 = items2.pop_last().unwrap();
                    for parent_entry in entry2.0.parents() {
                        items2.insert(IndexEntryByGeneration(parent_entry));
                    }
                }
                Ordering::Equal => {
                    result.insert(entry1.0.pos);
                    #[allow(unstable_name_collisions)]
                    items1.pop_last();
                    #[allow(unstable_name_collisions)]
                    items2.pop_last();
                }
            }
        }
        self.heads_pos(result)
    }

    pub fn walk_revs(&self, wanted: &[CommitId], unwanted: &[CommitId]) -> RevWalk<'a> {
        let mut rev_walk = RevWalk::new(self.clone());
        for pos in wanted.iter().map(|id| self.commit_id_to_pos(id).unwrap()) {
            rev_walk.add_wanted(pos);
        }
        for pos in unwanted.iter().map(|id| self.commit_id_to_pos(id).unwrap()) {
            rev_walk.add_unwanted(pos);
        }
        rev_walk
    }

    pub fn heads<'candidates>(
        &self,
        candidate_ids: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<CommitId> {
        let candidate_positions: BTreeSet<_> = candidate_ids
            .into_iter()
            .map(|id| self.commit_id_to_pos(id).unwrap())
            .collect();

        self.heads_pos(candidate_positions)
            .iter()
            .map(|pos| self.entry_by_pos(*pos).commit_id())
            .collect()
    }

    fn heads_pos(
        &self,
        mut candidate_positions: BTreeSet<IndexPosition>,
    ) -> BTreeSet<IndexPosition> {
        // Add all parents of the candidates to the work queue. The parents and their
        // ancestors are not heads.
        // Also find the smallest generation number among the candidates.
        let mut work = BinaryHeap::new();
        let mut min_generation = u32::MAX;
        for pos in &candidate_positions {
            let entry = self.entry_by_pos(*pos);
            min_generation = min(min_generation, entry.generation_number());
            for parent_entry in entry.parents() {
                work.push(IndexEntryByGeneration(parent_entry));
            }
        }

        // Walk ancestors of the parents of the candidates. Remove visited commits from
        // set of candidates. Stop walking when we have gone past the minimum
        // candidate generation.
        let mut visited = HashSet::new();
        while let Some(IndexEntryByGeneration(item)) = work.pop() {
            if !visited.insert(item.pos) {
                continue;
            }
            if item.generation_number() < min_generation {
                break;
            }
            candidate_positions.remove(&item.pos);
            for parent_entry in item.parents() {
                work.push(IndexEntryByGeneration(parent_entry));
            }
        }
        candidate_positions
    }

    pub fn topo_order<'input>(
        &self,
        input: impl IntoIterator<Item = &'input CommitId>,
    ) -> Vec<IndexEntry<'a>> {
        let mut entries_by_generation = input
            .into_iter()
            .map(|id| IndexEntryByPosition(self.entry_by_id(id).unwrap()))
            .collect_vec();
        entries_by_generation.sort();
        entries_by_generation
            .into_iter()
            .map(|key| key.0)
            .collect_vec()
    }
}

pub struct IndexLevelStats {
    pub num_commits: u32,
    pub name: Option<String>,
}

pub struct IndexStats {
    pub num_commits: u32,
    pub num_merges: u32,
    pub max_generation_number: u32,
    pub num_heads: u32,
    pub num_changes: u32,
    pub levels: Vec<IndexLevelStats>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct IndexEntryByPosition<'a>(IndexEntry<'a>);

impl Ord for IndexEntryByPosition<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.pos.cmp(&other.0.pos)
    }
}

impl PartialOrd for IndexEntryByPosition<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Eq, PartialEq)]
struct IndexEntryByGeneration<'a>(IndexEntry<'a>);

impl Ord for IndexEntryByGeneration<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .generation_number()
            .cmp(&other.0.generation_number())
            .then(self.0.pos.cmp(&other.0.pos))
    }
}

impl PartialOrd for IndexEntryByGeneration<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct RevWalkWorkItem<'a, T> {
    entry: IndexEntryByPosition<'a>,
    state: RevWalkWorkItemState<T>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RevWalkWorkItemState<T> {
    // Order matters: Unwanted should appear earlier in the max-heap.
    Wanted(T),
    Unwanted,
}

impl<'a, T> RevWalkWorkItem<'a, T> {
    fn is_wanted(&self) -> bool {
        matches!(self.state, RevWalkWorkItemState::Wanted(_))
    }

    fn map_wanted<U>(self, f: impl FnOnce(T) -> U) -> RevWalkWorkItem<'a, U> {
        RevWalkWorkItem {
            entry: self.entry,
            state: match self.state {
                RevWalkWorkItemState::Wanted(t) => RevWalkWorkItemState::Wanted(f(t)),
                RevWalkWorkItemState::Unwanted => RevWalkWorkItemState::Unwanted,
            },
        }
    }
}

#[derive(Clone)]
struct RevWalkQueue<'a, T> {
    index: CompositeIndex<'a>,
    items: BinaryHeap<RevWalkWorkItem<'a, T>>,
    unwanted_count: usize,
}

impl<'a, T: Ord> RevWalkQueue<'a, T> {
    fn new(index: CompositeIndex<'a>) -> Self {
        Self {
            index,
            items: BinaryHeap::new(),
            unwanted_count: 0,
        }
    }

    fn map_wanted<U: Ord>(self, mut f: impl FnMut(T) -> U) -> RevWalkQueue<'a, U> {
        RevWalkQueue {
            index: self.index,
            items: self
                .items
                .into_iter()
                .map(|x| x.map_wanted(&mut f))
                .collect(),
            unwanted_count: self.unwanted_count,
        }
    }

    fn push_wanted(&mut self, pos: IndexPosition, t: T) {
        self.items.push(RevWalkWorkItem {
            entry: IndexEntryByPosition(self.index.entry_by_pos(pos)),
            state: RevWalkWorkItemState::Wanted(t),
        });
    }

    fn push_unwanted(&mut self, pos: IndexPosition) {
        self.items.push(RevWalkWorkItem {
            entry: IndexEntryByPosition(self.index.entry_by_pos(pos)),
            state: RevWalkWorkItemState::Unwanted,
        });
        self.unwanted_count += 1;
    }

    fn push_wanted_parents(&mut self, entry: &IndexEntry<'_>, t: T)
    where
        T: Clone,
    {
        for pos in entry.parent_positions() {
            self.push_wanted(pos, t.clone());
        }
    }

    fn push_unwanted_parents(&mut self, entry: &IndexEntry<'_>) {
        for pos in entry.parent_positions() {
            self.push_unwanted(pos);
        }
    }

    fn pop(&mut self) -> Option<RevWalkWorkItem<'a, T>> {
        if let Some(x) = self.items.pop() {
            self.unwanted_count -= !x.is_wanted() as usize;
            Some(x)
        } else {
            None
        }
    }

    fn pop_eq(&mut self, entry: &IndexEntry<'_>) -> Option<RevWalkWorkItem<'a, T>> {
        if let Some(x) = self.items.peek() {
            (&x.entry.0 == entry).then(|| self.pop().unwrap())
        } else {
            None
        }
    }

    fn skip_while_eq(&mut self, entry: &IndexEntry<'_>) {
        while self.pop_eq(entry).is_some() {
            continue;
        }
    }
}

#[derive(Clone)]
pub struct RevWalk<'a> {
    queue: RevWalkQueue<'a, ()>,
}

impl<'a> RevWalk<'a> {
    fn new(index: CompositeIndex<'a>) -> Self {
        let queue = RevWalkQueue::new(index);
        Self { queue }
    }

    fn add_wanted(&mut self, pos: IndexPosition) {
        self.queue.push_wanted(pos, ());
    }

    fn add_unwanted(&mut self, pos: IndexPosition) {
        self.queue.push_unwanted(pos);
    }

    /// Filters entries by generation (or depth from the current wanted set.)
    ///
    /// The generation of the current wanted entries starts from 0.
    pub fn filter_by_generation(self, generation_range: Range<u32>) -> RevWalkGenerationRange<'a> {
        RevWalkGenerationRange {
            queue: self.queue.map_wanted(|()| 0),
            generation_range,
        }
    }
}

impl<'a> Iterator for RevWalk<'a> {
    type Item = IndexEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.queue.pop() {
            self.queue.skip_while_eq(&item.entry.0);
            if item.is_wanted() {
                self.queue.push_wanted_parents(&item.entry.0, ());
                return Some(item.entry.0);
            } else if self.queue.items.len() == self.queue.unwanted_count {
                // No more wanted entries to walk
                debug_assert!(!self.queue.items.iter().any(|x| x.is_wanted()));
                return None;
            } else {
                self.queue.push_unwanted_parents(&item.entry.0);
            }
        }

        debug_assert_eq!(
            self.queue.items.iter().filter(|x| !x.is_wanted()).count(),
            self.queue.unwanted_count
        );
        None
    }
}

#[derive(Clone)]
pub struct RevWalkGenerationRange<'a> {
    queue: RevWalkQueue<'a, u32>,
    generation_range: Range<u32>,
}

impl<'a> Iterator for RevWalkGenerationRange<'a> {
    type Item = IndexEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(item) = self.queue.pop() {
            if let RevWalkWorkItemState::Wanted(mut known_gen) = item.state {
                let mut some_in_range = self.generation_range.contains(&known_gen);
                if known_gen + 1 < self.generation_range.end {
                    self.queue.push_wanted_parents(&item.entry.0, known_gen + 1);
                }
                while let Some(x) = self.queue.pop_eq(&item.entry.0) {
                    // For wanted item, simply track all generation chains. This can
                    // be optimized if the wanted range is just upper/lower bounded.
                    // If the range is fully bounded and if the range is wide, we
                    // can instead extend 'gen' to a range of the same width, and
                    // merge overlapping generation ranges.
                    match x.state {
                        RevWalkWorkItemState::Wanted(gen) if known_gen != gen => {
                            some_in_range |= self.generation_range.contains(&gen);
                            if gen + 1 < self.generation_range.end {
                                self.queue.push_wanted_parents(&item.entry.0, gen + 1);
                            }
                            known_gen = gen;
                        }
                        RevWalkWorkItemState::Wanted(_) => {}
                        RevWalkWorkItemState::Unwanted => unreachable!(),
                    }
                }
                if some_in_range {
                    return Some(item.entry.0);
                }
            } else if self.queue.items.len() == self.queue.unwanted_count {
                // No more wanted entries to walk
                debug_assert!(!self.queue.items.iter().any(|x| x.is_wanted()));
                return None;
            } else {
                self.queue.skip_while_eq(&item.entry.0);
                self.queue.push_unwanted_parents(&item.entry.0);
            }
        }

        debug_assert_eq!(
            self.queue.items.iter().filter(|x| !x.is_wanted()).count(),
            self.queue.unwanted_count
        );
        None
    }
}

impl IndexSegment for ReadonlyIndex {
    fn segment_num_parent_commits(&self) -> u32 {
        self.num_parent_commits
    }

    fn segment_num_commits(&self) -> u32 {
        self.num_local_commits
    }

    fn segment_parent_file(&self) -> Option<&Arc<ReadonlyIndex>> {
        self.parent_file.as_ref()
    }

    fn segment_name(&self) -> Option<String> {
        Some(self.name.clone())
    }

    fn segment_commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        let lookup_pos = self.commit_id_byte_prefix_to_lookup_pos(commit_id)?;
        let entry = self.lookup_entry(lookup_pos);
        (&entry.commit_id() == commit_id).then(|| entry.pos())
    }

    fn segment_commit_id_to_neighbor_positions(
        &self,
        commit_id: &CommitId,
    ) -> (Option<IndexPosition>, Option<IndexPosition>) {
        if let Some(lookup_pos) = self.commit_id_byte_prefix_to_lookup_pos(commit_id) {
            let entry_commit_id = self.lookup_entry(lookup_pos).commit_id();
            let (prev_lookup_pos, next_lookup_pos) = match entry_commit_id.cmp(commit_id) {
                Ordering::Less => {
                    assert_eq!(lookup_pos + 1, self.num_local_commits);
                    (Some(lookup_pos), None)
                }
                Ordering::Equal => {
                    let succ = ((lookup_pos + 1)..self.num_local_commits).next();
                    (lookup_pos.checked_sub(1), succ)
                }
                Ordering::Greater => (lookup_pos.checked_sub(1), Some(lookup_pos)),
            };
            let prev_pos = prev_lookup_pos.map(|p| self.lookup_entry(p).pos());
            let next_pos = next_lookup_pos.map(|p| self.lookup_entry(p).pos());
            (prev_pos, next_pos)
        } else {
            (None, None)
        }
    }

    fn segment_resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        let min_bytes_prefix = CommitId::from_bytes(prefix.min_prefix_bytes());
        let lookup_pos = self
            .commit_id_byte_prefix_to_lookup_pos(&min_bytes_prefix)
            .unwrap_or(self.num_local_commits);
        let mut matches = (lookup_pos..self.num_local_commits)
            .map(|pos| self.lookup_entry(pos).commit_id())
            .take_while(|id| prefix.matches(id))
            .fuse();
        match (matches.next(), matches.next()) {
            (Some(id), None) => PrefixResolution::SingleMatch(id),
            (Some(_), Some(_)) => PrefixResolution::AmbiguousMatch,
            (None, _) => PrefixResolution::NoMatch,
        }
    }

    fn segment_generation_number(&self, local_pos: u32) -> u32 {
        self.graph_entry(local_pos).generation_number()
    }

    fn segment_commit_id(&self, local_pos: u32) -> CommitId {
        self.graph_entry(local_pos).commit_id()
    }

    fn segment_change_id(&self, local_pos: u32) -> ChangeId {
        self.graph_entry(local_pos).change_id()
    }

    fn segment_num_parents(&self, local_pos: u32) -> u32 {
        self.graph_entry(local_pos).num_parents()
    }

    fn segment_parent_positions(&self, local_pos: u32) -> Vec<IndexPosition> {
        let graph_entry = self.graph_entry(local_pos);
        let mut parent_entries = vec![];
        if graph_entry.num_parents() >= 1 {
            parent_entries.push(graph_entry.parent1_pos());
        }
        if graph_entry.num_parents() >= 2 {
            let mut parent_overflow_pos = graph_entry.parent2_overflow_pos();
            for _ in 1..graph_entry.num_parents() {
                parent_entries.push(self.overflow_parent(parent_overflow_pos));
                parent_overflow_pos += 1;
            }
        }
        parent_entries
    }

    fn segment_entry_by_pos(&self, pos: IndexPosition, local_pos: u32) -> IndexEntry {
        IndexEntry {
            source: self,
            local_pos,
            pos,
        }
    }
}

impl IndexSegment for MutableIndex {
    fn segment_num_parent_commits(&self) -> u32 {
        self.num_parent_commits
    }

    fn segment_num_commits(&self) -> u32 {
        self.graph.len() as u32
    }

    fn segment_parent_file(&self) -> Option<&Arc<ReadonlyIndex>> {
        self.parent_file.as_ref()
    }

    fn segment_name(&self) -> Option<String> {
        None
    }

    fn segment_commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        self.lookup.get(commit_id).cloned()
    }

    fn segment_commit_id_to_neighbor_positions(
        &self,
        commit_id: &CommitId,
    ) -> (Option<IndexPosition>, Option<IndexPosition>) {
        let prev_pos = self
            .lookup
            .range((Bound::Unbounded, Bound::Excluded(commit_id)))
            .next_back()
            .map(|(_, &pos)| pos);
        let next_pos = self
            .lookup
            .range((Bound::Excluded(commit_id), Bound::Unbounded))
            .next()
            .map(|(_, &pos)| pos);
        (prev_pos, next_pos)
    }

    fn segment_resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        let min_bytes_prefix = CommitId::from_bytes(prefix.min_prefix_bytes());
        let mut matches = self
            .lookup
            .range((Bound::Included(&min_bytes_prefix), Bound::Unbounded))
            .map(|(id, _pos)| id)
            .take_while(|&id| prefix.matches(id))
            .fuse();
        match (matches.next(), matches.next()) {
            (Some(id), None) => PrefixResolution::SingleMatch(id.clone()),
            (Some(_), Some(_)) => PrefixResolution::AmbiguousMatch,
            (None, _) => PrefixResolution::NoMatch,
        }
    }

    fn segment_generation_number(&self, local_pos: u32) -> u32 {
        self.graph[local_pos as usize].generation_number
    }

    fn segment_commit_id(&self, local_pos: u32) -> CommitId {
        self.graph[local_pos as usize].commit_id.clone()
    }

    fn segment_change_id(&self, local_pos: u32) -> ChangeId {
        self.graph[local_pos as usize].change_id.clone()
    }

    fn segment_num_parents(&self, local_pos: u32) -> u32 {
        self.graph[local_pos as usize].parent_positions.len() as u32
    }

    fn segment_parent_positions(&self, local_pos: u32) -> Vec<IndexPosition> {
        self.graph[local_pos as usize].parent_positions.clone()
    }

    fn segment_entry_by_pos(&self, pos: IndexPosition, local_pos: u32) -> IndexEntry {
        IndexEntry {
            source: self,
            local_pos,
            pos,
        }
    }
}

#[derive(Clone)]
pub struct IndexEntry<'a> {
    source: &'a dyn IndexSegment,
    pos: IndexPosition,
    // Position within the source segment
    local_pos: u32,
}

impl Debug for IndexEntry<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexEntry")
            .field("pos", &self.pos)
            .field("local_pos", &self.local_pos)
            .field("commit_id", &self.commit_id().hex())
            .finish()
    }
}

impl PartialEq for IndexEntry<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.pos == other.pos
    }
}
impl Eq for IndexEntry<'_> {}

impl Hash for IndexEntry<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pos.hash(state)
    }
}

impl<'a> IndexEntry<'a> {
    pub fn position(&self) -> IndexPosition {
        self.pos
    }

    pub fn generation_number(&self) -> u32 {
        self.source.segment_generation_number(self.local_pos)
    }

    pub fn commit_id(&self) -> CommitId {
        self.source.segment_commit_id(self.local_pos)
    }

    pub fn change_id(&self) -> ChangeId {
        self.source.segment_change_id(self.local_pos)
    }

    pub fn num_parents(&self) -> u32 {
        self.source.segment_num_parents(self.local_pos)
    }

    pub fn parent_positions(&self) -> Vec<IndexPosition> {
        self.source.segment_parent_positions(self.local_pos)
    }

    pub fn parents(&self) -> Vec<IndexEntry<'a>> {
        let composite = CompositeIndex(self.source);
        self.parent_positions()
            .into_iter()
            .map(|pos| composite.entry_by_pos(pos))
            .collect()
    }
}

impl ReadonlyIndex {
    pub(crate) fn load_from(
        file: &mut dyn Read,
        dir: PathBuf,
        name: String,
        commit_id_length: usize,
        change_id_length: usize,
    ) -> Result<Arc<ReadonlyIndex>, IndexLoadError> {
        let parent_filename_len = file.read_u32::<LittleEndian>()?;
        let num_parent_commits;
        let maybe_parent_file;
        if parent_filename_len > 0 {
            let mut parent_filename_bytes = vec![0; parent_filename_len as usize];
            file.read_exact(&mut parent_filename_bytes)?;
            let parent_filename = String::from_utf8(parent_filename_bytes).unwrap();
            let parent_file_path = dir.join(&parent_filename);
            let mut index_file = File::open(parent_file_path).unwrap();
            let parent_file = ReadonlyIndex::load_from(
                &mut index_file,
                dir,
                parent_filename,
                commit_id_length,
                change_id_length,
            )?;
            num_parent_commits = parent_file.num_parent_commits + parent_file.num_local_commits;
            maybe_parent_file = Some(parent_file);
        } else {
            num_parent_commits = 0;
            maybe_parent_file = None;
        };
        let num_commits = file.read_u32::<LittleEndian>()?;
        let num_parent_overflow_entries = file.read_u32::<LittleEndian>()?;
        let mut data = vec![];
        file.read_to_end(&mut data)?;
        let commit_graph_entry_size = CommitGraphEntry::size(commit_id_length, change_id_length);
        let graph_size = (num_commits as usize) * commit_graph_entry_size;
        let commit_lookup_entry_size = CommitLookupEntry::size(commit_id_length);
        let lookup_size = (num_commits as usize) * commit_lookup_entry_size;
        let parent_overflow_size = (num_parent_overflow_entries as usize) * 4;
        let expected_size = graph_size + lookup_size + parent_overflow_size;
        if data.len() != expected_size {
            return Err(IndexLoadError::IndexCorrupt(name));
        }
        let overflow_parent = data.split_off(graph_size + lookup_size);
        let lookup = data.split_off(graph_size);
        let graph = data;
        Ok(Arc::new(ReadonlyIndex {
            parent_file: maybe_parent_file,
            num_parent_commits,
            name,
            commit_id_length,
            change_id_length,
            commit_graph_entry_size,
            commit_lookup_entry_size,
            num_local_commits: num_commits,
            graph,
            lookup,
            overflow_parent,
        }))
    }

    pub fn as_index_ref(self: &ReadonlyIndex) -> IndexRef {
        IndexRef::Readonly(self)
    }

    pub fn num_commits(&self) -> u32 {
        CompositeIndex(self).num_commits()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn stats(&self) -> IndexStats {
        CompositeIndex(self).stats()
    }

    pub fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        CompositeIndex(self).commit_id_to_pos(commit_id)
    }

    pub fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        CompositeIndex(self).shortest_unique_commit_id_prefix_len(commit_id)
    }

    pub fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        CompositeIndex(self).resolve_prefix(prefix)
    }

    pub fn entry_by_id(&self, commit_id: &CommitId) -> Option<IndexEntry> {
        CompositeIndex(self).entry_by_id(commit_id)
    }

    pub fn entry_by_pos(&self, pos: IndexPosition) -> IndexEntry {
        CompositeIndex(self).entry_by_pos(pos)
    }

    pub fn has_id(&self, commit_id: &CommitId) -> bool {
        CompositeIndex(self).has_id(commit_id)
    }

    pub fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        CompositeIndex(self).is_ancestor(ancestor_id, descendant_id)
    }

    pub fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        CompositeIndex(self).common_ancestors(set1, set2)
    }

    pub fn walk_revs(&self, wanted: &[CommitId], unwanted: &[CommitId]) -> RevWalk {
        CompositeIndex(self).walk_revs(wanted, unwanted)
    }

    pub fn heads<'candidates>(
        &self,
        candidates: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<CommitId> {
        CompositeIndex(self).heads(candidates)
    }

    pub fn topo_order<'candidates>(
        &self,
        input: impl IntoIterator<Item = &'candidates CommitId>,
    ) -> Vec<IndexEntry> {
        CompositeIndex(self).topo_order(input)
    }

    fn graph_entry(&self, local_pos: u32) -> CommitGraphEntry {
        let offset = (local_pos as usize) * self.commit_graph_entry_size;
        CommitGraphEntry {
            data: &self.graph[offset..offset + self.commit_graph_entry_size],
            commit_id_length: self.commit_id_length,
            change_id_length: self.change_id_length,
        }
    }

    fn lookup_entry(&self, lookup_pos: u32) -> CommitLookupEntry {
        let offset = (lookup_pos as usize) * self.commit_lookup_entry_size;
        CommitLookupEntry {
            data: &self.lookup[offset..offset + self.commit_lookup_entry_size],
            commit_id_length: self.commit_id_length,
        }
    }

    fn overflow_parent(&self, overflow_pos: u32) -> IndexPosition {
        let offset = (overflow_pos as usize) * 4;
        IndexPosition(
            (&self.overflow_parent[offset..offset + 4])
                .read_u32::<LittleEndian>()
                .unwrap(),
        )
    }

    fn commit_id_byte_prefix_to_lookup_pos(&self, prefix: &CommitId) -> Option<u32> {
        if self.num_local_commits == 0 {
            // Avoid overflow when subtracting 1 below
            return None;
        }
        let mut low = 0;
        let mut high = self.num_local_commits - 1;

        // binary search for the commit id
        loop {
            let mid = (low + high) / 2;
            if high == low {
                return Some(mid);
            }
            let entry = self.lookup_entry(mid);
            if entry.commit_id_bytes() < prefix.as_bytes() {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    /// Generator of unique 16-byte ChangeId excluding root id
    fn change_id_generator() -> impl FnMut() -> ChangeId {
        let mut iter = (1_u128..).map(|n| ChangeId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    #[test_case(false; "memory")]
    #[test_case(true; "file")]
    fn index_empty(on_disk: bool) {
        let temp_dir = testutils::new_temp_dir();
        let index = MutableIndex::full(3, 16);
        let mut _saved_index = None;
        let index = if on_disk {
            _saved_index = Some(index.save_in(temp_dir.path().to_owned()).unwrap());
            IndexRef::Readonly(_saved_index.as_ref().unwrap())
        } else {
            IndexRef::Mutable(&index)
        };

        // Stats are as expected
        let stats = index.stats();
        assert_eq!(stats.num_commits, 0);
        assert_eq!(stats.num_heads, 0);
        assert_eq!(stats.max_generation_number, 0);
        assert_eq!(stats.num_merges, 0);
        assert_eq!(stats.num_changes, 0);
        assert_eq!(index.num_commits(), 0);
        // Cannot find any commits
        assert!(index.entry_by_id(&CommitId::from_hex("000000")).is_none());
        assert!(index.entry_by_id(&CommitId::from_hex("aaa111")).is_none());
        assert!(index.entry_by_id(&CommitId::from_hex("ffffff")).is_none());
        assert_eq!(index.iter().collect_vec(), vec![]);
    }

    #[test_case(false; "memory")]
    #[test_case(true; "file")]
    fn index_root_commit(on_disk: bool) {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        let id_0 = CommitId::from_hex("000000");
        let change_id0 = new_change_id();
        index.add_commit_data(id_0.clone(), change_id0.clone(), &[]);
        let mut _saved_index = None;
        let index = if on_disk {
            _saved_index = Some(index.save_in(temp_dir.path().to_owned()).unwrap());
            IndexRef::Readonly(_saved_index.as_ref().unwrap())
        } else {
            IndexRef::Mutable(&index)
        };

        // Stats are as expected
        let stats = index.stats();
        assert_eq!(stats.num_commits, 1);
        assert_eq!(stats.num_heads, 1);
        assert_eq!(stats.max_generation_number, 0);
        assert_eq!(stats.num_merges, 0);
        assert_eq!(stats.num_changes, 1);
        assert_eq!(index.num_commits(), 1);
        // Can find only the root commit
        assert_eq!(index.commit_id_to_pos(&id_0), Some(IndexPosition(0)));
        assert_eq!(index.commit_id_to_pos(&CommitId::from_hex("aaaaaa")), None);
        assert_eq!(index.commit_id_to_pos(&CommitId::from_hex("ffffff")), None);
        // Check properties of root entry
        let entry = index.entry_by_id(&id_0).unwrap();
        assert_eq!(entry.pos, IndexPosition(0));
        assert_eq!(entry.commit_id(), id_0);
        assert_eq!(entry.change_id(), change_id0);
        assert_eq!(entry.generation_number(), 0);
        assert_eq!(entry.num_parents(), 0);
        assert_eq!(entry.parent_positions(), Vec::<IndexPosition>::new());
        assert_eq!(entry.parents(), Vec::<IndexEntry>::new());
    }

    #[test]
    #[should_panic(expected = "parent commit is not indexed")]
    fn index_missing_parent_commit() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        index.add_commit_data(id_1, new_change_id(), &[id_0]);
    }

    #[test_case(false, false; "full in memory")]
    #[test_case(false, true; "full on disk")]
    #[test_case(true, false; "incremental in memory")]
    #[test_case(true, true; "incremental on disk")]
    fn index_multiple_commits(incremental: bool, on_disk: bool) {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 5
        // |\
        // 4 | 3
        // | |/
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let change_id0 = new_change_id();
        let id_1 = CommitId::from_hex("111111");
        let change_id1 = new_change_id();
        let id_2 = CommitId::from_hex("222222");
        let change_id2 = change_id1.clone();
        index.add_commit_data(id_0.clone(), change_id0, &[]);
        index.add_commit_data(id_1.clone(), change_id1.clone(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), change_id2.clone(), &[id_0.clone()]);

        // If testing incremental indexing, write the first three commits to one file
        // now and build the remainder as another segment on top.
        if incremental {
            let initial_file = index.save_in(temp_dir.path().to_owned()).unwrap();
            index = MutableIndex::incremental(initial_file);
        }

        let id_3 = CommitId::from_hex("333333");
        let change_id3 = new_change_id();
        let id_4 = CommitId::from_hex("444444");
        let change_id4 = new_change_id();
        let id_5 = CommitId::from_hex("555555");
        let change_id5 = change_id3.clone();
        index.add_commit_data(id_3.clone(), change_id3.clone(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), change_id4, &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), change_id5, &[id_4.clone(), id_2.clone()]);
        let mut _saved_index = None;
        let index = if on_disk {
            _saved_index = Some(index.save_in(temp_dir.path().to_owned()).unwrap());
            IndexRef::Readonly(_saved_index.as_ref().unwrap())
        } else {
            IndexRef::Mutable(&index)
        };

        // Stats are as expected
        let stats = index.stats();
        assert_eq!(stats.num_commits, 6);
        assert_eq!(stats.num_heads, 2);
        assert_eq!(stats.max_generation_number, 3);
        assert_eq!(stats.num_merges, 1);
        assert_eq!(stats.num_changes, 4);
        assert_eq!(index.num_commits(), 6);
        // Can find all the commits
        let entry_0 = index.entry_by_id(&id_0).unwrap();
        let entry_1 = index.entry_by_id(&id_1).unwrap();
        let entry_2 = index.entry_by_id(&id_2).unwrap();
        let entry_3 = index.entry_by_id(&id_3).unwrap();
        let entry_4 = index.entry_by_id(&id_4).unwrap();
        let entry_5 = index.entry_by_id(&id_5).unwrap();
        assert_eq!(
            index.iter().collect_vec(),
            vec![
                entry_0.clone(),
                entry_1.clone(),
                entry_2.clone(),
                entry_3.clone(),
                entry_4.clone(),
                entry_5.clone(),
            ]
        );
        // Check properties of some entries
        assert_eq!(entry_0.pos, IndexPosition(0));
        assert_eq!(entry_0.commit_id(), id_0);
        assert_eq!(entry_1.pos, IndexPosition(1));
        assert_eq!(entry_1.commit_id(), id_1);
        assert_eq!(entry_1.change_id(), change_id1);
        assert_eq!(entry_1.generation_number(), 1);
        assert_eq!(entry_1.num_parents(), 1);
        assert_eq!(entry_1.parent_positions(), vec![IndexPosition(0)]);
        assert_eq!(entry_1.parents().len(), 1);
        assert_eq!(entry_1.parents()[0].pos, IndexPosition(0));
        assert_eq!(entry_2.pos, IndexPosition(2));
        assert_eq!(entry_2.commit_id(), id_2);
        assert_eq!(entry_2.change_id(), change_id2);
        assert_eq!(entry_2.generation_number(), 1);
        assert_eq!(entry_2.num_parents(), 1);
        assert_eq!(entry_2.parent_positions(), vec![IndexPosition(0)]);
        assert_eq!(entry_3.change_id(), change_id3);
        assert_eq!(entry_3.generation_number(), 2);
        assert_eq!(entry_3.parent_positions(), vec![IndexPosition(2)]);
        assert_eq!(entry_4.pos, IndexPosition(4));
        assert_eq!(entry_4.generation_number(), 2);
        assert_eq!(entry_4.num_parents(), 1);
        assert_eq!(entry_4.parent_positions(), vec![IndexPosition(1)]);
        assert_eq!(entry_5.generation_number(), 3);
        assert_eq!(entry_5.num_parents(), 2);
        assert_eq!(
            entry_5.parent_positions(),
            vec![IndexPosition(4), IndexPosition(2)]
        );
        assert_eq!(entry_5.parents().len(), 2);
        assert_eq!(entry_5.parents()[0].pos, IndexPosition(4));
        assert_eq!(entry_5.parents()[1].pos, IndexPosition(2));
    }

    #[test_case(false; "in memory")]
    #[test_case(true; "on disk")]
    fn index_many_parents(on_disk: bool) {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        //     6
        //    /|\
        //   / | \
        //  / /|\ \
        // 1 2 3 4 5
        //  \ \|/ /
        //   \ | /
        //    \|/
        //     0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        let id_6 = CommitId::from_hex("666666");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_0]);
        index.add_commit_data(
            id_6.clone(),
            new_change_id(),
            &[id_1, id_2, id_3, id_4, id_5],
        );
        let mut _saved_index = None;
        let index = if on_disk {
            _saved_index = Some(index.save_in(temp_dir.path().to_owned()).unwrap());
            IndexRef::Readonly(_saved_index.as_ref().unwrap())
        } else {
            IndexRef::Mutable(&index)
        };

        // Stats are as expected
        let stats = index.stats();
        assert_eq!(stats.num_commits, 7);
        assert_eq!(index.iter().collect_vec().len(), 7);
        assert_eq!(stats.num_heads, 1);
        assert_eq!(stats.max_generation_number, 2);
        assert_eq!(stats.num_merges, 1);

        // The octopus merge has the right parents
        let entry_6 = index.entry_by_id(&id_6).unwrap();
        assert_eq!(entry_6.commit_id(), id_6.clone());
        assert_eq!(entry_6.num_parents(), 5);
        assert_eq!(
            entry_6.parent_positions(),
            vec![
                IndexPosition(1),
                IndexPosition(2),
                IndexPosition(3),
                IndexPosition(4),
                IndexPosition(5)
            ]
        );
        assert_eq!(entry_6.generation_number(), 2);
    }

    #[test]
    fn resolve_prefix() {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = index.save_in(temp_dir.path().to_owned()).unwrap();
        index = MutableIndex::incremental(initial_file);

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        index.add_commit_data(id_3, new_change_id(), &[]);
        index.add_commit_data(id_4, new_change_id(), &[]);
        index.add_commit_data(id_5, new_change_id(), &[]);

        // Can find commits given the full hex number
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new(&id_0.hex()).unwrap()),
            PrefixResolution::SingleMatch(id_0)
        );
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new(&id_1.hex()).unwrap()),
            PrefixResolution::SingleMatch(id_1)
        );
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new(&id_2.hex()).unwrap()),
            PrefixResolution::SingleMatch(id_2)
        );
        // Test nonexistent commits
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("ffffff").unwrap()),
            PrefixResolution::NoMatch
        );
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("000001").unwrap()),
            PrefixResolution::NoMatch
        );
        // Test ambiguous prefix
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("0").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
        // Test a globally unique prefix in initial part
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("009").unwrap()),
            PrefixResolution::SingleMatch(CommitId::from_hex("009999"))
        );
        // Test a globally unique prefix in incremental part
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("03").unwrap()),
            PrefixResolution::SingleMatch(CommitId::from_hex("033333"))
        );
        // Test a locally unique but globally ambiguous prefix
        assert_eq!(
            index.resolve_prefix(&HexPrefix::new("0554").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
    }

    #[test]
    #[allow(clippy::redundant_clone)] // allow id_n.clone()
    fn neighbor_commit_ids() {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000001");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = index.save_in(temp_dir.path().to_owned()).unwrap();
        index = MutableIndex::incremental(initial_file.clone());

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        index.add_commit_data(id_3.clone(), new_change_id(), &[]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[]);

        // Local lookup in readonly index, commit_id exists.
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&id_0),
            (None, Some(IndexPosition(1))),
        );
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&id_1),
            (Some(IndexPosition(0)), Some(IndexPosition(2))),
        );
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&id_2),
            (Some(IndexPosition(1)), None),
        );

        // Local lookup in readonly index, commit_id does not exist.
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("000000")),
            (None, Some(IndexPosition(0))),
        );
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("000002")),
            (Some(IndexPosition(0)), Some(IndexPosition(1))),
        );
        assert_eq!(
            initial_file.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("ffffff")),
            (Some(IndexPosition(2)), None),
        );

        // Local lookup in mutable index, commit_id exists. id_5 < id_3 < id_4
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&id_5),
            (None, Some(IndexPosition(3))),
        );
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&id_3),
            (Some(IndexPosition(5)), Some(IndexPosition(4))),
        );
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&id_4),
            (Some(IndexPosition(3)), None),
        );

        // Local lookup in mutable index, commit_id does not exist. id_5 < id_3 < id_4
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("033332")),
            (None, Some(IndexPosition(5))),
        );
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("033334")),
            (Some(IndexPosition(5)), Some(IndexPosition(3))),
        );
        assert_eq!(
            index.segment_commit_id_to_neighbor_positions(&CommitId::from_hex("ffffff")),
            (Some(IndexPosition(4)), None),
        );

        // Global lookup, commit_id exists. id_0 < id_1 < id_5 < id_3 < id_2 < id_4
        let composite_index = CompositeIndex(&index);
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_0),
            (None, Some(id_1.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_1),
            (Some(id_0.clone()), Some(id_5.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_5),
            (Some(id_1.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_3),
            (Some(id_5.clone()), Some(id_2.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_2),
            (Some(id_3.clone()), Some(id_4.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&id_4),
            (Some(id_2.clone()), None),
        );

        // Global lookup, commit_id doesn't exist. id_0 < id_1 < id_5 < id_3 < id_2 <
        // id_4
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&CommitId::from_hex("000000")),
            (None, Some(id_0.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&CommitId::from_hex("010000")),
            (Some(id_1.clone()), Some(id_5.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&CommitId::from_hex("033334")),
            (Some(id_5.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            composite_index.resolve_neighbor_commit_ids(&CommitId::from_hex("ffffff")),
            (Some(id_4.clone()), None),
        );
    }

    #[test]
    fn shortest_unique_commit_id_prefix() {
        let temp_dir = testutils::new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000001");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = index.save_in(temp_dir.path().to_owned()).unwrap();
        index = MutableIndex::incremental(initial_file);

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        index.add_commit_data(id_3.clone(), new_change_id(), &[]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[]);

        // Public API: calculate shortest unique prefix len with known commit_id
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_0), 3);
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_1), 3);
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_2), 5);
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_3), 5);
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_4), 4);
        assert_eq!(index.shortest_unique_commit_id_prefix_len(&id_5), 2);

        // Public API: calculate shortest unique prefix len with unknown commit_id
        assert_eq!(
            index.shortest_unique_commit_id_prefix_len(&CommitId::from_hex("000002")),
            6
        );
        assert_eq!(
            index.shortest_unique_commit_id_prefix_len(&CommitId::from_hex("010000")),
            2
        );
        assert_eq!(
            index.shortest_unique_commit_id_prefix_len(&CommitId::from_hex("033334")),
            6
        );
        assert_eq!(
            index.shortest_unique_commit_id_prefix_len(&CommitId::from_hex("ffffff")),
            1
        );
    }

    #[test]
    fn test_is_ancestor() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 5
        // |\
        // 4 | 3
        // | |/
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);

        assert!(index.is_ancestor(&id_0, &id_0));
        assert!(index.is_ancestor(&id_0, &id_1));
        assert!(index.is_ancestor(&id_2, &id_3));
        assert!(index.is_ancestor(&id_2, &id_5));
        assert!(index.is_ancestor(&id_1, &id_5));
        assert!(index.is_ancestor(&id_0, &id_5));
        assert!(!index.is_ancestor(&id_1, &id_0));
        assert!(!index.is_ancestor(&id_5, &id_3));
        assert!(!index.is_ancestor(&id_3, &id_5));
        assert!(!index.is_ancestor(&id_2, &id_4));
        assert!(!index.is_ancestor(&id_4, &id_2));
    }

    #[test]
    fn test_common_ancestors() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 5
        // |\
        // 4 |
        // | |
        // 1 2 3
        // | |/
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);

        assert_eq!(
            index.common_ancestors(&[id_0.clone()], &[id_0.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_5.clone()], &[id_5.clone()]),
            vec![id_5.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_1.clone()], &[id_2.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_2.clone()], &[id_1.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_1.clone()], &[id_4.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_4.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_3.clone()], &[id_5.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_5.clone()], &[id_3.clone()]),
            vec![id_0.clone()]
        );

        // With multiple commits in an input set
        assert_eq!(
            index.common_ancestors(&[id_0.clone(), id_1.clone()], &[id_0.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_0.clone(), id_1.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_1.clone(), id_2.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_1.clone(), id_2.clone()], &[id_4]),
            vec![id_1.clone()]
        );
        assert_eq!(
            index.common_ancestors(&[id_1.clone(), id_2.clone()], &[id_5]),
            vec![id_1.clone(), id_2.clone()]
        );
        assert_eq!(index.common_ancestors(&[id_1, id_2], &[id_3]), vec![id_0]);
    }

    #[test]
    fn test_common_ancestors_criss_cross() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 3 4
        // |X|
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_1.clone(), id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone(), id_2.clone()]);

        let mut common_ancestors = index.common_ancestors(&[id_3], &[id_4]);
        common_ancestors.sort();
        assert_eq!(common_ancestors, vec![id_1, id_2]);
    }

    #[test]
    fn test_common_ancestors_merge_with_ancestor() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 4   5
        // |\ /|
        // 1 2 3
        //  \|/
        //   0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1, new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3, new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_0.clone(), id_2.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_0, id_2.clone()]);

        let mut common_ancestors = index.common_ancestors(&[id_4], &[id_5]);
        common_ancestors.sort();
        assert_eq!(common_ancestors, vec![id_2]);
    }

    #[test]
    fn test_walk_revs() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 5
        // |\
        // 4 | 3
        // | |/
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);

        let walk_commit_ids = |wanted: &[CommitId], unwanted: &[CommitId]| {
            index
                .walk_revs(wanted, unwanted)
                .map(|entry| entry.commit_id())
                .collect_vec()
        };

        // No wanted commits
        assert!(walk_commit_ids(&[], &[]).is_empty());
        // Simple linear walk to roo
        assert_eq!(
            walk_commit_ids(&[id_4.clone()], &[]),
            vec![id_4.clone(), id_1.clone(), id_0.clone()]
        );
        // Commits that are both wanted and unwanted are not walked
        assert_eq!(walk_commit_ids(&[id_0.clone()], &[id_0.clone()]), vec![]);
        // Commits that are listed twice are only walked once
        assert_eq!(
            walk_commit_ids(&[id_0.clone(), id_0.clone()], &[]),
            vec![id_0.clone()]
        );
        // If a commit and its ancestor are both wanted, the ancestor still gets walked
        // only once
        assert_eq!(
            walk_commit_ids(&[id_0.clone(), id_1.clone()], &[]),
            vec![id_1.clone(), id_0.clone()]
        );
        // Ancestors of both wanted and unwanted commits are not walked
        assert_eq!(
            walk_commit_ids(&[id_2.clone()], &[id_1.clone()]),
            vec![id_2.clone()]
        );
        // Same as above, but the opposite order, to make sure that order in index
        // doesn't matter
        assert_eq!(
            walk_commit_ids(&[id_1.clone()], &[id_2.clone()]),
            vec![id_1.clone()]
        );
        // Two wanted nodes
        assert_eq!(
            walk_commit_ids(&[id_1.clone(), id_2.clone()], &[]),
            vec![id_2.clone(), id_1.clone(), id_0.clone()]
        );
        // Order of output doesn't depend on order of input
        assert_eq!(
            walk_commit_ids(&[id_2.clone(), id_1.clone()], &[]),
            vec![id_2.clone(), id_1.clone(), id_0]
        );
        // Two wanted nodes that share an unwanted ancestor
        assert_eq!(
            walk_commit_ids(&[id_5.clone(), id_3.clone()], &[id_2]),
            vec![id_5, id_4, id_3, id_1]
        );
    }

    #[test]
    fn test_walk_revs_filter_by_generation() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 8 6
        // | |
        // 7 5
        // |/|
        // 4 |
        // | 3
        // 2 |
        // |/
        // 1
        // |
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        let id_6 = CommitId::from_hex("666666");
        let id_7 = CommitId::from_hex("777777");
        let id_8 = CommitId::from_hex("888888");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_3.clone()]);
        index.add_commit_data(id_6.clone(), new_change_id(), &[id_5.clone()]);
        index.add_commit_data(id_7.clone(), new_change_id(), &[id_4.clone()]);
        index.add_commit_data(id_8.clone(), new_change_id(), &[id_7.clone()]);

        let walk_commit_ids = |wanted: &[CommitId], unwanted: &[CommitId], range: Range<u32>| {
            index
                .walk_revs(wanted, unwanted)
                .filter_by_generation(range)
                .map(|entry| entry.commit_id())
                .collect_vec()
        };

        // Simple generation bounds
        assert_eq!(walk_commit_ids(&[&id_8].map(Clone::clone), &[], 0..0), []);
        assert_eq!(
            walk_commit_ids(&[&id_2].map(Clone::clone), &[], 0..3),
            [&id_2, &id_1, &id_0].map(Clone::clone)
        );

        // Ancestors may be walked with different generations
        assert_eq!(
            walk_commit_ids(&[&id_6].map(Clone::clone), &[], 2..4),
            [&id_4, &id_3, &id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_5].map(Clone::clone), &[], 2..3),
            [&id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_5, &id_7].map(Clone::clone), &[], 2..3),
            [&id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_7, &id_8].map(Clone::clone), &[], 0..2),
            [&id_8, &id_7, &id_4].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_6, &id_7].map(Clone::clone), &[], 0..3),
            [&id_7, &id_6, &id_5, &id_4, &id_3, &id_2].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_6, &id_7].map(Clone::clone), &[], 2..3),
            [&id_4, &id_3, &id_2].map(Clone::clone)
        );

        // Ancestors of both wanted and unwanted commits are not walked
        assert_eq!(
            walk_commit_ids(&[&id_5].map(Clone::clone), &[&id_2].map(Clone::clone), 1..5),
            [&id_4, &id_3].map(Clone::clone)
        );
    }

    #[test]
    fn test_heads() {
        let mut new_change_id = change_id_generator();
        let mut index = MutableIndex::full(3, 16);
        // 5
        // |\
        // 4 | 3
        // | |/
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);

        // Empty input
        assert!(index.heads(&[]).is_empty());
        // Single head
        assert_eq!(index.heads(&[id_4.clone()]), vec![id_4.clone()]);
        // Single head and parent
        assert_eq!(index.heads(&[id_4.clone(), id_1]), vec![id_4.clone()]);
        // Single head and grand-parent
        assert_eq!(index.heads(&[id_4.clone(), id_0]), vec![id_4.clone()]);
        // Multiple heads
        assert_eq!(
            index.heads(&[id_4.clone(), id_3.clone()]),
            vec![id_3.clone(), id_4]
        );
        // Merge commit and ancestors
        assert_eq!(index.heads(&[id_5.clone(), id_2]), vec![id_5.clone()]);
        // Merge commit and other commit
        assert_eq!(index.heads(&[id_5.clone(), id_3.clone()]), vec![id_3, id_5]);
    }

    #[test]
    fn test_hex_prefix_prefixes() {
        let prefix = HexPrefix::new("").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"");

        let prefix = HexPrefix::new("1").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x10");

        let prefix = HexPrefix::new("12").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x12");

        let prefix = HexPrefix::new("123").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x12\x30");
    }

    #[test]
    fn test_hex_prefix_matches() {
        let id = CommitId::from_hex("1234");

        assert!(HexPrefix::new("").unwrap().matches(&id));
        assert!(HexPrefix::new("1").unwrap().matches(&id));
        assert!(HexPrefix::new("12").unwrap().matches(&id));
        assert!(HexPrefix::new("123").unwrap().matches(&id));
        assert!(HexPrefix::new("1234").unwrap().matches(&id));
        assert!(!HexPrefix::new("12345").unwrap().matches(&id));

        assert!(!HexPrefix::new("a").unwrap().matches(&id));
        assert!(!HexPrefix::new("1a").unwrap().matches(&id));
        assert!(!HexPrefix::new("12a").unwrap().matches(&id));
        assert!(!HexPrefix::new("123a").unwrap().matches(&id));
    }
}
