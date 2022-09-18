// Copyright 2020 Google LLC
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

use std::sync::Arc;

use jujutsu_lib::commit_builder::CommitBuilder;
use jujutsu_lib::op_store::{BranchTarget, RefTarget, WorkspaceId};
use jujutsu_lib::repo::ReadonlyRepo;
use jujutsu_lib::settings::UserSettings;
use jujutsu_lib::testutils;
use jujutsu_lib::testutils::{CommitGraphBuilder, TestRepo};
use jujutsu_lib::transaction::Transaction;
use maplit::{btreemap, hashset};
use test_case::test_case;

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_heads_empty(use_git: bool) {
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    assert_eq!(
        *repo.view().heads(),
        hashset! {repo.store().root_commit_id().clone()}
    );
    assert_eq!(
        *repo.view().public_heads(),
        hashset! {repo.store().root_commit_id().clone()}
    );
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_heads_fork(use_git: bool) {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction("test");

    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let initial = graph_builder.initial_commit();
    let child1 = graph_builder.commit_with_parents(&[&initial]);
    let child2 = graph_builder.commit_with_parents(&[&initial]);
    let repo = tx.commit();

    assert_eq!(
        *repo.view().heads(),
        hashset! {
            child1.id().clone(),
            child2.id().clone(),
        }
    );
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_heads_merge(use_git: bool) {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction("test");

    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let initial = graph_builder.initial_commit();
    let child1 = graph_builder.commit_with_parents(&[&initial]);
    let child2 = graph_builder.commit_with_parents(&[&initial]);
    let merge = graph_builder.commit_with_parents(&[&child1, &child2]);
    let repo = tx.commit();

    assert_eq!(*repo.view().heads(), hashset! {merge.id().clone()});
}

#[test]
fn test_merge_views_heads() {
    // Tests merging of the view's heads (by performing concurrent operations).
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction("test");
    let mut_repo = tx.mut_repo();
    let head_unchanged = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let head_remove_tx1 = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let head_remove_tx2 = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let public_head_unchanged =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.add_public_head(&public_head_unchanged);
    let public_head_remove_tx1 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.add_public_head(&public_head_remove_tx1);
    let public_head_remove_tx2 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.add_public_head(&public_head_remove_tx2);
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    tx1.mut_repo().remove_head(head_remove_tx1.id());
    tx1.mut_repo()
        .remove_public_head(public_head_remove_tx1.id());
    let head_add_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    let public_head_add_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo().add_public_head(&public_head_add_tx1);
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    tx2.mut_repo().remove_head(head_remove_tx2.id());
    tx2.mut_repo()
        .remove_public_head(public_head_remove_tx2.id());
    let head_add_tx2 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx2.mut_repo());
    let public_head_add_tx2 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx2.mut_repo());
    tx2.mut_repo().add_public_head(&public_head_add_tx2);
    tx2.commit();

    let repo = repo.reload_at_head(&settings).unwrap();

    let expected_heads = hashset! {
        head_unchanged.id().clone(),
        head_add_tx1.id().clone(),
        head_add_tx2.id().clone(),
        public_head_unchanged.id().clone(),
        public_head_remove_tx1.id().clone(),
        public_head_remove_tx2.id().clone(),
        public_head_add_tx1.id().clone(),
        public_head_add_tx2.id().clone(),
    };
    assert_eq!(repo.view().heads(), &expected_heads);

    let expected_public_heads = hashset! {
        public_head_unchanged.id().clone(),
        public_head_add_tx1.id().clone(),
        public_head_add_tx2.id().clone(),
    };
    assert_eq!(repo.view().public_heads(), &expected_public_heads);
}

#[test]
fn test_merge_views_checkout() {
    // Tests merging of the view's checkout (by performing concurrent operations).
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);
    let repo = &test_repo.repo;

    // Workspace 1 gets updated in both transactions.
    // Workspace 2 gets updated only in tx1.
    // Workspace 3 gets updated only in tx2.
    // Workspace 4 gets deleted in tx1 and modified in tx2.
    // Workspace 5 gets deleted in tx2 and modified in tx1.
    // Workspace 6 gets added in tx1.
    // Workspace 7 gets added in tx2.
    let mut initial_tx = repo.start_transaction("test");
    let commit1 =
        testutils::create_random_commit(&settings, repo).write_to_repo(initial_tx.mut_repo());
    let commit2 =
        testutils::create_random_commit(&settings, repo).write_to_repo(initial_tx.mut_repo());
    let commit3 =
        testutils::create_random_commit(&settings, repo).write_to_repo(initial_tx.mut_repo());
    let ws1_id = WorkspaceId::new("ws1".to_string());
    let ws2_id = WorkspaceId::new("ws2".to_string());
    let ws3_id = WorkspaceId::new("ws3".to_string());
    let ws4_id = WorkspaceId::new("ws4".to_string());
    let ws5_id = WorkspaceId::new("ws5".to_string());
    let ws6_id = WorkspaceId::new("ws6".to_string());
    let ws7_id = WorkspaceId::new("ws7".to_string());
    initial_tx
        .mut_repo()
        .set_wc_commit(ws1_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_wc_commit(ws2_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_wc_commit(ws3_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_wc_commit(ws4_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_wc_commit(ws5_id.clone(), commit1.id().clone());
    let repo = initial_tx.commit();

    let mut tx1 = repo.start_transaction("test");
    tx1.mut_repo()
        .set_wc_commit(ws1_id.clone(), commit2.id().clone());
    tx1.mut_repo()
        .set_wc_commit(ws2_id.clone(), commit2.id().clone());
    tx1.mut_repo().remove_wc_commit(&ws4_id);
    tx1.mut_repo()
        .set_wc_commit(ws5_id.clone(), commit2.id().clone());
    tx1.mut_repo()
        .set_wc_commit(ws6_id.clone(), commit2.id().clone());
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    tx2.mut_repo()
        .set_wc_commit(ws1_id.clone(), commit3.id().clone());
    tx2.mut_repo()
        .set_wc_commit(ws3_id.clone(), commit3.id().clone());
    tx2.mut_repo()
        .set_wc_commit(ws4_id.clone(), commit3.id().clone());
    tx2.mut_repo().remove_wc_commit(&ws5_id);
    tx2.mut_repo()
        .set_wc_commit(ws7_id.clone(), commit3.id().clone());
    // Make sure the end time different, assuming the clock has sub-millisecond
    // precision.
    std::thread::sleep(std::time::Duration::from_millis(1));
    tx2.commit();

    let repo = repo.reload_at_head(&settings).unwrap();

    // We currently arbitrarily pick the first transaction's checkout (first by
    // transaction end time).
    assert_eq!(repo.view().get_wc_commit_id(&ws1_id), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws2_id), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws3_id), Some(commit3.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws4_id), None);
    assert_eq!(repo.view().get_wc_commit_id(&ws5_id), None);
    assert_eq!(repo.view().get_wc_commit_id(&ws6_id), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws7_id), Some(commit3.id()));
}

#[test]
fn test_merge_views_branches() {
    // Tests merging of branches (by performing concurrent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction("test");
    let mut_repo = tx.mut_repo();
    let main_branch_local_tx0 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let main_branch_origin_tx0 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let main_branch_origin_tx1 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    let main_branch_alternate_tx0 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_local_branch(
        "main".to_string(),
        RefTarget::Normal(main_branch_local_tx0.id().clone()),
    );
    mut_repo.set_remote_branch(
        "main".to_string(),
        "origin".to_string(),
        RefTarget::Normal(main_branch_origin_tx0.id().clone()),
    );
    mut_repo.set_remote_branch(
        "main".to_string(),
        "alternate".to_string(),
        RefTarget::Normal(main_branch_alternate_tx0.id().clone()),
    );
    let feature_branch_local_tx0 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_git_ref(
        "feature".to_string(),
        RefTarget::Normal(feature_branch_local_tx0.id().clone()),
    );
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let main_branch_local_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo().set_local_branch(
        "main".to_string(),
        RefTarget::Normal(main_branch_local_tx1.id().clone()),
    );
    tx1.mut_repo().set_remote_branch(
        "main".to_string(),
        "origin".to_string(),
        RefTarget::Normal(main_branch_origin_tx1.id().clone()),
    );
    let feature_branch_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo().set_local_branch(
        "feature".to_string(),
        RefTarget::Normal(feature_branch_tx1.id().clone()),
    );
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    let main_branch_local_tx2 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx2.mut_repo());
    tx2.mut_repo().set_local_branch(
        "main".to_string(),
        RefTarget::Normal(main_branch_local_tx2.id().clone()),
    );
    tx2.mut_repo().set_remote_branch(
        "main".to_string(),
        "origin".to_string(),
        RefTarget::Normal(main_branch_origin_tx1.id().clone()),
    );
    tx2.commit();

    let repo = repo.reload_at_head(&settings).unwrap();
    let expected_main_branch = BranchTarget {
        local_target: Some(RefTarget::Conflict {
            removes: vec![main_branch_local_tx0.id().clone()],
            adds: vec![
                main_branch_local_tx1.id().clone(),
                main_branch_local_tx2.id().clone(),
            ],
        }),
        remote_targets: btreemap! {
            "origin".to_string() => RefTarget::Normal(main_branch_origin_tx1.id().clone()),
            "alternate".to_string() => RefTarget::Normal(main_branch_alternate_tx0.id().clone()),
        },
    };
    let expected_feature_branch = BranchTarget {
        local_target: Some(RefTarget::Normal(feature_branch_tx1.id().clone())),
        remote_targets: btreemap! {},
    };
    assert_eq!(
        repo.view().branches(),
        &btreemap! {
            "main".to_string() => expected_main_branch,
            "feature".to_string() => expected_feature_branch,
        }
    );
}

#[test]
fn test_merge_views_tags() {
    // Tests merging of tags (by performing concurrent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction("test");
    let mut_repo = tx.mut_repo();
    let v1_tx0 = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_tag("v1.0".to_string(), RefTarget::Normal(v1_tx0.id().clone()));
    let v2_tx0 = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_tag("v2.0".to_string(), RefTarget::Normal(v2_tx0.id().clone()));
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let v1_tx1 = testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo()
        .set_tag("v1.0".to_string(), RefTarget::Normal(v1_tx1.id().clone()));
    let v2_tx1 = testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo()
        .set_tag("v2.0".to_string(), RefTarget::Normal(v2_tx1.id().clone()));
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    let v1_tx2 = testutils::create_random_commit(&settings, &repo).write_to_repo(tx2.mut_repo());
    tx2.mut_repo()
        .set_tag("v1.0".to_string(), RefTarget::Normal(v1_tx2.id().clone()));
    tx2.commit();

    let repo = repo.reload_at_head(&settings).unwrap();
    let expected_v1 = RefTarget::Conflict {
        removes: vec![v1_tx0.id().clone()],
        adds: vec![v1_tx1.id().clone(), v1_tx2.id().clone()],
    };
    let expected_v2 = RefTarget::Normal(v2_tx1.id().clone());
    assert_eq!(
        repo.view().tags(),
        &btreemap! {
            "v1.0".to_string() => expected_v1,
            "v2.0".to_string() => expected_v2,
        }
    );
}

#[test]
fn test_merge_views_git_refs() {
    // Tests merging of git refs (by performing concurrent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction("test");
    let mut_repo = tx.mut_repo();
    let main_branch_tx0 = testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_git_ref(
        "refs/heads/main".to_string(),
        RefTarget::Normal(main_branch_tx0.id().clone()),
    );
    let feature_branch_tx0 =
        testutils::create_random_commit(&settings, repo).write_to_repo(mut_repo);
    mut_repo.set_git_ref(
        "refs/heads/feature".to_string(),
        RefTarget::Normal(feature_branch_tx0.id().clone()),
    );
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let main_branch_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo().set_git_ref(
        "refs/heads/main".to_string(),
        RefTarget::Normal(main_branch_tx1.id().clone()),
    );
    let feature_branch_tx1 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx1.mut_repo());
    tx1.mut_repo().set_git_ref(
        "refs/heads/feature".to_string(),
        RefTarget::Normal(feature_branch_tx1.id().clone()),
    );
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    let main_branch_tx2 =
        testutils::create_random_commit(&settings, &repo).write_to_repo(tx2.mut_repo());
    tx2.mut_repo().set_git_ref(
        "refs/heads/main".to_string(),
        RefTarget::Normal(main_branch_tx2.id().clone()),
    );
    tx2.commit();

    let repo = repo.reload_at_head(&settings).unwrap();
    let expected_main_branch = RefTarget::Conflict {
        removes: vec![main_branch_tx0.id().clone()],
        adds: vec![main_branch_tx1.id().clone(), main_branch_tx2.id().clone()],
    };
    let expected_feature_branch = RefTarget::Normal(feature_branch_tx1.id().clone());
    assert_eq!(
        repo.view().git_refs(),
        &btreemap! {
            "refs/heads/main".to_string() => expected_main_branch,
            "refs/heads/feature".to_string() => expected_feature_branch,
        }
    );
}

fn commit_transactions(settings: &UserSettings, txs: Vec<Transaction>) -> Arc<ReadonlyRepo> {
    let repo_loader = txs[0].base_repo().loader();
    let mut op_ids = vec![];
    for tx in txs {
        op_ids.push(tx.commit().op_id().clone());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let repo = repo_loader.load_at_head().resolve(settings).unwrap();
    // Test the setup. The assumption here is that the parent order matches the
    // order in which they were merged (which currently matches the transaction
    // commit order), so we want to know make sure they appear in a certain
    // order, so the caller can decide the order by passing them to this
    // function in a certain order.
    assert_eq!(*repo.operation().parent_ids(), op_ids);
    repo
}

#[test_case(false ; "rewrite first")]
#[test_case(true ; "add child first")]
fn test_merge_views_child_on_rewritten(child_first: bool) {
    // We start with just commit A. Operation 1 adds commit B on top. Operation 2
    // rewrites A as A2.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);

    let mut tx = test_repo.repo.start_transaction("test");
    let commit_a =
        testutils::create_random_commit(&settings, &test_repo.repo).write_to_repo(tx.mut_repo());
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let commit_b = testutils::create_random_commit(&settings, &repo)
        .set_parents(vec![commit_a.id().clone()])
        .write_to_repo(tx1.mut_repo());

    let mut tx2 = repo.start_transaction("test");
    let commit_a2 = CommitBuilder::for_rewrite_from(&settings, &commit_a)
        .set_description("A2".to_string())
        .write_to_repo(tx2.mut_repo());
    tx2.mut_repo().rebase_descendants(&settings).unwrap();

    let repo = if child_first {
        commit_transactions(&settings, vec![tx1, tx2])
    } else {
        commit_transactions(&settings, vec![tx2, tx1])
    };

    // A new B2 commit (B rebased onto A2) should be the only head.
    let heads = repo.view().heads();
    assert_eq!(heads.len(), 1);
    let b2_id = heads.iter().next().unwrap();
    let commit_b2 = repo.store().get_commit(b2_id).unwrap();
    assert_eq!(commit_b2.change_id(), commit_b.change_id());
    assert_eq!(commit_b2.parent_ids(), vec![commit_a2.id().clone()]);
}

#[test_case(false, false ; "add child on unchanged, rewrite first")]
#[test_case(false, true ; "add child on unchanged, add child first")]
#[test_case(true, false ; "add child on rewritten, rewrite first")]
#[test_case(true, true ; "add child on rewritten, add child first")]
fn test_merge_views_child_on_rewritten_divergent(on_rewritten: bool, child_first: bool) {
    // We start with divergent commits A2 and A3. Operation 1 adds commit B on top
    // of A2 or A3. Operation 2 rewrites A2 as A4. The result should be that B
    // gets rebased onto A4 if it was based on A2 before, but if it was based on
    // A3, it should remain there.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);

    let mut tx = test_repo.repo.start_transaction("test");
    let commit_a2 =
        testutils::create_random_commit(&settings, &test_repo.repo).write_to_repo(tx.mut_repo());
    let commit_a3 = testutils::create_random_commit(&settings, &test_repo.repo)
        .set_change_id(commit_a2.change_id().clone())
        .write_to_repo(tx.mut_repo());
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let parent = if on_rewritten { &commit_a2 } else { &commit_a3 };
    let commit_b = testutils::create_random_commit(&settings, &repo)
        .set_parents(vec![parent.id().clone()])
        .write_to_repo(tx1.mut_repo());

    let mut tx2 = repo.start_transaction("test");
    let commit_a4 = CommitBuilder::for_rewrite_from(&settings, &commit_a2)
        .set_description("A4".to_string())
        .write_to_repo(tx2.mut_repo());
    tx2.mut_repo().rebase_descendants(&settings).unwrap();

    let repo = if child_first {
        commit_transactions(&settings, vec![tx1, tx2])
    } else {
        commit_transactions(&settings, vec![tx2, tx1])
    };

    if on_rewritten {
        // A3 should remain as a head. The other head should be B2 (B rebased onto A4).
        let mut heads = repo.view().heads().clone();
        assert_eq!(heads.len(), 2);
        assert!(heads.remove(commit_a3.id()));
        let b2_id = heads.iter().next().unwrap();
        let commit_b2 = repo.store().get_commit(b2_id).unwrap();
        assert_eq!(commit_b2.change_id(), commit_b.change_id());
        assert_eq!(commit_b2.parent_ids(), vec![commit_a4.id().clone()]);
    } else {
        // No rebases should happen, so B and A4 should be the heads.
        let mut heads = repo.view().heads().clone();
        assert_eq!(heads.len(), 2);
        assert!(heads.remove(commit_b.id()));
        assert!(heads.remove(commit_a4.id()));
    }
}

#[test_case(false ; "abandon first")]
#[test_case(true ; "add child first")]
fn test_merge_views_child_on_abandoned(child_first: bool) {
    // We start with commit B on top of commit A. Operation 1 adds commit C on top.
    // Operation 2 abandons B.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(false);

    let mut tx = test_repo.repo.start_transaction("test");
    let commit_a =
        testutils::create_random_commit(&settings, &test_repo.repo).write_to_repo(tx.mut_repo());
    let commit_b = testutils::create_random_commit(&settings, &test_repo.repo)
        .set_parents(vec![commit_a.id().clone()])
        .write_to_repo(tx.mut_repo());
    let repo = tx.commit();

    let mut tx1 = repo.start_transaction("test");
    let commit_c = testutils::create_random_commit(&settings, &repo)
        .set_parents(vec![commit_b.id().clone()])
        .write_to_repo(tx1.mut_repo());

    let mut tx2 = repo.start_transaction("test");
    tx2.mut_repo()
        .record_abandoned_commit(commit_b.id().clone());
    tx2.mut_repo().rebase_descendants(&settings).unwrap();

    let repo = if child_first {
        commit_transactions(&settings, vec![tx1, tx2])
    } else {
        commit_transactions(&settings, vec![tx2, tx1])
    };

    // A new C2 commit (C rebased onto A) should be the only head.
    let heads = repo.view().heads();
    assert_eq!(heads.len(), 1);
    let id_c2 = heads.iter().next().unwrap();
    let commit_c2 = repo.store().get_commit(id_c2).unwrap();
    assert_eq!(commit_c2.change_id(), commit_c.change_id());
    assert_eq!(commit_c2.parent_ids(), vec![commit_a.id().clone()]);
}
