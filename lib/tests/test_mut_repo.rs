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

use jj_lib::backend::MergedTreeId;
use jj_lib::op_store::{RefTarget, WorkspaceId};
use jj_lib::repo::Repo;
use maplit::hashset;
use test_case::test_case;
use testutils::{
    assert_rebased, create_random_commit, write_random_commit, CommitGraphBuilder, TestRepo,
};

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_edit(use_git: bool) {
    // Test that MutableRepo::edit() uses the requested commit (not a new child)
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let wc_commit = write_random_commit(tx.mut_repo(), &settings);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let ws_id = WorkspaceId::default();
    tx.mut_repo().edit(ws_id.clone(), &wc_commit).unwrap();
    let repo = tx.commit();
    assert_eq!(repo.view().get_wc_commit_id(&ws_id), Some(wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout(use_git: bool) {
    // Test that MutableRepo::check_out() creates a child
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let wc_commit_parent = write_random_commit(tx.mut_repo(), &settings);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let ws_id = WorkspaceId::default();
    let wc_commit = tx
        .mut_repo()
        .check_out(ws_id.clone(), &settings, &wc_commit_parent)
        .unwrap();
    assert_eq!(wc_commit.tree_id(), wc_commit_parent.tree_id());
    assert_eq!(wc_commit.parents().len(), 1);
    assert_eq!(wc_commit.parents()[0].id(), wc_commit_parent.id());
    let repo = tx.commit();
    assert_eq!(repo.view().get_wc_commit_id(&ws_id), Some(wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout_previous_not_empty(use_git: bool) {
    // Test that MutableRepo::check_out() does not usually abandon the previous
    // commit.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let old_wc_commit = write_random_commit(mut_repo, &settings);
    let ws_id = WorkspaceId::default();
    mut_repo.edit(ws_id.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let new_wc_commit = write_random_commit(mut_repo, &settings);
    mut_repo.edit(ws_id, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants(&settings).unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout_previous_empty(use_git: bool) {
    // Test that MutableRepo::check_out() abandons the previous commit if it was
    // empty.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let old_wc_commit = mut_repo
        .new_commit(
            &settings,
            vec![repo.store().root_commit_id().clone()],
            MergedTreeId::Legacy(repo.store().empty_tree_id().clone()),
        )
        .write()
        .unwrap();
    let ws_id = WorkspaceId::default();
    mut_repo.edit(ws_id.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let new_wc_commit = write_random_commit(mut_repo, &settings);
    mut_repo.edit(ws_id, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants(&settings).unwrap();
    assert!(!mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout_previous_empty_with_description(use_git: bool) {
    // Test that MutableRepo::check_out() does not abandon the previous commit if it
    // has a non-empty description.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let old_wc_commit = mut_repo
        .new_commit(
            &settings,
            vec![repo.store().root_commit_id().clone()],
            MergedTreeId::Legacy(repo.store().empty_tree_id().clone()),
        )
        .set_description("not empty")
        .write()
        .unwrap();
    let ws_id = WorkspaceId::default();
    mut_repo.edit(ws_id.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let new_wc_commit = write_random_commit(mut_repo, &settings);
    mut_repo.edit(ws_id, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants(&settings).unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout_previous_empty_with_local_branch(use_git: bool) {
    // Test that MutableRepo::check_out() does not abandon the previous commit if it
    // is pointed by local branch.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let old_wc_commit = mut_repo
        .new_commit(
            &settings,
            vec![repo.store().root_commit_id().clone()],
            MergedTreeId::Legacy(repo.store().empty_tree_id().clone()),
        )
        .write()
        .unwrap();
    mut_repo.set_local_branch_target("b", RefTarget::normal(old_wc_commit.id().clone()));
    let ws_id = WorkspaceId::default();
    mut_repo.edit(ws_id.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let new_wc_commit = write_random_commit(mut_repo, &settings);
    mut_repo.edit(ws_id, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants(&settings).unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_checkout_previous_empty_non_head(use_git: bool) {
    // Test that MutableRepo::check_out() does not abandon the previous commit if it
    // was empty and is not a head
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let old_wc_commit = mut_repo
        .new_commit(
            &settings,
            vec![repo.store().root_commit_id().clone()],
            MergedTreeId::Legacy(repo.store().empty_tree_id().clone()),
        )
        .write()
        .unwrap();
    let old_child = mut_repo
        .new_commit(
            &settings,
            vec![old_wc_commit.id().clone()],
            old_wc_commit.tree_id().clone(),
        )
        .write()
        .unwrap();
    let ws_id = WorkspaceId::default();
    mut_repo.edit(ws_id.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let new_wc_commit = write_random_commit(mut_repo, &settings);
    mut_repo.edit(ws_id, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants(&settings).unwrap();
    assert_eq!(
        *mut_repo.view().heads(),
        hashset! {old_child.id().clone(), new_wc_commit.id().clone()}
    );
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_edit_initial(use_git: bool) {
    // Test that MutableRepo::edit() can be used on the initial working-copy commit
    // in a workspace
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let wc_commit = write_random_commit(tx.mut_repo(), &settings);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let workspace_id = WorkspaceId::new("new-workspace".to_string());
    tx.mut_repo()
        .edit(workspace_id.clone(), &wc_commit)
        .unwrap();
    let repo = tx.commit();
    assert_eq!(
        repo.view().get_wc_commit_id(&workspace_id),
        Some(wc_commit.id())
    );
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_add_head_success(use_git: bool) {
    // Test that MutableRepo::add_head() adds the head, and that it's still there
    // after commit. It should also be indexed.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    // Create a commit outside of the repo by using a temporary transaction. Then
    // add that as a head.
    let mut tx = repo.start_transaction(&settings, "test");
    let new_commit = write_random_commit(tx.mut_repo(), &settings);
    drop(tx);

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    assert!(!mut_repo.view().heads().contains(new_commit.id()));
    assert!(!mut_repo.index().has_id(new_commit.id()));
    mut_repo.add_head(&new_commit);
    assert!(mut_repo.view().heads().contains(new_commit.id()));
    assert!(mut_repo.index().has_id(new_commit.id()));
    let repo = tx.commit();
    assert!(repo.view().heads().contains(new_commit.id()));
    assert!(repo.index().has_id(new_commit.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_add_head_ancestor(use_git: bool) {
    // Test that MutableRepo::add_head() does not add a head if it's an ancestor of
    // an existing head.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let repo = tx.commit();

    assert_eq!(repo.view().heads(), &hashset! {commit3.id().clone()});
    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    mut_repo.add_head(&commit1);
    assert_eq!(repo.view().heads(), &hashset! {commit3.id().clone()});
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_add_head_not_immediate_child(use_git: bool) {
    // Test that MutableRepo::add_head() can be used for adding a head that is not
    // an immediate child of a current head.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let initial = write_random_commit(tx.mut_repo(), &settings);
    let repo = tx.commit();

    // Create some commits outside of the repo by using a temporary transaction.
    // Then add one of them as a head.
    let mut tx = repo.start_transaction(&settings, "test");
    let rewritten = create_random_commit(tx.mut_repo(), &settings)
        .set_change_id(initial.change_id().clone())
        .set_predecessors(vec![initial.id().clone()])
        .write()
        .unwrap();
    let child = create_random_commit(tx.mut_repo(), &settings)
        .set_parents(vec![rewritten.id().clone()])
        .write()
        .unwrap();
    drop(tx);

    assert_eq!(repo.view().heads(), &hashset! {initial.id().clone()});
    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    mut_repo.add_head(&child);
    assert_eq!(
        mut_repo.view().heads(),
        &hashset! {initial.id().clone(), child.id().clone()}
    );
    assert!(mut_repo.index().has_id(initial.id()));
    assert!(mut_repo.index().has_id(rewritten.id()));
    assert!(mut_repo.index().has_id(child.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_remove_head(use_git: bool) {
    // Test that MutableRepo::remove_head() removes the head, and that it's still
    // removed after commit. It should remain in the index, since we otherwise would
    // have to reindex everything.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    assert!(mut_repo.view().heads().contains(commit3.id()));
    mut_repo.remove_head(commit3.id());
    let heads = mut_repo.view().heads().clone();
    assert!(!heads.contains(commit3.id()));
    assert!(!heads.contains(commit2.id()));
    assert!(!heads.contains(commit1.id()));
    assert!(mut_repo.index().has_id(commit1.id()));
    assert!(mut_repo.index().has_id(commit2.id()));
    assert!(mut_repo.index().has_id(commit3.id()));
    let repo = tx.commit();
    let heads = repo.view().heads().clone();
    assert!(!heads.contains(commit3.id()));
    assert!(!heads.contains(commit2.id()));
    assert!(!heads.contains(commit1.id()));
    assert!(repo.index().has_id(commit1.id()));
    assert!(repo.index().has_id(commit2.id()));
    assert!(repo.index().has_id(commit3.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_add_public_head(use_git: bool) {
    // Test that MutableRepo::add_public_head() adds the head, and that it's still
    // there after commit.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let commit1 = write_random_commit(tx.mut_repo(), &settings);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    assert!(!mut_repo.view().public_heads().contains(commit1.id()));
    mut_repo.add_public_head(&commit1);
    assert!(mut_repo.view().public_heads().contains(commit1.id()));
    let repo = tx.commit();
    assert!(repo.view().public_heads().contains(commit1.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_add_public_head_ancestor(use_git: bool) {
    // Test that MutableRepo::add_public_head() does not add a public head if it's
    // an ancestor of an existing public head.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    tx.mut_repo().add_public_head(&commit2);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    assert!(!mut_repo.view().public_heads().contains(commit1.id()));
    mut_repo.add_public_head(&commit1);
    assert!(!mut_repo.view().public_heads().contains(commit1.id()));
    let repo = tx.commit();
    assert!(!repo.view().public_heads().contains(commit1.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_remove_public_head(use_git: bool) {
    // Test that MutableRepo::remove_public_head() removes the head, and that it's
    // still removed after commit.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let commit1 = write_random_commit(mut_repo, &settings);
    mut_repo.add_public_head(&commit1);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    assert!(mut_repo.view().public_heads().contains(commit1.id()));
    mut_repo.remove_public_head(commit1.id());
    assert!(!mut_repo.view().public_heads().contains(commit1.id()));
    let repo = tx.commit();
    assert!(!repo.view().public_heads().contains(commit1.id()));
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_has_changed(use_git: bool) {
    // Test that MutableRepo::has_changed() reports changes iff the view has changed
    // (e.g. not after setting a branch to point to where it was already
    // pointing).
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let commit1 = write_random_commit(mut_repo, &settings);
    let commit2 = write_random_commit(mut_repo, &settings);
    mut_repo.remove_head(commit2.id());
    mut_repo.add_public_head(&commit1);
    let ws_id = WorkspaceId::default();
    mut_repo
        .set_wc_commit(ws_id.clone(), commit1.id().clone())
        .unwrap();
    mut_repo.set_local_branch_target("main", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_remote_branch_target("main", "origin", RefTarget::normal(commit1.id().clone()));
    let repo = tx.commit();
    // Test the setup
    assert_eq!(repo.view().heads(), &hashset! {commit1.id().clone()});
    assert_eq!(repo.view().public_heads(), &hashset! {commit1.id().clone()});

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();

    mut_repo.add_public_head(&commit1);
    mut_repo.add_head(&commit1);
    mut_repo
        .set_wc_commit(ws_id.clone(), commit1.id().clone())
        .unwrap();
    mut_repo.set_local_branch_target("main", RefTarget::normal(commit1.id().clone()));
    mut_repo.set_remote_branch_target("main", "origin", RefTarget::normal(commit1.id().clone()));
    assert!(!mut_repo.has_changes());

    mut_repo.remove_public_head(commit2.id());
    mut_repo.remove_head(commit2.id());
    mut_repo.set_local_branch_target("stable", RefTarget::absent());
    mut_repo.set_remote_branch_target("stable", "origin", RefTarget::absent());
    assert!(!mut_repo.has_changes());

    mut_repo.add_head(&commit2);
    assert!(mut_repo.has_changes());
    mut_repo.remove_head(commit2.id());
    assert!(!mut_repo.has_changes());

    mut_repo.add_public_head(&commit2);
    assert!(mut_repo.has_changes());
    mut_repo.remove_public_head(commit2.id());
    // The commit was added as a visible head when we called has_changes() above.
    // That's a weird side-effect.
    // TODO: Should we make add_public_head() also add it as a visible head? Or
    // should we decouple the two sets completely?
    mut_repo.remove_head(commit2.id());
    assert!(!mut_repo.has_changes());

    mut_repo
        .set_wc_commit(ws_id.clone(), commit2.id().clone())
        .unwrap();
    assert!(mut_repo.has_changes());
    mut_repo.set_wc_commit(ws_id, commit1.id().clone()).unwrap();
    assert!(!mut_repo.has_changes());

    mut_repo.set_local_branch_target("main", RefTarget::normal(commit2.id().clone()));
    assert!(mut_repo.has_changes());
    mut_repo.set_local_branch_target("main", RefTarget::normal(commit1.id().clone()));
    assert!(!mut_repo.has_changes());

    mut_repo.set_remote_branch_target("main", "origin", RefTarget::normal(commit2.id().clone()));
    assert!(mut_repo.has_changes());
    mut_repo.set_remote_branch_target("main", "origin", RefTarget::normal(commit1.id().clone()));
    assert!(!mut_repo.has_changes());
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_rebase_descendants_simple(use_git: bool) {
    // Tests that MutableRepo::create_descendant_rebaser() creates a
    // DescendantRebaser that rebases descendants of rewritten and abandoned
    // commits.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);
    let commit5 = graph_builder.commit_with_parents(&[&commit4]);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let mut graph_builder = CommitGraphBuilder::new(&settings, mut_repo);
    let commit6 = graph_builder.commit_with_parents(&[&commit1]);
    mut_repo.record_rewritten_commit(commit2.id().clone(), commit6.id().clone());
    mut_repo.record_abandoned_commit(commit4.id().clone());
    let mut rebaser = mut_repo.create_descendant_rebaser(&settings);
    // Commit 3 got rebased onto commit 2's replacement, i.e. commit 6
    assert_rebased(rebaser.rebase_next().unwrap(), &commit3, &[&commit6]);
    // Commit 5 got rebased onto commit 4's parent, i.e. commit 1
    assert_rebased(rebaser.rebase_next().unwrap(), &commit5, &[&commit1]);
    assert!(rebaser.rebase_next().unwrap().is_none());
    // No more descendants to rebase if we try again.
    assert!(mut_repo
        .create_descendant_rebaser(&settings)
        .rebase_next()
        .unwrap()
        .is_none());
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_rebase_descendants_conflicting_rewrite(use_git: bool) {
    // Tests MutableRepo::create_descendant_rebaser() when a commit has been marked
    // as rewritten to several other commits.
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction(&settings, "test");
    let mut graph_builder = CommitGraphBuilder::new(&settings, tx.mut_repo());
    let commit1 = graph_builder.initial_commit();
    let commit2 = graph_builder.commit_with_parents(&[&commit1]);
    let _commit3 = graph_builder.commit_with_parents(&[&commit2]);
    let repo = tx.commit();

    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let mut graph_builder = CommitGraphBuilder::new(&settings, mut_repo);
    let commit4 = graph_builder.commit_with_parents(&[&commit1]);
    let commit5 = graph_builder.commit_with_parents(&[&commit1]);
    mut_repo.record_rewritten_commit(commit2.id().clone(), commit4.id().clone());
    mut_repo.record_rewritten_commit(commit2.id().clone(), commit5.id().clone());
    let mut rebaser = mut_repo.create_descendant_rebaser(&settings);
    // Commit 3 does *not* get rebased because it's unclear if it should go onto
    // commit 4 or commit 5
    assert!(rebaser.rebase_next().unwrap().is_none());
    // No more descendants to rebase if we try again.
    assert!(mut_repo
        .create_descendant_rebaser(&settings)
        .rebase_next()
        .unwrap()
        .is_none());
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_rename_remote(use_git: bool) {
    let settings = testutils::user_settings();
    let test_repo = TestRepo::init(use_git);
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction(&settings, "test");
    let mut_repo = tx.mut_repo();
    let commit = write_random_commit(mut_repo, &settings);
    let target = RefTarget::normal(commit.id().clone());
    mut_repo.set_remote_branch_target("main", "origin", target.clone());
    mut_repo.rename_remote("origin", "upstream");
    assert_eq!(mut_repo.get_remote_branch("main", "upstream"), target);
    assert_eq!(
        mut_repo.get_remote_branch("main", "origin"),
        RefTarget::absent()
    );
}
