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
use jj_lib::backend::TreeValue;
use jj_lib::merge::Merge;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::rebase_commit;
use testutils::create_tree;
use testutils::repo_path;
use testutils::TestRepo;

#[test]
fn test_simplify_conflict_after_resolving_parent() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Set up a repo like this:
    // D
    // | C
    // | B
    // |/
    // A
    //
    // Commit A has a file with 3 lines. B and D make conflicting changes to the
    // first line. C changes the third line. We then rebase B and C onto D,
    // which creates a conflict. We resolve the conflict in the first line and
    // rebase C2 (the rebased C) onto the resolved conflict. C3 should not have
    // a conflict since it changed an unrelated line.
    let path = repo_path("dir/file");
    let mut tx = repo.start_transaction();
    let tree_a = create_tree(repo, &[(path, "abc\ndef\nghi\n")]);
    let commit_a = tx
        .repo_mut()
        .new_commit(vec![repo.store().root_commit_id().clone()], tree_a.id())
        .write()
        .unwrap();
    let tree_b = create_tree(repo, &[(path, "Abc\ndef\nghi\n")]);
    let commit_b = tx
        .repo_mut()
        .new_commit(vec![commit_a.id().clone()], tree_b.id())
        .write()
        .unwrap();
    let tree_c = create_tree(repo, &[(path, "Abc\ndef\nGhi\n")]);
    let commit_c = tx
        .repo_mut()
        .new_commit(vec![commit_b.id().clone()], tree_c.id())
        .write()
        .unwrap();
    let tree_d = create_tree(repo, &[(path, "abC\ndef\nghi\n")]);
    let commit_d = tx
        .repo_mut()
        .new_commit(vec![commit_a.id().clone()], tree_d.id())
        .write()
        .unwrap();

    let commit_b2 = rebase_commit(tx.repo_mut(), commit_b, vec![commit_d.id().clone()]).unwrap();
    let commit_c2 = rebase_commit(tx.repo_mut(), commit_c, vec![commit_b2.id().clone()]).unwrap();

    // Test the setup: Both B and C should have conflicts.
    let tree_b2 = commit_b2.tree().unwrap();
    let tree_c2 = commit_b2.tree().unwrap();
    assert!(!tree_b2.path_value(path).unwrap().is_resolved());
    assert!(!tree_c2.path_value(path).unwrap().is_resolved());

    // Create the resolved B and rebase C on top.
    let tree_b3 = create_tree(repo, &[(path, "AbC\ndef\nghi\n")]);
    let commit_b3 = tx
        .repo_mut()
        .rewrite_commit(&commit_b2)
        .set_tree_id(tree_b3.id())
        .write()
        .unwrap();
    let commit_c3 = rebase_commit(tx.repo_mut(), commit_c2, vec![commit_b3.id().clone()]).unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    // The conflict should now be resolved.
    let tree_c2 = commit_c3.tree().unwrap();
    let resolved_value = tree_c2.path_value(path).unwrap();
    match resolved_value.into_resolved() {
        Ok(Some(TreeValue::File {
            id,
            executable: false,
            ..
        })) => {
            assert_eq!(
                testutils::read_file(repo.store(), path, &id),
                b"AbC\ndef\nGhi\n"
            );
        }
        other => {
            panic!("unexpected value: {other:#?}");
        }
    }
}

// TODO: Add tests for simplification of multi-way conflicts. Both the content
// and the executable bit need testing.

#[test]
fn test_rebase_linearize_lossy_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Test this rebase:
    // D    foo=2          D' foo=2
    // |\                  |
    // | C  foo=2          |
    // | |           =>    B  foo=2
    // B |  foo=2          |
    // |/                  |
    // A    foo=1          A  foo=1
    //
    // Since both B and C changed "1" to "2" but only one "2" remains in D, it
    // effectively discarded a change from "1" to "2". One reasonable result in
    // D' is therefore "1". However, since `jj show D` etc. currently don't tell
    // the user about the discarded change, it's surprising that the change in
    // commit D is interpreted that way. If we're going to change that, we will
    // probably also need to drop the "A+(A-B)=A" rule so it requires an
    // explicit action from the user to resolve such conflicts.
    let path = repo_path("foo");
    let mut tx = repo.start_transaction();
    let repo_mut = tx.repo_mut();
    let tree_1 = create_tree(repo, &[(path, "1")]);
    let tree_2 = create_tree(repo, &[(path, "2")]);
    let commit_a = repo_mut
        .new_commit(vec![repo.store().root_commit_id().clone()], tree_1.id())
        .write()
        .unwrap();
    let commit_b = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_2.id())
        .write()
        .unwrap();
    let commit_c = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_2.id())
        .write()
        .unwrap();
    let commit_d = repo_mut
        .new_commit(
            vec![commit_b.id().clone(), commit_c.id().clone()],
            tree_2.id(),
        )
        .write()
        .unwrap();

    let commit_d2 = rebase_commit(repo_mut, commit_d, vec![commit_b.id().clone()]).unwrap();

    assert_eq!(*commit_d2.tree_id(), tree_2.id());
}

#[test]
fn test_rebase_on_lossy_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Test this rebase:
    // D    foo=2          D'   foo=2+(3-1) (conflict)
    // |\                  |\
    // | C  foo=2          | C' foo=3
    // | |           =>    | |
    // B |  foo=2          B |  foo=2
    // |/                  |/
    // A    foo=1          A    foo=1
    //
    // Commit D effectively discarded a change from "1" to "2", so one
    // reasonable result in D' is "3". That's what the result would be if we
    // didn't have the "A+(A-B)=A" rule. However, because we resolve the
    // auto-merged parents to just "2" before the rebase in order to be
    // consistent with `jj show D` and other commands for inspecting the
    // commit, we instead get a conflict after the rebase.
    let path = repo_path("foo");
    let mut tx = repo.start_transaction();
    let repo_mut = tx.repo_mut();
    let tree_1 = create_tree(repo, &[(path, "1")]);
    let tree_2 = create_tree(repo, &[(path, "2")]);
    let tree_3 = create_tree(repo, &[(path, "3")]);
    let commit_a = repo_mut
        .new_commit(vec![repo.store().root_commit_id().clone()], tree_1.id())
        .write()
        .unwrap();
    let commit_b = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_2.id())
        .write()
        .unwrap();
    let commit_c = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_2.id())
        .write()
        .unwrap();
    let commit_d = repo_mut
        .new_commit(
            vec![commit_b.id().clone(), commit_c.id().clone()],
            tree_2.id(),
        )
        .write()
        .unwrap();

    let commit_c2 = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_3.id())
        .write()
        .unwrap();
    let commit_d2 = rebase_commit(
        repo_mut,
        commit_d,
        vec![commit_b.id().clone(), commit_c2.id().clone()],
    )
    .unwrap();

    let expected_tree_id = Merge::from_vec(vec![
        tree_2.id().to_merge(),
        tree_1.id().to_merge(),
        tree_3.id().to_merge(),
    ])
    .flatten();
    assert_eq!(*commit_d2.tree_id(), MergedTreeId::Merge(expected_tree_id));
}
