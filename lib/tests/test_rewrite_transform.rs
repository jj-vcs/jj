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

use jj_lib::commit::Commit;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::RewriteRefsOptions;
use maplit::hashmap;
use maplit::hashset;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

// Simulate some `jj sync` command that rebases B:: onto G while abandoning C
// (because it's presumably already in G).
//
// G
// | E
// | D F
// | |/
// | C
// | B
// |/
// A
#[test]
fn test_transform_descendants_sync() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    let mut rebased = HashMap::new();
    tx.repo_mut()
        .transform_descendants(vec![commit_b.id().clone()], async |mut rewriter| {
            rewriter.replace_parent(commit_a.id(), [commit_g.id()]);
            if *rewriter.old_commit() == commit_c {
                rewriter.abandon();
            } else {
                let old_commit_id = rewriter.old_commit().id().clone();
                let new_commit = rewriter.rebase().await?.write().await?;
                rebased.insert(old_commit_id, new_commit);
            }
            Ok(())
        })
        .block_on()
        .unwrap();
    assert_eq!(rebased.len(), 4);
    let new_commit_b = rebased.get(commit_b.id()).unwrap();
    let new_commit_d = rebased.get(commit_d.id()).unwrap();
    let new_commit_e = rebased.get(commit_e.id()).unwrap();
    let new_commit_f = rebased.get(commit_f.id()).unwrap();

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_e.id().clone(),
            new_commit_f.id().clone(),
        }
    );

    assert_eq!(new_commit_b.parent_ids(), vec![commit_g.id().clone()]);
    assert_eq!(new_commit_d.parent_ids(), vec![new_commit_b.id().clone()]);
    assert_eq!(new_commit_e.parent_ids(), vec![new_commit_d.id().clone()]);
    assert_eq!(new_commit_f.parent_ids(), vec![new_commit_b.id().clone()]);
}

// Transform just commit C replacing parent A by parent B. The parents should be
// deduplicated.
//
//   C
//  /|
// B |
// |/
// A
#[test]
fn test_transform_descendants_sync_linearize_merge() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a, &commit_b]);

    let mut rebased = HashMap::new();
    tx.repo_mut()
        .transform_descendants(vec![commit_c.id().clone()], async |mut rewriter| {
            rewriter.replace_parent(commit_a.id(), [commit_b.id()]);
            let old_commit_id = rewriter.old_commit().id().clone();
            let new_commit = rewriter.rebase().await?.write().await?;
            rebased.insert(old_commit_id, new_commit);
            Ok(())
        })
        .block_on()
        .unwrap();
    assert_eq!(rebased.len(), 1);
    let new_commit_c = rebased.get(commit_c.id()).unwrap();

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            new_commit_c.id().clone(),
        }
    );

    assert_eq!(new_commit_c.parent_ids(), vec![commit_b.id().clone()]);
}

// Reorder commits B and C by using the `new_parents_map`. Reordering has to be
// done outside of the typical callback since we must ensure that the new
// traversal order of the commits is valid.
//
// G
// | E
// | D F
// | |/
// | C
// | B
// |/
// A
#[test]
fn test_transform_descendants_new_parents_map() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_d]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    let options = RewriteRefsOptions::default();
    let mut rebased = HashMap::new();
    tx.repo_mut()
        .transform_descendants_with_options(
            vec![commit_b.id().clone()],
            &hashmap! {
                commit_b.id().clone() => vec![commit_c.id().clone()],
                commit_c.id().clone() => vec![commit_a.id().clone()],
            },
            &options,
            async |mut rewriter| {
                let old_commit_id = rewriter.old_commit().id().clone();
                if old_commit_id != *commit_b.id()
                    && let Some(new_commit_c) = rebased.get(commit_c.id())
                {
                    let new_commit_b: &Commit = rebased.get(commit_b.id()).unwrap();
                    rewriter.replace_parent(new_commit_c.id(), [new_commit_b.id()]);
                }
                let new_commit = rewriter.rebase().await?.write().await?;
                rebased.insert(old_commit_id, new_commit);
                Ok(())
            },
        )
        .block_on()
        .unwrap();
    assert_eq!(rebased.len(), 5);
    let new_commit_b = rebased.get(commit_b.id()).unwrap();
    let new_commit_c = rebased.get(commit_c.id()).unwrap();
    let new_commit_d = rebased.get(commit_d.id()).unwrap();
    let new_commit_e = rebased.get(commit_e.id()).unwrap();
    let new_commit_f = rebased.get(commit_f.id()).unwrap();

    assert_eq!(
        *tx.repo().view().heads(),
        hashset! {
            commit_g.id().clone(),
            new_commit_e.id().clone(),
            new_commit_f.id().clone(),
        }
    );

    assert_eq!(new_commit_c.parent_ids(), vec![commit_a.id().clone()]);
    assert_eq!(new_commit_b.parent_ids(), vec![new_commit_c.id().clone()]);
    assert_eq!(new_commit_d.parent_ids(), vec![new_commit_b.id().clone()]);
    assert_eq!(new_commit_e.parent_ids(), vec![new_commit_d.id().clone()]);
    assert_eq!(new_commit_f.parent_ids(), vec![new_commit_b.id().clone()]);
}
