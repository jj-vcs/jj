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

//! An on-disk index of the commits in a repository.
//!
//! Implements an index of the commits in a repository that conforms to the
//! trains in the [index module](crate::index). The index is stored on local
//! disk and contains an entry for every commit in the repository. See
//! [`DefaultReadonlyIndex`] and [`DefaultMutableIndex`].

mod bit_set;
mod changed_path;
mod composite;
mod entry;
mod mutable;
mod readonly;
mod rev_walk;
mod rev_walk_queue;
mod revset_engine;
mod revset_graph_iterator;
mod store;

pub use self::mutable::DefaultMutableIndex;
pub use self::readonly::ChangedPathIndexLevelStats;
pub use self::readonly::CommitIndexLevelStats;
pub use self::readonly::DefaultReadonlyIndex;
pub use self::readonly::DefaultReadonlyIndexRevset;
pub use self::readonly::IndexStats;
pub use self::readonly::ReadonlyIndexLoadError;
pub use self::store::DefaultIndexStore;
pub use self::store::DefaultIndexStoreError;
pub use self::store::DefaultIndexStoreInitError;

#[cfg(test)]
#[rustversion::attr(
    since(1.89),
    expect(clippy::cloned_ref_to_slice_refs, reason = "makes tests more readable")
)]
mod tests {
    use std::cmp::Reverse;
    use std::convert::Infallible;
    use std::ops::Range;
    use std::sync::Arc;

    use itertools::Itertools as _;
    use smallvec::smallvec_inline;
    use test_case::test_case;

    use super::changed_path::CompositeChangedPathIndex;
    use super::composite::AsCompositeIndex as _;
    use super::composite::CommitIndexSegment as _;
    use super::composite::CompositeCommitIndex;
    use super::composite::DynCommitIndexSegment;
    use super::entry::GlobalCommitPosition;
    use super::entry::SmallGlobalCommitPositionsVec;
    use super::mutable::MutableCommitIndexSegment;
    use super::readonly::ReadonlyCommitIndexSegment;
    use super::*;
    use crate::backend::ChangeId;
    use crate::backend::CommitId;
    use crate::default_index::entry::LocalCommitPosition;
    use crate::default_index::entry::SmallLocalCommitPositionsVec;
    use crate::default_index::readonly::FieldLengths;
    use crate::index::Index as _;
    use crate::object_id::HexPrefix;
    use crate::object_id::PrefixResolution;
    use crate::revset::PARENTS_RANGE_FULL;
    use crate::tests::new_temp_dir;

    const TEST_FIELD_LENGTHS: FieldLengths = FieldLengths {
        // TODO: align with commit_id_generator()?
        commit_id: 3,
        change_id: 16,
    };

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

    fn get_commit_index_stats(commits: &Arc<ReadonlyCommitIndexSegment>) -> IndexStats {
        let changed_paths = CompositeChangedPathIndex::null();
        let index = DefaultReadonlyIndex::from_segment(commits.clone(), changed_paths);
        index.stats()
    }

    fn common_ancestors(
        index: &DefaultMutableIndex,
        set1: &[CommitId],
        set2: &[CommitId],
    ) -> Vec<CommitId> {
        index.common_ancestors(set1, set2).unwrap()
    }

    fn is_ancestor(
        index: &DefaultMutableIndex,
        ancestor_id: &CommitId,
        descendant_id: &CommitId,
    ) -> bool {
        index.is_ancestor(ancestor_id, descendant_id).unwrap()
    }

    #[test_case(false; "memory")]
    #[test_case(true; "file")]
    fn index_empty(on_disk: bool) {
        let temp_dir = new_temp_dir();
        let mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);
        let index_segment: Box<DynCommitIndexSegment> = if on_disk {
            let saved_index = mutable_segment.save_in(temp_dir.path()).unwrap();
            // Stats are as expected
            let stats = get_commit_index_stats(&saved_index);
            assert_eq!(stats.num_commits, 0);
            assert_eq!(stats.num_heads, 0);
            assert_eq!(stats.max_generation_number, 0);
            assert_eq!(stats.num_merges, 0);
            assert_eq!(stats.num_changes, 0);
            Box::new(Arc::try_unwrap(saved_index).unwrap())
        } else {
            Box::new(mutable_segment)
        };
        let index = CompositeCommitIndex::new(index_segment.as_ref());
        assert_eq!(index.num_commits(), 0);

        // Cannot find any commits
        assert!(index.entry_by_id(&CommitId::from_hex("000000")).is_none());
        assert!(index.entry_by_id(&CommitId::from_hex("aaa111")).is_none());
        assert!(index.entry_by_id(&CommitId::from_hex("ffffff")).is_none());
    }

    #[test_case(false; "memory")]
    #[test_case(true; "file")]
    fn index_root_commit(on_disk: bool) {
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);
        let id_0 = CommitId::from_hex("000000");
        let change_id0 = new_change_id();
        mutable_segment.add_commit_data(id_0.clone(), change_id0.clone(), &[]);
        let index_segment: Box<DynCommitIndexSegment> = if on_disk {
            let saved_index = mutable_segment.save_in(temp_dir.path()).unwrap();
            // Stats are as expected
            let stats = get_commit_index_stats(&saved_index);
            assert_eq!(stats.num_commits, 1);
            assert_eq!(stats.num_heads, 1);
            assert_eq!(stats.max_generation_number, 0);
            assert_eq!(stats.num_merges, 0);
            assert_eq!(stats.num_changes, 1);
            Box::new(Arc::try_unwrap(saved_index).unwrap())
        } else {
            Box::new(mutable_segment)
        };
        let index = CompositeCommitIndex::new(index_segment.as_ref());
        assert_eq!(index.num_commits(), 1);

        // Can find only the root commit
        assert_eq!(index.commit_id_to_pos(&id_0), Some(GlobalCommitPosition(0)));
        assert_eq!(index.commit_id_to_pos(&CommitId::from_hex("aaaaaa")), None);
        assert_eq!(index.commit_id_to_pos(&CommitId::from_hex("ffffff")), None);
        // Check properties of root entry
        let entry = index.entry_by_id(&id_0).unwrap();
        assert_eq!(entry.position(), GlobalCommitPosition(0));
        assert_eq!(entry.commit_id(), id_0);
        assert_eq!(entry.change_id(), change_id0);
        assert_eq!(entry.generation_number(), 0);
        assert_eq!(entry.num_parents(), 0);
        assert_eq!(
            entry.parent_positions(),
            SmallGlobalCommitPositionsVec::new()
        );
        assert_eq!(entry.parents().len(), 0);
    }

    #[test]
    #[should_panic(expected = "parent commit is not indexed")]
    fn index_missing_parent_commit() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        index.add_commit_data(id_1, new_change_id(), &[id_0]);
    }

    #[test_case(false, false; "full in memory")]
    #[test_case(false, true; "full on disk")]
    #[test_case(true, false; "incremental in memory")]
    #[test_case(true, true; "incremental on disk")]
    fn index_multiple_commits(incremental: bool, on_disk: bool) {
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);
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
        mutable_segment.add_commit_data(id_0.clone(), change_id0, &[]);
        mutable_segment.add_commit_data(id_1.clone(), change_id1.clone(), &[id_0.clone()]);
        mutable_segment.add_commit_data(id_2.clone(), change_id2.clone(), &[id_0.clone()]);

        // If testing incremental indexing, write the first three commits to one file
        // now and build the remainder as another segment on top.
        if incremental {
            let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
            mutable_segment = MutableCommitIndexSegment::incremental(initial_file);
        }

        let id_3 = CommitId::from_hex("333333");
        let change_id3 = new_change_id();
        let id_4 = CommitId::from_hex("444444");
        let change_id4 = new_change_id();
        let id_5 = CommitId::from_hex("555555");
        let change_id5 = change_id3.clone();
        mutable_segment.add_commit_data(id_3.clone(), change_id3.clone(), &[id_2.clone()]);
        mutable_segment.add_commit_data(id_4.clone(), change_id4, &[id_1.clone()]);
        mutable_segment.add_commit_data(id_5.clone(), change_id5, &[id_4.clone(), id_2.clone()]);
        let index_segment: Box<DynCommitIndexSegment> = if on_disk {
            let saved_index = mutable_segment.save_in(temp_dir.path()).unwrap();
            // Stats are as expected
            let stats = get_commit_index_stats(&saved_index);
            assert_eq!(stats.num_commits, 6);
            assert_eq!(stats.num_heads, 2);
            assert_eq!(stats.max_generation_number, 3);
            assert_eq!(stats.num_merges, 1);
            assert_eq!(stats.num_changes, 4);
            Box::new(Arc::try_unwrap(saved_index).unwrap())
        } else {
            Box::new(mutable_segment)
        };
        let index = CompositeCommitIndex::new(index_segment.as_ref());
        assert_eq!(index.num_commits(), 6);

        // Can find all the commits
        let entry_0 = index.entry_by_id(&id_0).unwrap();
        let entry_1 = index.entry_by_id(&id_1).unwrap();
        let entry_2 = index.entry_by_id(&id_2).unwrap();
        let entry_3 = index.entry_by_id(&id_3).unwrap();
        let entry_4 = index.entry_by_id(&id_4).unwrap();
        let entry_5 = index.entry_by_id(&id_5).unwrap();
        // Check properties of some entries
        assert_eq!(entry_0.position(), GlobalCommitPosition(0));
        assert_eq!(entry_0.commit_id(), id_0);
        assert_eq!(entry_1.position(), GlobalCommitPosition(1));
        assert_eq!(entry_1.commit_id(), id_1);
        assert_eq!(entry_1.change_id(), change_id1);
        assert_eq!(entry_1.generation_number(), 1);
        assert_eq!(entry_1.num_parents(), 1);
        assert_eq!(
            entry_1.parent_positions(),
            smallvec_inline![GlobalCommitPosition(0)]
        );
        assert_eq!(entry_1.parents().len(), 1);
        assert_eq!(
            entry_1.parents().next().unwrap().position(),
            GlobalCommitPosition(0)
        );
        assert_eq!(entry_2.position(), GlobalCommitPosition(2));
        assert_eq!(entry_2.commit_id(), id_2);
        assert_eq!(entry_2.change_id(), change_id2);
        assert_eq!(entry_2.generation_number(), 1);
        assert_eq!(entry_2.num_parents(), 1);
        assert_eq!(
            entry_2.parent_positions(),
            smallvec_inline![GlobalCommitPosition(0)]
        );
        assert_eq!(entry_3.change_id(), change_id3);
        assert_eq!(entry_3.generation_number(), 2);
        assert_eq!(
            entry_3.parent_positions(),
            smallvec_inline![GlobalCommitPosition(2)]
        );
        assert_eq!(entry_4.position(), GlobalCommitPosition(4));
        assert_eq!(entry_4.generation_number(), 2);
        assert_eq!(entry_4.num_parents(), 1);
        assert_eq!(
            entry_4.parent_positions(),
            smallvec_inline![GlobalCommitPosition(1)]
        );
        assert_eq!(entry_5.generation_number(), 3);
        assert_eq!(entry_5.num_parents(), 2);
        assert_eq!(
            entry_5.parent_positions(),
            smallvec_inline![GlobalCommitPosition(4), GlobalCommitPosition(2)]
        );
        assert_eq!(entry_5.parents().len(), 2);
        assert_eq!(
            entry_5.parents().next().unwrap().position(),
            GlobalCommitPosition(4)
        );
        assert_eq!(
            entry_5.parents().nth(1).unwrap().position(),
            GlobalCommitPosition(2)
        );
    }

    #[test_case(false; "in memory")]
    #[test_case(true; "on disk")]
    fn index_many_parents(on_disk: bool) {
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);
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
        mutable_segment.add_commit_data(id_0.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        mutable_segment.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        mutable_segment.add_commit_data(id_3.clone(), new_change_id(), &[id_0.clone()]);
        mutable_segment.add_commit_data(id_4.clone(), new_change_id(), &[id_0.clone()]);
        mutable_segment.add_commit_data(id_5.clone(), new_change_id(), &[id_0]);
        mutable_segment.add_commit_data(
            id_6.clone(),
            new_change_id(),
            &[id_1, id_2, id_3, id_4, id_5],
        );
        let index_segment: Box<DynCommitIndexSegment> = if on_disk {
            let saved_index = mutable_segment.save_in(temp_dir.path()).unwrap();
            // Stats are as expected
            let stats = get_commit_index_stats(&saved_index);
            assert_eq!(stats.num_commits, 7);
            assert_eq!(stats.num_heads, 1);
            assert_eq!(stats.max_generation_number, 2);
            assert_eq!(stats.num_merges, 1);
            Box::new(Arc::try_unwrap(saved_index).unwrap())
        } else {
            Box::new(mutable_segment)
        };
        let index = CompositeCommitIndex::new(index_segment.as_ref());
        assert_eq!(index.num_commits(), 7);

        // The octopus merge has the right parents
        let entry_6 = index.entry_by_id(&id_6).unwrap();
        assert_eq!(entry_6.commit_id(), id_6.clone());
        assert_eq!(entry_6.num_parents(), 5);
        assert_eq!(
            entry_6.parent_positions(),
            smallvec_inline![
                GlobalCommitPosition(1),
                GlobalCommitPosition(2),
                GlobalCommitPosition(3),
                GlobalCommitPosition(4),
                GlobalCommitPosition(5),
            ]
        );
        assert_eq!(entry_6.generation_number(), 2);
    }

    #[test]
    fn resolve_commit_id_prefix() {
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        mutable_segment.add_commit_data(id_0.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_1.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file);

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        mutable_segment.add_commit_data(id_3, new_change_id(), &[]);
        mutable_segment.add_commit_data(id_4, new_change_id(), &[]);
        mutable_segment.add_commit_data(id_5, new_change_id(), &[]);

        let index = mutable_segment.as_composite();

        // Can find commits given the full hex number
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::from_id(&id_0)),
            PrefixResolution::SingleMatch(id_0)
        );
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::from_id(&id_1)),
            PrefixResolution::SingleMatch(id_1)
        );
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::from_id(&id_2)),
            PrefixResolution::SingleMatch(id_2)
        );
        // Test nonexistent commits
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("ffffff").unwrap()),
            PrefixResolution::NoMatch
        );
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("000001").unwrap()),
            PrefixResolution::NoMatch
        );
        // Test ambiguous prefix
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("0").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
        // Test a globally unique prefix in initial part
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("009").unwrap()),
            PrefixResolution::SingleMatch(CommitId::from_hex("009999"))
        );
        // Test a globally unique prefix in incremental part
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("03").unwrap()),
            PrefixResolution::SingleMatch(CommitId::from_hex("033333"))
        );
        // Test a locally unique but globally ambiguous prefix
        assert_eq!(
            index.resolve_commit_id_prefix(&HexPrefix::try_from_hex("0554").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
    }

    #[test]
    #[expect(clippy::redundant_clone)] // allow id_n.clone()
    fn neighbor_commit_ids() {
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000001");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        mutable_segment.add_commit_data(id_0.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_1.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file.clone());

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        mutable_segment.add_commit_data(id_3.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_4.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_5.clone(), new_change_id(), &[]);

        // Local lookup in readonly index, commit_id exists.
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&id_0),
            (None, Some(id_1.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&id_1),
            (Some(id_0.clone()), Some(id_2.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&id_2),
            (Some(id_1.clone()), None),
        );

        // Local lookup in readonly index, commit_id does not exist.
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&CommitId::from_hex("000000")),
            (None, Some(id_0.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&CommitId::from_hex("000002")),
            (Some(id_0.clone()), Some(id_1.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_commit_ids(&CommitId::from_hex("ffffff")),
            (Some(id_2.clone()), None),
        );

        // Local lookup in mutable index, commit_id exists. id_5 < id_3 < id_4
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&id_5),
            (None, Some(id_3.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&id_3),
            (Some(id_5.clone()), Some(id_4.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&id_4),
            (Some(id_3.clone()), None),
        );

        // Local lookup in mutable index, commit_id does not exist. id_5 < id_3 < id_4
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&CommitId::from_hex("033332")),
            (None, Some(id_5.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&CommitId::from_hex("033334")),
            (Some(id_5.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_commit_ids(&CommitId::from_hex("ffffff")),
            (Some(id_4.clone()), None),
        );

        // Global lookup, commit_id exists. id_0 < id_1 < id_5 < id_3 < id_2 < id_4
        let composite_index = CompositeCommitIndex::new(&mutable_segment);
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
        let temp_dir = new_temp_dir();
        let mut new_change_id = change_id_generator();
        let mut mutable_segment = MutableCommitIndexSegment::full(TEST_FIELD_LENGTHS);

        // Create some commits with different various common prefixes.
        let id_0 = CommitId::from_hex("000001");
        let id_1 = CommitId::from_hex("009999");
        let id_2 = CommitId::from_hex("055488");
        mutable_segment.add_commit_data(id_0.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_1.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_2.clone(), new_change_id(), &[]);

        // Write the first three commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file);

        let id_3 = CommitId::from_hex("055444");
        let id_4 = CommitId::from_hex("055555");
        let id_5 = CommitId::from_hex("033333");
        mutable_segment.add_commit_data(id_3.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_4.clone(), new_change_id(), &[]);
        mutable_segment.add_commit_data(id_5.clone(), new_change_id(), &[]);

        let index = mutable_segment.as_composite();

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
    fn resolve_change_id_prefix() {
        let temp_dir = new_temp_dir();
        let mut new_commit_id = commit_id_generator();
        let local_positions_vec = |positions: &[u32]| -> SmallLocalCommitPositionsVec {
            positions.iter().copied().map(LocalCommitPosition).collect()
        };
        let index_positions_vec = |positions: &[u32]| -> SmallGlobalCommitPositionsVec {
            positions
                .iter()
                .copied()
                .map(GlobalCommitPosition)
                .collect()
        };

        let id_0 = ChangeId::from_hex("00000001");
        let id_1 = ChangeId::from_hex("00999999");
        let id_2 = ChangeId::from_hex("05548888");
        let id_3 = ChangeId::from_hex("05544444");
        let id_4 = ChangeId::from_hex("05555555");
        let id_5 = ChangeId::from_hex("05555333");

        // Create some commits with different various common prefixes.
        let mut mutable_segment = MutableCommitIndexSegment::full(FieldLengths {
            commit_id: 16,
            change_id: 4,
        });
        mutable_segment.add_commit_data(new_commit_id(), id_0.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);

        // Write these commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file.clone());

        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_4.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_5.clone(), &[]);

        // Local lookup in readonly index with the full hex digits
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::from_id(&id_0)),
            PrefixResolution::SingleMatch((id_0.clone(), local_positions_vec(&[0])))
        );
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::from_id(&id_1)),
            PrefixResolution::SingleMatch((id_1.clone(), local_positions_vec(&[1, 3])))
        );
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::from_id(&id_2)),
            PrefixResolution::SingleMatch((id_2.clone(), local_positions_vec(&[2, 4, 5])))
        );

        // Local lookup in mutable index with the full hex digits
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::from_id(&id_1)),
            PrefixResolution::SingleMatch((id_1.clone(), local_positions_vec(&[3])))
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::from_id(&id_3)),
            PrefixResolution::SingleMatch((id_3.clone(), local_positions_vec(&[0, 1])))
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::from_id(&id_4)),
            PrefixResolution::SingleMatch((id_4.clone(), local_positions_vec(&[2])))
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::from_id(&id_5)),
            PrefixResolution::SingleMatch((id_5.clone(), local_positions_vec(&[4])))
        );

        // Local lookup with locally unknown prefix
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::try_from_hex("0555").unwrap()),
            PrefixResolution::NoMatch
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::try_from_hex("000").unwrap()),
            PrefixResolution::NoMatch
        );

        // Local lookup with locally unique prefix
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::try_from_hex("0554").unwrap()),
            PrefixResolution::SingleMatch((id_2.clone(), local_positions_vec(&[2, 4, 5])))
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::try_from_hex("0554").unwrap()),
            PrefixResolution::SingleMatch((id_3.clone(), local_positions_vec(&[0, 1])))
        );

        // Local lookup with locally ambiguous prefix
        assert_eq!(
            initial_file.resolve_change_id_prefix(&HexPrefix::try_from_hex("00").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
        assert_eq!(
            mutable_segment.resolve_change_id_prefix(&HexPrefix::try_from_hex("05555").unwrap()),
            PrefixResolution::AmbiguousMatch
        );

        let index = mutable_segment.as_composite();

        // Global lookup with the full hex digits
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_0)),
            PrefixResolution::SingleMatch((id_0.clone(), index_positions_vec(&[0])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_1)),
            PrefixResolution::SingleMatch((id_1.clone(), index_positions_vec(&[1, 3, 9])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_2)),
            PrefixResolution::SingleMatch((id_2.clone(), index_positions_vec(&[2, 4, 5])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_3)),
            PrefixResolution::SingleMatch((id_3.clone(), index_positions_vec(&[6, 7])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_4)),
            PrefixResolution::SingleMatch((id_4.clone(), index_positions_vec(&[8])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::from_id(&id_5)),
            PrefixResolution::SingleMatch((id_5.clone(), index_positions_vec(&[10])))
        );

        // Global lookup with unknown prefix
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("ffffffff").unwrap()),
            PrefixResolution::NoMatch
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("00000002").unwrap()),
            PrefixResolution::NoMatch
        );

        // Global lookup with globally unique prefix
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("000").unwrap()),
            PrefixResolution::SingleMatch((id_0.clone(), index_positions_vec(&[0])))
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("055553").unwrap()),
            PrefixResolution::SingleMatch((id_5.clone(), index_positions_vec(&[10])))
        );

        // Global lookup with globally unique prefix stored in both parts
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("009").unwrap()),
            PrefixResolution::SingleMatch((id_1.clone(), index_positions_vec(&[1, 3, 9])))
        );

        // Global lookup with locally ambiguous prefix
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("00").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("05555").unwrap()),
            PrefixResolution::AmbiguousMatch
        );

        // Global lookup with locally unique but globally ambiguous prefix
        assert_eq!(
            index.resolve_change_id_prefix(&HexPrefix::try_from_hex("0554").unwrap()),
            PrefixResolution::AmbiguousMatch
        );
    }

    #[test]
    fn neighbor_change_ids() {
        let temp_dir = new_temp_dir();
        let mut new_commit_id = commit_id_generator();

        let id_0 = ChangeId::from_hex("00000001");
        let id_1 = ChangeId::from_hex("00999999");
        let id_2 = ChangeId::from_hex("05548888");
        let id_3 = ChangeId::from_hex("05544444");
        let id_4 = ChangeId::from_hex("05555555");
        let id_5 = ChangeId::from_hex("05555333");

        // Create some commits with different various common prefixes.
        let mut mutable_segment = MutableCommitIndexSegment::full(FieldLengths {
            commit_id: 16,
            change_id: 4,
        });
        mutable_segment.add_commit_data(new_commit_id(), id_0.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);

        // Write these commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file.clone());

        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_4.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_5.clone(), &[]);

        // Local lookup in readonly index, change_id exists.
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&id_0),
            (None, Some(id_1.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&id_1),
            (Some(id_0.clone()), Some(id_2.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&id_2),
            (Some(id_1.clone()), None),
        );

        // Local lookup in readonly index, change_id does not exist.
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&ChangeId::from_hex("00000000")),
            (None, Some(id_0.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&ChangeId::from_hex("00000002")),
            (Some(id_0.clone()), Some(id_1.clone())),
        );
        assert_eq!(
            initial_file.resolve_neighbor_change_ids(&ChangeId::from_hex("ffffffff")),
            (Some(id_2.clone()), None),
        );

        // Local lookup in mutable index, change_id exists.
        // id_1 < id_3 < id_5 < id_4
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&id_1),
            (None, Some(id_3.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&id_3),
            (Some(id_1.clone()), Some(id_5.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&id_5),
            (Some(id_3.clone()), Some(id_4.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&id_4),
            (Some(id_5.clone()), None),
        );

        // Local lookup in mutable index, change_id does not exist.
        // id_1 < id_3 < id_5 < id_4
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&ChangeId::from_hex("00999998")),
            (None, Some(id_1.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&ChangeId::from_hex("01000000")),
            (Some(id_1.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&ChangeId::from_hex("05555332")),
            (Some(id_3.clone()), Some(id_5.clone())),
        );
        assert_eq!(
            mutable_segment.resolve_neighbor_change_ids(&ChangeId::from_hex("ffffffff")),
            (Some(id_4.clone()), None),
        );

        let index = mutable_segment.as_composite();

        // Global lookup, change_id exists.
        // id_0 < id_1 < id_3 < id_2 < id_5 < id_4
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_0),
            (None, Some(id_1.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_1),
            (Some(id_0.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_3),
            (Some(id_1.clone()), Some(id_2.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_2),
            (Some(id_3.clone()), Some(id_5.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_5),
            (Some(id_2.clone()), Some(id_4.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&id_4),
            (Some(id_5.clone()), None),
        );

        // Global lookup, change_id doesn't exist.
        // id_0 < id_1 < id_3 < id_2 < id_5 < id_4
        assert_eq!(
            index.resolve_neighbor_change_ids(&ChangeId::from_hex("00000000")),
            (None, Some(id_0.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&ChangeId::from_hex("01000000")),
            (Some(id_1.clone()), Some(id_3.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&ChangeId::from_hex("05544555")),
            (Some(id_3.clone()), Some(id_2.clone())),
        );
        assert_eq!(
            index.resolve_neighbor_change_ids(&ChangeId::from_hex("ffffffff")),
            (Some(id_4.clone()), None),
        );
    }

    #[test]
    fn shortest_unique_change_id_prefix() {
        let temp_dir = new_temp_dir();
        let mut new_commit_id = commit_id_generator();

        let id_0 = ChangeId::from_hex("00000001");
        let id_1 = ChangeId::from_hex("00999999");
        let id_2 = ChangeId::from_hex("05548888");
        let id_3 = ChangeId::from_hex("05544444");
        let id_4 = ChangeId::from_hex("05555555");
        let id_5 = ChangeId::from_hex("05555333");

        // Create some commits with different various common prefixes.
        let mut mutable_segment = MutableCommitIndexSegment::full(FieldLengths {
            commit_id: 16,
            change_id: 4,
        });
        mutable_segment.add_commit_data(new_commit_id(), id_0.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_2.clone(), &[]);

        // Write these commits to one file and build the remainder on top.
        let initial_file = mutable_segment.save_in(temp_dir.path()).unwrap();
        mutable_segment = MutableCommitIndexSegment::incremental(initial_file.clone());

        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_3.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_4.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_1.clone(), &[]);
        mutable_segment.add_commit_data(new_commit_id(), id_5.clone(), &[]);

        let index = mutable_segment.as_composite();

        // Calculate shortest unique prefix len with known change_id
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_0), 3);
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_1), 3);
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_2), 5);
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_3), 5);
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_4), 6);
        assert_eq!(index.shortest_unique_change_id_prefix_len(&id_5), 6);

        // Calculate shortest unique prefix len with unknown change_id
        assert_eq!(
            index.shortest_unique_change_id_prefix_len(&ChangeId::from_hex("00000002")),
            8
        );
        assert_eq!(
            index.shortest_unique_change_id_prefix_len(&ChangeId::from_hex("01000000")),
            2
        );
        assert_eq!(
            index.shortest_unique_change_id_prefix_len(&ChangeId::from_hex("05555344")),
            7
        );
        assert_eq!(
            index.shortest_unique_change_id_prefix_len(&ChangeId::from_hex("ffffffff")),
            1
        );
    }

    #[test]
    fn test_is_ancestor() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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

        assert!(is_ancestor(&index, &id_0, &id_0));
        assert!(is_ancestor(&index, &id_0, &id_1));
        assert!(is_ancestor(&index, &id_2, &id_3));
        assert!(is_ancestor(&index, &id_2, &id_5));
        assert!(is_ancestor(&index, &id_1, &id_5));
        assert!(is_ancestor(&index, &id_0, &id_5));
        assert!(!is_ancestor(&index, &id_1, &id_0));
        assert!(!is_ancestor(&index, &id_5, &id_3));
        assert!(!is_ancestor(&index, &id_3, &id_5));
        assert!(!is_ancestor(&index, &id_2, &id_4));
        assert!(!is_ancestor(&index, &id_4, &id_2));
    }

    #[test]
    fn test_common_ancestors() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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
        let id_6 = CommitId::from_hex("666666");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);
        index.add_commit_data(id_6.clone(), new_change_id(), &[id_4.clone()]);

        assert_eq!(
            common_ancestors(&index, &[id_0.clone()], &[id_0.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_5.clone()], &[id_5.clone()]),
            vec![id_5.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_1.clone()], &[id_2.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_2.clone()], &[id_1.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_1.clone()], &[id_4.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_4.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_3.clone()], &[id_5.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_5.clone()], &[id_3.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_2.clone()], &[id_6.clone()]),
            vec![id_0.clone()]
        );

        // With multiple commits in an input set
        assert_eq!(
            common_ancestors(&index, &[id_0.clone(), id_1.clone()], &[id_0.clone()]),
            vec![id_0.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_0.clone(), id_1.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_1.clone(), id_2.clone()], &[id_1.clone()]),
            vec![id_1.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_1.clone(), id_2.clone()], &[id_4]),
            vec![id_1.clone()]
        );
        assert_eq!(
            common_ancestors(&index, &[id_5.clone(), id_6.clone()], &[id_2.clone()]),
            &[id_2.clone()]
        );
        // Both 1 and 2 are returned since (5) expands to (2, 4), which expands
        // to (1,2) and matches the (1,2) of the first input set.
        assert_eq!(
            common_ancestors(&index, &[id_1.clone(), id_2.clone()], &[id_5]),
            vec![id_2.clone(), id_1.clone()]
        );
        assert_eq!(common_ancestors(&index, &[id_1, id_2], &[id_3]), vec![id_0]);
    }

    #[test]
    fn test_common_ancestors_criss_cross() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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

        let mut common_ancestors = common_ancestors(&index, &[id_3], &[id_4]);
        common_ancestors.sort();
        assert_eq!(common_ancestors, vec![id_1, id_2]);
    }

    #[test]
    fn test_common_ancestors_merge_with_ancestor() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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

        let mut common_ancestors = common_ancestors(&index, &[id_4], &[id_5]);
        common_ancestors.sort();
        assert_eq!(common_ancestors, vec![id_2]);
    }

    #[test]
    fn test_heads() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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
        assert!(index.heads(&mut [].iter()).unwrap().is_empty());
        // Single head
        assert_eq!(
            index.heads(&mut [id_4.clone()].iter()).unwrap(),
            vec![id_4.clone()]
        );
        // Single head and parent
        assert_eq!(
            index.heads(&mut [id_4.clone(), id_1].iter()).unwrap(),
            vec![id_4.clone()]
        );
        // Single head and grand-parent
        assert_eq!(
            index.heads(&mut [id_4.clone(), id_0].iter()).unwrap(),
            vec![id_4.clone()]
        );
        // Multiple heads
        assert_eq!(
            index
                .heads(&mut [id_3.clone(), id_4.clone()].iter())
                .unwrap(),
            vec![id_4.clone(), id_3.clone()]
        );
        // Duplicated inputs
        assert_eq!(
            index
                .heads(&mut [id_4.clone(), id_3.clone(), id_4.clone()].iter())
                .unwrap(),
            vec![id_4.clone(), id_3.clone()]
        );
        // Merge commit and ancestors
        assert_eq!(
            index.heads(&mut [id_5.clone(), id_2].iter()).unwrap(),
            vec![id_5.clone()]
        );
        // Merge commit and other commit
        assert_eq!(
            index
                .heads(&mut [id_5.clone(), id_3.clone()].iter())
                .unwrap(),
            vec![id_5.clone(), id_3.clone()]
        );

        assert_eq!(
            index.all_heads_for_gc().unwrap().collect_vec(),
            vec![id_3.clone(), id_5.clone()]
        );
    }

    #[test]
    fn test_heads_range_with_filter() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
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

        // Helper function to convert commit IDs to/from index positions and call
        // `heads_from_range_and_filter`.
        let heads_range = |roots: &[&CommitId],
                           heads: &[&CommitId],
                           parents_range: &Range<u32>,
                           filter: &dyn Fn(CommitId) -> bool|
         -> Vec<CommitId> {
            let roots = roots
                .iter()
                .map(|id| index.as_composite().commits().commit_id_to_pos(id).unwrap())
                .sorted_by_key(|&pos| Reverse(pos))
                .collect_vec();
            let heads = heads
                .iter()
                .map(|id| index.as_composite().commits().commit_id_to_pos(id).unwrap())
                .sorted_by_key(|&pos| Reverse(pos))
                .collect_vec();
            index
                .as_composite()
                .commits()
                .heads_from_range_and_filter::<Infallible>(roots, heads, parents_range, |pos| {
                    Ok(filter(
                        index.as_composite().commits().entry_by_pos(pos).commit_id(),
                    ))
                })
                .unwrap()
                .into_iter()
                .map(|pos| index.as_composite().commits().entry_by_pos(pos).commit_id())
                .collect_vec()
        };

        // heads(::none())
        assert!(heads_range(&[], &[], &PARENTS_RANGE_FULL, &|_| true).is_empty());
        // heads(all())
        assert_eq!(
            heads_range(&[], &[&id_5, &id_3], &PARENTS_RANGE_FULL, &|_| true),
            vec![id_5.clone(), id_3.clone()]
        );
        // heads(~5)
        assert_eq!(
            heads_range(&[], &[&id_5, &id_3], &PARENTS_RANGE_FULL, &|id| id != id_5),
            vec![id_4.clone(), id_3.clone()]
        );
        // heads(5..)
        assert_eq!(
            heads_range(&[&id_5], &[&id_5, &id_3], &PARENTS_RANGE_FULL, &|_| true),
            vec![id_3.clone()]
        );
        // heads(5.. ~ 3)
        assert!(
            heads_range(&[&id_5], &[&id_5, &id_3], &PARENTS_RANGE_FULL, &|id| id
                != id_3)
            .is_empty()
        );
        // heads(2..4)
        assert_eq!(
            heads_range(&[&id_2], &[&id_4], &PARENTS_RANGE_FULL, &|_| true),
            vec![id_4.clone()]
        );
        // heads(2..4 ~ 4)
        assert_eq!(
            heads_range(&[&id_2], &[&id_4], &PARENTS_RANGE_FULL, &|id| id != id_4),
            vec![id_1.clone()]
        );
        // heads((3 | 1).. ~ 5)
        assert_eq!(
            heads_range(
                &[&id_3, &id_1],
                &[&id_5, &id_3],
                &PARENTS_RANGE_FULL,
                &|id| id != id_5
            ),
            vec![id_4.clone()]
        );
        // heads(::(5 | 4) ~ 5)
        assert_eq!(
            heads_range(&[], &[&id_5, &id_4], &PARENTS_RANGE_FULL, &|id| id != id_5),
            vec![id_4.clone(), id_2.clone()]
        );
        // heads(::(5 | 5))
        assert_eq!(
            heads_range(&[], &[&id_5, &id_5], &PARENTS_RANGE_FULL, &|_| true),
            vec![id_5.clone()]
        );
        // heads(::5 ~ 5)
        assert_eq!(
            heads_range(&[], &[&id_5], &PARENTS_RANGE_FULL, &|id| id != id_5),
            vec![id_4.clone(), id_2.clone()]
        );
        // heads(first_ancestors(5) ~ 5)
        assert_eq!(
            heads_range(&[], &[&id_5], &(0..1), &|id| id != id_5),
            vec![id_4.clone()]
        );
        // heads(ancestors(5, parents_range=1..2) ~ 5)
        assert_eq!(
            heads_range(&[], &[&id_5], &(1..2), &|id| id != id_5),
            vec![id_2.clone()]
        );
    }
}
