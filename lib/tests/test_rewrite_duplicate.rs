// Copyright 2024 The Jujutsu Authors
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

use std::collections::HashMap;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::duplicate_commits;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::assert_tree_eq;
use testutils::create_tree;
use testutils::repo_path;

#[test]
fn test_duplicate_linear_contents() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let path_1 = repo_path("file1");
    let path_2 = repo_path("file2");
    let empty_tree = repo.store().empty_merged_tree();
    let tree_1 = create_tree(repo, &[(path_1, "content1")]);
    let tree_2 = create_tree(repo, &[(path_2, "content2")]);
    let tree_1_2 = create_tree(repo, &[(path_1, "content1"), (path_2, "content2")]);

    // E [=file2]
    // D [-file1, =file2]
    // C [=file1, +file2]
    // B [+file1]
    // A []
    let mut tx = repo.start_transaction();
    let commit_a = tx
        .repo_mut()
        .new_commit(
            vec![repo.store().root_commit_id().clone()],
            empty_tree.clone(),
        )
        .write()
        .unwrap();
    let commit_b = tx
        .repo_mut()
        .new_commit(vec![commit_a.id().clone()], tree_1.clone())
        .write()
        .unwrap();
    let commit_c = tx
        .repo_mut()
        .new_commit(vec![commit_b.id().clone()], tree_1_2.clone())
        .write()
        .unwrap();
    let commit_d = tx
        .repo_mut()
        .new_commit(vec![commit_c.id().clone()], tree_2.clone())
        .write()
        .unwrap();
    let commit_e = tx
        .repo_mut()
        .new_commit(vec![commit_d.id().clone()], tree_2.clone())
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();

    let duplicate_in_between = |tx: &mut Transaction,
                                target_commits: &[&CommitId],
                                parent_commit_ids: &[&CommitId],
                                children_commit_ids: &[&CommitId]| {
        duplicate_commits(
            tx.repo_mut(),
            &target_commits.iter().copied().cloned().collect_vec(),
            &HashMap::new(),
            &parent_commit_ids.iter().copied().cloned().collect_vec(),
            &children_commit_ids.iter().copied().cloned().collect_vec(),
        )
        .block_on()
        .unwrap()
    };
    let duplicate_onto =
        |tx: &mut Transaction, target_commits: &[&CommitId], parent_commit_ids: &[&CommitId]| {
            duplicate_in_between(tx, target_commits, parent_commit_ids, &[])
        };

    // Duplicate empty commit onto empty ancestor tree
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_e.id()], &[commit_a.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_e.id()].tree(), empty_tree);

    // Duplicate empty commit onto non-empty ancestor tree
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_e.id()], &[commit_b.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_e.id()].tree(), tree_1);

    // Duplicate non-empty commit onto empty ancestor tree
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_c.id()], &[commit_a.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_c.id()].tree(), tree_2);

    // Duplicate non-empty commit onto non-empty ancestor tree
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_d.id()], &[commit_b.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_d.id()].tree(), empty_tree);

    // Duplicate non-empty commit onto non-empty descendant tree
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_b.id()], &[commit_d.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_b.id()].tree(), tree_1_2);

    // Duplicate multiple contiguous commits
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_e.id(), commit_d.id()], &[commit_b.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_d.id()].tree(), empty_tree);
    assert_tree_eq!(stats.duplicated_commits[commit_e.id()].tree(), empty_tree);

    // Duplicate multiple non-contiguous commits
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_e.id(), commit_c.id()], &[commit_a.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_c.id()].tree(), tree_2);
    assert_tree_eq!(stats.duplicated_commits[commit_e.id()].tree(), tree_2);

    // Duplicate onto multiple parents
    let mut tx = repo.start_transaction();
    let stats = duplicate_onto(&mut tx, &[commit_d.id()], &[commit_c.id(), commit_b.id()]);
    assert_tree_eq!(stats.duplicated_commits[commit_d.id()].tree(), tree_2);

    // Insert duplicated commit
    let mut tx = repo.start_transaction();
    let stats = duplicate_in_between(
        &mut tx,
        &[commit_b.id()],
        &[commit_d.id()],
        &[commit_e.id()],
    );
    assert_tree_eq!(stats.duplicated_commits[commit_b.id()].tree(), tree_1_2);
    let [head_id] = tx.repo().view().heads().iter().collect_array().unwrap();
    assert_ne!(head_id, commit_e.id());
    assert_tree_eq!(
        tx.repo().store().get_commit(head_id).unwrap().tree(),
        tree_1_2
    );
}
