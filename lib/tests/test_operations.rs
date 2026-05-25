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

use std::path::Path;
use std::slice;
use std::sync::Arc;
use std::time::SystemTime;

use assert_matches::assert_matches;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::evolution::walk_predecessors;
use jj_lib::index::Index;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::op_walk;
use jj_lib::op_walk::OpsetEvaluationError;
use jj_lib::op_walk::OpsetResolutionError;
use jj_lib::operation::Operation;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::settings::UserSettings;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn get_predecessors(repo: &ReadonlyRepo, id: &CommitId) -> Vec<CommitId> {
    let entries: Vec<_> = walk_predecessors(repo, slice::from_ref(id))
        .try_collect()
        .block_on()
        .expect("unreachable predecessors shouldn't be visited");
    let first = entries
        .first()
        .expect("specified commit should be reachable");
    first.predecessor_ids().to_vec()
}

fn list_dir(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .sorted()
        .collect()
}

fn index_has_id(index: &dyn Index, commit_id: &CommitId) -> bool {
    index.has_id(commit_id).unwrap()
}

#[test]
fn test_unpublished_operation() -> TestResult {
    // Test that the operation doesn't get published until that's requested.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let op_heads_dir = test_repo.repo_path().join("op_heads").join("heads");
    let op_id0 = repo.op_id().clone();
    assert_eq!(list_dir(&op_heads_dir), vec![repo.op_id().hex()]);

    let mut tx1 = repo.start_transaction();
    write_random_commit(tx1.repo_mut());
    let unpublished_op = tx1.write("transaction 1").block_on()?;
    let op_id1 = unpublished_op.operation().id().clone();
    assert_ne!(op_id1, op_id0);
    assert_eq!(list_dir(&op_heads_dir), vec![op_id0.hex()]);
    unpublished_op.publish().block_on()?;
    assert_eq!(list_dir(&op_heads_dir), vec![op_id1.hex()]);
    Ok(())
}

#[test]
fn test_consecutive_operations() -> TestResult {
    // Test that consecutive operations result in a single op-head on disk after
    // each operation
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let op_heads_dir = test_repo.repo_path().join("op_heads").join("heads");
    let op_id0 = repo.op_id().clone();
    assert_eq!(list_dir(&op_heads_dir), vec![repo.op_id().hex()]);

    let mut tx1 = repo.start_transaction();
    write_random_commit(tx1.repo_mut());
    let op_id1 = tx1
        .commit("transaction 1")
        .block_on()?
        .operation()
        .id()
        .clone();
    assert_ne!(op_id1, op_id0);
    assert_eq!(list_dir(&op_heads_dir), vec![op_id1.hex()]);

    let repo = repo.reload_at_head().block_on()?;
    let mut tx2 = repo.start_transaction();
    write_random_commit(tx2.repo_mut());
    let op_id2 = tx2
        .commit("transaction 2")
        .block_on()?
        .operation()
        .id()
        .clone();
    assert_ne!(op_id2, op_id0);
    assert_ne!(op_id2, op_id1);
    assert_eq!(list_dir(&op_heads_dir), vec![op_id2.hex()]);

    // Reloading the repo makes no difference (there are no conflicting operations
    // to resolve).
    let _repo = repo.reload_at_head().block_on()?;
    assert_eq!(list_dir(&op_heads_dir), vec![op_id2.hex()]);
    Ok(())
}

#[test]
fn test_concurrent_operations() -> TestResult {
    // Test that consecutive operations result in multiple op-heads on disk until
    // the repo has been reloaded (which currently happens right away).
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let op_heads_dir = test_repo.repo_path().join("op_heads").join("heads");
    let op_id0 = repo.op_id().clone();
    assert_eq!(list_dir(&op_heads_dir), vec![repo.op_id().hex()]);

    let mut tx1 = repo.start_transaction();
    write_random_commit(tx1.repo_mut());
    let op_id1 = tx1
        .commit("transaction 1")
        .block_on()?
        .operation()
        .id()
        .clone();
    assert_ne!(op_id1, op_id0);
    assert_eq!(list_dir(&op_heads_dir), vec![op_id1.hex()]);

    // After both transactions have committed, we should have two op-heads on disk,
    // since they were run in parallel.
    let mut tx2 = repo.start_transaction();
    write_random_commit(tx2.repo_mut());
    let op_id2 = tx2
        .commit("transaction 2")
        .block_on()?
        .operation()
        .id()
        .clone();
    assert_ne!(op_id2, op_id0);
    assert_ne!(op_id2, op_id1);
    let mut actual_heads_on_disk = list_dir(&op_heads_dir);
    actual_heads_on_disk.sort();
    let mut expected_heads_on_disk = vec![op_id1.hex(), op_id2.hex()];
    expected_heads_on_disk.sort();
    assert_eq!(actual_heads_on_disk, expected_heads_on_disk);

    // Reloading the repo causes the operations to be merged
    let repo = repo.reload_at_head().block_on()?;
    let merged_op_id = repo.op_id().clone();
    assert_ne!(merged_op_id, op_id0);
    assert_ne!(merged_op_id, op_id1);
    assert_ne!(merged_op_id, op_id2);
    assert_eq!(list_dir(&op_heads_dir), vec![merged_op_id.hex()]);
    Ok(())
}

fn assert_heads(repo: &dyn Repo, expected: Vec<&CommitId>) {
    let expected = expected.iter().copied().cloned().collect();
    assert_eq!(*repo.view().heads(), expected);
}

#[test]
fn test_isolation() -> TestResult {
    // Test that two concurrent transactions don't see each other's changes.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let initial = write_random_commit_with_parents(tx.repo_mut(), &[]);
    let repo = tx.commit("test").block_on()?;

    let mut tx1 = repo.start_transaction();
    let mut_repo1 = tx1.repo_mut();
    let mut tx2 = repo.start_transaction();
    let mut_repo2 = tx2.repo_mut();

    assert_heads(repo.as_ref(), vec![initial.id()]);
    assert_heads(mut_repo1, vec![initial.id()]);
    assert_heads(mut_repo2, vec![initial.id()]);

    let rewrite1 = mut_repo1
        .rewrite_commit(&initial)
        .set_description("rewrite1")
        .write_unwrap();
    mut_repo1.rebase_descendants().block_on()?;
    let rewrite2 = mut_repo2
        .rewrite_commit(&initial)
        .set_description("rewrite2")
        .write_unwrap();
    mut_repo2.rebase_descendants().block_on()?;

    // Neither transaction has committed yet, so each transaction sees its own
    // commit.
    assert_heads(repo.as_ref(), vec![initial.id()]);
    assert_heads(mut_repo1, vec![rewrite1.id()]);
    assert_heads(mut_repo2, vec![rewrite2.id()]);

    // The base repo and tx2 don't see the commits from tx1.
    tx1.commit("transaction 1").block_on()?;
    assert_heads(repo.as_ref(), vec![initial.id()]);
    assert_heads(mut_repo2, vec![rewrite2.id()]);

    // The base repo still doesn't see the commits after both transactions commit.
    tx2.commit("transaction 2").block_on()?;
    assert_heads(repo.as_ref(), vec![initial.id()]);
    // After reload, the base repo sees both rewrites.
    let repo = repo.reload_at_head().block_on()?;
    assert_heads(repo.as_ref(), vec![rewrite1.id(), rewrite2.id()]);
    Ok(())
}

#[test]
fn test_stored_commit_predecessors() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let loader = repo.loader();

    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = tx
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten")
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on()?;
    let repo = tx.commit("test").block_on()?;

    // Reload operation from disk.
    let op = loader.load_operation(repo.op_id()).block_on()?;
    assert!(op.stores_commit_predecessors());
    assert_matches!(op.predecessors_for_commit(commit1.id()), Some([]));
    assert_matches!(op.predecessors_for_commit(commit2.id()), Some([id]) if id == commit1.id());

    // Save operation without the predecessors as old jj would do.
    let mut data = op.store_operation().clone();
    data.commit_predecessors = None;
    let op_id = loader.op_store().write_operation(&data).block_on()?;
    assert_ne!(&op_id, op.id());
    let op = loader.load_operation(&op_id).block_on()?;
    assert!(!op.stores_commit_predecessors());
    Ok(())
}

#[test]
fn test_reparent_range_linear() -> TestResult {
    let test_repo = TestRepo::init();
    let repo_0 = test_repo.repo;
    let loader = repo_0.loader();
    let op_store = repo_0.op_store();

    let read_op = |id| loader.load_operation(id).block_on().unwrap();

    fn op_parents<const N: usize>(op: &Operation) -> [Operation; N] {
        let parents = op.parents().block_on().unwrap();
        parents.try_into().unwrap()
    }

    // Set up linear operation graph:
    // D
    // C
    // B
    // A
    // 0 (initial)
    let random_tx = |repo: &Arc<ReadonlyRepo>| {
        let mut tx = repo.start_transaction();
        write_random_commit(tx.repo_mut());
        tx
    };
    let repo_a = random_tx(&repo_0).commit("op A").block_on()?;
    let repo_b = random_tx(&repo_a).commit("op B").block_on()?;
    let repo_c = random_tx(&repo_b).commit("op C").block_on()?;
    let repo_d = random_tx(&repo_c).commit("op D").block_on()?;

    // Reparent B..D (=C|D) onto A:
    // D'
    // C'
    // A
    // 0 (initial)
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_b.operation()),
        slice::from_ref(repo_d.operation()),
        repo_a.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 2);
    assert_eq!(stats.unreachable_count, 1);
    let new_op_d = read_op(&stats.new_head_ids[0]);
    assert_eq!(new_op_d.metadata(), repo_d.operation().metadata());
    assert_eq!(new_op_d.view_id(), repo_d.operation().view_id());
    let [new_op_c] = op_parents(&new_op_d);
    assert_eq!(new_op_c.metadata(), repo_c.operation().metadata());
    assert_eq!(new_op_c.view_id(), repo_c.operation().view_id());
    assert_eq!(new_op_c.parent_ids(), slice::from_ref(repo_a.op_id()));

    // Reparent empty range onto A
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_d.operation()),
        slice::from_ref(repo_d.operation()),
        repo_a.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids, vec![repo_a.op_id().clone()]);
    assert_eq!(stats.rewritten_count, 0);
    assert_eq!(stats.unreachable_count, 3);
    Ok(())
}

#[test]
fn test_reparent_range_branchy() -> TestResult {
    let test_repo = TestRepo::init();
    let repo_0 = test_repo.repo;
    let loader = repo_0.loader();
    let op_store = repo_0.op_store();

    let read_op = |id| loader.load_operation(id).block_on().unwrap();

    fn op_parents<const N: usize>(op: &Operation) -> [Operation; N] {
        let parents = op.parents().block_on().unwrap();
        parents.try_into().unwrap()
    }

    // Set up branchy operation graph:
    // G
    // |\
    // | F
    // E |
    // D |
    // |/
    // C
    // B
    // A
    // 0 (initial)
    let random_tx = |repo: &Arc<ReadonlyRepo>| {
        let mut tx = repo.start_transaction();
        write_random_commit(tx.repo_mut());
        tx
    };
    let repo_a = random_tx(&repo_0).commit("op A").block_on()?;
    let repo_b = random_tx(&repo_a).commit("op B").block_on()?;
    let repo_c = random_tx(&repo_b).commit("op C").block_on()?;
    let repo_d = random_tx(&repo_c).commit("op D").block_on()?;
    let tx_e = random_tx(&repo_d);
    let tx_f = random_tx(&repo_c);
    let repo_g = testutils::commit_transactions(vec![tx_e, tx_f]);
    let [op_e, op_f] = op_parents(repo_g.operation());

    // Reparent D..G (= E|F|G) onto B:
    // G'
    // |\
    // | F'
    // E'|
    // |/
    // B
    // A
    // 0 (initial)
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_d.operation()),
        slice::from_ref(repo_g.operation()),
        repo_b.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 3);
    assert_eq!(stats.unreachable_count, 2);
    let new_op_g = read_op(&stats.new_head_ids[0]);
    assert_eq!(new_op_g.metadata(), repo_g.operation().metadata());
    assert_eq!(new_op_g.view_id(), repo_g.operation().view_id());
    let [new_op_e, new_op_f] = op_parents(&new_op_g);
    assert_eq!(new_op_e.parent_ids(), slice::from_ref(repo_b.op_id()));
    assert_eq!(new_op_f.parent_ids(), slice::from_ref(repo_b.op_id()));

    // Reparent B..G (=C|D|E|F|G) onto A:
    // G'
    // |\
    // | F'
    // E'|
    // D'|
    // |/
    // C'
    // A
    // 0 (initial)
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_b.operation()),
        slice::from_ref(repo_g.operation()),
        repo_a.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 5);
    assert_eq!(stats.unreachable_count, 1);
    let new_op_g = read_op(&stats.new_head_ids[0]);
    assert_eq!(new_op_g.metadata(), repo_g.operation().metadata());
    assert_eq!(new_op_g.view_id(), repo_g.operation().view_id());
    let [new_op_e, new_op_f] = op_parents(&new_op_g);
    let [new_op_d] = op_parents(&new_op_e);
    assert_eq!(new_op_d.parent_ids(), new_op_f.parent_ids());
    let [new_op_c] = op_parents(&new_op_d);
    assert_eq!(new_op_c.parent_ids(), slice::from_ref(repo_a.op_id()));

    // Reparent (E|F)..G (=G) onto D:
    // G'
    // D
    // C
    // B
    // A
    // 0 (initial)
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        &[op_e.clone(), op_f.clone()],
        slice::from_ref(repo_g.operation()),
        repo_d.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 1);
    assert_eq!(stats.unreachable_count, 2);
    let new_op_g = read_op(&stats.new_head_ids[0]);
    assert_eq!(new_op_g.metadata(), repo_g.operation().metadata());
    assert_eq!(new_op_g.view_id(), repo_g.operation().view_id());
    assert_eq!(new_op_g.parent_ids(), slice::from_ref(repo_d.op_id()));

    // Reparent C..F (=F) onto D (ignoring G):
    // F'
    // D
    // C
    // B
    // A
    // 0 (initial)
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_c.operation()),
        slice::from_ref(&op_f),
        repo_d.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 1);
    assert_eq!(stats.unreachable_count, 0);
    let new_op_f = read_op(&stats.new_head_ids[0]);
    assert_eq!(new_op_f.metadata(), op_f.metadata());
    assert_eq!(new_op_f.view_id(), op_f.view_id());
    assert_eq!(new_op_f.parent_ids(), slice::from_ref(repo_d.op_id()));
    Ok(())
}

#[test_case(false; "legacy commit.predecessors")]
#[test_case(true; "op.commit_predecessors")]
fn test_reparent_discarding_predecessors(op_stores_commit_predecessors: bool) -> TestResult {
    let test_repo = TestRepo::init();
    let repo_0 = test_repo.repo;
    let loader = repo_0.loader();
    let op_store = repo_0.op_store();

    let repo_at = |id: &OperationId| {
        let op = loader.load_operation(id).block_on().unwrap();
        loader.load_at(&op).block_on().unwrap()
    };
    let head_commits = |repo: &dyn Repo| {
        repo.view()
            .heads()
            .iter()
            .map(|id| repo.store().get_commit(id).unwrap())
            .collect_vec()
    };

    // Set up rewriting as follows:
    //
    //   op1     op2     op3     op4
    //   B0      B0 B1      B1
    //   |       |  |       |
    //   A0      A0 A1   A0 A1   A2
    let mut tx = repo_0.start_transaction();
    let commit_a0 = write_random_commit(tx.repo_mut());
    let commit_b0 = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a0]);
    let repo_1 = tx.commit("op1").block_on()?;

    let mut tx = repo_1.start_transaction();
    let commit_a1 = tx
        .repo_mut()
        .rewrite_commit(&commit_a0)
        .set_description("a1")
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on()?;
    let [commit_b1] = head_commits(tx.repo()).try_into().unwrap();
    tx.repo_mut().add_head(&commit_b0).block_on()?; // resurrect rewritten commits
    let repo_2 = tx.commit("op2").block_on()?;

    let mut tx = repo_2.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b0);
    tx.repo_mut().rebase_descendants().block_on()?;
    let repo_3 = tx.commit("op3").block_on()?;

    let mut tx = repo_3.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_a0);
    tx.repo_mut().record_abandoned_commit(&commit_b1);
    let commit_a2 = tx
        .repo_mut()
        .rewrite_commit(&commit_a1)
        .set_description("a2")
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on()?;
    let repo_4 = tx.commit("op4").block_on()?;

    let repo_4 = if op_stores_commit_predecessors {
        repo_4
    } else {
        // Save operation without the predecessors as old jj would do. We only
        // need to rewrite the head operation since walk_predecessors() will
        // fall back to the legacy code path immediately.
        let mut data = repo_4.operation().store_operation().clone();
        data.commit_predecessors = None;
        let op_id = op_store.write_operation(&data).block_on()?;
        repo_at(&op_id)
    };

    // Sanity check for the setup
    assert_eq!(repo_1.view().heads().len(), 1);
    assert_eq!(repo_2.view().heads().len(), 2);
    assert_eq!(repo_3.view().heads().len(), 2);
    assert_eq!(repo_4.view().heads().len(), 1);
    assert_eq!(repo_4.index().all_heads_for_gc()?.count(), 3);
    assert_eq!(
        repo_4.operation().stores_commit_predecessors(),
        op_stores_commit_predecessors
    );
    assert_eq!(
        get_predecessors(&repo_4, commit_a1.id()),
        [commit_a0.id().clone()]
    );
    assert_eq!(
        get_predecessors(&repo_4, commit_a2.id()),
        [commit_a1.id().clone()]
    );
    assert_eq!(
        get_predecessors(&repo_4, commit_b1.id()),
        [commit_b0.id().clone()]
    );

    // Abandon op1
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_1.operation()),
        slice::from_ref(repo_4.operation()),
        repo_0.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 3);
    assert_eq!(stats.unreachable_count, 1);
    let repo = repo_at(&stats.new_head_ids[0]);
    // A0 - B0 are still reachable
    assert!(index_has_id(repo.index(), commit_a0.id()));
    assert!(index_has_id(repo.index(), commit_b0.id()));
    assert_eq!(
        get_predecessors(&repo, commit_a1.id()),
        [commit_a0.id().clone()]
    );
    assert_eq!(
        get_predecessors(&repo, commit_b1.id()),
        [commit_b0.id().clone()]
    );
    assert_eq!(get_predecessors(&repo, commit_a0.id()), []);
    assert_eq!(get_predecessors(&repo, commit_b0.id()), []);

    // Abandon op1 and op2
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_2.operation()),
        slice::from_ref(repo_4.operation()),
        repo_0.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 2);
    assert_eq!(stats.unreachable_count, 2);
    let repo = repo_at(&stats.new_head_ids[0]);
    // A0 is still reachable
    assert!(index_has_id(repo.index(), commit_a0.id()));
    if op_stores_commit_predecessors {
        // B0 is no longer reachable
        assert!(!index_has_id(repo.index(), commit_b0.id()));
        // the predecessor record `A1: A0` no longer exists
        assert_eq!(get_predecessors(&repo, commit_a1.id()), []);
        // Unreachable predecessors should be excluded
        assert_eq!(get_predecessors(&repo, commit_b1.id()), []);
    } else {
        // B0 is retained because it is immediate predecessor of B1
        assert!(index_has_id(repo.index(), commit_b0.id()));
        assert_eq!(
            get_predecessors(&repo, commit_a1.id()),
            [commit_a0.id().clone()]
        );
        assert_eq!(
            get_predecessors(&repo, commit_b1.id()),
            [commit_b0.id().clone()]
        );
    }

    // Abandon op1, op2, and op3
    let stats = op_walk::reparent_range(
        op_store.as_ref(),
        slice::from_ref(repo_3.operation()),
        slice::from_ref(repo_4.operation()),
        repo_0.operation(),
    )
    .block_on()?;
    assert_eq!(stats.new_head_ids.len(), 1);
    assert_eq!(stats.rewritten_count, 1);
    assert_eq!(stats.unreachable_count, 3);
    let repo = repo_at(&stats.new_head_ids[0]);
    // A0 is no longer reachable
    assert!(!index_has_id(repo.index(), commit_a0.id()));
    // A1 is still reachable through A2
    assert!(index_has_id(repo.index(), commit_a1.id()));
    assert_eq!(
        get_predecessors(&repo, commit_a2.id()),
        [commit_a1.id().clone()]
    );
    assert_eq!(get_predecessors(&repo, commit_a1.id()), []);
    Ok(())
}

fn stable_op_id_settings() -> UserSettings {
    let mut config = testutils::base_user_config();
    config.add_layer(
        ConfigLayer::parse(
            ConfigSource::User,
            "debug.operation-timestamp = 2001-02-03T04:05:06+07:00",
        )
        .unwrap(),
    );
    UserSettings::from_config(config).unwrap()
}

#[test]
fn test_resolve_op_id() -> TestResult {
    let settings = stable_op_id_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;
    let loader = repo.loader();

    let mut operations = Vec::new();
    // The actual value of `i` doesn't matter, we just need to make sure we end
    // up with hashes with ambiguous prefixes.
    for i in (1..5).chain([10, 24]) {
        let tx = repo.start_transaction();
        let repo = tx.commit(format!("transaction {i}")).block_on()?;
        operations.push(repo.operation().clone());
    }
    // Snapshot of operation hex ids (changes on rebase; ambiguous prefix depends on
    // base)
    insta::assert_debug_snapshot!(operations.iter().map(|op| op.id().hex()).collect_vec(), @r#"
    [
        "68ed1e50d1169d6ffdcc66a975a5d4fd44a05dce62a9fbbee0c995878b8680544ee19831fda7a17d414a257410ce6f70375e1746e8d76216866d4df6509166da",
        "efe8073bf24180446a5f0ddbd2195129c01c112d06fd846c2256d207c9f1264e078107ae8df716a91ed56709aafc198fa0cc5ee8fc508204a16e1808ae2fd60a",
        "41cd4f03c95558284f5ee478e4289cde576e26ab0304672a0f379ebaff99ac754c5ed228289743f676c61443f99eb590ec835c8b5f9342f4bfbf6816121c8096",
        "9742cdef1bed927d85f55d602b9ae79ff78082441f2ab7a5fa022c7f09a443b9ff7fb0ac18e8bde6ac6ca447854c489941dfd49609ba250f8b132e2b44dfc6c5",
        "572e4444063a345ec88a2886dbb2671a3776e939249f5886b79d02a94f44f7cf3e8d5a0d290efc93d7861d7e862bca0bd57c71ec224263ae53c8c4413e081147",
        "1061688577c51257c8fa58e9c032c1a2f1a332df6ebe8bb3d030d74e193bbdc1022d0de4b9ad442812853dc6159cad5fa5e8fbf5dca312e800699bf176c61c9a",
    ]
    "#);

    let repo_loader = repo.loader();
    let resolve = |op_str: &str| op_walk::resolve_op_for_load(repo_loader, op_str).block_on();

    // Full id
    assert_eq!(resolve(&operations[0].id().hex())?, operations[0]);
    // Short id, odd length
    assert_eq!(resolve(&operations[0].id().hex()[..3])?, operations[0]);
    // Short id, even length
    assert_eq!(resolve(&operations[1].id().hex()[..2])?, operations[1]);
    // Non-existent prefix (no operation starts with '7' with current base)
    assert_matches!(
        resolve("7"),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::NoSuchOperation(_)
        ))
    );
    // Empty id
    assert_matches!(
        resolve(""),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::InvalidIdPrefix(_)
        ))
    );
    // Unknown id
    assert_matches!(
        resolve("deadbee"),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::NoSuchOperation(_)
        ))
    );
    // Virtual root id
    let root_operation = loader.root_operation().block_on();
    assert_eq!(resolve(&root_operation.id().hex()).unwrap(), root_operation);
    assert_eq!(resolve("00").unwrap(), root_operation);
    // operations[4] starts with "57"
    assert_eq!(resolve("57").unwrap(), operations[4]);
    // "0" now uniquely matches root (no operations start with "0")
    assert_eq!(resolve("0").unwrap(), root_operation);
    Ok(())
}

#[test]
fn test_resolve_current_op() -> TestResult {
    let settings = stable_op_id_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = test_repo.repo;

    assert_eq!(
        op_walk::resolve_op_with_repo(&repo, "@").block_on()?,
        *repo.operation()
    );
    Ok(())
}

#[test]
fn test_resolve_op_parents_children() -> TestResult {
    // Use monotonic timestamp to stabilize merge order of transactions
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let mut repo = &test_repo.repo;

    let mut repos = Vec::new();
    for _ in 0..3 {
        let tx = repo.start_transaction();
        repos.push(tx.commit("test").block_on()?);
        repo = repos.last().unwrap();
    }
    let operations = repos.iter().map(|repo| repo.operation()).collect_vec();

    // Parent
    let op2_id_hex = operations[2].id().hex();
    assert_eq!(
        op_walk::resolve_op_with_repo(repo, &format!("{op2_id_hex}-")).block_on()?,
        *operations[1]
    );
    assert_eq!(
        op_walk::resolve_op_with_repo(repo, &format!("{op2_id_hex}--")).block_on()?,
        *operations[0]
    );
    // "{op2_id_hex}----" is the root operation
    assert_matches!(
        op_walk::resolve_op_with_repo(repo, &format!("{op2_id_hex}-----")).block_on(),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::EmptyOperations(_)
        ))
    );

    // Child
    let op0_id_hex = operations[0].id().hex();
    assert_eq!(
        op_walk::resolve_op_with_repo(repo, &format!("{op0_id_hex}+")).block_on()?,
        *operations[1]
    );
    assert_eq!(
        op_walk::resolve_op_with_repo(repo, &format!("{op0_id_hex}++")).block_on()?,
        *operations[2]
    );
    assert_matches!(
        op_walk::resolve_op_with_repo(repo, &format!("{op0_id_hex}+++")).block_on(),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::EmptyOperations(_)
        ))
    );

    // Child of parent
    assert_eq!(
        op_walk::resolve_op_with_repo(repo, &format!("{op2_id_hex}--+")).block_on()?,
        *operations[1]
    );

    // Child at old repo: new operations shouldn't be visible
    assert_eq!(
        op_walk::resolve_op_with_repo(&repos[1], &format!("{op0_id_hex}+")).block_on()?,
        *operations[1]
    );
    assert_matches!(
        op_walk::resolve_op_with_repo(&repos[0], &format!("{op0_id_hex}+")).block_on(),
        Err(OpsetEvaluationError::OpsetResolution(
            OpsetResolutionError::EmptyOperations(_)
        ))
    );

    // Merge and fork
    let tx1 = repo.start_transaction();
    let tx2 = repo.start_transaction();
    let repo = testutils::commit_transactions(vec![tx1, tx2]);
    let parent_op_ids = repo.operation().parent_ids();

    // The subexpression that resolves to multiple operations (i.e. the accompanying
    // op ids) should be reported, not the full expression provided by the user.
    let op5_id_hex = repo.operation().id().hex();
    let parents_op_str = format!("{op5_id_hex}-");
    let error = op_walk::resolve_op_with_repo(&repo, &parents_op_str)
        .block_on()
        .unwrap_err();
    assert_eq!(
        extract_multiple_operations_error(&error).unwrap(),
        (&parents_op_str, parent_op_ids)
    );
    let grandparents_op_str = format!("{op5_id_hex}--");
    let error = op_walk::resolve_op_with_repo(&repo, &grandparents_op_str)
        .block_on()
        .unwrap_err();
    assert_eq!(
        extract_multiple_operations_error(&error).unwrap(),
        (&parents_op_str, parent_op_ids)
    );
    let children_of_parents_op_str = format!("{op5_id_hex}-+");
    let error = op_walk::resolve_op_with_repo(&repo, &children_of_parents_op_str)
        .block_on()
        .unwrap_err();
    assert_eq!(
        extract_multiple_operations_error(&error).unwrap(),
        (&parents_op_str, parent_op_ids)
    );

    let op2_id_hex = operations[2].id().hex();
    let op_str = format!("{op2_id_hex}+");
    let error = op_walk::resolve_op_with_repo(&repo, &op_str)
        .block_on()
        .unwrap_err();
    assert_eq!(
        extract_multiple_operations_error(&error).unwrap(),
        (&op_str, parent_op_ids)
    );
    Ok(())
}

#[test]
fn test_walk_ancestors() -> TestResult {
    let test_repo = TestRepo::init();
    let repo_0 = test_repo.repo;
    let loader = repo_0.loader();

    fn op_parents<const N: usize>(op: &Operation) -> [Operation; N] {
        let parents = op.parents().block_on().unwrap();
        parents.try_into().unwrap()
    }

    fn collect_ancestors(head_ops: &[Operation]) -> Vec<Operation> {
        op_walk::walk_ancestors(head_ops)
            .try_collect()
            .block_on()
            .unwrap()
    }

    fn collect_ancestors_range(head_ops: &[Operation], root_ops: &[Operation]) -> Vec<Operation> {
        op_walk::walk_ancestors_range(head_ops, root_ops)
            .try_collect()
            .block_on()
            .unwrap()
    }

    // Set up operation graph:
    // H
    // G
    // |\
    // | F
    // E |
    // D |
    // |/
    // C
    // | B
    // A |
    // |/
    // 0 (initial)
    let repo_a = repo_0.start_transaction().commit("op A").block_on()?;
    let repo_b = repo_0
        .start_transaction()
        .write("op B")
        .block_on()?
        .leave_unpublished();
    let repo_c = repo_a.start_transaction().commit("op C").block_on()?;
    let repo_d = repo_c.start_transaction().commit("op D").block_on()?;
    let tx_e = repo_d.start_transaction();
    let tx_f = repo_c.start_transaction();
    let repo_g = testutils::commit_transactions(vec![tx_e, tx_f]);
    let [op_e, op_f] = op_parents(repo_g.operation());
    let repo_h = repo_g.start_transaction().commit("op H").block_on()?;

    // At merge, parents are visited in forward order, which isn't important.
    assert_eq!(
        collect_ancestors(slice::from_ref(repo_h.operation())),
        [
            repo_h.operation().clone(),
            repo_g.operation().clone(),
            op_e.clone(),
            repo_d.operation().clone(),
            op_f.clone(),
            repo_c.operation().clone(),
            repo_a.operation().clone(),
            loader.root_operation().block_on(),
        ]
    );

    // Ancestors of multiple heads
    assert_eq!(
        collect_ancestors(&[op_f.clone(), repo_b.operation().clone()]),
        [
            op_f.clone(),
            repo_c.operation().clone(),
            repo_a.operation().clone(),
            repo_b.operation().clone(),
            loader.root_operation().block_on(),
        ]
    );

    // Exclude direct ancestor
    assert_eq!(
        collect_ancestors_range(
            slice::from_ref(repo_h.operation()),
            slice::from_ref(repo_d.operation()),
        ),
        [
            repo_h.operation().clone(),
            repo_g.operation().clone(),
            op_e.clone(),
            op_f.clone(),
        ]
    );

    // Exclude indirect ancestor
    assert_eq!(
        collect_ancestors_range(slice::from_ref(&op_e), slice::from_ref(&op_f)),
        [op_e.clone(), repo_d.operation().clone()]
    );

    // Exclude far ancestor
    assert_eq!(
        collect_ancestors_range(
            slice::from_ref(repo_h.operation()),
            slice::from_ref(repo_a.operation()),
        ),
        [
            repo_h.operation().clone(),
            repo_g.operation().clone(),
            op_e.clone(),
            repo_d.operation().clone(),
            op_f.clone(),
            repo_c.operation().clone(),
        ]
    );

    // Exclude ancestors of descendant
    assert_eq!(
        collect_ancestors_range(
            slice::from_ref(repo_g.operation()),
            slice::from_ref(repo_h.operation()),
        ),
        []
    );

    // Exclude multiple roots
    assert_eq!(
        collect_ancestors_range(
            slice::from_ref(repo_g.operation()),
            &[repo_d.operation().clone(), op_f.clone()],
        ),
        [repo_g.operation().clone(), op_e.clone()]
    );
    Ok(())
}

#[test]
fn test_gc() -> TestResult {
    let settings = stable_op_id_settings();
    let test_repo = TestRepo::init_with_settings(&settings);
    let op_dir = test_repo.repo_path().join("op_store").join("operations");
    let view_dir = test_repo.repo_path().join("op_store").join("views");
    let repo_0 = test_repo.repo;
    let op_store = repo_0.op_store();

    // Set up operation graph:
    //
    //   F
    //   E (empty)
    // D |
    // C |
    // |/
    // B
    // A
    // 0 (root)
    let empty_tx = |repo: &Arc<ReadonlyRepo>| repo.start_transaction();
    let random_tx = |repo: &Arc<ReadonlyRepo>| {
        let mut tx = repo.start_transaction();
        write_random_commit(tx.repo_mut());
        tx
    };
    let repo_a = random_tx(&repo_0).commit("op A").block_on()?;
    let repo_b = random_tx(&repo_a).commit("op B").block_on()?;
    let repo_c = random_tx(&repo_b).commit("op C").block_on()?;
    let repo_d = random_tx(&repo_c).commit("op D").block_on()?;
    let repo_e = empty_tx(&repo_b).commit("op E").block_on()?;
    let repo_f = random_tx(&repo_e).commit("op F").block_on()?;

    // Sanity check for the original state
    let mut expected_op_entries = list_dir(&op_dir);
    let mut expected_view_entries = list_dir(&view_dir);
    assert_eq!(expected_op_entries.len(), 6);
    assert_eq!(expected_view_entries.len(), 5);

    // No heads, but all kept by file modification time
    op_store.gc(&[], SystemTime::UNIX_EPOCH).block_on()?;
    assert_eq!(list_dir(&op_dir), expected_op_entries);
    assert_eq!(list_dir(&view_dir), expected_view_entries);

    // All reachable from heads
    let now = SystemTime::now();
    let head_ids = [repo_d.op_id().clone(), repo_f.op_id().clone()];
    op_store.gc(&head_ids, now).block_on()?;
    assert_eq!(list_dir(&op_dir), expected_op_entries);
    assert_eq!(list_dir(&view_dir), expected_view_entries);

    // E|F are no longer reachable, but E's view is still reachable
    op_store
        .gc(slice::from_ref(repo_d.op_id()), now)
        .block_on()?;
    expected_op_entries
        .retain(|name| *name != repo_e.op_id().hex() && *name != repo_f.op_id().hex());
    expected_view_entries.retain(|name| *name != repo_f.operation().view_id().hex());
    assert_eq!(list_dir(&op_dir), expected_op_entries);
    assert_eq!(list_dir(&view_dir), expected_view_entries);

    // B|C|D are no longer reachable
    op_store
        .gc(slice::from_ref(repo_a.op_id()), now)
        .block_on()?;
    expected_op_entries.retain(|name| {
        *name != repo_b.op_id().hex()
            && *name != repo_c.op_id().hex()
            && *name != repo_d.op_id().hex()
    });
    expected_view_entries.retain(|name| {
        *name != repo_b.operation().view_id().hex()
            && *name != repo_c.operation().view_id().hex()
            && *name != repo_d.operation().view_id().hex()
    });
    assert_eq!(list_dir(&op_dir), expected_op_entries);
    assert_eq!(list_dir(&view_dir), expected_view_entries);

    // Sanity check for the last state
    assert_eq!(expected_op_entries.len(), 1);
    assert_eq!(expected_view_entries.len(), 1);
    Ok(())
}

#[track_caller]
fn extract_multiple_operations_error(
    error: &OpsetEvaluationError,
) -> Option<(&String, &[OperationId])> {
    if let OpsetEvaluationError::OpsetResolution(OpsetResolutionError::MultipleOperations {
        expr,
        candidates,
    }) = error
    {
        Some((expr, candidates))
    } else {
        None
    }
}
