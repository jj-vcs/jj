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

use std::slice;

use assert_matches::assert_matches;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::evolution::accumulate_predecessors;
use jj_lib::evolution::walk_predecessors;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::settings::UserSettings;
use maplit::btreemap;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::commit_transactions;
use testutils::write_random_commit;

fn collect_predecessors(repo: &ReadonlyRepo, start_commit: &CommitId) -> Vec<CommitEvolutionEntry> {
    walk_predecessors(repo, slice::from_ref(start_commit))
        .try_collect()
        .unwrap()
}

#[test]
fn test_walk_predecessors_basic() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;
    let root_commit = repo0.store().root_commit();

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    // The root commit has no associated operation because it isn't "created" at
    // the root operation.
    let entries = collect_predecessors(&repo2, root_commit.id());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].commit, root_commit);
    assert_eq!(entries[0].operation.as_ref(), None);
    assert_eq!(entries[0].predecessor_ids(), []);

    let entries = collect_predecessors(&repo2, commit1.id());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].commit, commit1);
    assert_eq!(entries[0].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[0].predecessor_ids(), []);

    let entries = collect_predecessors(&repo2, commit2.id());
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].commit, commit2);
    assert_eq!(entries[0].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(entries[0].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[1].commit, commit1);
    assert_eq!(entries[1].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[1].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_basic_legacy_op() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;
    let loader = repo0.loader();

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    // Save operation without the predecessors as old jj would do. We only need
    // to rewrite the head operation since walk_predecessors() will fall back to
    // the legacy code path immediately.
    let repo2 = {
        let mut data = repo2.operation().store_operation().clone();
        data.commit_predecessors = None;
        let op_id = loader.op_store().write_operation(&data).block_on().unwrap();
        let op = loader.load_operation(&op_id).unwrap();
        loader.load_at(&op).unwrap()
    };

    let entries = collect_predecessors(&repo2, commit2.id());
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].commit, commit2);
    assert_eq!(entries[0].operation.as_ref(), None);
    assert_eq!(entries[0].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[1].commit, commit1);
    assert_eq!(entries[1].operation.as_ref(), None);
    assert_eq!(entries[1].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_concurrent_ops() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx2 = repo1.start_transaction();
    let commit2 = tx2
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 2")
        .write()
        .unwrap();
    tx2.repo_mut().rebase_descendants().unwrap();
    let mut tx3 = repo1.start_transaction();
    let commit3 = tx3
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 3")
        .write()
        .unwrap();
    tx3.repo_mut().rebase_descendants().unwrap();
    let repo4 = commit_transactions(vec![tx2, tx3]);
    let [op2, op3] = repo4
        .operation()
        .parents()
        .map(Result::unwrap)
        .collect_array()
        .unwrap();

    let mut tx = repo4.start_transaction();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("rewritten 4")
        .write()
        .unwrap();
    let commit5 = tx
        .repo_mut()
        .rewrite_commit(&commit3)
        .set_description("rewritten 5")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo5 = tx.commit("test").unwrap();

    let entries = collect_predecessors(&repo5, commit4.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit4);
    assert_eq!(entries[0].operation.as_ref(), Some(repo5.operation()));
    assert_eq!(entries[0].predecessor_ids(), [commit2.id().clone()]);
    assert_eq!(entries[1].commit, commit2);
    assert_eq!(entries[1].operation.as_ref(), Some(&op2));
    assert_eq!(entries[1].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[2].commit, commit1);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);

    let entries = collect_predecessors(&repo5, commit5.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit5);
    assert_eq!(entries[0].operation.as_ref(), Some(repo5.operation()));
    assert_eq!(entries[0].predecessor_ids(), [commit3.id().clone()]);
    assert_eq!(entries[1].commit, commit3);
    assert_eq!(entries[1].operation.as_ref(), Some(&op3));
    assert_eq!(entries[1].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[2].commit, commit1);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_multiple_predecessors_across_ops() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = write_random_commit(tx.repo_mut());
    let repo2 = tx.commit("test").unwrap();

    let mut tx = repo2.start_transaction();
    let commit3 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_predecessors(vec![commit2.id().clone(), commit1.id().clone()])
        .set_description("rewritten")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo3 = tx.commit("test").unwrap();

    // Predecessor commits are emitted in chronological (operation) order.
    let entries = collect_predecessors(&repo3, commit3.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit3);
    assert_eq!(entries[0].operation.as_ref(), Some(repo3.operation()));
    assert_eq!(
        entries[0].predecessor_ids(),
        [commit2.id().clone(), commit1.id().clone()]
    );
    assert_eq!(entries[1].commit, commit2);
    assert_eq!(entries[1].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(entries[1].predecessor_ids(), []);
    assert_eq!(entries[2].commit, commit1);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_multiple_predecessors_within_op() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit3 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_predecessors(vec![commit1.id().clone(), commit2.id().clone()])
        .set_description("rewritten")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    let entries = collect_predecessors(&repo2, commit3.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit3);
    assert_eq!(entries[0].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(
        entries[0].predecessor_ids(),
        [commit1.id().clone(), commit2.id().clone()]
    );
    assert_eq!(entries[1].commit, commit1);
    assert_eq!(entries[1].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[1].predecessor_ids(), []);
    assert_eq!(entries[2].commit, commit2);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_transitive() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 2")
        .write()
        .unwrap();
    let commit3 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("rewritten 3")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    let entries = collect_predecessors(&repo2, commit3.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit3);
    assert_eq!(entries[0].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(entries[0].predecessor_ids(), [commit2.id().clone()]);
    assert_eq!(entries[1].commit, commit2);
    assert_eq!(entries[1].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(entries[1].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[2].commit, commit1);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_transitive_graph_order() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    // 5    : op2
    // |\
    // 4 3  : op1
    // | |  :
    // | 2  :
    // |/   :
    // 1    :

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 2")
        .write()
        .unwrap();
    let commit3 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("rewritten 3")
        .write()
        .unwrap();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 4")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit5 = tx
        .repo_mut()
        .rewrite_commit(&commit4)
        .set_predecessors(vec![commit4.id().clone(), commit3.id().clone()])
        .set_description("rewritten 5")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    let entries = collect_predecessors(&repo2, commit5.id());
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0].commit, commit5);
    assert_eq!(entries[0].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(
        entries[0].predecessor_ids(),
        [commit4.id().clone(), commit3.id().clone()]
    );
    assert_eq!(entries[1].commit, commit4);
    assert_eq!(entries[1].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[1].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[2].commit, commit3);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), [commit2.id().clone()]);
    assert_eq!(entries[3].commit, commit2);
    assert_eq!(entries[3].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[3].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[4].commit, commit1);
    assert_eq!(entries[4].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[4].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_unsimplified() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    // 3
    // |\
    // | 2
    // |/
    // 1

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let mut tx = repo1.start_transaction();
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 2")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo2 = tx.commit("test").unwrap();

    let mut tx = repo2.start_transaction();
    let commit3 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_predecessors(vec![commit1.id().clone(), commit2.id().clone()])
        .set_description("rewritten 3")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo3 = tx.commit("test").unwrap();

    let entries = collect_predecessors(&repo3, commit3.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit3);
    assert_eq!(entries[0].operation.as_ref(), Some(repo3.operation()));
    assert_eq!(
        entries[0].predecessor_ids(),
        [commit1.id().clone(), commit2.id().clone()]
    );
    assert_eq!(entries[1].commit, commit2);
    assert_eq!(entries[1].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(entries[1].predecessor_ids(), [commit1.id().clone()]);
    assert_eq!(entries[2].commit, commit1);
    assert_eq!(entries[2].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[2].predecessor_ids(), []);
}

#[test]
fn test_walk_predecessors_direct_cycle_within_op() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;
    let loader = repo0.loader();

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let repo1 = {
        let mut data = repo1.operation().store_operation().clone();
        data.commit_predecessors = Some(btreemap! {
            commit1.id().clone() => vec![commit1.id().clone()],
        });
        let op_id = loader.op_store().write_operation(&data).block_on().unwrap();
        let op = loader.load_operation(&op_id).unwrap();
        loader.load_at(&op).unwrap()
    };
    assert_matches!(
        walk_predecessors(&repo1, slice::from_ref(commit1.id())).next(),
        Some(Err(WalkPredecessorsError::CycleDetected(_)))
    );
}

#[test]
fn test_walk_predecessors_indirect_cycle_within_op() {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;
    let loader = repo0.loader();

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit(tx.repo_mut());
    let commit3 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").unwrap();

    let repo1 = {
        let mut data = repo1.operation().store_operation().clone();
        data.commit_predecessors = Some(btreemap! {
            commit1.id().clone() => vec![commit3.id().clone()],
            commit2.id().clone() => vec![commit1.id().clone()],
            commit3.id().clone() => vec![commit2.id().clone()],
        });
        let op_id = loader.op_store().write_operation(&data).block_on().unwrap();
        let op = loader.load_operation(&op_id).unwrap();
        loader.load_at(&op).unwrap()
    };
    assert_matches!(
        walk_predecessors(&repo1, slice::from_ref(commit3.id())).next(),
        Some(Err(WalkPredecessorsError::CycleDetected(_)))
    );
}

#[test]
fn test_accumulate_predecessors() {
    // Stabilize commit IDs
    let mut config = testutils::base_user_config();
    let mut layer = ConfigLayer::empty(ConfigSource::User);
    layer
        .set_value("debug.commit-timestamp", "2001-02-03T04:05:06+07:00")
        .unwrap();
    config.add_layer(layer);
    let settings = UserSettings::from_config(config).unwrap();

    let test_repo = TestRepo::init_with_settings(&settings);
    let repo_0 = test_repo.repo;

    fn new_commit(repo: &mut MutableRepo, desc: &str) -> Commit {
        repo.new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .set_description(desc)
        .write()
        .unwrap()
    }

    fn rewrite_commit(repo: &mut MutableRepo, predecessors: &[&Commit], desc: &str) -> Commit {
        repo.rewrite_commit(predecessors[0])
            .set_predecessors(predecessors.iter().map(|c| c.id().clone()).collect())
            .set_description(desc)
            .write()
            .unwrap()
    }

    // Set up operation graph:
    //
    //     {commit: predecessors}
    //   D {d1: [a1], d2: [a2]}
    // C | {c1: [b1], c2: [b2, a3], c3: [c2]}
    // B | {b1: [a1], b2: [a2, a3]}
    // |/
    // A   {a1: [], a2: [], a3: []}
    // 0

    let mut tx = repo_0.start_transaction();
    let commit_a1 = new_commit(tx.repo_mut(), "a1");
    let commit_a2 = new_commit(tx.repo_mut(), "a2");
    let commit_a3 = new_commit(tx.repo_mut(), "a3");
    let repo_a = tx.commit("a").unwrap();

    let mut tx = repo_a.start_transaction();
    let commit_b1 = rewrite_commit(tx.repo_mut(), &[&commit_a1], "b1");
    let commit_b2 = rewrite_commit(tx.repo_mut(), &[&commit_a2, &commit_a3], "b2");
    tx.repo_mut().rebase_descendants().unwrap();
    let repo_b = tx.commit("b").unwrap();

    let mut tx = repo_b.start_transaction();
    let commit_c1 = rewrite_commit(tx.repo_mut(), &[&commit_b1], "c1");
    let commit_c2 = rewrite_commit(tx.repo_mut(), &[&commit_b2, &commit_a3], "c2");
    let commit_c3 = rewrite_commit(tx.repo_mut(), &[&commit_c2], "c3");
    tx.repo_mut().rebase_descendants().unwrap();
    let repo_c = tx.commit("c").unwrap();

    let mut tx = repo_a.start_transaction();
    let commit_d1 = rewrite_commit(tx.repo_mut(), &[&commit_a1], "d1");
    let commit_d2 = rewrite_commit(tx.repo_mut(), &[&commit_a2], "d2");
    tx.repo_mut().rebase_descendants().unwrap();
    let repo_d = tx.commit("d").unwrap();

    // Empty old/new ops
    let predecessors = accumulate_predecessors(&[], slice::from_ref(repo_c.operation())).unwrap();
    assert!(predecessors.is_empty());
    let predecessors = accumulate_predecessors(slice::from_ref(repo_c.operation()), &[]).unwrap();
    assert!(predecessors.is_empty());

    // Empty range
    let predecessors = accumulate_predecessors(
        slice::from_ref(repo_c.operation()),
        slice::from_ref(repo_c.operation()),
    )
    .unwrap();
    assert!(predecessors.is_empty());

    // Single forward operation
    let predecessors = accumulate_predecessors(
        slice::from_ref(repo_c.operation()),
        slice::from_ref(repo_b.operation()),
    )
    .unwrap();
    assert_eq!(
        predecessors,
        btreemap! {
            commit_c1.id().clone() => vec![commit_b1.id().clone()],
            commit_c2.id().clone() => vec![commit_b2.id().clone(), commit_a3.id().clone()],
            commit_c3.id().clone() => vec![commit_b2.id().clone(), commit_a3.id().clone()],
        }
    );

    // Multiple forward operations
    let predecessors = accumulate_predecessors(
        slice::from_ref(repo_c.operation()),
        slice::from_ref(repo_a.operation()),
    )
    .unwrap();
    assert_eq!(
        predecessors,
        btreemap! {
            commit_b1.id().clone() => vec![commit_a1.id().clone()],
            commit_b2.id().clone() => vec![commit_a2.id().clone(), commit_a3.id().clone()],
            commit_c1.id().clone() => vec![commit_a1.id().clone()],
            commit_c2.id().clone() => vec![commit_a2.id().clone(), commit_a3.id().clone()],
            commit_c3.id().clone() => vec![commit_a2.id().clone(), commit_a3.id().clone()],
        }
    );

    // Multiple reverse operations
    let predecessors = accumulate_predecessors(
        slice::from_ref(repo_a.operation()),
        slice::from_ref(repo_c.operation()),
    )
    .unwrap();
    assert_eq!(
        predecessors,
        btreemap! {
            commit_a1.id().clone() => vec![commit_c1.id().clone()],
            commit_a2.id().clone() => vec![commit_c3.id().clone()],
            commit_a3.id().clone() => vec![commit_c3.id().clone()],
            commit_b1.id().clone() => vec![commit_c1.id().clone()],
            commit_b2.id().clone() => vec![commit_c3.id().clone()],
            commit_c2.id().clone() => vec![commit_c3.id().clone()],
        }
    );

    // Sibling operations
    let predecessors = accumulate_predecessors(
        slice::from_ref(repo_d.operation()),
        slice::from_ref(repo_c.operation()),
    )
    .unwrap();
    assert_eq!(
        predecessors,
        btreemap! {
            commit_a1.id().clone() => vec![commit_c1.id().clone()],
            commit_a2.id().clone() => vec![commit_c3.id().clone()],
            commit_b1.id().clone() => vec![commit_c1.id().clone()],
            commit_b2.id().clone() => vec![commit_c3.id().clone()],
            commit_c2.id().clone() => vec![commit_c3.id().clone()],
            commit_d1.id().clone() => vec![commit_c1.id().clone()],
            commit_d2.id().clone() => vec![commit_c3.id().clone()],
        }
    );
}
