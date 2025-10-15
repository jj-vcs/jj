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

use std::cmp::max;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Debug;
use std::io::Write as _;
use std::iter;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use blake2::Blake2b512;
use digest::Digest as _;
use itertools::Itertools as _;
use pollster::FutureExt as _;
use smallvec::SmallVec;
use smallvec::smallvec;
use tempfile::NamedTempFile;

use super::changed_path::CompositeChangedPathIndex;
use super::changed_path::collect_changed_paths;
use super::composite::AsCompositeIndex;
use super::composite::ChangeIdIndexImpl;
use super::composite::CommitIndexSegment;
use super::composite::CommitIndexSegmentId;
use super::composite::CompositeCommitIndex;
use super::composite::CompositeIndex;
use super::composite::DynCommitIndexSegment;
use super::entry::GlobalCommitPosition;
use super::entry::LocalCommitPosition;
use super::entry::SmallGlobalCommitPositionsVec;
use super::entry::SmallLocalCommitPositionsVec;
use super::readonly::COMMIT_INDEX_SEGMENT_FILE_FORMAT_VERSION;
use super::readonly::DefaultReadonlyIndex;
use super::readonly::FieldLengths;
use super::readonly::OVERFLOW_FLAG;
use super::readonly::ReadonlyCommitIndexSegment;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::persist_content_addressed_temp_file;
use crate::index::ChangeIdIndex;
use crate::index::Index;
use crate::index::IndexError;
use crate::index::IndexResult;
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

#[derive(Clone, Debug)]
struct MutableGraphEntry {
    commit_id: CommitId,
    change_id: ChangeId,
    generation_number: u32,
    parent_positions: SmallGlobalCommitPositionsVec,
}

#[derive(Clone)]
pub(super) struct MutableCommitIndexSegment {
    parent_file: Option<Arc<ReadonlyCommitIndexSegment>>,
    num_parent_commits: u32,
    field_lengths: FieldLengths,
    graph: Vec<MutableGraphEntry>,
    commit_lookup: BTreeMap<CommitId, LocalCommitPosition>,
    change_lookup: BTreeMap<ChangeId, SmallLocalCommitPositionsVec>,
}

impl Debug for MutableCommitIndexSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("MutableCommitIndexSegment")
            .field("parent_file", &self.parent_file)
            .finish_non_exhaustive()
    }
}

impl MutableCommitIndexSegment {
    pub(super) fn full(field_lengths: FieldLengths) -> Self {
        Self {
            parent_file: None,
            num_parent_commits: 0,
            field_lengths,
            graph: vec![],
            commit_lookup: BTreeMap::new(),
            change_lookup: BTreeMap::new(),
        }
    }

    pub(super) fn incremental(parent_file: Arc<ReadonlyCommitIndexSegment>) -> Self {
        let num_parent_commits = parent_file.as_composite().num_commits();
        let field_lengths = parent_file.field_lengths();
        Self {
            parent_file: Some(parent_file),
            num_parent_commits,
            field_lengths,
            graph: vec![],
            commit_lookup: BTreeMap::new(),
            change_lookup: BTreeMap::new(),
        }
    }

    pub(super) fn as_composite(&self) -> &CompositeCommitIndex {
        CompositeCommitIndex::new(self)
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
        let local_pos = LocalCommitPosition(u32::try_from(self.graph.len()).unwrap());
        self.commit_lookup
            .insert(entry.commit_id.clone(), local_pos);
        self.change_lookup
            .entry(entry.change_id.clone())
            // positions are inherently sorted
            .and_modify(|positions| positions.push(local_pos))
            .or_insert(smallvec![local_pos]);
        self.graph.push(entry);
    }

    pub(super) fn add_commits_from(&mut self, other_segment: &DynCommitIndexSegment) {
        let other = CompositeCommitIndex::new(other_segment);
        for pos in other_segment.num_parent_commits()..other.num_commits() {
            let entry = other.entry_by_pos(GlobalCommitPosition(pos));
            let parent_ids = entry.parents().map(|entry| entry.commit_id()).collect_vec();
            self.add_commit_data(entry.commit_id(), entry.change_id(), &parent_ids);
        }
    }

    pub(super) fn merge_in(&mut self, other: &Arc<ReadonlyCommitIndexSegment>) {
        // Collect other segments down to the common ancestor segment
        let files_to_add = itertools::merge_join_by(
            self.as_composite().ancestor_files_without_local(),
            iter::once(other).chain(other.as_composite().ancestor_files_without_local()),
            |own, other| {
                let own_num_commits = own.as_composite().num_commits();
                let other_num_commits = other.as_composite().num_commits();
                own_num_commits.cmp(&other_num_commits).reverse()
            },
        )
        .take_while(|own_other| {
            own_other
                .as_ref()
                .both()
                .is_none_or(|(own, other)| own.id() != other.id())
        })
        .filter_map(|own_other| own_other.right())
        .collect_vec();

        for &file in files_to_add.iter().rev() {
            self.add_commits_from(file.as_ref());
        }
    }

    fn serialize_parent_filename(&self, buf: &mut Vec<u8>) {
        if let Some(parent_file) = &self.parent_file {
            let hex = parent_file.id().hex();
            buf.extend(u32::try_from(hex.len()).unwrap().to_le_bytes());
            buf.extend_from_slice(hex.as_bytes());
        } else {
            buf.extend(0_u32.to_le_bytes());
        }
    }

    fn serialize_local_entries(&self, buf: &mut Vec<u8>) {
        assert_eq!(self.graph.len(), self.commit_lookup.len());
        debug_assert_eq!(
            self.graph.len(),
            self.change_lookup.values().flatten().count()
        );

        let num_commits = u32::try_from(self.graph.len()).unwrap();
        buf.extend(num_commits.to_le_bytes());
        let num_change_ids = u32::try_from(self.change_lookup.len()).unwrap();
        buf.extend(num_change_ids.to_le_bytes());
        // We'll write the actual values later
        let parent_overflow_offset = buf.len();
        buf.extend(0_u32.to_le_bytes());
        let change_overflow_offset = buf.len();
        buf.extend(0_u32.to_le_bytes());

        // Positions of change ids in the sorted table
        let change_id_pos_map: HashMap<&ChangeId, u32> = self
            .change_lookup
            .keys()
            .enumerate()
            .map(|(i, change_id)| (change_id, u32::try_from(i).unwrap()))
            .collect();

        let mut parent_overflow = vec![];
        for entry in &self.graph {
            buf.extend(entry.generation_number.to_le_bytes());

            match entry.parent_positions.as_slice() {
                [] => {
                    buf.extend((!0_u32).to_le_bytes());
                    buf.extend((!0_u32).to_le_bytes());
                }
                [GlobalCommitPosition(pos1)] => {
                    assert!(*pos1 < OVERFLOW_FLAG);
                    buf.extend(pos1.to_le_bytes());
                    buf.extend((!0_u32).to_le_bytes());
                }
                [GlobalCommitPosition(pos1), GlobalCommitPosition(pos2)] => {
                    assert!(*pos1 < OVERFLOW_FLAG);
                    assert!(*pos2 < OVERFLOW_FLAG);
                    buf.extend(pos1.to_le_bytes());
                    buf.extend(pos2.to_le_bytes());
                }
                positions => {
                    let overflow_pos = u32::try_from(parent_overflow.len()).unwrap();
                    let num_parents = u32::try_from(positions.len()).unwrap();
                    assert!(overflow_pos < OVERFLOW_FLAG);
                    assert!(num_parents < OVERFLOW_FLAG);
                    buf.extend((!overflow_pos).to_le_bytes());
                    buf.extend((!num_parents).to_le_bytes());
                    parent_overflow.extend_from_slice(positions);
                }
            }

            buf.extend(change_id_pos_map[&entry.change_id].to_le_bytes());

            assert_eq!(
                entry.commit_id.as_bytes().len(),
                self.field_lengths.commit_id
            );
            buf.extend_from_slice(entry.commit_id.as_bytes());
        }

        for LocalCommitPosition(pos) in self.commit_lookup.values() {
            buf.extend(pos.to_le_bytes());
        }

        for change_id in self.change_lookup.keys() {
            assert_eq!(change_id.as_bytes().len(), self.field_lengths.change_id);
            buf.extend_from_slice(change_id.as_bytes());
        }

        let mut change_overflow = vec![];
        for positions in self.change_lookup.values() {
            match positions.as_slice() {
                [] => panic!("change id lookup entry must not be empty"),
                // Optimize for imported commits
                [LocalCommitPosition(pos1)] => {
                    assert!(*pos1 < OVERFLOW_FLAG);
                    buf.extend(pos1.to_le_bytes());
                }
                positions => {
                    let overflow_pos = u32::try_from(change_overflow.len()).unwrap();
                    assert!(overflow_pos < OVERFLOW_FLAG);
                    buf.extend((!overflow_pos).to_le_bytes());
                    change_overflow.extend_from_slice(positions);
                }
            }
        }

        let num_parent_overflow = u32::try_from(parent_overflow.len()).unwrap();
        buf[parent_overflow_offset..][..4].copy_from_slice(&num_parent_overflow.to_le_bytes());
        for GlobalCommitPosition(pos) in parent_overflow {
            buf.extend(pos.to_le_bytes());
        }

        let num_change_overflow = u32::try_from(change_overflow.len()).unwrap();
        buf[change_overflow_offset..][..4].copy_from_slice(&num_change_overflow.to_le_bytes());
        for LocalCommitPosition(pos) in change_overflow {
            buf.extend(pos.to_le_bytes());
        }
    }

    /// If the mutable segment has more than half the commits of its parent
    /// segment, return mutable segment with the commits from both. This is done
    /// recursively, so the stack of index segments has O(log n) files.
    pub(super) fn maybe_squash_with_ancestors(self) -> Self {
        let mut num_new_commits = self.num_local_commits();
        let mut files_to_squash = vec![];
        let mut base_parent_file = None;
        for parent_file in self.as_composite().ancestor_files_without_local() {
            // TODO: We should probably also squash if the parent file has less than N
            // commits, regardless of how many (few) are in `self`.
            if 2 * num_new_commits < parent_file.num_local_commits() {
                base_parent_file = Some(parent_file.clone());
                break;
            }
            num_new_commits += parent_file.num_local_commits();
            files_to_squash.push(parent_file.clone());
        }

        if files_to_squash.is_empty() {
            return self;
        }

        let mut squashed = if let Some(parent_file) = base_parent_file {
            Self::incremental(parent_file)
        } else {
            Self::full(self.field_lengths)
        };
        for parent_file in files_to_squash.iter().rev() {
            squashed.add_commits_from(parent_file.as_ref());
        }
        squashed.add_commits_from(&self);
        squashed
    }

    pub(super) fn save_in(self, dir: &Path) -> Result<Arc<ReadonlyCommitIndexSegment>, PathError> {
        if self.num_local_commits() == 0 && self.parent_file.is_some() {
            return Ok(self.parent_file.unwrap());
        }

        let mut buf = Vec::new();
        buf.extend(COMMIT_INDEX_SEGMENT_FILE_FORMAT_VERSION.to_le_bytes());
        self.serialize_parent_filename(&mut buf);
        let local_entries_offset = buf.len();
        self.serialize_local_entries(&mut buf);
        let mut hasher = Blake2b512::new();
        hasher.update(&buf);
        let index_file_id = CommitIndexSegmentId::from_bytes(&hasher.finalize());
        let index_file_path = dir.join(index_file_id.hex());

        let mut temp_file = NamedTempFile::new_in(dir).context(dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(&buf).context(temp_file.path())?;
        persist_content_addressed_temp_file(temp_file, &index_file_path)
            .context(&index_file_path)?;

        Ok(ReadonlyCommitIndexSegment::load_with_parent_file(
            &mut &buf[local_entries_offset..],
            index_file_id,
            self.parent_file,
            self.field_lengths,
        )
        .expect("in-memory index data should be valid and readable"))
    }
}

impl CommitIndexSegment for MutableCommitIndexSegment {
    fn num_parent_commits(&self) -> u32 {
        self.num_parent_commits
    }

    fn num_local_commits(&self) -> u32 {
        self.graph.len().try_into().unwrap()
    }

    fn parent_file(&self) -> Option<&Arc<ReadonlyCommitIndexSegment>> {
        self.parent_file.as_ref()
    }

    fn commit_id_to_pos(&self, commit_id: &CommitId) -> Option<LocalCommitPosition> {
        self.commit_lookup.get(commit_id).copied()
    }

    fn resolve_neighbor_commit_ids(
        &self,
        commit_id: &CommitId,
    ) -> (Option<CommitId>, Option<CommitId>) {
        let (prev_id, next_id) = resolve_neighbor_ids(&self.commit_lookup, commit_id);
        (prev_id.cloned(), next_id.cloned())
    }

    fn resolve_commit_id_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId> {
        let min_bytes_prefix = CommitId::from_bytes(prefix.min_prefix_bytes());
        resolve_id_prefix(&self.commit_lookup, prefix, &min_bytes_prefix).map(|(id, _)| id.clone())
    }

    fn resolve_neighbor_change_ids(
        &self,
        change_id: &ChangeId,
    ) -> (Option<ChangeId>, Option<ChangeId>) {
        let (prev_id, next_id) = resolve_neighbor_ids(&self.change_lookup, change_id);
        (prev_id.cloned(), next_id.cloned())
    }

    fn resolve_change_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> PrefixResolution<(ChangeId, SmallLocalCommitPositionsVec)> {
        let min_bytes_prefix = ChangeId::from_bytes(prefix.min_prefix_bytes());
        resolve_id_prefix(&self.change_lookup, prefix, &min_bytes_prefix)
            .map(|(id, positions)| (id.clone(), positions.clone()))
    }

    fn generation_number(&self, local_pos: LocalCommitPosition) -> u32 {
        self.graph[local_pos.0 as usize].generation_number
    }

    fn commit_id(&self, local_pos: LocalCommitPosition) -> CommitId {
        self.graph[local_pos.0 as usize].commit_id.clone()
    }

    fn change_id(&self, local_pos: LocalCommitPosition) -> ChangeId {
        self.graph[local_pos.0 as usize].change_id.clone()
    }

    fn num_parents(&self, local_pos: LocalCommitPosition) -> u32 {
        self.graph[local_pos.0 as usize]
            .parent_positions
            .len()
            .try_into()
            .unwrap()
    }

    fn parent_positions(&self, local_pos: LocalCommitPosition) -> SmallGlobalCommitPositionsVec {
        self.graph[local_pos.0 as usize].parent_positions.clone()
    }
}

/// In-memory mutable records for the on-disk commit index backend.
pub struct DefaultMutableIndex(CompositeIndex);

impl DefaultMutableIndex {
    pub(super) fn full(lengths: FieldLengths) -> Self {
        let commits = Box::new(MutableCommitIndexSegment::full(lengths));
        // Changed-path index isn't enabled by default.
        let mut changed_paths = CompositeChangedPathIndex::null();
        changed_paths.make_mutable();
        Self(CompositeIndex::from_mutable(commits, changed_paths))
    }

    pub(super) fn incremental(parent_index: &DefaultReadonlyIndex) -> Self {
        let commits = Box::new(MutableCommitIndexSegment::incremental(
            parent_index.readonly_commits().clone(),
        ));
        let mut changed_paths = parent_index.changed_paths().clone();
        changed_paths.make_mutable();
        Self(CompositeIndex::from_mutable(commits, changed_paths))
    }

    pub(super) fn into_segment(
        self,
    ) -> (Box<MutableCommitIndexSegment>, CompositeChangedPathIndex) {
        self.0.into_mutable().expect("must have mutable")
    }

    fn mutable_commits(&mut self) -> &mut MutableCommitIndexSegment {
        self.0.mutable_commits().expect("must have mutable")
    }

    /// Returns the number of all indexed commits.
    pub fn num_commits(&self) -> u32 {
        self.0.commits().num_commits()
    }

    #[tracing::instrument(skip(self))]
    pub(super) async fn add_commit(&mut self, commit: &Commit) -> BackendResult<()> {
        let new_commit_pos = GlobalCommitPosition(self.num_commits());
        self.add_commit_data(
            commit.id().clone(),
            commit.change_id().clone(),
            commit.parent_ids(),
        );
        if new_commit_pos == GlobalCommitPosition(self.num_commits()) {
            return Ok(()); // commit already indexed
        }
        if self.0.changed_paths().next_mutable_commit_pos() == Some(new_commit_pos) {
            self.add_commit_changed_paths(commit).await?;
        }
        Ok(())
    }

    pub(super) fn add_commit_data(
        &mut self,
        commit_id: CommitId,
        change_id: ChangeId,
        parent_ids: &[CommitId],
    ) {
        self.mutable_commits()
            .add_commit_data(commit_id, change_id, parent_ids);
    }

    // CompositeChangedPathIndex::add_commit() isn't implemented because we need
    // a commit index to merge parent trees, which means we need to borrow self.
    async fn add_commit_changed_paths(&mut self, commit: &Commit) -> BackendResult<()> {
        let paths = collect_changed_paths(self, commit).await?;
        self.0.changed_paths_mut().add_changed_paths(paths);
        Ok(())
    }

    pub(super) fn merge_in(&mut self, other: &DefaultReadonlyIndex) {
        let start_commit_pos = GlobalCommitPosition(self.num_commits());
        self.mutable_commits().merge_in(other.readonly_commits());
        if self.0.changed_paths().next_mutable_commit_pos() == Some(start_commit_pos) {
            let other_commits = other.as_composite().commits();
            for self_pos in (start_commit_pos.0..self.num_commits()).map(GlobalCommitPosition) {
                let entry = self.0.commits().entry_by_pos(self_pos);
                let other_pos = other_commits.commit_id_to_pos(&entry.commit_id()).unwrap();
                let Some(paths) = other.changed_paths().changed_paths(other_pos) else {
                    break; // no more indexed paths in other index
                };
                let paths = paths.map(|path| path.to_owned()).collect();
                self.0.changed_paths_mut().add_changed_paths(paths);
            }
        }
    }
}

impl AsCompositeIndex for DefaultMutableIndex {
    fn as_composite(&self) -> &CompositeIndex {
        &self.0
    }
}

impl Index for DefaultMutableIndex {
    fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> IndexResult<usize> {
        self.0.shortest_unique_commit_id_prefix_len(commit_id)
    }

    fn resolve_commit_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> IndexResult<PrefixResolution<CommitId>> {
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

    fn all_heads_for_gc(&self) -> IndexResult<Box<dyn Iterator<Item = CommitId> + '_>> {
        self.0.all_heads_for_gc()
    }

    fn heads(&self, candidates: &mut dyn Iterator<Item = &CommitId>) -> IndexResult<Vec<CommitId>> {
        self.0.heads(candidates)
    }

    fn changed_paths_in_commit(
        &self,
        commit_id: &CommitId,
    ) -> IndexResult<Option<Box<dyn Iterator<Item = RepoPathBuf> + '_>>> {
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

impl MutableIndex for DefaultMutableIndex {
    fn as_index(&self) -> &dyn Index {
        self
    }

    fn change_id_index(
        &self,
        heads: &mut dyn Iterator<Item = &CommitId>,
    ) -> Box<dyn ChangeIdIndex + '_> {
        Box::new(ChangeIdIndexImpl::new(self, heads))
    }

    fn add_commit(&mut self, commit: &Commit) -> IndexResult<()> {
        Self::add_commit(self, commit)
            .block_on()
            .map_err(|err| IndexError::Other(err.into()))
    }

    fn merge_in(&mut self, other: &dyn ReadonlyIndex) -> IndexResult<()> {
        let other: &DefaultReadonlyIndex = other
            .downcast_ref()
            .expect("index to merge in must be a DefaultReadonlyIndex");
        Self::merge_in(self, other);
        Ok(())
    }
}

fn resolve_neighbor_ids<'a, K: Ord, V>(
    lookup_table: &'a BTreeMap<K, V>,
    id: &K,
) -> (Option<&'a K>, Option<&'a K>) {
    let prev_id = lookup_table
        .range((Bound::Unbounded, Bound::Excluded(id)))
        .next_back()
        .map(|(id, _)| id);
    let next_id = lookup_table
        .range((Bound::Excluded(id), Bound::Unbounded))
        .next()
        .map(|(id, _)| id);
    (prev_id, next_id)
}

fn resolve_id_prefix<'a, K: ObjectId + Ord, V>(
    lookup_table: &'a BTreeMap<K, V>,
    prefix: &HexPrefix,
    min_bytes_prefix: &K,
) -> PrefixResolution<(&'a K, &'a V)> {
    let mut matches = lookup_table
        .range((Bound::Included(min_bytes_prefix), Bound::Unbounded))
        .take_while(|&(id, _)| prefix.matches(id))
        .fuse();
    match (matches.next(), matches.next()) {
        (Some(entry), None) => PrefixResolution::SingleMatch(entry),
        (Some(_), Some(_)) => PrefixResolution::AmbiguousMatch,
        (None, _) => PrefixResolution::NoMatch,
    }
}
