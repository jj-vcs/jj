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

#![allow(missing_docs)]

use std::any::Any;
use std::cmp::max;
use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use blake2::Blake2b512;
use digest::Digest;
use itertools::Itertools;
use smallvec::SmallVec;
use tempfile::NamedTempFile;

use super::composite::{AsCompositeIndex, CompositeIndex, IndexSegment};
use super::entry::{IndexPosition, SmallIndexPositionsVec};
use super::readonly::{DefaultReadonlyIndex, ReadonlyIndexSegment};
use crate::backend::{ChangeId, CommitId, ObjectId};
use crate::commit::Commit;
use crate::file_util::persist_content_addressed_temp_file;
use crate::index::{HexPrefix, Index, MutableIndex, PrefixResolution, ReadonlyIndex};
use crate::revset::{ResolvedExpression, Revset, RevsetEvaluationError};
use crate::store::Store;

#[derive(Debug)]
struct MutableGraphEntry {
    commit_id: CommitId,
    change_id: ChangeId,
    generation_number: u32,
    parent_positions: SmallIndexPositionsVec,
}

pub(super) struct MutableIndexSegment {
    parent_file: Option<Arc<ReadonlyIndexSegment>>,
    num_parent_commits: u32,
    commit_id_length: usize,
    change_id_length: usize,
    graph: Vec<MutableGraphEntry>,
    lookup: BTreeMap<CommitId, IndexPosition>,
}

impl MutableIndexSegment {
    pub(super) fn full(commit_id_length: usize, change_id_length: usize) -> Self {
        Self {
            parent_file: None,
            num_parent_commits: 0,
            commit_id_length,
            change_id_length,
            graph: vec![],
            lookup: BTreeMap::new(),
        }
    }

    pub(super) fn incremental(parent_file: Arc<ReadonlyIndexSegment>) -> Self {
        let num_parent_commits = parent_file.as_composite().num_commits();
        let commit_id_length = parent_file.commit_id_length();
        let change_id_length = parent_file.change_id_length();
        Self {
            parent_file: Some(parent_file),
            num_parent_commits,
            commit_id_length,
            change_id_length,
            graph: vec![],
            lookup: BTreeMap::new(),
        }
    }

    pub(super) fn as_composite(&self) -> CompositeIndex {
        CompositeIndex::new(self)
    }

    pub(super) fn add_commit(&mut self, commit: &Commit) {
        self.add_commit_data(
            commit.id().clone(),
            commit.change_id().clone(),
            commit.parent_ids(),
        );
    }

    pub(super) fn add_commit_data(
        &mut self,
        commit_id: CommitId,
        change_id: ChangeId,
        parent_ids: &[CommitId],
    ) {
        if self.as_composite().has_id(&commit_id) {
            return;
        }
        let mut entry = MutableGraphEntry {
            commit_id,
            change_id,
            generation_number: 0,
            parent_positions: SmallVec::new(),
        };
        for parent_id in parent_ids {
            let parent_entry = self
                .as_composite()
                .entry_by_id(parent_id)
                .expect("parent commit is not indexed");
            entry.generation_number = max(
                entry.generation_number,
                parent_entry.generation_number() + 1,
            );
            entry.parent_positions.push(parent_entry.position());
        }
        self.lookup.insert(
            entry.commit_id.clone(),
            IndexPosition(u32::try_from(self.graph.len()).unwrap() + self.num_parent_commits),
        );
        self.graph.push(entry);
    }

    pub(super) fn add_commits_from(&mut self, other_segment: &dyn IndexSegment) {
        let other = CompositeIndex::new(other_segment);
        for pos in other_segment.segment_num_parent_commits()..other.num_commits() {
            let entry = other.entry_by_pos(IndexPosition(pos));
            let parent_ids = entry.parents().map(|entry| entry.commit_id()).collect_vec();
            self.add_commit_data(entry.commit_id(), entry.change_id(), &parent_ids);
        }
    }

    pub(super) fn merge_in(&mut self, other: Arc<ReadonlyIndexSegment>) {
        let mut maybe_own_ancestor = self.parent_file.clone();
        let mut maybe_other_ancestor = Some(other);
        let mut files_to_add = vec![];
        loop {
            if maybe_other_ancestor.is_none() {
                break;
            }
            let other_ancestor = maybe_other_ancestor.as_ref().unwrap();
            if maybe_own_ancestor.is_none() {
                files_to_add.push(other_ancestor.clone());
                maybe_other_ancestor = other_ancestor.segment_parent_file().cloned();
                continue;
            }
            let own_ancestor = maybe_own_ancestor.as_ref().unwrap();
            if own_ancestor.name() == other_ancestor.name() {
                break;
            }
            if own_ancestor.as_composite().num_commits()
                < other_ancestor.as_composite().num_commits()
            {
                files_to_add.push(other_ancestor.clone());
                maybe_other_ancestor = other_ancestor.segment_parent_file().cloned();
            } else {
                maybe_own_ancestor = own_ancestor.segment_parent_file().cloned();
            }
        }

        for file in files_to_add.iter().rev() {
            self.add_commits_from(file.as_ref());
        }
    }

    fn serialize_parent_filename(&self, buf: &mut Vec<u8>) {
        if let Some(parent_file) = &self.parent_file {
            buf.extend(
                u32::try_from(parent_file.name().len())
                    .unwrap()
                    .to_le_bytes(),
            );
            buf.extend_from_slice(parent_file.name().as_bytes());
        } else {
            buf.extend(0_u32.to_le_bytes());
        }
    }

    fn serialize_local_entries(&self, buf: &mut Vec<u8>) {
        assert_eq!(self.graph.len(), self.lookup.len());

        let num_commits = u32::try_from(self.graph.len()).unwrap();
        buf.extend(num_commits.to_le_bytes());
        // We'll write the actual value later
        let parent_overflow_offset = buf.len();
        buf.extend(0_u32.to_le_bytes());

        let mut parent_overflow = vec![];
        for entry in &self.graph {
            let flags = 0_u32;
            buf.extend(flags.to_le_bytes());

            buf.extend(entry.generation_number.to_le_bytes());

            buf.extend(
                u32::try_from(entry.parent_positions.len())
                    .unwrap()
                    .to_le_bytes(),
            );
            let mut parent1_pos = IndexPosition(0);
            let parent_overflow_pos = u32::try_from(parent_overflow.len()).unwrap();
            for (i, parent_pos) in entry.parent_positions.iter().enumerate() {
                if i == 0 {
                    parent1_pos = *parent_pos;
                } else {
                    parent_overflow.push(*parent_pos);
                }
            }
            buf.extend(parent1_pos.0.to_le_bytes());
            buf.extend(parent_overflow_pos.to_le_bytes());

            assert_eq!(entry.change_id.as_bytes().len(), self.change_id_length);
            buf.extend_from_slice(entry.change_id.as_bytes());

            assert_eq!(entry.commit_id.as_bytes().len(), self.commit_id_length);
            buf.extend_from_slice(entry.commit_id.as_bytes());
        }

        for (commit_id, pos) in &self.lookup {
            buf.extend_from_slice(commit_id.as_bytes());
            buf.extend(pos.0.to_le_bytes());
        }

        buf[parent_overflow_offset..][..4]
            .copy_from_slice(&u32::try_from(parent_overflow.len()).unwrap().to_le_bytes());
        for parent_pos in parent_overflow {
            buf.extend(parent_pos.0.to_le_bytes());
        }
    }

    /// If the MutableIndex has more than half the commits of its parent
    /// ReadonlyIndex, return MutableIndex with the commits from both. This
    /// is done recursively, so the stack of index files has O(log n) files.
    fn maybe_squash_with_ancestors(self) -> MutableIndexSegment {
        let mut num_new_commits = self.segment_num_commits();
        let mut files_to_squash = vec![];
        let mut base_parent_file = None;
        for parent_file in self.as_composite().ancestor_files_without_local() {
            // TODO: We should probably also squash if the parent file has less than N
            // commits, regardless of how many (few) are in `self`.
            if 2 * num_new_commits < parent_file.segment_num_commits() {
                base_parent_file = Some(parent_file.clone());
                break;
            }
            num_new_commits += parent_file.segment_num_commits();
            files_to_squash.push(parent_file.clone());
        }

        if files_to_squash.is_empty() {
            return self;
        }

        let mut squashed = if let Some(parent_file) = base_parent_file {
            MutableIndexSegment::incremental(parent_file)
        } else {
            MutableIndexSegment::full(self.commit_id_length, self.change_id_length)
        };
        for parent_file in files_to_squash.iter().rev() {
            squashed.add_commits_from(parent_file.as_ref());
        }
        squashed.add_commits_from(&self);
        squashed
    }

    pub(super) fn save_in(self, dir: &Path) -> io::Result<Arc<ReadonlyIndexSegment>> {
        if self.segment_num_commits() == 0 && self.parent_file.is_some() {
            return Ok(self.parent_file.unwrap());
        }

        let mut buf = Vec::new();
        self.serialize_parent_filename(&mut buf);
        let local_entries_offset = buf.len();
        self.serialize_local_entries(&mut buf);
        let mut hasher = Blake2b512::new();
        hasher.update(&buf);
        let index_file_id_hex = hex::encode(hasher.finalize());
        let index_file_path = dir.join(&index_file_id_hex);

        let mut temp_file = NamedTempFile::new_in(dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(&buf)?;
        persist_content_addressed_temp_file(temp_file, index_file_path)?;

        Ok(ReadonlyIndexSegment::load_with_parent_file(
            &mut &buf[local_entries_offset..],
            index_file_id_hex,
            self.parent_file,
            self.commit_id_length,
            self.change_id_length,
        )
        .expect("in-memory index data should be valid and readable"))
    }
}

impl IndexSegment for MutableIndexSegment {
    fn segment_num_parent_commits(&self) -> u32 {
        self.num_parent_commits
    }

    fn segment_num_commits(&self) -> u32 {
        self.graph.len().try_into().unwrap()
    }

    fn segment_parent_file(&self) -> Option<&Arc<ReadonlyIndexSegment>> {
        self.parent_file.as_ref()
    }

    fn segment_name(&self) -> Option<String> {
        None
    }

    fn segment_commit_id_to_pos(&self, commit_id: &CommitId) -> Option<IndexPosition> {
        self.lookup.get(commit_id).cloned()
    }

    fn segment_resolve_neighbor_commit_ids(
        &self,
        commit_id: &CommitId,
    ) -> (Option<CommitId>, Option<CommitId>) {
        let prev_id = self
            .lookup
            .range((Bound::Unbounded, Bound::Excluded(commit_id)))
            .next_back()
            .map(|(id, _)| id.clone());
        let next_id = self
            .lookup
            .range((Bound::Excluded(commit_id), Bound::Unbounded))
            .next()
            .map(|(id, _)| id.clone());
        (prev_id, next_id)
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
        self.graph[local_pos as usize]
            .parent_positions
            .len()
            .try_into()
            .unwrap()
    }

    fn segment_parent_positions(&self, local_pos: u32) -> SmallIndexPositionsVec {
        self.graph[local_pos as usize].parent_positions.clone()
    }
}

/// In-memory mutable records for the on-disk commit index backend.
pub struct DefaultMutableIndex(MutableIndexSegment);

impl DefaultMutableIndex {
    pub(crate) fn full(commit_id_length: usize, change_id_length: usize) -> Self {
        let mutable_segment = MutableIndexSegment::full(commit_id_length, change_id_length);
        DefaultMutableIndex(mutable_segment)
    }

    pub(super) fn incremental(parent_file: Arc<ReadonlyIndexSegment>) -> Self {
        let mutable_segment = MutableIndexSegment::incremental(parent_file);
        DefaultMutableIndex(mutable_segment)
    }

    #[cfg(test)]
    pub(crate) fn add_commit_data(
        &mut self,
        commit_id: CommitId,
        change_id: ChangeId,
        parent_ids: &[CommitId],
    ) {
        self.0.add_commit_data(commit_id, change_id, parent_ids);
    }

    pub(super) fn squash_and_save_in(self, dir: &Path) -> io::Result<Arc<ReadonlyIndexSegment>> {
        self.0.maybe_squash_with_ancestors().save_in(dir)
    }
}

impl AsCompositeIndex for DefaultMutableIndex {
    fn as_composite(&self) -> CompositeIndex<'_> {
        self.0.as_composite()
    }
}

impl Index for DefaultMutableIndex {
    fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize {
        self.as_composite()
            .shortest_unique_commit_id_prefix_len(commit_id)
    }

    fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        self.as_composite().resolve_prefix(prefix)
    }

    fn has_id(&self, commit_id: &CommitId) -> bool {
        self.as_composite().has_id(commit_id)
    }

    fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool {
        self.as_composite().is_ancestor(ancestor_id, descendant_id)
    }

    fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId> {
        self.as_composite().common_ancestors(set1, set2)
    }

    fn heads(&self, candidates: &mut dyn Iterator<Item = &CommitId>) -> Vec<CommitId> {
        self.as_composite().heads(candidates)
    }

    fn topo_order(&self, input: &mut dyn Iterator<Item = &CommitId>) -> Vec<CommitId> {
        self.as_composite().topo_order(input)
    }

    fn evaluate_revset<'index>(
        &'index self,
        expression: &ResolvedExpression,
        store: &Arc<Store>,
    ) -> Result<Box<dyn Revset<'index> + 'index>, RevsetEvaluationError> {
        self.as_composite().evaluate_revset(expression, store)
    }
}

impl MutableIndex for DefaultMutableIndex {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        Box::new(*self)
    }

    fn as_index(&self) -> &dyn Index {
        self
    }

    fn add_commit(&mut self, commit: &Commit) {
        self.0.add_commit(commit);
    }

    fn merge_in(&mut self, other: &dyn ReadonlyIndex) {
        let other = other
            .as_any()
            .downcast_ref::<DefaultReadonlyIndex>()
            .expect("index to merge in must be a DefaultReadonlyIndex");
        self.0.merge_in(other.as_segment().clone());
    }
}
