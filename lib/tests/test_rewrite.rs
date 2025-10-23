// Copyright 2021 The Jujutsu Authors
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

use assert_matches::assert_matches;
use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::commit::Commit;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::FilesMatcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::CommitRewriter;
use jj_lib::rewrite::CommitWithSelection;
use jj_lib::rewrite::EmptyBehavior;
use jj_lib::rewrite::MoveCommitsTarget;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RewriteRefsOptions;
use jj_lib::rewrite::find_duplicate_divergent_commits;
use jj_lib::rewrite::find_recursive_merge_commits;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::rewrite::rebase_commit_with_options;
use jj_lib::rewrite::restore_tree;
use maplit::hashmap;
use maplit::hashset;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::assert_abandoned_with_parent;
use testutils::assert_rebased_onto;
use testutils::create_random_commit;
use testutils::create_tree;
use testutils::create_tree_with;
use testutils::rebase_descendants_with_options_return_map;
use testutils::repo_path;
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

/// Based on https://lore.kernel.org/git/Pine.LNX.4.44.0504271254120.4678-100000@wax.eds.org/
/// (found in t/t6401-merge-criss-cross.sh in the git.git repo).
#[test]
fn test_merge_criss_cross() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let path = repo_path("file");
    let tree_a = create_tree(repo, &[(path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n")]);
    let tree_b = create_tree(repo, &[(path, "1\n2\n3\n4\n5\n6\n7\n8B\n9\n")]);
    let tree_c = create_tree(repo, &[(path, "1\n2\n3C\n4\n5\n6\n7\n8\n9\n")]);
    let tree_d = create_tree(repo, &[(path, "1\n2\n3C\n4\n5\n6\n7\n8D\n9\n")]);
    let tree_e = create_tree(repo, &[(path, "1\n2\n3E\n4\n5\n6\n7\n8B\n9\n")]);
    let tree_expected = create_tree(repo, &[(path, "1\n2\n3E\n4\n5\n6\n7\n8D\n9\n")]);

    let mut tx = repo.start_transaction();
    let mut make_commit = |description, parents, tree_id| {
        tx.repo_mut()
            .new_commit(parents, tree_id)
            .set_description(description)
            .write()
            .unwrap()
    };
    let commit_a = make_commit(
        "A",
        vec![repo.store().root_commit_id().clone()],
        tree_a.id(),
    );
    let commit_b = make_commit("B", vec![commit_a.id().clone()], tree_b.id());
    let commit_c = make_commit("C", vec![commit_a.id().clone()], tree_c.id());
    let commit_d = make_commit(
        "D",
        vec![commit_b.id().clone(), commit_c.id().clone()],
        tree_d.id(),
    );
    let commit_e = make_commit(
        "E",
        vec![commit_b.id().clone(), commit_c.id().clone()],
        tree_e.id(),
    );
    let merged = merge_commit_trees(tx.repo_mut(), &[commit_d, commit_e])
        .block_on()
        .unwrap();

    assert_eq!(merged, tree_expected);
}

#[test]
fn test_find_recursive_merge_commits() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);

    let commit_id_merge = find_recursive_merge_commits(
        tx.repo().store(),
        tx.repo().index(),
        vec![commit_d.id().clone(), commit_e.id().clone()],
    )
    .unwrap();

    assert_eq!(
        commit_id_merge,
        Merge::from_vec(vec![
            commit_d.id().clone(),
            commit_b.id().clone(),
            commit_a.id().clone(),
            commit_c.id().clone(),
            commit_e.id().clone(),
        ])
    );
}

#[test]
fn test_restore_tree() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let path1 = repo_path("file1");
    let path2 = repo_path("dir1/file2");
    let path3 = repo_path("dir1/file3");
    let path4 = repo_path("dir2/file4");
    let left = create_tree(repo, &[(path2, "left"), (path3, "left"), (path4, "left")]);
    let right = create_tree(
        repo,
        &[(path1, "right"), (path2, "right"), (path3, "right")],
    );

    // Restore everything using EverythingMatcher
    let restored = restore_tree(&left, &right, &EverythingMatcher)
        .block_on()
        .unwrap();
    assert_eq!(restored, left.id());

    // Restore everything using FilesMatcher
    let restored = restore_tree(
        &left,
        &right,
        &FilesMatcher::new([&path1, &path2, &path3, &path4]),
    )
    .block_on()
    .unwrap();
    assert_eq!(restored, left.id());

    // Restore some files
    let restored = restore_tree(&left, &right, &FilesMatcher::new([path1, path2]))
        .block_on()
        .unwrap();
    let expected = create_tree(repo, &[(path2, "left"), (path3, "right")]);
    assert_eq!(restored, expected.id());
}

#[test]
fn test_rebase_descendants_sideways() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit F. Commits C-E should be rebased.
    //
    // F
    // | D
    // | C E
    // | |/
    // | B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert_eq!(rebase_map.len(), 3);
    let new_commit_c = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_f.id()]);
    let new_commit_d =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[new_commit_c.id()]);
    let new_commit_e = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_e, &[commit_f.id()]);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_d.id().clone(),
            new_commit_e.id().clone()
        }
    );
}

#[test]
fn test_rebase_descendants_forward() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit F. Commits C and E should be rebased onto F.
    // Commit D does not get rebased because it's an ancestor of the
    // destination. Commit G does not get replaced because it's already in
    // place.
    // TODO: The above is not what actually happens! The test below shows what
    // actually happens: D and F also get rebased onto F, so we end up with
    // duplicates. Consider if it's worth supporting the case above better or if
    // that decision belongs with the caller (as we currently force it to do by
    // not supporting it in DescendantRebaser).
    //
    // G
    // F E
    // |/
    // D C
    // |/
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_f]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_d =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[(commit_f.id())]);
    let new_commit_f =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_f, &[new_commit_d.id()]);
    let new_commit_c =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[new_commit_f.id()]);
    let new_commit_e =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_e, &[new_commit_d.id()]);
    let new_commit_g =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_g, &[new_commit_f.id()]);
    assert_eq!(rebase_map.len(), 5);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_c.id().clone(),
            new_commit_e.id().clone(),
            new_commit_g.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_reorder() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit E was replaced by commit D, and commit C was replaced by commit F
    // (attempting to to reorder C and E), and commit G was replaced by commit
    // H.
    //
    // I
    // G H
    // E F
    // C D
    // |/
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e]);
    let commit_h = write_random_commit_with_parents(tx.repo_mut(), &[&commit_f]);
    let commit_i = write_random_commit_with_parents(tx.repo_mut(), &[&commit_g]);

    tx.repo_mut()
        .set_rewritten_commit(commit_e.id().clone(), commit_d.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_c.id().clone(), commit_f.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_g.id().clone(), commit_h.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_i = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_i, &[commit_h.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_i.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_backward() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit C was replaced by commit B. Commit D should be rebased.
    //
    // D
    // C
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);

    tx.repo_mut()
        .set_rewritten_commit(commit_c.id().clone(), commit_b.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_d = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_b.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {new_commit_d.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_chain_becomes_branchy() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit E and commit C was replaced by commit F.
    // Commit F should get rebased onto E, and commit D should get rebased onto
    // the rebased F.
    //
    // D
    // C F
    // |/
    // B E
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_e.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_c.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_f = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_f, &[commit_e.id()]);
    let new_commit_d =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[new_commit_f.id()]);
    assert_eq!(rebase_map.len(), 2);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_d.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_internal_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit F. Commits C-E should be rebased.
    //
    // F
    // | E
    // | |\
    // | C D
    // | |/
    // | B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c, &commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_c = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_f.id()]);
    let new_commit_d = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_f.id()]);
    let new_commit_e = assert_rebased_onto(
        tx.repo_mut(),
        &rebase_map,
        &commit_e,
        &[new_commit_c.id(), new_commit_d.id()],
    );
    assert_eq!(rebase_map.len(), 3);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { new_commit_e.id().clone() }
    );
}

#[test]
fn test_rebase_descendants_external_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit C was replaced by commit F. Commits E should be rebased. The rebased
    // commit E should have F as first parent and commit D as second parent.
    //
    // F
    // | E
    // | |\
    // | C D
    // | |/
    // | B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c, &commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_c.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_e = assert_rebased_onto(
        tx.repo_mut(),
        &rebase_map,
        &commit_e,
        &[commit_f.id(), commit_d.id()],
    );
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {new_commit_e.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_abandon() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B and commit E were abandoned. Commit C and commit D should get
    // rebased onto commit A. Commit F should get rebased onto the new commit D.
    //
    // F
    // E
    // D C
    // |/
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e]);

    tx.repo_mut().record_abandoned_commit(&commit_b);
    tx.repo_mut().record_abandoned_commit(&commit_e);
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_c = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_a.id()]);
    let new_commit_d = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_a.id()]);
    let new_commit_f =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_f, &[new_commit_d.id()]);
    assert_eq!(rebase_map.len(), 3);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_c.id().clone(),
            new_commit_f.id().clone()
        }
    );
}

#[test]
fn test_rebase_descendants_abandon_no_descendants() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B and C were abandoned. Commit A should become a head.
    //
    // C
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);

    tx.repo_mut().record_abandoned_commit(&commit_b);
    tx.repo_mut().record_abandoned_commit(&commit_c);
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert_eq!(rebase_map.len(), 0);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            commit_a.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_abandon_and_replace() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit E. Commit C was abandoned. Commit D should
    // get rebased onto commit E.
    //
    //   D
    //   C
    // E B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_e.id().clone());
    tx.repo_mut().record_abandoned_commit(&commit_c);
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_d = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_e.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { new_commit_d.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_abandon_degenerate_merge_simplify() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was abandoned. Commit D should get rebased to have only C as parent
    // (not A and C).
    //
    // D
    // |\
    // B C
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);

    tx.repo_mut().record_abandoned_commit(&commit_b);
    let rebase_map = rebase_descendants_with_options_return_map(
        tx.repo_mut(),
        &RebaseOptions {
            simplify_ancestor_merge: true,
            ..Default::default()
        },
    );
    let new_commit_d = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_c.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {new_commit_d.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_abandon_degenerate_merge_preserve() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was abandoned. Commit D should get rebased to have A and C as
    // parents.
    //
    // D
    // |\
    // B C
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);

    tx.repo_mut().record_abandoned_commit(&commit_b);
    let rebase_map = rebase_descendants_with_options_return_map(
        tx.repo_mut(),
        &RebaseOptions {
            simplify_ancestor_merge: false,
            ..Default::default()
        },
    );
    let new_commit_d = assert_rebased_onto(
        tx.repo_mut(),
        &rebase_map,
        &commit_d,
        &[commit_a.id(), commit_c.id()],
    );
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {new_commit_d.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_abandon_widen_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit E was abandoned. Commit F should get rebased to have B, C, and D as
    // parents (in that order).
    //
    // F
    // |\
    // E \
    // |\ \
    // B C D
    //  \|/
    //   A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e, &commit_d]);

    tx.repo_mut().record_abandoned_commit(&commit_e);
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_f = assert_rebased_onto(
        tx.repo_mut(),
        &rebase_map,
        &commit_f,
        &[commit_b.id(), commit_c.id(), commit_d.id()],
    );
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { new_commit_f.id().clone()}
    );
}

#[test]
fn test_rebase_descendants_multiple_sideways() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B and commit D were both replaced by commit F. Commit C and commit E
    // should get rebased onto it.
    //
    // C E
    // B D F
    // | |/
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_f.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_d.id().clone(), commit_f.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_c = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_f.id()]);
    let new_commit_e = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_e, &[commit_f.id()]);
    assert_eq!(rebase_map.len(), 2);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_c.id().clone(),
            new_commit_e.id().clone()
        }
    );
}

#[test]
#[should_panic(expected = "cycle")]
fn test_rebase_descendants_multiple_swap() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit D. Commit D was replaced by commit B.
    // This results in an infinite loop and a panic
    //
    // C E
    // B D
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let _commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let _commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_d.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_d.id().clone(), commit_b.id().clone());
    let _ = tx.repo_mut().rebase_descendants(); // Panics because of the cycle
}

#[test]
fn test_rebase_descendants_multiple_no_descendants() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit C. Commit C was replaced by commit B.
    //
    // B C
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_c.id().clone());
    tx.repo_mut()
        .set_rewritten_commit(commit_c.id().clone(), commit_b.id().clone());
    let result = tx.repo_mut().rebase_descendants();
    assert_matches!(result, Err(err) if err.to_string().contains("Cycle"));
}

#[test]
fn test_rebase_descendants_divergent_rewrite() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit B2. Commit D was replaced by commits D2 and
    // D3. Commit F was replaced by commit F2. Commit C should be rebased onto
    // B2. Commit E should not be rebased. Commit G should be rebased onto
    // commit F2.
    //
    // G
    // F
    // E
    // D
    // C
    // B
    // | F2
    // |/
    // | D3
    // |/
    // | D2
    // |/
    // | B2
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_f]);
    let commit_b2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_f2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_b2.id().clone());
    // Commit D becomes divergent
    tx.repo_mut().set_divergent_rewrite(
        commit_d.id().clone(),
        vec![commit_d2.id().clone(), commit_d3.id().clone()],
    );
    tx.repo_mut()
        .set_rewritten_commit(commit_f.id().clone(), commit_f2.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let new_commit_c =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_b2.id()]);
    let new_commit_g =
        assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_g, &[commit_f2.id()]);
    assert_eq!(rebase_map.len(), 2); // Commit E is not rebased

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_c.id().clone(),
            commit_d2.id().clone(),
            commit_d3.id().clone(),
            commit_e.id().clone(),
            new_commit_g.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_repeated() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit B2. Commit C should get rebased. Rebasing
    // descendants again should have no effect (C should not get rebased again).
    // We then replace B2 by B3. C should now get rebased onto B3.
    //
    // C
    // B
    // | B3
    // |/
    // | B2
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);

    let commit_b2 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("b2")
        .write()
        .unwrap();
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let commit_c2 = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_b2.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            commit_c2.id().clone(),
        }
    );

    // We made no more changes, so nothing should be rebased.
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert_eq!(rebase_map.len(), 0);

    // Now mark B3 as rewritten from B2 and rebase descendants again.
    let commit_b3 = tx
        .repo_mut()
        .rewrite_commit(&commit_b2)
        .set_description("b3")
        .write()
        .unwrap();
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    let commit_c3 = assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c2, &[commit_b3.id()]);
    assert_eq!(rebase_map.len(), 1);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            // commit_b.id().clone(),
            commit_c3.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_contents() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B was replaced by commit D. Commit C should have the changes from
    // commit C and commit D, but not the changes from commit B.
    //
    // D
    // | C
    // | B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "content")]);
    let commit_a = tx
        .repo_mut()
        .new_commit(vec![repo.store().root_commit_id().clone()], tree1.id())
        .write()
        .unwrap();
    let path2 = repo_path("file2");
    let tree2 = create_tree(repo, &[(path2, "content")]);
    let commit_b = tx
        .repo_mut()
        .new_commit(vec![commit_a.id().clone()], tree2.id())
        .write()
        .unwrap();
    let path3 = repo_path("file3");
    let tree3 = create_tree(repo, &[(path3, "content")]);
    let commit_c = tx
        .repo_mut()
        .new_commit(vec![commit_b.id().clone()], tree3.id())
        .write()
        .unwrap();
    let path4 = repo_path("file4");
    let tree4 = create_tree(repo, &[(path4, "content")]);
    let commit_d = tx
        .repo_mut()
        .new_commit(vec![commit_a.id().clone()], tree4.id())
        .write()
        .unwrap();

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_d.id().clone());
    let rebase_map =
        rebase_descendants_with_options_return_map(tx.repo_mut(), &RebaseOptions::default());
    assert_eq!(rebase_map.len(), 1);
    let new_commit_c = repo
        .store()
        .get_commit(rebase_map.get(commit_c.id()).unwrap())
        .unwrap();

    let tree_b = commit_b.tree().unwrap();
    let tree_c = commit_c.tree().unwrap();
    let tree_d = commit_d.tree().unwrap();
    let new_tree_c = new_commit_c.tree().unwrap();
    assert_eq!(
        new_tree_c.path_value(path3).unwrap(),
        tree_c.path_value(path3).unwrap()
    );
    assert_eq!(
        new_tree_c.path_value(path4).unwrap(),
        tree_d.path_value(path4).unwrap()
    );
    assert_ne!(
        new_tree_c.path_value(path2).unwrap(),
        tree_b.path_value(path2).unwrap()
    );
}

#[test]
fn test_rebase_descendants_basic_bookmark_update() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" points to commit B. B gets rewritten as B2. Bookmark main
    // should be updated to point to B2.
    //
    // B main         B2 main
    // |         =>   |
    // A              A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    tx.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_b.id().clone()));
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_b2 = tx.repo_mut().rewrite_commit(&commit_b).write().unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::normal(commit_b2.id().clone())
    );

    assert_eq!(*tx.repo().view().heads(), hashset! {commit_b2.id().clone()});
}

#[test]
fn test_rebase_descendants_bookmark_move_two_steps() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" points to bookmark C. C gets rewritten as C2 and B gets
    // rewritten as B2. C2 should be rebased onto B2, creating C3, and main
    // should be updated to point to C3.
    //
    // C2 C main      C3 main
    // | /            |
    // |/        =>   |
    // B B2           B2
    // |/             |
    // A              A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    tx.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_c.id().clone()));
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_b2 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("different")
        .write()
        .unwrap();
    let commit_c2 = tx
        .repo_mut()
        .rewrite_commit(&commit_c)
        .set_description("more different")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let heads = tx.repo().view().heads();
    assert_eq!(heads.len(), 1);
    let c3_id = heads.iter().next().unwrap().clone();
    let commit_c3 = repo.store().get_commit(&c3_id).unwrap();
    assert_ne!(commit_c3.id(), commit_c2.id());
    assert_eq!(commit_c3.parent_ids(), vec![commit_b2.id().clone()]);
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::normal(commit_c3.id().clone())
    );
}

#[test]
fn test_rebase_descendants_basic_bookmark_update_with_non_local_bookmark() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" points to commit B. B gets rewritten as B2. Bookmark main
    // should be updated to point to B2. Remote bookmark main@origin and tag v1
    // should not get updated.
    //
    //                                B2 main
    // B main main@origin v1          | B main@origin v1
    // |                         =>   |/
    // A                              A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_b_remote_ref = RemoteRef {
        target: RefTarget::normal(commit_b.id().clone()),
        state: RemoteRefState::Tracked,
    };
    tx.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_b.id().clone()));
    tx.repo_mut()
        .set_remote_bookmark(remote_symbol("main", "origin"), commit_b_remote_ref.clone());
    tx.repo_mut()
        .set_local_tag_target("v1".as_ref(), RefTarget::normal(commit_b.id().clone()));
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_b2 = tx.repo_mut().rewrite_commit(&commit_b).write().unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::normal(commit_b2.id().clone())
    );
    // The remote bookmark and tag should not get updated
    assert_eq!(
        tx.repo()
            .get_remote_bookmark(remote_symbol("main", "origin")),
        commit_b_remote_ref
    );
    assert_eq!(
        tx.repo().get_local_tag("v1".as_ref()),
        RefTarget::normal(commit_b.id().clone())
    );

    // Commit B is no longer visible even though the remote bookmark points to it.
    // (The user can still see it using e.g. the `remote_bookmarks()` revset.)
    assert_eq!(*tx.repo().view().heads(), hashset! {commit_b2.id().clone()});
}

#[test_case(false; "slide down abandoned")]
#[test_case(true; "delete abandoned")]
fn test_rebase_descendants_update_bookmark_after_abandon(delete_abandoned_bookmarks: bool) {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Commit B is abandoned. Local bookmarks should be deleted or moved
    // accordingly, whereas remote bookmarks should not get updated.
    //
    // C other
    // |
    // B main main@origin        C2 other
    // |                    =>   |
    // A                         A main (if delete_abandoned_bookmarks = false)
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_b_remote_ref = RemoteRef {
        target: RefTarget::normal(commit_b.id().clone()),
        state: RemoteRefState::Tracked,
    };
    tx.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_b.id().clone()));
    tx.repo_mut()
        .set_remote_bookmark(remote_symbol("main", "origin"), commit_b_remote_ref.clone());
    tx.repo_mut()
        .set_local_bookmark_target("other".as_ref(), RefTarget::normal(commit_c.id().clone()));
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b);
    let options = RebaseOptions {
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks,
        },
        ..Default::default()
    };
    let rebase_map = rebase_descendants_with_options_return_map(tx.repo_mut(), &options);
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        if delete_abandoned_bookmarks {
            RefTarget::absent()
        } else {
            RefTarget::normal(commit_a.id().clone())
        }
    );
    assert_eq!(
        tx.repo()
            .get_remote_bookmark(remote_symbol("main", "origin"))
            .target,
        RefTarget::normal(commit_b.id().clone())
    );
    assert_eq!(
        tx.repo().get_local_bookmark("other".as_ref()),
        RefTarget::normal(rebase_map[commit_c.id()].clone())
    );

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { rebase_map[commit_c.id()].clone() }
    );
}

#[test]
fn test_rebase_descendants_update_bookmarks_after_divergent_rewrite() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" points to commit B. B gets rewritten as {B2, B3, B4}, then
    // B4 as {B41, B42}. Bookmark main should become a conflict pointing to {B2,
    // B3, B41, B42}.
    //
    //                                  C other
    //                C other           | B42 main?
    // C other        | B4 main?        |/B41 main?
    // |              |/B3 main?        |/B3 main?
    // B main         |/B2 main?        |/B2 main?
    // |         =>   |/           =>   |/
    // A              A                 A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    tx.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_b.id().clone()));
    tx.repo_mut()
        .set_local_bookmark_target("other".as_ref(), RefTarget::normal(commit_c.id().clone()));
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_b2 = tx.repo_mut().rewrite_commit(&commit_b).write().unwrap();
    // Different description so they're not the same commit
    let commit_b3 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("different")
        .write()
        .unwrap();
    // Different description so they're not the same commit
    let commit_b4 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("more different")
        .write()
        .unwrap();
    tx.repo_mut().set_divergent_rewrite(
        commit_b.id().clone(),
        vec![
            commit_b2.id().clone(),
            commit_b3.id().clone(),
            commit_b4.id().clone(),
        ],
    );
    let commit_b41 = tx.repo_mut().rewrite_commit(&commit_b4).write().unwrap();
    let commit_b42 = tx
        .repo_mut()
        .rewrite_commit(&commit_b4)
        .set_description("different")
        .write()
        .unwrap();
    tx.repo_mut().set_divergent_rewrite(
        commit_b4.id().clone(),
        vec![commit_b41.id().clone(), commit_b42.id().clone()],
    );
    tx.repo_mut().rebase_descendants().unwrap();

    let main_target = tx.repo().get_local_bookmark("main".as_ref());
    assert!(main_target.has_conflict());
    // If the bookmark were moved at each rewrite point, there would be separate
    // negative terms: { commit_b => 2, commit_b4 => 1 }. Since we flatten
    // intermediate rewrites, commit_b4 doesn't appear in the removed_ids.
    assert_eq!(
        main_target.removed_ids().counts(),
        hashmap! { commit_b.id() => 3 },
    );
    assert_eq!(
        main_target.added_ids().counts(),
        hashmap! {
            commit_b2.id() => 1,
            commit_b3.id() => 1,
            commit_b41.id() => 1,
            commit_b42.id() => 1,
        },
    );

    let other_target = tx.repo().get_local_bookmark("other".as_ref());
    assert_eq!(other_target.as_normal(), Some(commit_c.id()));

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            commit_b2.id().clone(),
            commit_b3.id().clone(),
            commit_b41.id().clone(),
            commit_b42.id().clone(),
            commit_c.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_rewrite_updates_bookmark_conflict() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" is a conflict removing commit A and adding commits B and C.
    // A gets rewritten as A2 and A3. B gets rewritten as B2 and B2. The bookmark
    // should become a conflict removing A and B, and adding B2, B3, C.
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit(tx.repo_mut());
    let commit_c = write_random_commit(tx.repo_mut());
    tx.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::from_legacy_form(
            [commit_a.id().clone()],
            [commit_b.id().clone(), commit_c.id().clone()],
        ),
    );
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_a2 = tx.repo_mut().rewrite_commit(&commit_a).write().unwrap();
    // Different description so they're not the same commit
    let commit_a3 = tx
        .repo_mut()
        .rewrite_commit(&commit_a)
        .set_description("different")
        .write()
        .unwrap();
    let commit_b2 = tx.repo_mut().rewrite_commit(&commit_b).write().unwrap();
    // Different description so they're not the same commit
    let commit_b3 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("different")
        .write()
        .unwrap();
    tx.repo_mut().set_divergent_rewrite(
        commit_a.id().clone(),
        vec![commit_a2.id().clone(), commit_a3.id().clone()],
    );
    tx.repo_mut().set_divergent_rewrite(
        commit_b.id().clone(),
        vec![commit_b2.id().clone(), commit_b3.id().clone()],
    );
    tx.repo_mut().rebase_descendants().unwrap();

    let target = tx.repo().get_local_bookmark("main".as_ref());
    assert!(target.has_conflict());
    assert_eq!(
        target.removed_ids().counts(),
        hashmap! { commit_a.id() => 1, commit_b.id() => 1 },
    );
    assert_eq!(
        target.added_ids().counts(),
        hashmap! {
            commit_c.id() => 1,
            commit_b2.id() => 1,
            commit_b3.id() => 1,
        },
    );

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            commit_a2.id().clone(),
            commit_a3.id().clone(),
            commit_b2.id().clone(),
            commit_b3.id().clone(),
            commit_c.id().clone(),
        }
    );
}

#[test]
fn test_rebase_descendants_rewrite_resolves_bookmark_conflict() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" is a conflict removing ancestor commit A and adding commit B
    // and C (maybe it moved forward to B locally and moved forward to C
    // remotely). Now B gets rewritten as B2, which is a descendant of C (maybe
    // B was automatically rebased on top of the updated remote). That
    // would result in a conflict removing A and adding B2 and C. However, since C
    // is a descendant of A, and B2 is a descendant of C, the conflict gets
    // resolved to B2.
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    tx.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::from_legacy_form(
            [commit_a.id().clone()],
            [commit_b.id().clone(), commit_c.id().clone()],
        ),
    );
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_b2 = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_parents(vec![commit_c.id().clone()])
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::normal(commit_b2.id().clone())
    );

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! { commit_b2.id().clone()}
    );
}

#[test_case(false; "slide down abandoned")]
#[test_case(true; "delete abandoned")]
fn test_rebase_descendants_bookmark_delete_modify_abandon(delete_abandoned_bookmarks: bool) {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" initially points to commit A. One operation rewrites it to
    // point to B (child of A). A concurrent operation deletes the bookmark. That
    // leaves the bookmark pointing to "0-A+B". We now abandon B.
    //
    // - If delete_abandoned_bookmarks = false, that should result in the bookmark
    //   pointing to "0-A+A=0".
    // - If delete_abandoned_bookmarks = true, that should result in the bookmark
    //   pointing to "0-A+0=0".
    //
    // In both cases, the bookmark should be deleted.
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    tx.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::from_legacy_form([commit_a.id().clone()], [commit_b.id().clone()]),
    );
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b);
    let options = RebaseOptions {
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks,
        },
        ..Default::default()
    };
    let _rebase_map = rebase_descendants_with_options_return_map(tx.repo_mut(), &options);
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::absent()
    );
}

#[test_case(false; "slide down abandoned")]
#[test_case(true; "delete abandoned")]
fn test_rebase_descendants_bookmark_move_forward_abandon(delete_abandoned_bookmarks: bool) {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" initially points to commit A. Two concurrent operations
    // rewrites it to point to A's children. That leaves the bookmark pointing
    // to "B-A+C". We now abandon B.
    //
    // - If delete_abandoned_bookmarks = false, that should result in the bookmark
    //   pointing to "A-A+C=C", so the conflict should be resolved.
    // - If delete_abandoned_bookmarks = true, that should result in the bookmark
    //   pointing to "0-A+C".
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    tx.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::from_merge(Merge::from_vec(vec![
            Some(commit_b.id().clone()),
            Some(commit_a.id().clone()),
            Some(commit_c.id().clone()),
        ])),
    );
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b);
    let options = RebaseOptions {
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks,
        },
        ..Default::default()
    };
    let _rebase_map = rebase_descendants_with_options_return_map(tx.repo_mut(), &options);
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        if delete_abandoned_bookmarks {
            RefTarget::from_merge(Merge::from_vec(vec![
                None,
                Some(commit_a.id().clone()),
                Some(commit_c.id().clone()),
            ]))
        } else {
            RefTarget::normal(commit_c.id().clone())
        }
    );
}

#[test_case(false; "slide down abandoned")]
#[test_case(true; "delete abandoned")]
fn test_rebase_descendants_bookmark_move_sideways_abandon(delete_abandoned_bookmarks: bool) {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Bookmark "main" initially points to commit A. Two concurrent operations
    // rewrites it to point to A's siblings. That leaves the bookmark pointing
    // to "B-A+C". We now abandon B.
    //
    // - If delete_abandoned_bookmarks = false, that should result in the bookmark
    //   pointing to "A.parent-A+C".
    // - If delete_abandoned_bookmarks = true, that should result in the bookmark
    //   pointing to "0-A+C".
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit(tx.repo_mut());
    let commit_c = write_random_commit(tx.repo_mut());
    tx.repo_mut().set_local_bookmark_target(
        "main".as_ref(),
        RefTarget::from_merge(Merge::from_vec(vec![
            Some(commit_b.id().clone()),
            Some(commit_a.id().clone()),
            Some(commit_c.id().clone()),
        ])),
    );
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b);
    let options = RebaseOptions {
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks,
        },
        ..Default::default()
    };
    let _rebase_map = rebase_descendants_with_options_return_map(tx.repo_mut(), &options);
    assert_eq!(
        tx.repo().get_local_bookmark("main".as_ref()),
        if delete_abandoned_bookmarks {
            RefTarget::from_merge(Merge::from_vec(vec![
                None,
                Some(commit_a.id().clone()),
                Some(commit_c.id().clone()),
            ]))
        } else {
            RefTarget::from_merge(Merge::from_vec(vec![
                Some(repo.store().root_commit_id().clone()),
                Some(commit_a.id().clone()),
                Some(commit_c.id().clone()),
            ]))
        }
    );
}

#[test]
fn test_rebase_descendants_update_checkout() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Checked-out commit B was replaced by commit C. C should become
    // checked out.
    //
    // C B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let ws1_name = WorkspaceNameBuf::from("ws1");
    let ws2_name = WorkspaceNameBuf::from("ws2");
    let ws3_name = WorkspaceNameBuf::from("ws3");
    tx.repo_mut()
        .set_wc_commit(ws1_name.clone(), commit_b.id().clone())
        .unwrap();
    tx.repo_mut()
        .set_wc_commit(ws2_name.clone(), commit_b.id().clone())
        .unwrap();
    tx.repo_mut()
        .set_wc_commit(ws3_name.clone(), commit_a.id().clone())
        .unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    let commit_c = tx
        .repo_mut()
        .rewrite_commit(&commit_b)
        .set_description("C")
        .write()
        .unwrap();
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    // Workspaces 1 and 2 had B checked out, so they get updated to C. Workspace 3
    // had A checked out, so it doesn't get updated.
    assert_eq!(repo.view().get_wc_commit_id(&ws1_name), Some(commit_c.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws2_name), Some(commit_c.id()));
    assert_eq!(repo.view().get_wc_commit_id(&ws3_name), Some(commit_a.id()));
}

#[test]
fn test_rebase_descendants_update_checkout_abandoned() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Checked-out commit B was abandoned. A child of A
    // should become checked out.
    //
    // B
    // |
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let ws1_name = WorkspaceNameBuf::from("ws1");
    let ws2_name = WorkspaceNameBuf::from("ws2");
    let ws3_name = WorkspaceNameBuf::from("ws3");
    tx.repo_mut()
        .set_wc_commit(ws1_name.clone(), commit_b.id().clone())
        .unwrap();
    tx.repo_mut()
        .set_wc_commit(ws2_name.clone(), commit_b.id().clone())
        .unwrap();
    tx.repo_mut()
        .set_wc_commit(ws3_name.clone(), commit_a.id().clone())
        .unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_b);
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    // Workspaces 1 and 2 had B checked out, so they get updated to the same new
    // commit on top of C. Workspace 3 had A checked out, so it doesn't get updated.
    assert_eq!(
        repo.view().get_wc_commit_id(&ws1_name),
        repo.view().get_wc_commit_id(&ws2_name)
    );
    let checkout = repo
        .store()
        .get_commit(repo.view().get_wc_commit_id(&ws1_name).unwrap())
        .unwrap();
    assert_eq!(checkout.parent_ids(), vec![commit_a.id().clone()]);
    assert_eq!(repo.view().get_wc_commit_id(&ws3_name), Some(commit_a.id()));
}

#[test]
fn test_rebase_descendants_update_checkout_abandoned_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Checked-out merge commit D was abandoned. A new merge commit should become
    // checked out.
    //
    // D
    // |\
    // B C
    // |/
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b, &commit_c]);
    let ws_name = WorkspaceName::DEFAULT.to_owned();
    tx.repo_mut()
        .set_wc_commit(ws_name.clone(), commit_d.id().clone())
        .unwrap();
    let repo = tx.commit("test").unwrap();

    let mut tx = repo.start_transaction();
    tx.repo_mut().record_abandoned_commit(&commit_d);
    tx.repo_mut().rebase_descendants().unwrap();
    let repo = tx.commit("test").unwrap();

    let new_checkout_id = repo.view().get_wc_commit_id(&ws_name).unwrap();
    let checkout = repo.store().get_commit(new_checkout_id).unwrap();
    assert_eq!(
        checkout.parent_ids(),
        vec![commit_b.id().clone(), commit_c.id().clone()]
    );
}

#[test_case(EmptyBehavior::Keep; "keep all commits")]
#[test_case(EmptyBehavior::AbandonNewlyEmpty; "abandon newly empty commits")]
#[test_case(EmptyBehavior::AbandonAllEmpty ; "abandon all empty commits")]
fn test_empty_commit_option(empty_behavior: EmptyBehavior) {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Rebase a previously empty commit, a newly empty commit, and a commit with
    // actual changes.
    //
    // BD (commit B joined with commit D)
    // |   H (empty, no parent tree changes)
    // |   |
    // |   G
    // |   |
    // |   F (clean merge)
    // |  /|\
    // | C D E (empty, but parent tree changes)
    // |  \|/
    // |   B
    // A__/
    let mut tx = repo.start_transaction();
    let mut_repo = tx.repo_mut();
    let create_fixed_tree = |paths: &[&str]| {
        create_tree_with(repo, |builder| {
            for path in paths {
                builder.file(repo_path(path), path);
            }
        })
    };

    // The commit_with_parents function generates non-empty merge commits, so it
    // isn't suitable for this test case.
    let tree_b = create_fixed_tree(&["B"]);
    let tree_c = create_fixed_tree(&["B", "C"]);
    let tree_d = create_fixed_tree(&["B", "D"]);
    let tree_f = create_fixed_tree(&["B", "C", "D"]);
    let tree_g = create_fixed_tree(&["B", "C", "D", "G"]);

    let commit_a = write_random_commit(mut_repo);

    let mut create_commit = |parents: &[&Commit], tree: &MergedTree| {
        create_random_commit(mut_repo)
            .set_parents(
                parents
                    .iter()
                    .map(|commit| commit.id().clone())
                    .collect_vec(),
            )
            .set_tree_id(tree.id())
            .write()
            .unwrap()
    };
    let commit_b = create_commit(&[&commit_a], &tree_b);
    let commit_c = create_commit(&[&commit_b], &tree_c);
    let commit_d = create_commit(&[&commit_b], &tree_d);
    let commit_e = create_commit(&[&commit_b], &tree_b);
    let commit_f = create_commit(&[&commit_c, &commit_d, &commit_e], &tree_f);
    let commit_g = create_commit(&[&commit_f], &tree_g);
    let commit_h = create_commit(&[&commit_g], &tree_g);
    let commit_bd = create_commit(&[&commit_a], &tree_d);

    tx.repo_mut()
        .set_rewritten_commit(commit_b.id().clone(), commit_bd.id().clone());
    let rebase_map = rebase_descendants_with_options_return_map(
        tx.repo_mut(),
        &RebaseOptions {
            empty: empty_behavior,
            rewrite_refs: RewriteRefsOptions {
                delete_abandoned_bookmarks: false,
            },
            simplify_ancestor_merge: true,
        },
    );

    let new_head = match empty_behavior {
        EmptyBehavior::Keep => {
            // The commit C isn't empty.
            let new_commit_c =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_bd.id()]);
            let new_commit_d =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_d, &[commit_bd.id()]);
            let new_commit_e =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_e, &[commit_bd.id()]);
            let new_commit_f = assert_rebased_onto(
                tx.repo_mut(),
                &rebase_map,
                &commit_f,
                &[new_commit_c.id(), new_commit_d.id(), new_commit_e.id()],
            );
            let new_commit_g =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_g, &[new_commit_f.id()]);
            assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_h, &[new_commit_g.id()])
        }
        EmptyBehavior::AbandonAllEmpty => {
            // The commit C isn't empty.
            let new_commit_c =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_bd.id()]);
            // D and E are empty, and F is a clean merge with only one child. Thus, F is
            // also considered empty.
            assert_abandoned_with_parent(tx.repo_mut(), &rebase_map, &commit_d, commit_bd.id());
            assert_abandoned_with_parent(tx.repo_mut(), &rebase_map, &commit_e, commit_bd.id());
            assert_abandoned_with_parent(tx.repo_mut(), &rebase_map, &commit_f, new_commit_c.id());
            let new_commit_g =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_g, &[new_commit_c.id()]);
            assert_abandoned_with_parent(tx.repo_mut(), &rebase_map, &commit_h, new_commit_g.id())
        }
        EmptyBehavior::AbandonNewlyEmpty => {
            // The commit C isn't empty.
            let new_commit_c =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_c, &[commit_bd.id()]);

            // The changes in D are included in BD, so D is newly empty.
            assert_abandoned_with_parent(tx.repo_mut(), &rebase_map, &commit_d, commit_bd.id());
            // E was already empty, so F is a merge commit with C and E as parents.
            // Although it's empty, we still keep it because we don't want to drop merge
            // commits.
            let new_commit_e =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_e, &[commit_bd.id()]);
            let new_commit_f = assert_rebased_onto(
                tx.repo_mut(),
                &rebase_map,
                &commit_f,
                &[new_commit_c.id(), new_commit_e.id()],
            );
            let new_commit_g =
                assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_g, &[new_commit_f.id()]);
            assert_rebased_onto(tx.repo_mut(), &rebase_map, &commit_h, &[new_commit_g.id()])
        }
    };

    assert_eq!(rebase_map.len(), 6);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_head.id().clone(),
        }
    );
}

#[test]
fn test_rebase_abandoning_empty() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // Rebase B onto B2, where B2 and B have the same tree, abandoning all empty
    // commits.
    //
    // We expect B, D, E, and G to be skipped because they're empty. F remains
    // as it's not empty.
    // F G (empty)
    // |/
    // E (WC, empty)  D (empty)       F' E' (WC, empty)
    // |             /                |/
    // C-------------                 C'
    // |                           => |
    // B B2                           B2
    // |/                             |
    // A                              A

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_c.id().clone()])
        .set_tree_id(commit_c.tree_id().clone())
        .write()
        .unwrap();
    let commit_e = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_c.id().clone()])
        .set_tree_id(commit_c.tree_id().clone())
        .write()
        .unwrap();
    let commit_b2 = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_a.id().clone()])
        .set_tree_id(commit_b.tree_id().clone())
        .write()
        .unwrap();
    let commit_f = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_e.id().clone()])
        .write()
        .unwrap();
    let commit_g = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_e.id().clone()])
        .set_tree_id(commit_e.tree_id().clone())
        .write()
        .unwrap();

    let workspace = WorkspaceNameBuf::from("ws");
    tx.repo_mut()
        .set_wc_commit(workspace.clone(), commit_e.id().clone())
        .unwrap();

    let rebase_options = RebaseOptions {
        empty: EmptyBehavior::AbandonAllEmpty,
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks: false,
        },
        simplify_ancestor_merge: true,
    };
    let rewriter = CommitRewriter::new(tx.repo_mut(), commit_b, vec![commit_b2.id().clone()]);
    rebase_commit_with_options(rewriter, &rebase_options).unwrap();
    let rebase_map = rebase_descendants_with_options_return_map(tx.repo_mut(), &rebase_options);
    assert_eq!(rebase_map.len(), 5);
    let new_commit_c = assert_rebased_onto(tx.repo(), &rebase_map, &commit_c, &[commit_b2.id()]);
    assert_abandoned_with_parent(tx.repo(), &rebase_map, &commit_d, new_commit_c.id());
    assert_abandoned_with_parent(tx.repo(), &rebase_map, &commit_e, new_commit_c.id());
    let new_commit_f = assert_rebased_onto(tx.repo(), &rebase_map, &commit_f, &[new_commit_c.id()]);
    assert_abandoned_with_parent(tx.repo(), &rebase_map, &commit_g, new_commit_c.id());

    let new_wc_commit_id = tx
        .repo()
        .view()
        .get_wc_commit_id(&workspace)
        .unwrap()
        .clone();
    let new_wc_commit = tx.repo().store().get_commit(&new_wc_commit_id).unwrap();
    assert_eq!(new_wc_commit.parent_ids(), &[new_commit_c.id().clone()]);

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {new_commit_f.id().clone(), new_wc_commit_id.clone()}
    );
}

#[test]
fn test_commit_with_selection() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let root_tree = repo.store().root_commit().tree().unwrap();
    let commit = write_random_commit(tx.repo_mut());
    let commit_tree = commit.tree().unwrap();
    let empty_selection = CommitWithSelection {
        commit: commit.clone(),
        selected_tree: root_tree.clone(),
        parent_tree: root_tree.clone(),
    };
    assert!(empty_selection.is_empty_selection());
    assert!(!empty_selection.is_full_selection());

    let full_selection = CommitWithSelection {
        commit,
        selected_tree: commit_tree,
        parent_tree: root_tree,
    };
    assert!(!full_selection.is_empty_selection());
    assert!(full_selection.is_full_selection());
}

#[test]
fn test_find_duplicate_divergent_commits() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let store = repo.store();
    // We want to create the following tree with divergent changes A, B, and C:
    //     E
    //     |
    // A2  C2
    // |   |
    // C1  B2
    // |   |
    // B1  D
    // |  /
    // | /
    // A1

    let tree_a1 = create_tree(repo, &[(repo_path("file1"), "a\n")]);
    // Commit b deletes file1 and adds file2
    let tree_b1 = create_tree(repo, &[(repo_path("file2"), "b\n")]);
    // Commit c appends to file2
    let tree_c1 = create_tree(repo, &[(repo_path("file2"), "b\nc\n")]);
    // Commit a2 re-adds file1, the same as commit a1. Since it already has a1
    // as an ancestor, it shouldn't be abandoned even though it is divergent.
    let tree_a2 = create_tree(
        repo,
        &[(repo_path("file1"), "a\n"), (repo_path("file2"), "b\nc\n")],
    );
    let tree_d = create_tree(
        repo,
        &[(repo_path("file1"), "a\n"), (repo_path("file3"), "d\n")],
    );
    let tree_b2 = create_tree(
        repo,
        &[(repo_path("file2"), "b\n"), (repo_path("file3"), "d\n")],
    );
    let tree_c2 = create_tree(
        repo,
        &[(repo_path("file2"), "b\nc\n"), (repo_path("file3"), "d\n")],
    );
    let tree_e = create_tree(
        repo,
        &[
            (repo_path("file2"), "b\nc\n"),
            (repo_path("file3"), "d\ne\n"),
        ],
    );

    let mut make_commit = |change_id_byte, tree_id, parents| {
        tx.repo_mut()
            .new_commit(parents, tree_id)
            .set_change_id(ChangeId::new(vec![
                change_id_byte;
                store.change_id_length()
            ]))
            .write()
            .unwrap()
    };

    let commit_a1 = make_commit(0xAA, tree_a1.id(), vec![store.root_commit_id().clone()]);
    let commit_b1 = make_commit(0xBB, tree_b1.id(), vec![commit_a1.id().clone()]);
    let commit_c1 = make_commit(0xCC, tree_c1.id(), vec![commit_b1.id().clone()]);
    let commit_a2 = make_commit(0xAA, tree_a2.id(), vec![commit_c1.id().clone()]);
    let commit_d = make_commit(0xDD, tree_d.id(), vec![commit_a1.id().clone()]);
    let commit_b2 = make_commit(0xBB, tree_b2.id(), vec![commit_d.id().clone()]);
    let commit_c2 = make_commit(0xCC, tree_c2.id(), vec![commit_b2.id().clone()]);
    let commit_e = make_commit(0xEE, tree_e.id(), vec![commit_c2.id().clone()]);

    // Simulate rebase of "d::" onto "a2"
    let duplicate_commits = find_duplicate_divergent_commits(
        tx.repo(),
        &[commit_a2.id().clone()],
        &MoveCommitsTarget::Roots(vec![commit_d.id().clone()]),
    )
    .unwrap();
    // Commits b2 and c2 are duplicates
    assert_eq!(duplicate_commits, &[commit_c2.clone(), commit_b2.clone()]);

    // Simulate rebase of "b1::" onto "e"
    let duplicate_commits = find_duplicate_divergent_commits(
        tx.repo(),
        &[commit_e.id().clone()],
        &MoveCommitsTarget::Roots(vec![commit_b1.id().clone()]),
    )
    .unwrap();
    // Commits b1 and c1 are duplicates. Commit a2 is not a duplicate, because
    // it already had a1 as an ancestor before the rebase.
    assert_eq!(duplicate_commits, &[commit_c1.clone(), commit_b1.clone()]);

    // Simulate rebase of "d | c2 | e" onto "a2"
    let duplicate_commits = find_duplicate_divergent_commits(
        tx.repo(),
        &[commit_a2.id().clone()],
        &MoveCommitsTarget::Commits(vec![
            commit_d.id().clone(),
            commit_c2.id().clone(),
            commit_e.id().clone(),
        ]),
    )
    .unwrap();
    // Commit c2 is a duplicate
    assert_eq!(duplicate_commits, std::slice::from_ref(&commit_c2));
}
