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
use jj_lib::evolution::walk_predecessors;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use maplit::btreemap;
use testutils::commit_transactions;
use testutils::write_random_commit;
use testutils::TestRepo;

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
        let op_id = loader.op_store().write_operation(&data).unwrap();
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

    // Predecessor commits are emitted in reverse within operation.
    let entries = collect_predecessors(&repo2, commit3.id());
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].commit, commit3);
    assert_eq!(entries[0].operation.as_ref(), Some(repo2.operation()));
    assert_eq!(
        entries[0].predecessor_ids(),
        [commit1.id().clone(), commit2.id().clone()]
    );
    assert_eq!(entries[1].commit, commit2);
    assert_eq!(entries[1].operation.as_ref(), Some(repo1.operation()));
    assert_eq!(entries[1].predecessor_ids(), []);
    assert_eq!(entries[2].commit, commit1);
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
    // 3 4  : op1
    // | |  :
    // 2 |  :
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
        .rewrite_commit(&commit3)
        .set_predecessors(vec![commit3.id().clone(), commit4.id().clone()])
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
        [commit3.id().clone(), commit4.id().clone()]
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
        let op_id = loader.op_store().write_operation(&data).unwrap();
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
        let op_id = loader.op_store().write_operation(&data).unwrap();
        let op = loader.load_operation(&op_id).unwrap();
        loader.load_at(&op).unwrap()
    };
    assert_matches!(
        walk_predecessors(&repo1, slice::from_ref(commit3.id())).next(),
        Some(Err(WalkPredecessorsError::CycleDetected(_)))
    );
}
