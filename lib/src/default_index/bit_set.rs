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

use std::cmp::min;

use super::composite::CompositeCommitIndex;
use super::entry::GlobalCommitPosition;

/// Unit of buffer allocation, which is 4kB.
const PAGE_SIZE_IN_BITS: u32 = 4096 * 8;
const PAGE_SIZE_IN_WORDS: u32 = PAGE_SIZE_IN_BITS / u64::BITS;

/// Bit set of [`GlobalCommitPosition`]s.
#[derive(Clone, Debug)]
pub(super) struct PositionsBitSet {
    data: Vec<u64>,
    bitset_len: u32,
}

impl PositionsBitSet {
    /// Creates bit set of the specified capacity.
    pub fn with_capacity(len: u32) -> Self {
        Self {
            data: vec![],
            bitset_len: u32::div_ceil(len, u64::BITS),
        }
    }

    /// Creates bit set with the maximum position.
    pub fn with_max_pos(max_pos: GlobalCommitPosition) -> Self {
        assert_ne!(max_pos, GlobalCommitPosition::MAX);
        Self::with_capacity(max_pos.0 + 1)
    }

    fn ensure_data(&mut self, bitset_pos: u32) {
        if usize::try_from(bitset_pos).unwrap() < self.data.len() {
            return;
        }

        // Do not pre-allocate large buffer which might not be used. We could
        // manage buffer chunks as Vec<Vec<u64>>, but the extra indirection
        // appeared to make get() slow.
        let new_len = usize::try_from(min(
            u32::div_ceil(bitset_pos + 1, PAGE_SIZE_IN_WORDS) * PAGE_SIZE_IN_WORDS,
            self.bitset_len,
        ))
        .unwrap();
        if self.data.is_empty() {
            self.data = vec![0; new_len]; // request zeroed page
        } else {
            self.data.resize(new_len, 0); // realloc + copy
        }
    }

    fn to_global_pos(&self, (bitset_pos, bit_pos): (u32, u32)) -> GlobalCommitPosition {
        let bitset_rev_pos = self.bitset_len - bitset_pos - 1;
        GlobalCommitPosition(bitset_rev_pos * u64::BITS + bit_pos)
    }

    fn to_bitset_pos(&self, pos: GlobalCommitPosition) -> (u32, u32) {
        let bitset_rev_pos = pos.0 / u64::BITS;
        let bit_pos = pos.0 % u64::BITS;
        let bitset_pos = self.bitset_len - bitset_rev_pos - 1;
        (bitset_pos, bit_pos)
    }

    /// Returns `true` if the given `pos` is set.
    ///
    /// Panics if the `pos` exceeds the capacity.
    pub fn get(&self, pos: GlobalCommitPosition) -> bool {
        self.get_bit(self.to_bitset_pos(pos))
    }

    fn get_bit(&self, (bitset_pos, bit_pos): (u32, u32)) -> bool {
        let bit = 1_u64 << bit_pos;
        match self.data.get(usize::try_from(bitset_pos).unwrap()) {
            Some(word) => *word & bit != 0,
            None => false,
        }
    }

    /// Sets `pos` to true.
    ///
    /// Panics if the `pos` exceeds the capacity.
    pub fn set(&mut self, pos: GlobalCommitPosition) {
        self.set_bit(self.to_bitset_pos(pos));
    }

    fn set_bit(&mut self, (bitset_pos, bit_pos): (u32, u32)) {
        self.ensure_data(bitset_pos);
        let bit = 1_u64 << bit_pos;
        self.data[usize::try_from(bitset_pos).unwrap()] |= bit;
    }

    /// Sets `pos` to true. Returns `true` if the old value was set.
    ///
    /// Panics if the `pos` exceeds the capacity.
    pub fn get_set(&mut self, pos: GlobalCommitPosition) -> bool {
        self.get_set_bit(self.to_bitset_pos(pos))
    }

    fn get_set_bit(&mut self, (bitset_pos, bit_pos): (u32, u32)) -> bool {
        self.ensure_data(bitset_pos);
        let bit = 1_u64 << bit_pos;
        let word = &mut self.data[usize::try_from(bitset_pos).unwrap()];
        let old = *word & bit != 0;
        *word |= bit;
        old
    }
}

/// Computes ancestors set lazily.
///
/// This is similar to `RevWalk` functionality-wise, but implemented with the
/// different design goals:
///
/// * optimized for dense ancestors set
/// * optimized for testing set membership
/// * no iterator API (which could be implemented on top)
#[derive(Clone, Debug)]
pub(super) struct AncestorsBitSet {
    bitset: PositionsBitSet,
    next_bitset_pos_to_visit: u32,
}

impl AncestorsBitSet {
    /// Creates bit set of the specified capacity.
    pub fn with_capacity(len: u32) -> Self {
        let bitset = PositionsBitSet::with_capacity(len);
        let next_bitset_pos_to_visit = bitset.bitset_len;
        Self {
            bitset,
            next_bitset_pos_to_visit,
        }
    }

    /// Adds head `pos` to the set.
    ///
    /// Panics if the `pos` exceeds the capacity.
    pub fn add_head(&mut self, pos: GlobalCommitPosition) {
        let (bitset_pos, bit_pos) = self.bitset.to_bitset_pos(pos);
        self.bitset.set_bit((bitset_pos, bit_pos));
        self.next_bitset_pos_to_visit = min(self.next_bitset_pos_to_visit, bitset_pos);
    }

    /// Returns `true` if the given `pos` is ancestors of the heads.
    ///
    /// Panics if the `pos` exceeds the capacity or has not been visited yet.
    pub fn contains(&self, pos: GlobalCommitPosition) -> bool {
        let (bitset_pos, bit_pos) = self.bitset.to_bitset_pos(pos);
        assert!(bitset_pos < self.next_bitset_pos_to_visit);
        self.bitset.get_bit((bitset_pos, bit_pos))
    }

    /// Updates set by visiting ancestors until the given `to_visit_pos`.
    pub fn visit_until(
        &mut self,
        index: &CompositeCommitIndex,
        to_visit_pos: GlobalCommitPosition,
    ) {
        let (last_bitset_pos_to_visit, _) = self.bitset.to_bitset_pos(to_visit_pos);
        if last_bitset_pos_to_visit < self.next_bitset_pos_to_visit {
            return;
        }
        self.bitset.ensure_data(last_bitset_pos_to_visit);
        for visiting_bitset_pos in self.next_bitset_pos_to_visit..=last_bitset_pos_to_visit {
            let mut unvisited_bits =
                self.bitset.data[usize::try_from(visiting_bitset_pos).unwrap()];
            while unvisited_bits != 0 {
                let bit_pos = u64::BITS - unvisited_bits.leading_zeros() - 1; // from MSB
                unvisited_bits ^= 1_u64 << bit_pos;
                let current_pos = self.bitset.to_global_pos((visiting_bitset_pos, bit_pos));
                for parent_pos in index.entry_by_pos(current_pos).parent_positions() {
                    assert!(parent_pos < current_pos);
                    let (parent_bitset_pos, parent_bit_pos) = self.bitset.to_bitset_pos(parent_pos);
                    self.bitset.ensure_data(parent_bitset_pos);
                    let bit = 1_u64 << parent_bit_pos;
                    self.bitset.data[usize::try_from(parent_bitset_pos).unwrap()] |= bit;
                    if visiting_bitset_pos == parent_bitset_pos {
                        unvisited_bits |= bit;
                    }
                }
            }
        }
        self.next_bitset_pos_to_visit = last_bitset_pos_to_visit + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::composite::AsCompositeIndex as _;
    use super::super::mutable::DefaultMutableIndex;
    use super::super::readonly::FieldLengths;
    use super::*;
    use crate::backend::ChangeId;
    use crate::backend::CommitId;

    /// Generator of unique 16-byte CommitId excluding root id
    fn commit_id_generator() -> impl FnMut() -> CommitId {
        let mut iter = (1_u128..).map(|n| CommitId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    /// Generator of unique 16-byte ChangeId excluding root id
    fn change_id_generator() -> impl FnMut() -> ChangeId {
        let mut iter = (1_u128..).map(|n| ChangeId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    #[test]
    fn test_positions_bit_set() {
        // Create with empty capacity, which is useless, but shouldn't panic
        let _set = PositionsBitSet::with_capacity(0);

        let mut set = PositionsBitSet::with_capacity(128);
        assert!(!set.get(GlobalCommitPosition(0)));
        assert!(!set.get(GlobalCommitPosition(127)));
        set.set(GlobalCommitPosition(0));
        assert!(set.get(GlobalCommitPosition(0)));
        assert!(!set.get(GlobalCommitPosition(1)));
        assert!(!set.get(GlobalCommitPosition(127)));
        let old = set.get_set(GlobalCommitPosition(127));
        assert!(!old);
        assert!(!set.get(GlobalCommitPosition(63)));
        assert!(!set.get(GlobalCommitPosition(64)));
        assert!(set.get(GlobalCommitPosition(127)));
        let old = set.get_set(GlobalCommitPosition(127));
        assert!(old);
    }

    #[test]
    fn test_positions_bit_set_allocation() {
        // Exactly one page
        let mut set = PositionsBitSet::with_capacity(PAGE_SIZE_IN_BITS);
        assert!(set.data.is_empty());
        assert!(!set.get(GlobalCommitPosition(0)));
        assert!(!set.get(GlobalCommitPosition(PAGE_SIZE_IN_BITS - 1)));
        assert!(set.data.is_empty());
        set.set(GlobalCommitPosition(0));
        assert_eq!(set.data.len(), usize::try_from(PAGE_SIZE_IN_WORDS).unwrap());
        set.set(GlobalCommitPosition(PAGE_SIZE_IN_BITS - 1));
        assert_eq!(set.data.len(), usize::try_from(PAGE_SIZE_IN_WORDS).unwrap());
        assert!(set.get(GlobalCommitPosition(0)));
        assert!(set.get(GlobalCommitPosition(PAGE_SIZE_IN_BITS - 1)));

        // Two pages
        let mut set = PositionsBitSet::with_capacity(PAGE_SIZE_IN_BITS + 1);
        assert!(set.data.is_empty());
        set.set(GlobalCommitPosition(u64::BITS));
        set.set(GlobalCommitPosition(PAGE_SIZE_IN_BITS));
        assert_eq!(set.data.len(), usize::try_from(PAGE_SIZE_IN_WORDS).unwrap());
        let old = set.get_set(GlobalCommitPosition(u64::BITS - 1));
        assert!(!old);
        assert_eq!(
            set.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS + 1).unwrap()
        );
        assert!(set.get(GlobalCommitPosition(u64::BITS - 1)));
        assert!(set.get(GlobalCommitPosition(u64::BITS)));
        assert!(set.get(GlobalCommitPosition(PAGE_SIZE_IN_BITS)));

        // Exactly three pages
        let mut set = PositionsBitSet::with_capacity(PAGE_SIZE_IN_BITS * 3);
        assert!(set.data.is_empty());
        set.set(GlobalCommitPosition(PAGE_SIZE_IN_BITS * 2));
        assert_eq!(set.data.len(), usize::try_from(PAGE_SIZE_IN_WORDS).unwrap());
        set.set(GlobalCommitPosition(PAGE_SIZE_IN_BITS));
        assert_eq!(
            set.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS * 2).unwrap()
        );
        set.set(GlobalCommitPosition(0));
        assert_eq!(
            set.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS * 3).unwrap()
        );
        assert!(set.get(GlobalCommitPosition(0)));
        assert!(set.get(GlobalCommitPosition(PAGE_SIZE_IN_BITS)));
        assert!(set.get(GlobalCommitPosition(PAGE_SIZE_IN_BITS * 2)));
    }

    #[test]
    fn test_ancestors_bit_set() {
        let mut new_commit_id = commit_id_generator();
        let mut new_change_id = change_id_generator();
        let mut mutable_index = DefaultMutableIndex::full(FieldLengths {
            commit_id: 16,
            change_id: 16,
        });

        // F      F = 256
        // |\     E = 193,194,195,..,254
        // E | D  D = 192,255
        // | |/   C = 66,68,70,..,190
        // B C    B = 65,67,69,..,189,191
        // |/     A = 0,1,2,..,64
        // A
        let id_a0 = new_commit_id();
        mutable_index.add_commit_data(id_a0.clone(), new_change_id(), &[]);
        let id_a64 = (1..=64).fold(id_a0.clone(), |parent_id, i| {
            assert_eq!(mutable_index.num_commits(), i);
            let id = new_commit_id();
            mutable_index.add_commit_data(id.clone(), new_change_id(), &[parent_id]);
            id
        });
        let (id_b189, id_c190) = (65..=190).step_by(2).fold(
            (id_a64.clone(), id_a64.clone()),
            |(parent_id_b, parent_id_c), i| {
                assert_eq!(mutable_index.num_commits(), i);
                let id_b = new_commit_id();
                let id_c = new_commit_id();
                mutable_index.add_commit_data(id_b.clone(), new_change_id(), &[parent_id_b]);
                mutable_index.add_commit_data(id_c.clone(), new_change_id(), &[parent_id_c]);
                (id_b, id_c)
            },
        );
        let id_b191 = new_commit_id();
        mutable_index.add_commit_data(id_b191.clone(), new_change_id(), &[id_b189]);
        let id_d192 = new_commit_id();
        mutable_index.add_commit_data(
            id_d192.clone(),
            new_change_id(),
            std::slice::from_ref(&id_c190),
        );
        let id_e254 = (193..=254).fold(id_b191.clone(), |parent_id, i| {
            assert_eq!(mutable_index.num_commits(), i);
            let id = new_commit_id();
            mutable_index.add_commit_data(id.clone(), new_change_id(), &[parent_id]);
            id
        });
        let id_d255 = new_commit_id();
        mutable_index.add_commit_data(
            id_d255.clone(),
            new_change_id(),
            std::slice::from_ref(&id_d192),
        );
        let id_f256 = new_commit_id();
        mutable_index.add_commit_data(
            id_f256.clone(),
            new_change_id(),
            &[id_c190.clone(), id_e254.clone()],
        );
        assert_eq!(mutable_index.num_commits(), 257);

        let index = mutable_index.as_composite().commits();
        let to_pos = |id: &CommitId| index.commit_id_to_pos(id).unwrap();
        let new_ancestors_set = |heads: &[&CommitId]| {
            let mut set = AncestorsBitSet::with_capacity(index.num_commits());
            for &id in heads {
                set.add_head(to_pos(id));
            }
            set
        };

        // Nothing reachable
        let set = new_ancestors_set(&[]);
        assert_eq!(set.next_bitset_pos_to_visit, 5);
        for pos in (0..=256).map(GlobalCommitPosition) {
            assert!(!set.contains(pos), "{pos:?} should be unreachable");
        }

        // All reachable
        let mut set = new_ancestors_set(&[&id_f256, &id_d255]);
        assert_eq!(set.next_bitset_pos_to_visit, 0);
        set.visit_until(index, to_pos(&id_f256));
        assert_eq!(set.next_bitset_pos_to_visit, 1);
        assert!(set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_d192));
        assert_eq!(set.next_bitset_pos_to_visit, 2);
        assert!(set.contains(to_pos(&id_e254)));
        assert!(set.contains(to_pos(&id_d255)));
        assert!(set.contains(to_pos(&id_d192)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.next_bitset_pos_to_visit, 5);
        set.visit_until(index, to_pos(&id_f256)); // should be noop
        assert_eq!(set.next_bitset_pos_to_visit, 5);
        for pos in (0..=256).map(GlobalCommitPosition) {
            assert!(set.contains(pos), "{pos:?} should be reachable");
        }

        // A, B, C, E, F are reachable
        let mut set = new_ancestors_set(&[&id_f256]);
        assert_eq!(set.next_bitset_pos_to_visit, 0);
        set.visit_until(index, to_pos(&id_f256));
        assert_eq!(set.next_bitset_pos_to_visit, 1);
        assert!(set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_d192));
        assert_eq!(set.next_bitset_pos_to_visit, 2);
        assert!(!set.contains(to_pos(&id_d255)));
        assert!(!set.contains(to_pos(&id_d192)));
        set.visit_until(index, to_pos(&id_c190));
        assert_eq!(set.next_bitset_pos_to_visit, 3);
        assert!(set.contains(to_pos(&id_c190)));
        set.visit_until(index, to_pos(&id_a64));
        assert_eq!(set.next_bitset_pos_to_visit, 4);
        assert!(set.contains(to_pos(&id_b191)));
        assert!(set.contains(to_pos(&id_a64)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.next_bitset_pos_to_visit, 5);
        assert!(set.contains(to_pos(&id_a0)));

        // A, C, D are reachable
        let mut set = new_ancestors_set(&[&id_d255]);
        assert_eq!(set.next_bitset_pos_to_visit, 1);
        assert!(!set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_e254));
        assert_eq!(set.next_bitset_pos_to_visit, 2);
        assert!(!set.contains(to_pos(&id_e254)));
        set.visit_until(index, to_pos(&id_d255));
        assert_eq!(set.next_bitset_pos_to_visit, 2);
        assert!(set.contains(to_pos(&id_d255)));
        set.visit_until(index, to_pos(&id_b191));
        assert_eq!(set.next_bitset_pos_to_visit, 3);
        assert!(!set.contains(to_pos(&id_b191)));
        set.visit_until(index, to_pos(&id_c190));
        assert_eq!(set.next_bitset_pos_to_visit, 3);
        assert!(set.contains(to_pos(&id_c190)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.next_bitset_pos_to_visit, 5);
        assert!(set.contains(to_pos(&id_a64)));
        assert!(set.contains(to_pos(&id_a0)));
    }

    #[test]
    fn test_ancestors_bit_set_allocation() {
        let mut new_commit_id = commit_id_generator();
        let mut new_change_id = change_id_generator();
        let mut mutable_index = DefaultMutableIndex::full(FieldLengths {
            commit_id: 16,
            change_id: 16,
        });
        // Linear history of two-page size
        let id_0 = new_commit_id();
        mutable_index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        (1..=PAGE_SIZE_IN_BITS).fold(id_0.clone(), |parent_id, i| {
            assert_eq!(mutable_index.num_commits(), i);
            let id = new_commit_id();
            mutable_index.add_commit_data(id.clone(), new_change_id(), &[parent_id]);
            id
        });
        let index = mutable_index.as_composite().commits();

        let mut set = AncestorsBitSet::with_capacity(index.num_commits());
        assert_eq!(set.next_bitset_pos_to_visit, PAGE_SIZE_IN_WORDS + 1);

        // Mark the head commit
        set.add_head(GlobalCommitPosition(PAGE_SIZE_IN_BITS));
        assert_eq!(set.next_bitset_pos_to_visit, 0);
        assert_eq!(
            set.bitset.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS).unwrap()
        );

        set.visit_until(index, GlobalCommitPosition(PAGE_SIZE_IN_BITS));
        assert_eq!(set.next_bitset_pos_to_visit, 1);
        assert_eq!(
            set.bitset.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS).unwrap()
        );
        assert!(set.contains(GlobalCommitPosition(PAGE_SIZE_IN_BITS)));

        set.visit_until(index, GlobalCommitPosition(PAGE_SIZE_IN_BITS - 1));
        assert_eq!(set.next_bitset_pos_to_visit, 2);
        assert_eq!(
            set.bitset.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS).unwrap()
        );
        assert!(set.contains(GlobalCommitPosition(PAGE_SIZE_IN_BITS - 1)));

        // Parent link across page boundary
        set.visit_until(index, GlobalCommitPosition(u64::BITS));
        assert_eq!(set.next_bitset_pos_to_visit, PAGE_SIZE_IN_WORDS);
        assert_eq!(
            set.bitset.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS + 1).unwrap()
        );
        assert!(set.contains(GlobalCommitPosition(u64::BITS)));

        set.visit_until(index, GlobalCommitPosition(0));
        assert_eq!(set.next_bitset_pos_to_visit, PAGE_SIZE_IN_WORDS + 1);
        assert_eq!(
            set.bitset.data.len(),
            usize::try_from(PAGE_SIZE_IN_WORDS + 1).unwrap()
        );
        assert!(set.contains(GlobalCommitPosition(0)));
    }
}
