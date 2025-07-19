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

use std::fmt::Debug;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;

use smallvec::SmallVec;

use super::composite::CompositeCommitIndex;
use super::composite::DynCommitIndexSegment;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::object_id::ObjectId as _;

/// Global commit index position.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub(super) struct GlobalCommitPosition(pub(super) u32);

impl GlobalCommitPosition {
    pub const MIN: Self = GlobalCommitPosition(u32::MIN);
    pub const MAX: Self = GlobalCommitPosition(u32::MAX);
}

/// Local commit position within an index segment.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub(super) struct LocalCommitPosition(pub(super) u32);

// SmallVec reuses two pointer-size fields as inline area, which meas we can
// inline up to 16 bytes (on 64-bit platform) for free.
pub(super) type SmallGlobalCommitPositionsVec = SmallVec<[GlobalCommitPosition; 4]>;
pub(super) type SmallLocalCommitPositionsVec = SmallVec<[LocalCommitPosition; 4]>;

#[derive(Clone)]
pub(super) struct CommitIndexEntry<'a> {
    source: &'a DynCommitIndexSegment,
    pos: GlobalCommitPosition,
    /// Position within the source segment
    local_pos: LocalCommitPosition,
}

impl Debug for CommitIndexEntry<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommitIndexEntry")
            .field("pos", &self.pos)
            .field("local_pos", &self.local_pos)
            .field("commit_id", &self.commit_id().hex())
            .finish()
    }
}

impl PartialEq for CommitIndexEntry<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.pos == other.pos
    }
}

impl Eq for CommitIndexEntry<'_> {}

impl Hash for CommitIndexEntry<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pos.hash(state);
    }
}

impl<'a> CommitIndexEntry<'a> {
    pub(super) fn new(
        source: &'a DynCommitIndexSegment,
        pos: GlobalCommitPosition,
        local_pos: LocalCommitPosition,
    ) -> Self {
        CommitIndexEntry {
            source,
            pos,
            local_pos,
        }
    }

    pub fn position(&self) -> GlobalCommitPosition {
        self.pos
    }

    pub fn generation_number(&self) -> u32 {
        self.source.generation_number(self.local_pos)
    }

    pub fn commit_id(&self) -> CommitId {
        self.source.commit_id(self.local_pos)
    }

    pub fn change_id(&self) -> ChangeId {
        self.source.change_id(self.local_pos)
    }

    pub fn num_parents(&self) -> u32 {
        self.source.num_parents(self.local_pos)
    }

    pub fn parent_positions(&self) -> SmallGlobalCommitPositionsVec {
        self.source.parent_positions(self.local_pos)
    }

    pub fn parents(&self) -> impl ExactSizeIterator<Item = CommitIndexEntry<'a>> + use<'a> {
        let composite = CompositeCommitIndex::new(self.source);
        self.parent_positions()
            .into_iter()
            .map(move |pos| composite.entry_by_pos(pos))
    }
}
