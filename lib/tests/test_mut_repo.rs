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

use jj_lib::backend::CommitId;
use jj_lib::index::Index;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::RebaseOptions;
use maplit::hashset;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::assert_rebased_onto;
use testutils::create_random_commit;
use testutils::create_random_tree;
use testutils::rebase_descendants_with_options_return_map;
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

fn index_has_id(index: &dyn Index, commit_id: &CommitId) -> bool {
    index.has_id(commit_id).unwrap()
}

#[test]
fn test_edit() {
    // Test that MutableRepo::edit() uses the requested commit (not a new child)
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let wc_commit = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    tx.repo_mut().edit(ws_name.clone(), &wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();
    assert_eq!(repo.view().get_wc_commit_id(&ws_name), Some(wc_commit.id()));
}

#[test]
fn test_checkout() {
    // Test that MutableRepo::check_out() creates a child
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let wc_commit_parent = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    let wc_commit = tx
        .repo_mut()
        .check_out(ws_name.clone(), &wc_commit_parent)
        .unwrap();
    assert_eq!(wc_commit.tree_ids(), wc_commit_parent.tree_ids());
    assert_eq!(wc_commit.parent_ids().len(), 1);
    assert_eq!(&wc_commit.parent_ids()[0], wc_commit_parent.id());
    let repo = tx.commit("test").unwrap();
    assert_eq!(repo.view().get_wc_commit_id(&ws_name), Some(wc_commit.id()));
}

#[test]
fn test_edit_previous_not_empty() {
    // Test that MutableRepo::edit() does not usually abandon the previous
    // commit.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = write_random_commit(mut_repo);
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty() {
    // Test that MutableRepo::edit() abandons the previous commit if it was
    // empty.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(!mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty_merge() {
    // Test that MutableRepo::edit() abandons the previous commit if it was
    // an empty merge commit.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_parent1 = write_random_commit(mut_repo);
    let old_parent2 = write_random_commit(mut_repo);
    let empty_tree = repo.store().root_commit().tree();
    let old_parent_tree = old_parent1
        .tree()
        .merge(empty_tree, old_parent2.tree())
        .block_on()
        .unwrap();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![old_parent1.id().clone(), old_parent2.id().clone()],
            repo.store().empty_merged_tree(),
        )
        .set_tree(old_parent_tree)
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(!mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty_with_description() {
    // Test that MutableRepo::edit() does not abandon the previous commit if it
    // has a non-empty description.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .set_description("not empty")
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty_with_local_bookmark() {
    // Test that MutableRepo::edit() does not abandon the previous commit if it
    // is pointed by local bookmark.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();
    mut_repo.set_local_bookmark_target("b".as_ref(), RefTarget::normal(old_wc_commit.id().clone()));
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty_with_other_workspace() {
    // Test that MutableRepo::edit() does not abandon the previous commit if it
    // is pointed by another workspace
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let other_ws_name = WorkspaceNameBuf::from("other");
    mut_repo
        .edit(other_ws_name.clone(), &old_wc_commit)
        .unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_edit_previous_empty_non_head() {
    // Test that MutableRepo::edit() does not abandon the previous commit if it
    // was empty and is not a head
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();
    let old_child = mut_repo
        .new_commit(vec![old_wc_commit.id().clone()], old_wc_commit.tree())
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let new_wc_commit = write_random_commit(mut_repo);
    mut_repo.edit(ws_name, &new_wc_commit).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert_eq!(
        *mut_repo.view().heads(),
        hashset! {old_child.id().clone(), new_wc_commit.id().clone()}
    );
}

#[test]
fn test_edit_initial() {
    // Test that MutableRepo::edit() can be used on the initial working-copy commit
    // in a workspace
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let wc_commit = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let ws_name = WorkspaceNameBuf::from("new-workspace");
    tx.repo_mut().edit(ws_name.clone(), &wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();
    assert_eq!(repo.view().get_wc_commit_id(&ws_name), Some(wc_commit.id()));
}

#[test]
fn test_edit_hidden_commit() {
    // Test that MutableRepo::edit() edits a hidden commit and updates
    // the view head ids.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let wc_commit = write_random_commit(tx.repo_mut());

    // Intentionally not doing tx.commit, so the commit id is not tracked
    // in the view head ids.

    let mut tx = repo.start_transaction();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    tx.repo_mut().edit(ws_name.clone(), &wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();
    assert_eq!(repo.view().get_wc_commit_id(&ws_name), Some(wc_commit.id()));
    assert_eq!(*repo.view().heads(), hashset! {wc_commit.id().clone()});
}

#[test]
fn test_add_head_success() {
    // Test that MutableRepo::add_head() adds the head, and that it's still there
    // after commit. It should also be indexed.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Create a commit outside of the repo by using a temporary transaction. Then
    // add that as a head.
    let mut tx = repo.start_transaction();
    let new_commit = write_random_commit(tx.repo_mut());
    drop(tx);

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    assert!(!mut_repo.view().heads().contains(new_commit.id()));
    assert!(!index_has_id(mut_repo.index(), new_commit.id()));
    mut_repo.add_head(&new_commit).unwrap();
    assert!(mut_repo.view().heads().contains(new_commit.id()));
    assert!(index_has_id(mut_repo.index(), new_commit.id()));
    let repo = tx.commit("test").unwrap();
    assert!(repo.view().heads().contains(new_commit.id()));
    assert!(index_has_id(repo.index(), new_commit.id()));
}

#[test]
fn test_add_head_ancestor() {
    // Test that MutableRepo::add_head() does not add a head if it's an ancestor of
    // an existing head.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let repo = tx.commit("test").unwrap();

    assert_eq!(repo.view().heads(), &hashset! {commit3.id().clone()});
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo.add_head(&commit1).unwrap();
    assert_eq!(repo.view().heads(), &hashset! {commit3.id().clone()});
}

#[test]
fn test_add_head_not_immediate_child() {
    // Test that MutableRepo::add_head() can be used for adding a head that is not
    // an immediate child of a current head.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let initial = write_random_commit(tx.repo_mut());
    let repo = tx.commit("test").unwrap();

    // Create some commits outside of the repo by using a temporary transaction.
    // Then add one of them as a head.
    let mut tx = repo.start_transaction();
    let rewritten = create_random_commit(tx.repo_mut())
        .set_change_id(initial.change_id().clone())
        .set_predecessors(vec![initial.id().clone()])
        .write()
        .unwrap();
    let child = write_random_commit_with_parents(tx.repo_mut(), &[&rewritten]);
    drop(tx);

    assert_eq!(repo.view().heads(), &hashset! {initial.id().clone()});
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo.add_head(&child).unwrap();
    assert_eq!(
        mut_repo.view().heads(),
        &hashset! {initial.id().clone(), child.id().clone()}
    );
    assert!(index_has_id(mut_repo.index(), initial.id()));
    assert!(index_has_id(mut_repo.index(), rewritten.id()));
    assert!(index_has_id(mut_repo.index(), child.id()));
}

#[test]
fn test_remove_head() {
    // Test that MutableRepo::remove_head() removes the head, and that it's still
    // removed after commit. It should remain in the index, since we otherwise would
    // have to reindex everything.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    assert!(mut_repo.view().heads().contains(commit3.id()));
    mut_repo.remove_head(commit3.id());
    let heads = mut_repo.view().heads().clone();
    assert!(!heads.contains(commit3.id()));
    assert!(!heads.contains(commit2.id()));
    assert!(!heads.contains(commit1.id()));
    assert!(index_has_id(mut_repo.index(), commit1.id()));
    assert!(index_has_id(mut_repo.index(), commit2.id()));
    assert!(index_has_id(mut_repo.index(), commit3.id()));
    let repo = tx.commit("test").unwrap();
    let heads = repo.view().heads().clone();
    assert!(!heads.contains(commit3.id()));
    assert!(!heads.contains(commit2.id()));
    assert!(!heads.contains(commit1.id()));
    assert!(index_has_id(repo.index(), commit1.id()));
    assert!(index_has_id(repo.index(), commit2.id()));
    assert!(index_has_id(repo.index(), commit3.id()));
}

#[test]
fn test_has_changed() {
    // Test that MutableRepo::has_changed() reports changes iff the view has changed
    // (e.g. not after setting a bookmark to point to where it was already
    // pointing).
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let normal_remote_ref = |id: &CommitId| RemoteRef {
        target: RefTarget::normal(id.clone()),
        state: RemoteRefState::Tracked, // doesn't matter
    };

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit1 = write_random_commit(mut_repo);
    let commit2 = write_random_commit(mut_repo);
    mut_repo.remove_head(commit2.id());
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo
        .set_wc_commit(ws_name.clone(), commit1.id().clone())
        .unwrap();
    mut_repo.set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit1.id().clone()));
    mut_repo.set_remote_bookmark(
        remote_symbol("main", "origin"),
        normal_remote_ref(commit1.id()),
    );
    let repo = tx.commit("test").unwrap();
    // Test the setup
    assert_eq!(repo.view().heads(), &hashset! {commit1.id().clone()});

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();

    mut_repo.add_head(&commit1).unwrap();
    mut_repo
        .set_wc_commit(ws_name.clone(), commit1.id().clone())
        .unwrap();
    mut_repo.set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit1.id().clone()));
    mut_repo.set_remote_bookmark(
        remote_symbol("main", "origin"),
        normal_remote_ref(commit1.id()),
    );
    assert!(!mut_repo.has_changes());

    mut_repo.remove_head(commit2.id());
    mut_repo.set_local_bookmark_target("stable".as_ref(), RefTarget::absent());
    mut_repo.set_remote_bookmark(remote_symbol("stable", "origin"), RemoteRef::absent());
    assert!(!mut_repo.has_changes());

    mut_repo.add_head(&commit2).unwrap();
    assert!(mut_repo.has_changes());
    mut_repo.remove_head(commit2.id());
    assert!(!mut_repo.has_changes());

    mut_repo
        .set_wc_commit(ws_name.clone(), commit2.id().clone())
        .unwrap();
    assert!(mut_repo.has_changes());
    mut_repo
        .set_wc_commit(ws_name, commit1.id().clone())
        .unwrap();
    assert!(!mut_repo.has_changes());

    mut_repo.set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit2.id().clone()));
    assert!(mut_repo.has_changes());
    mut_repo.set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit1.id().clone()));
    mut_repo.remove_head(commit2.id());
    assert!(!mut_repo.has_changes());

    mut_repo.set_remote_bookmark(
        remote_symbol("main", "origin"),
        normal_remote_ref(commit2.id()),
    );
    assert!(mut_repo.has_changes());
    mut_repo.set_remote_bookmark(
        remote_symbol("main", "origin"),
        normal_remote_ref(commit1.id()),
    );
    assert!(!mut_repo.has_changes());
}

#[test]
fn test_rebase_descendants_simple() {
    // There are many additional tests of this functionality in `test_rewrite.rs`.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let commit4 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit5 = write_random_commit_with_parents(tx.repo_mut(), &[&commit4]);
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit6 = write_random_commit_with_parents(mut_repo, &[&commit1]);
    mut_repo.set_rewritten_commit(commit2.id().clone(), commit6.id().clone());
    mut_repo.record_abandoned_commit(&commit4);
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    // Commit 3 got rebased onto commit 2's replacement, i.e. commit 6
    assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit3, &[commit6.id()]);
    // Commit 5 got rebased onto commit 4's parent, i.e. commit 1
    assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit5, &[commit1.id()]);
    assert_eq!(rebase_map.len(), 2);

    // No more descendants to rebase if we try again.
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert_eq!(rebase_map.len(), 0);
}

#[test]
fn test_rebase_descendants_divergent_rewrite() {
    // Test rebasing descendants when one commit was rewritten to several other
    // commits. There are many additional tests of this functionality in
    // `test_rewrite.rs`.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let _commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit4 = write_random_commit_with_parents(mut_repo, &[&commit1]);
    let commit5 = write_random_commit_with_parents(mut_repo, &[&commit1]);
    mut_repo.set_divergent_rewrite(
        commit2.id().clone(),
        vec![commit4.id().clone(), commit5.id().clone()],
    );
    // Commit 3 does *not* get rebased because it's unclear if it should go onto
    // commit 4 or commit 5
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert!(rebase_map.is_empty());
}

#[test]
fn test_rename_remote() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let commit = write_random_commit(mut_repo);
    let remote_ref = RemoteRef {
        target: RefTarget::normal(commit.id().clone()),
        state: RemoteRefState::Tracked, // doesn't matter
    };
    mut_repo.set_remote_bookmark(remote_symbol("main", "origin"), remote_ref.clone());
    mut_repo.rename_remote("origin".as_ref(), "upstream".as_ref());
    assert_eq!(
        mut_repo.get_remote_bookmark(remote_symbol("main", "upstream")),
        remote_ref
    );
    assert_eq!(
        mut_repo.get_remote_bookmark(remote_symbol("main", "origin")),
        RemoteRef::absent()
    );
}

#[test]
fn test_remove_wc_commit_previous_not_discardable() {
    // Test that MutableRepo::remove_wc_commit() does not usually abandon the
    // previous commit.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = write_random_commit(mut_repo);
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo.remove_wc_commit(&ws_name).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_remove_wc_commit_previous_discardable() {
    // Test that MutableRepo::remove_wc_commit() abandons the previous commit
    // if it was discardable.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let old_wc_commit = mut_repo
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            repo.store().empty_merged_tree(),
        )
        .write()
        .unwrap();
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    mut_repo.edit(ws_name.clone(), &old_wc_commit).unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo.remove_wc_commit(&ws_name).unwrap();
    mut_repo.rebase_descendants().unwrap();
    assert!(!mut_repo.view().heads().contains(old_wc_commit.id()));
}

#[test]
fn test_reparent_descendants() {
    // Test that MutableRepo::reparent_descendants() reparents descendants of
    // rewritten commits without altering their content.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit(tx.repo_mut());
    let commit_child_a_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a, &commit_b]);
    let commit_grandchild_a_b =
        write_random_commit_with_parents(tx.repo_mut(), &[&commit_child_a_b]);
    let commit_child_a = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_child_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let mut_repo = tx.repo_mut();
    for (bookmark, commit) in [
        ("b", &commit_b),
        ("child_a_b", &commit_child_a_b),
        ("grandchild_a_b", &commit_grandchild_a_b),
        ("child_a", &commit_child_a),
        ("child_b", &commit_child_b),
    ] {
        mut_repo
            .set_local_bookmark_target(bookmark.as_ref(), RefTarget::normal(commit.id().clone()));
    }
    let repo = tx.commit("test").unwrap();

    // Rewrite "commit_a".
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    mut_repo
        .rewrite_commit(&commit_a)
        .set_tree(create_random_tree(&repo))
        .write()
        .unwrap();
    let reparented = mut_repo.reparent_descendants().unwrap();
    // "child_a_b", "grandchild_a_b" and "child_a" (3 commits) must have been
    // reparented.
    assert_eq!(reparented, 3);
    let repo = tx.commit("test").unwrap();

    for (bookmark, commit) in [
        ("b", &commit_b),
        ("child_a_b", &commit_child_a_b),
        ("grandchild_a_b", &commit_grandchild_a_b),
        ("child_a", &commit_child_a),
        ("child_b", &commit_child_b),
    ] {
        let rewritten_id = repo
            .view()
            .get_local_bookmark(bookmark.as_ref())
            .as_normal()
            .unwrap()
            .clone();
        if matches!(bookmark, "b" | "child_b") {
            // "b" and "child_b" have been kept untouched.
            assert_eq!(commit.id(), &rewritten_id);
        } else {
            // All commits except "b", and "child_b" have been reparented while keeping
            // their content.
            assert_ne!(commit.id(), &rewritten_id);
            let rewritten_commit = repo.store().get_commit(&rewritten_id).unwrap();
            assert_eq!(commit.tree_ids(), rewritten_commit.tree_ids());
            let (parent_ids, rewritten_parent_ids) =
                (commit.parent_ids(), rewritten_commit.parent_ids());
            assert_eq!(parent_ids.len(), rewritten_parent_ids.len());
            assert_ne!(parent_ids, rewritten_parent_ids);
        }
    }
}

#[test]
fn test_bookmark_hidden_commit() {
    // Test that MutableRepo::set_local_bookmark_target() on a hidden commit makes
    // it visible.
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root_commit = repo.store().root_commit();

    let mut tx = repo.start_transaction();
    let commit = write_random_commit(tx.repo_mut());
    tx.repo_mut().remove_head(commit.id());
    let repo = tx.commit("test").unwrap();
    // Test the setup
    assert_eq!(*repo.view().heads(), hashset! {root_commit.id().clone()});

    let mut tx = repo.start_transaction();
    tx.repo_mut()
        .set_local_bookmark_target("b".as_ref(), RefTarget::normal(commit.id().clone()));
    let repo = tx.commit("test").unwrap();
    assert_eq!(*repo.view().heads(), hashset! {commit.id().clone()});
}
