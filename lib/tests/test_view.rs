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

use jujutsu_lib::op_store::{BranchTarget, RefTarget, WorkspaceId};
use jujutsu_lib::testutils;
use jujutsu_lib::testutils::CommitGraphBuilder;
use maplit::{btreemap, hashset};
use test_case::test_case;

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_heads_empty(use_git: bool) {
    let settings = testutils::user_settings();
    let test_repo = testutils::init_repo(&settings, use_git);
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
    let test_repo = testutils::init_repo(&settings, use_git);
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
    let test_repo = testutils::init_repo(&settings, use_git);
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
    let test_repo = testutils::init_repo(&settings, false);
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

    let repo = repo.reload_at_head();

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
    let test_repo = testutils::init_repo(&settings, false);
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
        .set_checkout(ws1_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_checkout(ws2_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_checkout(ws3_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_checkout(ws4_id.clone(), commit1.id().clone());
    initial_tx
        .mut_repo()
        .set_checkout(ws5_id.clone(), commit1.id().clone());
    let repo = initial_tx.commit();

    let mut tx1 = repo.start_transaction("test");
    tx1.mut_repo()
        .set_checkout(ws1_id.clone(), commit2.id().clone());
    tx1.mut_repo()
        .set_checkout(ws2_id.clone(), commit2.id().clone());
    tx1.mut_repo().remove_checkout(&ws4_id);
    tx1.mut_repo()
        .set_checkout(ws5_id.clone(), commit2.id().clone());
    tx1.mut_repo()
        .set_checkout(ws6_id.clone(), commit2.id().clone());
    tx1.commit();

    let mut tx2 = repo.start_transaction("test");
    tx2.mut_repo()
        .set_checkout(ws1_id.clone(), commit3.id().clone());
    tx2.mut_repo()
        .set_checkout(ws3_id.clone(), commit3.id().clone());
    tx2.mut_repo()
        .set_checkout(ws4_id.clone(), commit3.id().clone());
    tx2.mut_repo().remove_checkout(&ws5_id);
    tx2.mut_repo()
        .set_checkout(ws7_id.clone(), commit3.id().clone());
    // Make sure the end time different, assuming the clock has sub-millisecond
    // precision.
    std::thread::sleep(std::time::Duration::from_millis(1));
    tx2.commit();

    let repo = repo.reload_at_head();

    // We currently arbitrarily pick the first transaction's checkout (first by
    // transaction end time).
    assert_eq!(repo.view().get_checkout(&ws1_id), Some(commit2.id()));
    assert_eq!(repo.view().get_checkout(&ws2_id), Some(commit2.id()));
    assert_eq!(repo.view().get_checkout(&ws3_id), Some(commit3.id()));
    assert_eq!(repo.view().get_checkout(&ws4_id), None);
    assert_eq!(repo.view().get_checkout(&ws5_id), None);
    assert_eq!(repo.view().get_checkout(&ws6_id), Some(commit2.id()));
    assert_eq!(repo.view().get_checkout(&ws7_id), Some(commit3.id()));
}

#[test]
fn test_merge_views_branches() {
    // Tests merging of branches (by performing concurrent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let settings = testutils::user_settings();
    let test_repo = testutils::init_repo(&settings, false);
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

    let repo = repo.reload_at_head();
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
    let test_repo = testutils::init_repo(&settings, false);
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

    let repo = repo.reload_at_head();
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
    let test_repo = testutils::init_repo(&settings, false);
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

    let repo = repo.reload_at_head();
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
