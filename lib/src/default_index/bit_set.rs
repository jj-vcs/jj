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

use super::composite::CompositeCommitIndex;
use super::entry::GlobalCommitPosition;

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
    bitset: Vec<u64>,
    last_visited_bitset_pos: u32,
}

impl AncestorsBitSet {
    /// Creates bit set of the specified capacity.
    pub fn with_capacity(len: u32) -> Self {
        let bitset_len = usize::try_from(u32::div_ceil(len, u64::BITS)).unwrap();
        AncestorsBitSet {
            bitset: vec![0; bitset_len], // request zeroed page
            last_visited_bitset_pos: 0,
        }
    }

    /// Adds head `pos` to the set.
    ///
    /// Panics if the `pos` exceeds the capacity.
    pub fn add_head(&mut self, pos: GlobalCommitPosition) {
        let bitset_pos = pos.0 / u64::BITS;
        let bit = 1_u64 << (pos.0 % u64::BITS);
        self.bitset[usize::try_from(bitset_pos).unwrap()] |= bit;
        self.last_visited_bitset_pos = max(self.last_visited_bitset_pos, bitset_pos + 1);
    }

    /// Returns `true` if the given `pos` is ancestors of the heads.
    ///
    /// Panics if the `pos` exceeds the capacity or has not been visited yet.
    pub fn contains(&self, pos: GlobalCommitPosition) -> bool {
        let bitset_pos = pos.0 / u64::BITS;
        let bit = 1_u64 << (pos.0 % u64::BITS);
        assert!(bitset_pos >= self.last_visited_bitset_pos);
        self.bitset[usize::try_from(bitset_pos).unwrap()] & bit != 0
    }

    /// Updates set by visiting ancestors until the given `to_visit_pos`.
    pub fn visit_until(
        &mut self,
        index: &CompositeCommitIndex,
        to_visit_pos: GlobalCommitPosition,
    ) {
        let to_visit_bitset_pos = to_visit_pos.0 / u64::BITS;
        if to_visit_bitset_pos >= self.last_visited_bitset_pos {
            return;
        }
        for visiting_bitset_pos in (to_visit_bitset_pos..self.last_visited_bitset_pos).rev() {
            let mut unvisited_bits = self.bitset[usize::try_from(visiting_bitset_pos).unwrap()];
            while unvisited_bits != 0 {
                let bit_pos = u64::BITS - unvisited_bits.leading_zeros() - 1; // from MSB
                unvisited_bits ^= 1_u64 << bit_pos;
                let current_pos = GlobalCommitPosition(visiting_bitset_pos * u64::BITS + bit_pos);
                for parent_pos in index.entry_by_pos(current_pos).parent_positions() {
                    assert!(parent_pos < current_pos);
                    let parent_bitset_pos = parent_pos.0 / u64::BITS;
                    let bit = 1_u64 << (parent_pos.0 % u64::BITS);
                    self.bitset[usize::try_from(parent_bitset_pos).unwrap()] |= bit;
                    if visiting_bitset_pos == parent_bitset_pos {
                        unvisited_bits |= bit;
                    }
                }
            }
        }
        self.last_visited_bitset_pos = to_visit_bitset_pos;
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
        mutable_index.add_commit_data(id_d192.clone(), new_change_id(), &[id_c190.clone()]);
        let id_e254 = (193..=254).fold(id_b191.clone(), |parent_id, i| {
            assert_eq!(mutable_index.num_commits(), i);
            let id = new_commit_id();
            mutable_index.add_commit_data(id.clone(), new_change_id(), &[parent_id]);
            id
        });
        let id_d255 = new_commit_id();
        mutable_index.add_commit_data(id_d255.clone(), new_change_id(), &[id_d192.clone()]);
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
        assert_eq!(set.last_visited_bitset_pos, 0);
        for pos in (0..=256).map(GlobalCommitPosition) {
            assert!(!set.contains(pos), "{pos:?} should be unreachable");
        }

        // All reachable
        let mut set = new_ancestors_set(&[&id_f256, &id_d255]);
        assert_eq!(set.last_visited_bitset_pos, 5);
        set.visit_until(index, to_pos(&id_f256));
        assert_eq!(set.last_visited_bitset_pos, 4);
        assert!(set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_d192));
        assert_eq!(set.last_visited_bitset_pos, 3);
        assert!(set.contains(to_pos(&id_e254)));
        assert!(set.contains(to_pos(&id_d255)));
        assert!(set.contains(to_pos(&id_d192)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.last_visited_bitset_pos, 0);
        set.visit_until(index, to_pos(&id_f256)); // should be noop
        assert_eq!(set.last_visited_bitset_pos, 0);
        for pos in (0..=256).map(GlobalCommitPosition) {
            assert!(set.contains(pos), "{pos:?} should be reachable");
        }

        // A, B, C, E, F are reachable
        let mut set = new_ancestors_set(&[&id_f256]);
        assert_eq!(set.last_visited_bitset_pos, 5);
        set.visit_until(index, to_pos(&id_f256));
        assert_eq!(set.last_visited_bitset_pos, 4);
        assert!(set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_d192));
        assert_eq!(set.last_visited_bitset_pos, 3);
        assert!(!set.contains(to_pos(&id_d255)));
        assert!(!set.contains(to_pos(&id_d192)));
        set.visit_until(index, to_pos(&id_c190));
        assert_eq!(set.last_visited_bitset_pos, 2);
        assert!(set.contains(to_pos(&id_c190)));
        set.visit_until(index, to_pos(&id_a64));
        assert_eq!(set.last_visited_bitset_pos, 1);
        assert!(set.contains(to_pos(&id_b191)));
        assert!(set.contains(to_pos(&id_a64)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.last_visited_bitset_pos, 0);
        assert!(set.contains(to_pos(&id_a0)));

        // A, C, D are reachable
        let mut set = new_ancestors_set(&[&id_d255]);
        assert_eq!(set.last_visited_bitset_pos, 4);
        assert!(!set.contains(to_pos(&id_f256)));
        set.visit_until(index, to_pos(&id_e254));
        assert_eq!(set.last_visited_bitset_pos, 3);
        assert!(!set.contains(to_pos(&id_e254)));
        set.visit_until(index, to_pos(&id_d255));
        assert_eq!(set.last_visited_bitset_pos, 3);
        assert!(set.contains(to_pos(&id_d255)));
        set.visit_until(index, to_pos(&id_b191));
        assert_eq!(set.last_visited_bitset_pos, 2);
        assert!(!set.contains(to_pos(&id_b191)));
        set.visit_until(index, to_pos(&id_c190));
        assert_eq!(set.last_visited_bitset_pos, 2);
        assert!(set.contains(to_pos(&id_c190)));
        set.visit_until(index, to_pos(&id_a0));
        assert_eq!(set.last_visited_bitset_pos, 0);
        assert!(set.contains(to_pos(&id_a64)));
        assert!(set.contains(to_pos(&id_a0)));
    }
}
