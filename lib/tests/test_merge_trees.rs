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
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::merge::Merge;
use jj_lib::merge::SameChange;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::rebase_commit;
use jj_lib::settings::UserSettings;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::create_tree;
use testutils::repo_path;

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

    let commit_b2 = rebase_commit(tx.repo_mut(), commit_b, vec![commit_d.id().clone()])
        .block_on()
        .unwrap();
    let commit_c2 = rebase_commit(tx.repo_mut(), commit_c, vec![commit_b2.id().clone()])
        .block_on()
        .unwrap();

    // Test the setup: Both B and C should have conflicts.
    let tree_b2 = commit_b2.tree();
    let tree_c2 = commit_b2.tree();
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
    let commit_c3 = rebase_commit(tx.repo_mut(), commit_c2, vec![commit_b3.id().clone()])
        .block_on()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    // The conflict should now be resolved.
    let tree_c2 = commit_c3.tree();
    let resolved_value = tree_c2.path_value(path).unwrap();
    match resolved_value.into_resolved() {
        Ok(Some(TreeValue::File {
            id,
            executable: false,
            copy_id: _,
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

#[test_case(SameChange::Keep)]
#[test_case(SameChange::Accept)]
fn test_rebase_linearize_lossy_merge(same_change: SameChange) {
    let settings = settings_with_same_change(same_change);
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = &test_repo.repo;

    // Test this rebase:
    // D    foo=2          D' foo=1 or 2
    // |\                  |
    // | C  foo=2          |
    // | |           =>    B  foo=2
    // B |  foo=2          |
    // |/                  |
    // A    foo=1          A  foo=1
    //
    // Since both B and C changed "1" to "2" but only one "2" remains in D, it
    // effectively discarded a change from "1" to "2". With `SameChange::Keep`,
    // D' is therefore "1". However, with `SameChange::Accept`, `jj show D` etc.
    // currently don't tell the user about the discarded change, so it's
    // surprising that the change in commit D is interpreted that way.
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

    match same_change {
        SameChange::Keep => assert!(!commit_d.is_empty(repo_mut).unwrap()),
        SameChange::Accept => assert!(commit_d.is_empty(repo_mut).unwrap()),
    }

    let commit_d2 = rebase_commit(repo_mut, commit_d, vec![commit_b.id().clone()])
        .block_on()
        .unwrap();

    match same_change {
        SameChange::Keep => assert_eq!(*commit_d2.tree_id(), tree_1.id()),
        SameChange::Accept => assert_eq!(*commit_d2.tree_id(), tree_2.id()),
    }
}

#[test_case(SameChange::Keep)]
#[test_case(SameChange::Accept)]
fn test_rebase_on_lossy_merge(same_change: SameChange) {
    let settings = settings_with_same_change(same_change);
    let test_repo = TestRepo::init_with_settings(&settings);
    let repo = &test_repo.repo;

    // Test this rebase:
    // D    foo=2          D'   foo=3 or 2+(3-1) (conflict)
    // |\                  |\
    // | C  foo=2          | C' foo=3
    // | |           =>    | |
    // B |  foo=2          B |  foo=2
    // |/                  |/
    // A    foo=1          A    foo=1
    //
    // Commit D effectively discarded a change from "1" to "2", so one
    // reasonable result in D' is "3". That's the result with
    // `SameChange::Keep`. However, with `SameChange::Accept`, we resolve the
    // auto-merged parents to just "2" before the rebase in order to be
    // consistent with `jj show D` and other commands for inspecting the commit,
    // so we instead get a conflict after the rebase.
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

    match same_change {
        SameChange::Keep => assert!(!commit_d.is_empty(repo_mut).unwrap()),
        SameChange::Accept => assert!(commit_d.is_empty(repo_mut).unwrap()),
    }

    let commit_c2 = repo_mut
        .new_commit(vec![commit_a.id().clone()], tree_3.id())
        .write()
        .unwrap();
    let commit_d2 = rebase_commit(
        repo_mut,
        commit_d,
        vec![commit_b.id().clone(), commit_c2.id().clone()],
    )
    .block_on()
    .unwrap();

    match same_change {
        SameChange::Keep => assert_eq!(*commit_d2.tree_id(), tree_3.id()),
        SameChange::Accept => {
            let expected_tree_id = Merge::from_vec(vec![
                tree_2.id().into_merge(),
                tree_1.id().into_merge(),
                tree_3.id().into_merge(),
            ])
            .flatten();
            assert_eq!(*commit_d2.tree_id(), MergedTreeId::new(expected_tree_id));
        }
    }
}

fn settings_with_same_change(same_change: SameChange) -> UserSettings {
    let mut config = testutils::base_user_config();
    let mut layer = ConfigLayer::empty(ConfigSource::User);
    let same_change_str = match same_change {
        SameChange::Keep => "keep",
        SameChange::Accept => "accept",
    };
    layer
        .set_value("merge.same-change", same_change_str)
        .unwrap();
    config.add_layer(layer);
    UserSettings::from_config(config).unwrap()
}
