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

use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use assert_matches::assert_matches;
use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::default_index::DefaultIndexStore;
use jj_lib::default_index::DefaultIndexStoreError;
use jj_lib::default_index::DefaultMutableIndex;
use jj_lib::default_index::DefaultReadonlyIndex;
use jj_lib::index::Index;
use jj_lib::object_id::HexPrefix;
use jj_lib::object_id::ObjectId as _;
use jj_lib::object_id::PrefixResolution;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::revset::GENERATION_RANGE_FULL;
use jj_lib::revset::PARENTS_RANGE_FULL;
use jj_lib::revset::ResolvedExpression;
use maplit::hashset;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::assert_tree_eq;
use testutils::commit_transactions;
use testutils::create_tree;
use testutils::repo_path;
use testutils::repo_path_buf;
use testutils::test_backend::TestBackend;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn remote_symbol<'a, N, M>(name: &'a N, remote: &'a M) -> RemoteRefSymbol<'a>
where
    N: AsRef<RefName> + ?Sized,
    M: AsRef<RemoteName> + ?Sized,
{
    RemoteRefSymbol {
        name: name.as_ref(),
        remote: remote.as_ref(),
    }
}

fn enable_changed_path_index(repo: &ReadonlyRepo) -> Arc<ReadonlyRepo> {
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();
    default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), 0)
        .block_on()
        .unwrap();
    repo.reload_at(repo.operation()).unwrap()
}

fn collect_changed_paths(repo: &ReadonlyRepo, commit_id: &CommitId) -> Option<Vec<RepoPathBuf>> {
    repo.index()
        .changed_paths_in_commit(commit_id)
        .unwrap()
        .map(|paths| paths.collect())
}

fn index_has_id(index: &dyn Index, commit_id: &CommitId) -> bool {
    index.has_id(commit_id).unwrap()
}

fn is_ancestor(
    index: &DefaultReadonlyIndex,
    ancestor_id: &CommitId,
    descendant_id: &CommitId,
) -> bool {
    index.is_ancestor(ancestor_id, descendant_id).unwrap()
}

#[test]
fn test_index_commits_empty_repo() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let index = as_readonly_index(repo);
    // There should be just the root commit
    assert_eq!(index.num_commits(), 1);

    // Check the generation numbers of the root and the working copy
    assert_eq!(
        index
            .generation_number(repo.store().root_commit_id())
            .unwrap(),
        0
    );
}

#[test]
fn test_index_commits_standard_cases() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    //   o H
    // o | G
    // o | F
    // |\|
    // | o E
    // | o D
    // | o C
    // o | B
    // |/
    // o A
    // | o working copy
    // |/
    // o root

    let root_commit_id = repo.store().root_commit_id();
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_e]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_f]);
    let commit_h = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e]);
    let repo = tx.commit("test").unwrap();

    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 8 more
    assert_eq!(index.num_commits(), 1 + 8);

    let stats = index.stats();
    assert_eq!(stats.num_commits, 1 + 8);
    assert_eq!(stats.num_merges, 1);
    assert_eq!(stats.max_generation_number, 6);

    assert_eq!(index.generation_number(root_commit_id).unwrap(), 0);
    assert_eq!(index.generation_number(commit_a.id()).unwrap(), 1);
    assert_eq!(index.generation_number(commit_b.id()).unwrap(), 2);
    assert_eq!(index.generation_number(commit_c.id()).unwrap(), 2);
    assert_eq!(index.generation_number(commit_d.id()).unwrap(), 3);
    assert_eq!(index.generation_number(commit_e.id()).unwrap(), 4);
    assert_eq!(index.generation_number(commit_f.id()).unwrap(), 5);
    assert_eq!(index.generation_number(commit_g.id()).unwrap(), 6);
    assert_eq!(index.generation_number(commit_h.id()).unwrap(), 5);

    assert!(is_ancestor(index, root_commit_id, commit_a.id()));
    assert!(!is_ancestor(index, commit_a.id(), root_commit_id));

    assert!(is_ancestor(index, root_commit_id, commit_b.id()));
    assert!(!is_ancestor(index, commit_b.id(), root_commit_id));

    assert!(!is_ancestor(index, commit_b.id(), commit_c.id()));

    assert!(is_ancestor(index, commit_a.id(), commit_b.id()));
    assert!(is_ancestor(index, commit_a.id(), commit_e.id()));
    assert!(is_ancestor(index, commit_a.id(), commit_f.id()));
    assert!(is_ancestor(index, commit_a.id(), commit_g.id()));
    assert!(is_ancestor(index, commit_a.id(), commit_h.id()));
}

#[test]
fn test_index_commits_criss_cross() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let num_generations = 50;

    // Create a long chain of criss-crossed merges. If they were traversed without
    // keeping track of visited nodes, it would be 2^50 visits, so if this test
    // finishes in reasonable time, we know that we don't do a naive traversal.
    let mut tx = repo.start_transaction();
    let mut left_commits = vec![write_random_commit(tx.repo_mut())];
    let mut right_commits = vec![write_random_commit(tx.repo_mut())];
    for generation in 1..num_generations {
        let new_left = write_random_commit_with_parents(
            tx.repo_mut(),
            &[
                &left_commits[generation - 1],
                &right_commits[generation - 1],
            ],
        );
        let new_right = write_random_commit_with_parents(
            tx.repo_mut(),
            &[
                &left_commits[generation - 1],
                &right_commits[generation - 1],
            ],
        );
        left_commits.push(new_left);
        right_commits.push(new_right);
    }
    let repo = tx.commit("test").unwrap();

    let index = as_readonly_index(&repo);
    // There should the root commit, plus 2 for each generation
    assert_eq!(index.num_commits(), 1 + 2 * (num_generations as u32));

    let stats = index.stats();
    assert_eq!(stats.num_commits, 1 + 2 * (num_generations as u32));
    // The first generations are not merges
    assert_eq!(stats.num_merges, 2 * (num_generations as u32 - 1));
    assert_eq!(stats.max_generation_number, num_generations as u32);

    // Check generation numbers
    for generation in 0..num_generations {
        assert_eq!(
            index
                .generation_number(left_commits[generation].id())
                .unwrap(),
            (generation as u32) + 1
        );
        assert_eq!(
            index
                .generation_number(right_commits[generation].id())
                .unwrap(),
            (generation as u32) + 1
        );
    }

    // The left and right commits of the same generation should not be ancestors of
    // each other
    for generation in 0..num_generations {
        assert!(!is_ancestor(
            index,
            left_commits[generation].id(),
            right_commits[generation].id()
        ));
        assert!(!is_ancestor(
            index,
            right_commits[generation].id(),
            left_commits[generation].id()
        ));
    }

    // Both sides of earlier generations should be ancestors. Check a few different
    // earlier generations.
    for generation in 1..num_generations {
        for ancestor_side in &[&left_commits, &right_commits] {
            for descendant_side in &[&left_commits, &right_commits] {
                assert!(is_ancestor(
                    index,
                    ancestor_side[0].id(),
                    descendant_side[generation].id()
                ));
                assert!(is_ancestor(
                    index,
                    ancestor_side[generation - 1].id(),
                    descendant_side[generation].id()
                ));
                assert!(is_ancestor(
                    index,
                    ancestor_side[generation / 2].id(),
                    descendant_side[generation].id()
                ));
            }
        }
    }

    let count_revs = |wanted: &[CommitId], unwanted: &[CommitId], generation| {
        // Constructs ResolvedExpression directly to bypass tree optimization.
        let expression = ResolvedExpression::Range {
            roots: ResolvedExpression::Commits(unwanted.to_vec()).into(),
            heads: ResolvedExpression::Commits(wanted.to_vec()).into(),
            generation,
            parents_range: PARENTS_RANGE_FULL,
        };
        let revset = index.evaluate_revset(&expression, repo.store()).unwrap();
        // Don't switch to more efficient .count() implementation. Here we're
        // testing the iterator behavior.
        revset.iter().count()
    };

    // RevWalk deduplicates chains by entry.
    assert_eq!(
        count_revs(
            &[left_commits[num_generations - 1].id().clone()],
            &[],
            GENERATION_RANGE_FULL,
        ),
        2 * num_generations
    );
    assert_eq!(
        count_revs(
            &[right_commits[num_generations - 1].id().clone()],
            &[],
            GENERATION_RANGE_FULL,
        ),
        2 * num_generations
    );
    assert_eq!(
        count_revs(
            &[left_commits[num_generations - 1].id().clone()],
            &[left_commits[num_generations - 2].id().clone()],
            GENERATION_RANGE_FULL,
        ),
        2
    );
    assert_eq!(
        count_revs(
            &[right_commits[num_generations - 1].id().clone()],
            &[right_commits[num_generations - 2].id().clone()],
            GENERATION_RANGE_FULL,
        ),
        2
    );

    // RevWalkGenerationRange deduplicates chains by (entry, generation), which may
    // be more expensive than RevWalk, but should still finish in reasonable time.
    assert_eq!(
        count_revs(
            &[left_commits[num_generations - 1].id().clone()],
            &[],
            0..(num_generations + 1) as u64,
        ),
        2 * num_generations
    );
    assert_eq!(
        count_revs(
            &[right_commits[num_generations - 1].id().clone()],
            &[],
            0..(num_generations + 1) as u64,
        ),
        2 * num_generations
    );
    assert_eq!(
        count_revs(
            &[left_commits[num_generations - 1].id().clone()],
            &[left_commits[num_generations - 2].id().clone()],
            0..(num_generations + 1) as u64,
        ),
        2
    );
    assert_eq!(
        count_revs(
            &[right_commits[num_generations - 1].id().clone()],
            &[right_commits[num_generations - 2].id().clone()],
            0..(num_generations + 1) as u64,
        ),
        2
    );
}

#[test]
fn test_index_commits_previous_operations() {
    // Test that commits visible only in previous operations are indexed.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    // Remove commit B and C in one operation and make sure they're still
    // visible in the index after that operation.
    // o C
    // o B
    // o A
    // | o working copy
    // |/
    // o root

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().remove_head(commit_c.id());
    let repo = tx.commit("test").unwrap();

    // Delete index from disk
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();
    default_index_store.reinit().unwrap();

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 3 more
    assert_eq!(index.num_commits(), 1 + 3);

    let stats = index.stats();
    assert_eq!(stats.num_commits, 1 + 3);
    assert_eq!(stats.num_merges, 0);
    assert_eq!(stats.max_generation_number, 3);

    assert_eq!(index.generation_number(commit_a.id()).unwrap(), 1);
    assert_eq!(index.generation_number(commit_b.id()).unwrap(), 2);
    assert_eq!(index.generation_number(commit_c.id()).unwrap(), 3);
}

#[test]
fn test_index_commits_hidden_but_referenced() {
    // Test that hidden-but-referenced commits are indexed.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    // Remote bookmarks are usually visible at a certain point in operation
    // history, but that's not guaranteed if old operations have been discarded.
    // This can also happen if imported remote bookmarks get immediately
    // abandoned because the other bookmark has moved.
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit(tx.repo_mut());
    let commit_c = write_random_commit(tx.repo_mut());
    tx.repo_mut().remove_head(commit_a.id());
    tx.repo_mut().remove_head(commit_b.id());
    tx.repo_mut().remove_head(commit_c.id());
    tx.repo_mut().set_remote_bookmark(
        remote_symbol("bookmark", "origin"),
        RemoteRef {
            target: RefTarget::from_legacy_form(
                [commit_a.id().clone()],
                [commit_b.id().clone(), commit_c.id().clone()],
            ),
            state: jj_lib::op_store::RemoteRefState::New,
        },
    );

    let repo = tx.commit("test").unwrap();
    // All commits should be indexed
    assert!(index_has_id(repo.index(), commit_a.id()));
    assert!(index_has_id(repo.index(), commit_b.id()));
    assert!(index_has_id(repo.index(), commit_c.id()));

    // Delete index from disk
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();
    default_index_store.reinit().unwrap();

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    // All commits should be reindexed
    assert!(index_has_id(repo.index(), commit_a.id()));
    assert!(index_has_id(repo.index(), commit_b.id()));
    assert!(index_has_id(repo.index(), commit_c.id()));
}

#[test]
fn test_index_commits_incremental() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    // Create A in one operation, then B and C in another. Check that the index is
    // valid after.
    // o C
    // o B
    // o A
    // | o working copy
    // |/
    // o root

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit_with_parents(tx.repo_mut(), &[]);
    let repo = tx.commit("test").unwrap();

    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 1 more
    assert_eq!(index.num_commits(), 1 + 1);

    let mut tx = repo.start_transaction();
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    tx.commit("test").unwrap();

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 3 more
    assert_eq!(index.num_commits(), 1 + 3);

    let stats = index.stats();
    assert_eq!(stats.num_commits, 1 + 3);
    assert_eq!(stats.num_merges, 0);
    assert_eq!(stats.max_generation_number, 3);
    assert_eq!(stats.commit_levels.len(), 1);
    assert_eq!(stats.commit_levels[0].num_commits, 4);

    assert_eq!(index.generation_number(root_commit.id()).unwrap(), 0);
    assert_eq!(index.generation_number(commit_a.id()).unwrap(), 1);
    assert_eq!(index.generation_number(commit_b.id()).unwrap(), 2);
    assert_eq!(index.generation_number(commit_c.id()).unwrap(), 3);
}

#[test]
fn test_index_commits_incremental_empty_transaction() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    // Create A in one operation, then just an empty transaction. Check that the
    // index is valid after.
    // o A
    // | o working copy
    // |/
    // o root

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit_with_parents(tx.repo_mut(), &[&root_commit]);
    let repo = tx.commit("test").unwrap();

    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 1 more
    assert_eq!(index.num_commits(), 1 + 1);

    repo.start_transaction().commit("test").unwrap();

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    let index = as_readonly_index(&repo);
    // There should be the root commit, plus 1 more
    assert_eq!(index.num_commits(), 1 + 1);

    let stats = index.stats();
    assert_eq!(stats.num_commits, 1 + 1);
    assert_eq!(stats.num_merges, 0);
    assert_eq!(stats.max_generation_number, 1);
    assert_eq!(stats.commit_levels.len(), 1);
    assert_eq!(stats.commit_levels[0].num_commits, 2);

    assert_eq!(index.generation_number(root_commit.id()).unwrap(), 0);
    assert_eq!(index.generation_number(commit_a.id()).unwrap(), 1);
}

#[test]
fn test_index_commits_incremental_already_indexed() {
    // Tests that trying to add a commit that's already been added is a no-op.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Create A in one operation, then try to add it again an new transaction.
    // o A
    // | o working copy
    // |/
    // o root

    let root_commit = repo.store().root_commit();
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit_with_parents(tx.repo_mut(), &[&root_commit]);
    let repo = tx.commit("test").unwrap();

    assert!(index_has_id(repo.index(), commit_a.id()));
    assert_eq!(as_readonly_index(&repo).num_commits(), 1 + 1);
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo.add_head(&commit_a).unwrap();
    assert_eq!(as_mutable_index(mut_repo).num_commits(), 1 + 1);
}

#[must_use]
fn create_n_commits(repo: &Arc<ReadonlyRepo>, num_commits: i32) -> Arc<ReadonlyRepo> {
    let mut tx = repo.start_transaction();
    for _ in 0..num_commits {
        write_random_commit(tx.repo_mut());
    }
    tx.commit("test").unwrap()
}

fn as_readonly_index(repo: &Arc<ReadonlyRepo>) -> &DefaultReadonlyIndex {
    repo.readonly_index().downcast_ref().unwrap()
}

fn as_mutable_index(repo: &MutableRepo) -> &DefaultMutableIndex {
    repo.mutable_index().downcast_ref().unwrap()
}

fn commits_by_level(repo: &Arc<ReadonlyRepo>) -> Vec<u32> {
    as_readonly_index(repo)
        .stats()
        .commit_levels
        .iter()
        .map(|level| level.num_commits)
        .collect()
}

#[test]
fn test_index_commits_incremental_squashed() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 1);
    assert_eq!(commits_by_level(&repo), vec![2]);
    let repo = create_n_commits(&repo, 1);
    assert_eq!(commits_by_level(&repo), vec![3]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 2);
    assert_eq!(commits_by_level(&repo), vec![3]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 100);
    assert_eq!(commits_by_level(&repo), vec![101]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 1);
    let repo = create_n_commits(&repo, 2);
    let repo = create_n_commits(&repo, 4);
    let repo = create_n_commits(&repo, 8);
    let repo = create_n_commits(&repo, 16);
    let repo = create_n_commits(&repo, 32);
    assert_eq!(commits_by_level(&repo), vec![64]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 32);
    let repo = create_n_commits(&repo, 16);
    let repo = create_n_commits(&repo, 8);
    let repo = create_n_commits(&repo, 4);
    let repo = create_n_commits(&repo, 2);
    assert_eq!(commits_by_level(&repo), vec![57, 6]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 30);
    let repo = create_n_commits(&repo, 15);
    let repo = create_n_commits(&repo, 7);
    let repo = create_n_commits(&repo, 3);
    let repo = create_n_commits(&repo, 1);
    assert_eq!(commits_by_level(&repo), vec![31, 15, 7, 3, 1]);

    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let repo = create_n_commits(repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    let repo = create_n_commits(&repo, 10);
    assert_eq!(commits_by_level(&repo), vec![71, 20]);
}

#[test]
fn test_reindex_no_segments_dir() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    assert!(index_has_id(repo.index(), commit_a.id()));

    // jj <= 0.14 doesn't have "segments" directory
    let segments_dir = test_repo.repo_path().join("index").join("segments");
    assert!(segments_dir.is_dir());
    fs::remove_dir_all(&segments_dir).unwrap();

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    assert!(index_has_id(repo.index(), commit_a.id()));
}

#[test]
fn test_reindex_corrupt_segment_files() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    assert!(index_has_id(repo.index(), commit_a.id()));

    // Corrupt the index files
    let segments_dir = test_repo.repo_path().join("index").join("segments");
    for entry in segments_dir.read_dir().unwrap() {
        let entry = entry.unwrap();
        // u32: file format version
        // u32: parent segment file name length (0 means root)
        // u32: number of local commit entries
        // u32: number of local change ids
        // u32: number of overflow parent entries
        // u32: number of overflow change id positions
        fs::write(entry.path(), b"\0".repeat(24)).unwrap();
    }

    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path());
    assert!(index_has_id(repo.index(), commit_a.id()));
}

#[test]
fn test_reindex_from_merged_operation() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // The following operation log:
    // x (add head, index will be missing)
    // x (add head, index will be missing)
    // |\
    // o o (remove head)
    // o o (add head)
    // |/
    // o
    let mut txs = Vec::new();
    for _ in 0..2 {
        let mut tx = repo.start_transaction();
        let commit = write_random_commit(tx.repo_mut());
        let repo = tx.commit("test").unwrap();
        let mut tx = repo.start_transaction();
        tx.repo_mut().remove_head(commit.id());
        txs.push(tx);
    }
    let repo = commit_transactions(txs);
    let mut op_ids_to_delete = Vec::new();
    op_ids_to_delete.push(repo.op_id());
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    op_ids_to_delete.push(repo.op_id());
    let operation_to_reload = repo.operation();

    // Sanity check before corrupting the index store
    let index = as_readonly_index(&repo);
    assert_eq!(index.num_commits(), 4);

    let op_links_dir = test_repo.repo_path().join("index").join("op_links");
    let legacy_operations_dir = test_repo.repo_path().join("index").join("operations");
    for &op_id in &op_ids_to_delete {
        fs::remove_file(op_links_dir.join(op_id.hex())).unwrap();
        fs::remove_file(legacy_operations_dir.join(op_id.hex())).unwrap();
    }

    // When re-indexing, one of the merge parent operations will be selected as
    // the parent index segment. The commits in the other side should still be
    // reachable.
    let repo = repo.reload_at(operation_to_reload).unwrap();
    let index = as_readonly_index(&repo);
    assert_eq!(index.num_commits(), 4);
}

#[test]
fn test_reindex_missing_commit() {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init();
    let test_env = &test_repo.env;
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let missing_commit = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    let bad_op_id = repo.op_id();

    let mut tx = repo.start_transaction();
    tx.repo_mut().remove_head(missing_commit.id());
    let repo = tx.commit("test").unwrap();

    // Remove historical head commit to simulate bad GC.
    let test_backend: &TestBackend = repo.store().backend_impl().unwrap();
    test_backend.remove_commit_unchecked(missing_commit.id());
    let repo = test_env.load_repo_at_head(&settings, test_repo.repo_path()); // discard cache
    assert!(repo.store().get_commit(missing_commit.id()).is_err());

    // Reindexing error should include the operation id where the commit
    // couldn't be found.
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();
    default_index_store.reinit().unwrap();
    let err = default_index_store
        .build_index_at_operation(repo.operation(), repo.store())
        .block_on()
        .unwrap_err();
    assert_matches!(err, DefaultIndexStoreError::IndexCommits { op_id, .. } if op_id == *bad_op_id);
}

/// Test that .jj/repo/index/type is created when the repo is created.
#[test]
fn test_index_store_type() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    assert_eq!(as_readonly_index(repo).num_commits(), 1);
    let index_store_type_path = test_repo.repo_path().join("index").join("type");
    assert_eq!(
        std::fs::read_to_string(index_store_type_path).unwrap(),
        "default"
    );
}

#[test]
fn test_read_legacy_operation_link_file() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Delete new operation link files and directory
    let op_links_dir = test_repo.repo_path().join("index").join("op_links");
    fs::remove_dir_all(&op_links_dir).unwrap();

    // Reload repo and index
    let repo = repo.reload_at(repo.operation()).unwrap();
    repo.readonly_index();
    // Existing index should still be readable, so new operation link file won't
    // be created
    assert!(!op_links_dir.join(repo.op_id().hex()).exists());

    // New operation link file and directory can be created
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    assert!(op_links_dir.join(repo.op_id().hex()).exists());
}

#[test]
fn test_changed_path_segments() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();

    // Changed-path index should be disabled by default
    let segments_dir = test_repo.repo_path().join("index").join("changed_paths");
    let count_segment_files = || {
        let entries = segments_dir.read_dir().unwrap();
        entries.process_results(|entries| entries.count()).unwrap()
    };
    assert_eq!(count_segment_files(), 0);
    let stats = as_readonly_index(repo).stats();
    assert_eq!(stats.changed_path_commits_range, None);
    assert_eq!(stats.changed_path_levels.len(), 0);

    let repo = enable_changed_path_index(repo);
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.changed_path_commits_range, Some(1..1));
    assert_eq!(stats.changed_path_levels.len(), 0);

    let tree1 = create_tree(&repo, &[(repo_path("a"), "")]);
    let tree2 = create_tree(&repo, &[(repo_path("a"), ""), (repo_path("b"), "")]);

    // Add new commit with changed-path index enabled
    let mut tx = repo.start_transaction();
    let commit1 = tx
        .repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree1)
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(count_segment_files(), 1);
    assert_eq!(stats.changed_path_commits_range, Some(1..2));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 1);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 1);
    assert_eq!(stats.changed_path_levels[0].num_paths, 1);
    assert_eq!(collect_changed_paths(&repo, root_commit_id), None);
    assert_eq!(
        collect_changed_paths(&repo, commit1.id()),
        Some(vec![repo_path_buf("a")])
    );

    // Add one more commit, segment files should be squashed
    let mut tx = repo.start_transaction();
    let commit2 = tx
        .repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree2)
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(count_segment_files(), 2);
    assert_eq!(stats.changed_path_commits_range, Some(1..3));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 2);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 3);
    assert_eq!(stats.changed_path_levels[0].num_paths, 2);
    assert_eq!(collect_changed_paths(&repo, root_commit_id), None);
    assert_eq!(
        collect_changed_paths(&repo, commit1.id()),
        Some(vec![repo_path_buf("a")])
    );
    assert_eq!(
        collect_changed_paths(&repo, commit2.id()),
        Some(vec![repo_path_buf("a"), repo_path_buf("b")])
    );
}

#[test]
fn test_build_changed_path_segments() {
    let test_repo = TestRepo::init();
    let repo = test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();

    let mut tx = repo.start_transaction();
    for i in 1..10 {
        let tree = create_tree(&repo, &[(repo_path(&i.to_string()), "")]);
        tx.repo_mut()
            .new_commit(vec![root_commit_id.clone()], tree)
            .write()
            .unwrap();
    }
    let repo = tx.commit("test").unwrap();

    // Index the last 4 commits
    default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), 4)
        .block_on()
        .unwrap();
    let repo = repo.reload_at(repo.operation()).unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.changed_path_commits_range, Some(6..10));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 4);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 4);
    assert_eq!(stats.changed_path_levels[0].num_paths, 4);

    // Index remainders
    default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), u32::MAX)
        .block_on()
        .unwrap();
    let repo = repo.reload_at(repo.operation()).unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.changed_path_commits_range, Some(0..10));
    assert_eq!(stats.changed_path_levels.len(), 2);
    assert_eq!(stats.changed_path_levels[0].num_commits, 6);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 5);
    assert_eq!(stats.changed_path_levels[0].num_paths, 5);
    assert_eq!(stats.changed_path_levels[1].num_commits, 4);
    assert_eq!(stats.changed_path_levels[1].num_changed_paths, 4);
    assert_eq!(stats.changed_path_levels[1].num_paths, 4);
}

#[test]
fn test_build_changed_path_segments_partially_enabled() {
    let test_repo = TestRepo::init();
    let repo = test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();
    let default_index_store: &DefaultIndexStore = repo.index_store().downcast_ref().unwrap();

    // Partially enable index by merging two operations
    let tx1 = {
        let mut tx = repo.start_transaction();
        for i in 1..5 {
            let tree = create_tree(&repo, &[(repo_path(&i.to_string()), "")]);
            tx.repo_mut()
                .new_commit(vec![root_commit_id.clone()], tree)
                .write()
                .unwrap();
        }
        let repo = tx.commit("test").unwrap();
        let repo = enable_changed_path_index(&repo);
        let mut tx = repo.start_transaction();
        let tree = create_tree(&repo, &[(repo_path("5"), "")]);
        tx.repo_mut()
            .new_commit(vec![root_commit_id.clone()], tree)
            .write()
            .unwrap();
        tx
    };
    let mut tx2 = repo.start_transaction();
    for i in 6..10 {
        let tree = create_tree(&repo, &[(repo_path(&i.to_string()), "")]);
        tx2.repo_mut()
            .new_commit(vec![root_commit_id.clone()], tree)
            .write()
            .unwrap();
    }
    let repo = commit_transactions(vec![tx1, tx2]);
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.num_commits, 10);
    assert_eq!(stats.changed_path_commits_range, Some(5..6));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 1);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 1);
    assert_eq!(stats.changed_path_levels[0].num_paths, 1);

    // Index later commits from the mid point
    default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), 2)
        .block_on()
        .unwrap();
    let repo = repo.reload_at(repo.operation()).unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.changed_path_commits_range, Some(5..8));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 3);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 3);
    assert_eq!(stats.changed_path_levels[0].num_paths, 3);

    // Index later and earlier commits from the mid point
    default_index_store
        .build_changed_path_index_at_operation(repo.op_id(), repo.store(), 3)
        .block_on()
        .unwrap();
    let repo = repo.reload_at(repo.operation()).unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.changed_path_commits_range, Some(4..10));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 6);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 6);
    assert_eq!(stats.changed_path_levels[0].num_paths, 6);
}

#[test]
fn test_merge_changed_path_segments_both_enabled() {
    let test_repo = TestRepo::init();
    let repo = enable_changed_path_index(&test_repo.repo);
    let root_commit_id = repo.store().root_commit_id();

    let tree1 = create_tree(&repo, &[(repo_path("a"), "")]);
    let tree2 = create_tree(&repo, &[(repo_path("a"), ""), (repo_path("b"), "")]);
    let tree3 = create_tree(&repo, &[(repo_path("c"), ""), (repo_path("d"), "")]);

    // Add index segment that will be squashed
    let mut tx = repo.start_transaction();
    tx.repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree1)
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();

    // Merge concurrent index segments without the common base segment
    let mut tx1 = repo.start_transaction();
    tx1.repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree2)
        .write()
        .unwrap();
    let mut tx2 = repo.start_transaction();
    tx2.repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree3)
        .write()
        .unwrap();
    let repo = commit_transactions(vec![tx1, tx2]);
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.num_commits, 4);
    assert_eq!(stats.changed_path_commits_range, Some(1..4));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 3);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 5);
    assert_eq!(stats.changed_path_levels[0].num_paths, 4);
}

#[test]
fn test_merge_changed_path_segments_enabled_and_disabled() {
    let test_repo = TestRepo::init();
    let repo = test_repo.repo;
    let root_commit_id = repo.store().root_commit_id();

    let tree1 = create_tree(&repo, &[(repo_path("a"), "")]);
    let tree2 = create_tree(&repo, &[(repo_path("b"), "")]);
    let tree3 = create_tree(&repo, &[(repo_path("c"), "")]);

    // Enable changed-path index only in tx1
    let tx1 = {
        let mut tx = repo.start_transaction();
        tx.repo_mut()
            .new_commit(vec![root_commit_id.clone()], tree1)
            .write()
            .unwrap();
        let repo = tx.commit("test").unwrap();
        let repo = enable_changed_path_index(&repo);
        let mut tx = repo.start_transaction();
        tx.repo_mut()
            .new_commit(vec![root_commit_id.clone()], tree2)
            .write()
            .unwrap();
        tx
    };
    let mut tx2 = repo.start_transaction();
    tx2.repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree3)
        .write()
        .unwrap();
    let repo = commit_transactions(vec![tx1, tx2]);
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.num_commits, 4);
    assert_eq!(stats.changed_path_commits_range, Some(2..3));
    assert_eq!(stats.changed_path_levels.len(), 1);
    assert_eq!(stats.changed_path_levels[0].num_commits, 1);
    assert_eq!(stats.changed_path_levels[0].num_changed_paths, 1);
    assert_eq!(stats.changed_path_levels[0].num_paths, 1);

    // Changed paths in new commit can no longer be indexed
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();
    let stats = as_readonly_index(&repo).stats();
    assert_eq!(stats.num_commits, 5);
    assert_eq!(stats.changed_path_commits_range, Some(2..3));
}

#[test_case(false; "without changed-path index")]
#[test_case(true; "with changed-path index")]
fn test_commit_is_empty(indexed: bool) {
    let test_repo = TestRepo::init();
    let repo = if indexed {
        enable_changed_path_index(&test_repo.repo)
    } else {
        test_repo.repo
    };
    let root_commit_id = repo.store().root_commit_id();
    let root_tree = repo.store().empty_merged_tree();

    let tree2 = create_tree(&repo, &[(repo_path("a"), "")]);
    let tree3 = create_tree(&repo, &[(repo_path("b"), "")]);
    let tree4 = create_tree(&repo, &[(repo_path("a"), ""), (repo_path("b"), "")]);

    let mut tx = repo.start_transaction();
    let commit1 = tx
        .repo_mut()
        .new_commit(vec![root_commit_id.clone()], root_tree.clone())
        .write()
        .unwrap();
    let commit2 = tx
        .repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree2)
        .write()
        .unwrap();
    let commit3 = tx
        .repo_mut()
        .new_commit(vec![root_commit_id.clone()], tree3)
        .write()
        .unwrap();
    let commit4 = tx
        .repo_mut()
        .new_commit(
            vec![
                commit1.id().clone(),
                commit2.id().clone(),
                commit3.id().clone(),
            ],
            tree4.clone(),
        )
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();

    // Sanity check
    let stats = as_readonly_index(&repo).stats();
    if indexed {
        assert_eq!(stats.changed_path_commits_range, Some(1..5));
    } else {
        assert_eq!(stats.changed_path_commits_range, None);
    }

    assert!(commit1.is_empty(repo.as_ref()).unwrap());
    assert!(!commit2.is_empty(repo.as_ref()).unwrap());
    assert!(!commit3.is_empty(repo.as_ref()).unwrap());
    assert!(commit4.is_empty(repo.as_ref()).unwrap());

    assert_tree_eq!(commit1.parent_tree(repo.as_ref()).unwrap(), root_tree);
    assert_tree_eq!(commit2.parent_tree(repo.as_ref()).unwrap(), root_tree);
    assert_tree_eq!(commit3.parent_tree(repo.as_ref()).unwrap(), root_tree);
    assert_tree_eq!(commit4.parent_tree(repo.as_ref()).unwrap(), tree4);
}

#[test]
fn test_change_id_index() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();

    let root_commit = repo.store().root_commit();
    let mut commit_number = 0;
    let mut commit_with_change_id = |change_id| {
        commit_number += 1;
        tx.repo_mut()
            .new_commit(vec![root_commit.id().clone()], root_commit.tree())
            .set_change_id(ChangeId::from_hex(change_id))
            .set_description(format!("commit {commit_number}"))
            .write()
            .unwrap()
    };
    let commit_1 = commit_with_change_id("abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let commit_2 = commit_with_change_id("aaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let commit_3 = commit_with_change_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let commit_4 = commit_with_change_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let commit_5 = commit_with_change_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    let index_for_heads = |commits: &[&Commit]| {
        tx.repo()
            .mutable_index()
            .change_id_index(&mut commits.iter().map(|commit| commit.id()))
    };
    let change_id_index = index_for_heads(&[&commit_1, &commit_2, &commit_3, &commit_4, &commit_5]);
    let prefix_len = |commit: &Commit| {
        change_id_index
            .shortest_unique_prefix_len(commit.change_id())
            .unwrap()
    };
    assert_eq!(prefix_len(&root_commit), 1);
    assert_eq!(prefix_len(&commit_1), 2);
    assert_eq!(prefix_len(&commit_2), 6);
    assert_eq!(prefix_len(&commit_3), 6);
    assert_eq!(prefix_len(&commit_4), 1);
    assert_eq!(prefix_len(&commit_5), 1);
    let resolve_prefix = |prefix: &str| {
        change_id_index
            .resolve_prefix(&HexPrefix::try_from_hex(prefix).unwrap())
            .unwrap()
            .map(HashSet::from_iter)
    };
    // Ambiguous matches
    assert_eq!(resolve_prefix("a"), PrefixResolution::AmbiguousMatch);
    assert_eq!(resolve_prefix("aaaaa"), PrefixResolution::AmbiguousMatch);
    // Exactly the necessary length
    assert_eq!(
        resolve_prefix("0"),
        PrefixResolution::SingleMatch(hashset! {root_commit.id().clone()})
    );
    assert_eq!(
        resolve_prefix("aaaaaa"),
        PrefixResolution::SingleMatch(hashset! {commit_3.id().clone()})
    );
    assert_eq!(
        resolve_prefix("aaaaab"),
        PrefixResolution::SingleMatch(hashset! {commit_2.id().clone()})
    );
    assert_eq!(
        resolve_prefix("ab"),
        PrefixResolution::SingleMatch(hashset! {commit_1.id().clone()})
    );
    assert_eq!(
        resolve_prefix("b"),
        PrefixResolution::SingleMatch(hashset! {commit_4.id().clone(), commit_5.id().clone()})
    );
    // Longer than necessary
    assert_eq!(
        resolve_prefix("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        PrefixResolution::SingleMatch(hashset! {commit_3.id().clone()})
    );
    // No match
    assert_eq!(resolve_prefix("ba"), PrefixResolution::NoMatch);

    // Test with an index containing only some of the commits. The shortest
    // length doesn't have to be minimized further, but unreachable commits
    // should never be included in the resolved set.
    let change_id_index = index_for_heads(&[&commit_1, &commit_2]);
    let resolve_prefix = |prefix: &str| {
        change_id_index
            .resolve_prefix(&HexPrefix::try_from_hex(prefix).unwrap())
            .unwrap()
            .map(HashSet::from_iter)
    };
    assert_eq!(
        resolve_prefix("0"),
        PrefixResolution::SingleMatch(hashset! {root_commit.id().clone()})
    );
    assert_eq!(
        resolve_prefix("aaaaab"),
        PrefixResolution::SingleMatch(hashset! {commit_2.id().clone()})
    );
    assert_eq!(resolve_prefix("aaaaaa"), PrefixResolution::NoMatch);
    assert_eq!(resolve_prefix("a"), PrefixResolution::AmbiguousMatch);
    assert_eq!(resolve_prefix("b"), PrefixResolution::NoMatch);
}
