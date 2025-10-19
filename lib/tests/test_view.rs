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

use std::collections::BTreeMap;

use itertools::Itertools as _;
use jj_lib::op_store::LocalRemoteRefTarget;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use maplit::btreemap;
use maplit::hashset;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::commit_transactions;
use testutils::create_random_commit;
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

#[test]
fn test_heads_empty() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    assert_eq!(
        *repo.view().heads(),
        hashset! {repo.store().root_commit_id().clone()}
    );
}

#[test]
fn test_heads_fork() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction();

    let initial = write_random_commit(tx.repo_mut());
    let child1 = write_random_commit_with_parents(tx.repo_mut(), &[&initial]);
    let child2 = write_random_commit_with_parents(tx.repo_mut(), &[&initial]);
    let repo = tx.commit("test").block_on().unwrap();

    assert_eq!(
        *repo.view().heads(),
        hashset! {
            child1.id().clone(),
            child2.id().clone(),
        }
    );
}

#[test]
fn test_heads_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction();

    let initial = write_random_commit(tx.repo_mut());
    let child1 = write_random_commit_with_parents(tx.repo_mut(), &[&initial]);
    let child2 = write_random_commit_with_parents(tx.repo_mut(), &[&initial]);
    let merge = write_random_commit_with_parents(tx.repo_mut(), &[&child1, &child2]);
    let repo = tx.commit("test").block_on().unwrap();

    assert_eq!(*repo.view().heads(), hashset! {merge.id().clone()});
}

#[test]
fn test_merge_views_heads() {
    // Tests merging of the view's heads (by performing divergent operations).
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let head_unchanged = write_random_commit(mut_repo);
    let head_remove_tx1 = write_random_commit(mut_repo);
    let head_remove_tx2 = write_random_commit(mut_repo);
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    tx1.repo_mut().remove_head(head_remove_tx1.id());
    let head_add_tx1 = write_random_commit(tx1.repo_mut());

    let mut tx2 = repo.start_transaction();
    tx2.repo_mut().remove_head(head_remove_tx2.id());
    let head_add_tx2 = write_random_commit(tx2.repo_mut());

    let repo = commit_transactions(vec![tx1, tx2]);

    let expected_heads = hashset! {
        head_unchanged.id().clone(),
        head_add_tx1.id().clone(),
        head_add_tx2.id().clone(),
    };
    assert_eq!(repo.view().heads(), &expected_heads);
}

#[test]
fn test_merge_views_checkout() {
    // Tests merging of the view's checkout (by performing divergent operations).
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Workspace 1 gets updated in both transactions.
    // Workspace 2 gets updated only in tx1.
    // Workspace 3 gets updated only in tx2.
    // Workspace 4 gets deleted in tx1 and modified in tx2.
    // Workspace 5 gets deleted in tx2 and modified in tx1.
    // Workspace 6 gets added in tx1.
    // Workspace 7 gets added in tx2.
    let mut initial_tx = repo.start_transaction();
    let commit1 = write_random_commit(initial_tx.repo_mut());
    let commit2 = write_random_commit(initial_tx.repo_mut());
    let commit3 = write_random_commit(initial_tx.repo_mut());
    let ws1_name = WorkspaceNameBuf::from("ws1");
    let ws2_name = WorkspaceNameBuf::from("ws2");
    let ws3_name = WorkspaceNameBuf::from("ws3");
    let ws4_name = WorkspaceNameBuf::from("ws4");
    let ws5_name = WorkspaceNameBuf::from("ws5");
    let ws6_name = WorkspaceNameBuf::from("ws6");
    let ws7_name = WorkspaceNameBuf::from("ws7");
    initial_tx
        .repo_mut()
        .set_wc_commit(ws1_name.clone(), commit1.id().clone())
        .unwrap();
    initial_tx
        .repo_mut()
        .set_wc_commit(ws2_name.clone(), commit1.id().clone())
        .unwrap();
    initial_tx
        .repo_mut()
        .set_wc_commit(ws3_name.clone(), commit1.id().clone())
        .unwrap();
    initial_tx
        .repo_mut()
        .set_wc_commit(ws4_name.clone(), commit1.id().clone())
        .unwrap();
    initial_tx
        .repo_mut()
        .set_wc_commit(ws5_name.clone(), commit1.id().clone())
        .unwrap();
    let repo = initial_tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    tx1.repo_mut()
        .set_wc_commit(ws1_name.clone(), commit2.id().clone())
        .unwrap();
    tx1.repo_mut()
        .set_wc_commit(ws2_name.clone(), commit2.id().clone())
        .unwrap();
    tx1.repo_mut().remove_wc_commit(&ws4_name).unwrap();
    tx1.repo_mut()
        .set_wc_commit(ws5_name.clone(), commit2.id().clone())
        .unwrap();
    tx1.repo_mut()
        .set_wc_commit(ws6_name.clone(), commit2.id().clone())
        .unwrap();

    let mut tx2 = repo.start_transaction();
    tx2.repo_mut()
        .set_wc_commit(ws1_name.clone(), commit3.id().clone())
        .unwrap();
    tx2.repo_mut()
        .set_wc_commit(ws3_name.clone(), commit3.id().clone())
        .unwrap();
    tx2.repo_mut()
        .set_wc_commit(ws4_name.clone(), commit3.id().clone())
        .unwrap();
    tx2.repo_mut().remove_wc_commit(&ws5_name).unwrap();
    tx2.repo_mut()
        .set_wc_commit(ws7_name.clone(), commit3.id().clone())
        .unwrap();

    let repo = commit_transactions(vec![tx1, tx2]);

    // We currently arbitrarily pick the first transaction's working-copy commit
    // (first by transaction end time).
    assert_eq!(repo.view().get_wc_commit_id(&ws1_name), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws2_name), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws3_name), Some(commit3.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws4_name), None);
    assert_eq!(repo.view().get_wc_commit_id(&ws5_name), None);
    assert_eq!(repo.view().get_wc_commit_id(&ws6_name), Some(commit2.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws7_name), Some(commit3.id()));
}

#[test]
fn test_merge_views_bookmarks() {
    // Tests merging of bookmarks (by performing concurrent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let main_bookmark_local_tx0 = write_random_commit(mut_repo);
    let main_bookmark_origin_tx0 = write_random_commit(mut_repo);
    let main_bookmark_alternate_tx0 = write_random_commit(mut_repo);
    let main_bookmark_origin_tx0_remote_ref = RemoteRef {
        target: RefTarget::normal(main_bookmark_origin_tx0.id().clone()),
        state: RemoteRefState::New,
    };
    let main_bookmark_alternate_tx0_remote_ref = RemoteRef {
        target: RefTarget::normal(main_bookmark_alternate_tx0.id().clone()),
        state: RemoteRefState::Tracked,
    };
    mut_repo.set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::normal(main_bookmark_local_tx0.id().clone()),
    );
    mut_repo.set_remote_bookmark(
        remote_symbol("main", "origin"),
        main_bookmark_origin_tx0_remote_ref,
    );
    mut_repo.set_remote_bookmark(
        remote_symbol("main", "alternate"),
        main_bookmark_alternate_tx0_remote_ref.clone(),
    );
    let feature_bookmark_local_tx0 = write_random_commit(mut_repo);
    mut_repo.set_local_bookmark_target(
        "feature".as_ref(),
        RefTarget::normal(feature_bookmark_local_tx0.id().clone()),
    );
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let main_bookmark_local_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::normal(main_bookmark_local_tx1.id().clone()),
    );
    let feature_bookmark_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_local_bookmark_target(
        "feature".as_ref(),
        RefTarget::normal(feature_bookmark_tx1.id().clone()),
    );

    let mut tx2 = repo.start_transaction();
    let main_bookmark_local_tx2 = write_random_commit(tx2.repo_mut());
    let main_bookmark_origin_tx2 = write_random_commit(tx2.repo_mut());
    let main_bookmark_origin_tx2_remote_ref = RemoteRef {
        target: RefTarget::normal(main_bookmark_origin_tx2.id().clone()),
        state: RemoteRefState::Tracked,
    };
    tx2.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::normal(main_bookmark_local_tx2.id().clone()),
    );
    tx2.repo_mut().set_remote_bookmark(
        remote_symbol("main", "origin"),
        main_bookmark_origin_tx2_remote_ref.clone(),
    );

    let repo = commit_transactions(vec![tx1, tx2]);
    let expected_main_bookmark = LocalRemoteRefTarget {
        local_target: &RefTarget::from_legacy_form(
            [main_bookmark_local_tx0.id().clone()],
            [
                main_bookmark_local_tx1.id().clone(),
                main_bookmark_local_tx2.id().clone(),
            ],
        ),
        remote_refs: vec![
            (
                "alternate".as_ref(),
                &main_bookmark_alternate_tx0_remote_ref,
            ),
            // tx1: unchanged, tx2: new -> tracking
            ("origin".as_ref(), &main_bookmark_origin_tx2_remote_ref),
        ],
    };
    let expected_feature_bookmark = LocalRemoteRefTarget {
        local_target: &RefTarget::normal(feature_bookmark_tx1.id().clone()),
        remote_refs: vec![],
    };
    assert_eq!(
        repo.view().bookmarks().collect::<BTreeMap<_, _>>(),
        btreemap! {
            "main".as_ref() => expected_main_bookmark,
            "feature".as_ref() => expected_feature_bookmark,
        }
    );
}

#[test]
fn test_merge_views_tags() {
    // Tests merging of tags (by performing divergent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let v1_tx0 = write_random_commit(mut_repo);
    mut_repo.set_local_tag_target("v1.0".as_ref(), RefTarget::normal(v1_tx0.id().clone()));
    let v2_tx0 = write_random_commit(mut_repo);
    mut_repo.set_local_tag_target("v2.0".as_ref(), RefTarget::normal(v2_tx0.id().clone()));
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let v1_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut()
        .set_local_tag_target("v1.0".as_ref(), RefTarget::normal(v1_tx1.id().clone()));
    let v2_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut()
        .set_local_tag_target("v2.0".as_ref(), RefTarget::normal(v2_tx1.id().clone()));

    let mut tx2 = repo.start_transaction();
    let v1_tx2 = write_random_commit(tx2.repo_mut());
    tx2.repo_mut()
        .set_local_tag_target("v1.0".as_ref(), RefTarget::normal(v1_tx2.id().clone()));

    let repo = commit_transactions(vec![tx1, tx2]);
    let expected_v1 = RefTarget::from_legacy_form(
        [v1_tx0.id().clone()],
        [v1_tx1.id().clone(), v1_tx2.id().clone()],
    );
    let expected_v2 = RefTarget::normal(v2_tx1.id().clone());
    assert_eq!(
        repo.view().local_tags().collect_vec(),
        vec![
            ("v1.0".as_ref(), &expected_v1),
            ("v2.0".as_ref(), &expected_v2),
        ]
    );
}

#[test]
fn test_merge_views_remote_tags() {
    // Tests merging of remote tags (by performing divergent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let v1_origin_tx0 = write_random_commit(mut_repo);
    mut_repo.set_remote_tag(
        remote_symbol("v1.0", "origin"),
        RemoteRef {
            target: RefTarget::normal(v1_origin_tx0.id().clone()),
            state: RemoteRefState::New,
        },
    );
    let v1_upstream_tx0 = write_random_commit(mut_repo);
    mut_repo.set_remote_tag(
        remote_symbol("v1.0", "upstream"),
        RemoteRef {
            target: RefTarget::normal(v1_upstream_tx0.id().clone()),
            state: RemoteRefState::Tracked,
        },
    );
    let v2_upstream_tx0 = write_random_commit(mut_repo);
    mut_repo.set_remote_tag(
        remote_symbol("v2.0", "upstream"),
        RemoteRef {
            target: RefTarget::normal(v2_upstream_tx0.id().clone()),
            state: RemoteRefState::Tracked,
        },
    );
    let repo = tx.commit("test").block_on().unwrap();

    // v1.0@origin: tx0 (new) -> tx1 (new)
    // v2.0@upstream: tx0 (tracked) -> tx1 (tracked)
    let mut tx1 = repo.start_transaction();
    let v1_origin_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_remote_tag(
        remote_symbol("v1.0", "origin"),
        RemoteRef {
            target: RefTarget::normal(v1_origin_tx1.id().clone()),
            state: RemoteRefState::New,
        },
    );
    let v2_upstream_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_remote_tag(
        remote_symbol("v2.0", "upstream"),
        RemoteRef {
            target: RefTarget::normal(v2_upstream_tx1.id().clone()),
            state: RemoteRefState::Tracked,
        },
    );

    // v1.0@origin: tx0 (new) -> tx2 (tracked)
    // v1.0@upstream: tx0 (tracked) -> tx2 (new)
    let mut tx2 = repo.start_transaction();
    let v1_origin_tx2 = write_random_commit(tx2.repo_mut());
    tx2.repo_mut().set_remote_tag(
        remote_symbol("v1.0", "origin"),
        RemoteRef {
            target: RefTarget::normal(v1_origin_tx2.id().clone()),
            state: RemoteRefState::Tracked,
        },
    );
    let v1_upstream_tx2 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_remote_tag(
        remote_symbol("v1.0", "upstream"),
        RemoteRef {
            target: RefTarget::normal(v1_upstream_tx2.id().clone()),
            state: RemoteRefState::New,
        },
    );

    let repo = commit_transactions(vec![tx1, tx2]);
    let expected_v1_origin = RemoteRef {
        target: RefTarget::from_legacy_form(
            [v1_origin_tx0.id().clone()],
            [v1_origin_tx1.id().clone(), v1_origin_tx2.id().clone()],
        ),
        state: RemoteRefState::Tracked,
    };
    let expected_v1_upstream = RemoteRef {
        target: RefTarget::normal(v1_upstream_tx2.id().clone()),
        state: RemoteRefState::New,
    };
    let expected_v2_upstream = RemoteRef {
        target: RefTarget::normal(v2_upstream_tx1.id().clone()),
        state: RemoteRefState::Tracked,
    };
    assert_eq!(
        repo.view().all_remote_tags().collect_vec(),
        vec![
            (remote_symbol("v1.0", "origin"), &expected_v1_origin),
            (remote_symbol("v1.0", "upstream"), &expected_v1_upstream),
            (remote_symbol("v2.0", "upstream"), &expected_v2_upstream),
        ]
    );
}

#[test]
fn test_merge_views_git_refs() {
    // Tests merging of git refs (by performing divergent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let main_bookmark_tx0 = write_random_commit(mut_repo);
    mut_repo.set_git_ref_target(
        "refs/heads/main".as_ref(),
        RefTarget::normal(main_bookmark_tx0.id().clone()),
    );
    let feature_bookmark_tx0 = write_random_commit(mut_repo);
    mut_repo.set_git_ref_target(
        "refs/heads/feature".as_ref(),
        RefTarget::normal(feature_bookmark_tx0.id().clone()),
    );
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let main_bookmark_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_git_ref_target(
        "refs/heads/main".as_ref(),
        RefTarget::normal(main_bookmark_tx1.id().clone()),
    );
    let feature_bookmark_tx1 = write_random_commit(tx1.repo_mut());
    tx1.repo_mut().set_git_ref_target(
        "refs/heads/feature".as_ref(),
        RefTarget::normal(feature_bookmark_tx1.id().clone()),
    );

    let mut tx2 = repo.start_transaction();
    let main_bookmark_tx2 = write_random_commit(tx2.repo_mut());
    tx2.repo_mut().set_git_ref_target(
        "refs/heads/main".as_ref(),
        RefTarget::normal(main_bookmark_tx2.id().clone()),
    );

    let repo = commit_transactions(vec![tx1, tx2]);
    let expected_main_bookmark = RefTarget::from_legacy_form(
        [main_bookmark_tx0.id().clone()],
        [
            main_bookmark_tx1.id().clone(),
            main_bookmark_tx2.id().clone(),
        ],
    );
    let expected_feature_bookmark = RefTarget::normal(feature_bookmark_tx1.id().clone());
    assert_eq!(
        repo.view().git_refs(),
        &btreemap! {
            "refs/heads/main".into() => expected_main_bookmark,
            "refs/heads/feature".into() => expected_feature_bookmark,
        }
    );
}

#[test]
fn test_merge_views_git_heads() {
    // Tests merging of git heads (by performing divergent operations). See
    // test_refs.rs for tests of merging of individual ref targets.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx0 = repo.start_transaction();
    let tx0_head = write_random_commit(tx0.repo_mut());
    tx0.repo_mut()
        .set_git_head_target(RefTarget::normal(tx0_head.id().clone()));
    let repo = tx0.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let tx1_head = write_random_commit(tx1.repo_mut());
    tx1.repo_mut()
        .set_git_head_target(RefTarget::normal(tx1_head.id().clone()));

    let mut tx2 = repo.start_transaction();
    let tx2_head = write_random_commit(tx2.repo_mut());
    tx2.repo_mut()
        .set_git_head_target(RefTarget::normal(tx2_head.id().clone()));

    let repo = commit_transactions(vec![tx1, tx2]);
    let expected_git_head = RefTarget::from_legacy_form(
        [tx0_head.id().clone()],
        [tx1_head.id().clone(), tx2_head.id().clone()],
    );
    assert_eq!(repo.view().git_head(), &expected_git_head);
}

#[test]
fn test_merge_views_divergent() {
    // We start with just commit A. Operation 1 rewrites it as A2. Operation 2
    // rewrites it as A3.
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let commit_a2 = tx1
        .repo_mut()
        .rewrite_commit(&commit_a)
        .set_description("A2")
        .write()
        .unwrap();
    tx1.repo_mut().rebase_descendants().unwrap();

    let mut tx2 = repo.start_transaction();
    let commit_a3 = tx2
        .repo_mut()
        .rewrite_commit(&commit_a)
        .set_description("A3")
        .write()
        .unwrap();
    tx2.repo_mut().rebase_descendants().unwrap();

    let repo = commit_transactions(vec![tx1, tx2]);

    // A2 and A3 should be heads.
    assert_eq!(
        *repo.view().heads(),
        hashset! {commit_a2.id().clone(), commit_a3.id().clone()}
    );
}

#[test_case(false ; "rewrite first")]
#[test_case(true ; "add child first")]
fn test_merge_views_child_on_rewritten(child_first: bool) {
    // We start with just commit A. Operation 1 adds commit B on top. Operation 2
    // rewrites A as A2.
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let commit_b = write_random_commit_with_parents(tx1.repo_mut(), &[&commit_a]);

    let mut tx2 = repo.start_transaction();
    let commit_a2 = tx2
        .repo_mut()
        .rewrite_commit(&commit_a)
        .set_description("A2")
        .write()
        .unwrap();
    tx2.repo_mut().rebase_descendants().unwrap();

    let repo = if child_first {
        commit_transactions(vec![tx1, tx2])
    } else {
        commit_transactions(vec![tx2, tx1])
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
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit_a2 = write_random_commit(tx.repo_mut());
    let commit_a3 = create_random_commit(tx.repo_mut())
        .set_change_id(commit_a2.change_id().clone())
        .write()
        .unwrap();
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let parent = if on_rewritten { &commit_a2 } else { &commit_a3 };
    let commit_b = write_random_commit_with_parents(tx1.repo_mut(), &[parent]);

    let mut tx2 = repo.start_transaction();
    let commit_a4 = tx2
        .repo_mut()
        .rewrite_commit(&commit_a2)
        .set_description("A4")
        .write()
        .unwrap();
    tx2.repo_mut().rebase_descendants().unwrap();

    let repo = if child_first {
        commit_transactions(vec![tx1, tx2])
    } else {
        commit_transactions(vec![tx2, tx1])
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
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let repo = tx.commit("test").block_on().unwrap();

    let mut tx1 = repo.start_transaction();
    let commit_c = write_random_commit_with_parents(tx1.repo_mut(), &[&commit_b]);

    let mut tx2 = repo.start_transaction();
    tx2.repo_mut().record_abandoned_commit(&commit_b);
    tx2.repo_mut().rebase_descendants().unwrap();

    let repo = if child_first {
        commit_transactions(vec![tx1, tx2])
    } else {
        commit_transactions(vec![tx2, tx1])
    };

    // A new C2 commit (C rebased onto A) should be the only head.
    let heads = repo.view().heads();
    assert_eq!(heads.len(), 1);
    let id_c2 = heads.iter().next().unwrap();
    let commit_c2 = repo.store().get_commit(id_c2).unwrap();
    assert_eq!(commit_c2.change_id(), commit_c.change_id());
    assert_eq!(commit_c2.parent_ids(), vec![commit_a.id().clone()]);
}
