// Copyright 2026 The Jujutsu Authors
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
use std::collections::HashSet;
use std::sync::Arc;

use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::commit::Commit;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::TruncatedEvolutionGraph;
use jj_lib::converge::find_divergent_changes;
use jj_lib::converge::remove_descendants;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn make_change_id(repo: &TestRepo, byte: u8) -> ChangeId {
    ChangeId::new(vec![byte; repo.repo.store().change_id_length()])
}

fn create_commit(
    tx: &mut Transaction,
    parents: &[&CommitId],
    tree: &MergedTree,
    author: &Signature,
    desc: &str,
    change_id: Option<&ChangeId>,
) -> Commit {
    let repo = tx.repo_mut();
    let parents: Vec<CommitId> = parents.iter().map(|p| (*p).clone()).collect::<Vec<_>>();
    let builder = repo
        .new_commit(parents, tree.clone())
        .set_author(author.clone())
        .set_description(desc.to_string())
        .set_tree(tree.clone());
    match change_id {
        Some(change_id) => builder.set_change_id(change_id.clone()),
        None => builder,
    }
    .write_unwrap()
}

fn assert_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    expected: &[(&ChangeId, &[Commit])],
) -> TestResult<CommitsByChangeId> {
    let expected_divergent_commits: HashMap<ChangeId, HashSet<CommitId>> = expected
        .iter()
        .map(|(change_id, commits)| {
            (
                (*change_id).clone(),
                commits.iter().map(|c| c.id().clone()).collect(),
            )
        })
        .collect();
    let actual = find_divergent_changes(repo, RevsetExpression::all()).block_on()?;
    let simplified: HashMap<ChangeId, HashSet<CommitId>> = actual
        .clone()
        .into_iter()
        .map(|(change_id, commits)| (change_id, commits.into_keys().collect::<HashSet<_>>()))
        .collect();
    assert_eq!(simplified, expected_divergent_commits);
    Ok(actual)
}

#[test]
fn test_find_divergent_changes_none_found() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root = repo.store().root_commit_id();

    let empty_tree = repo.store().empty_merged_tree();
    let author = Signature {
        name: "author1".to_string(),
        email: "author1".to_string(),
        timestamp: Timestamp::now(),
    };

    let mut tx = repo.start_transaction();
    let _commit_1 = create_commit(&mut tx, &[root], &empty_tree, &author, "commit 1", None);
    let _commit_2 = create_commit(&mut tx, &[root], &empty_tree, &author, "commit 2", None);
    let repo = tx.commit("test").block_on()?;

    let result = find_divergent_changes(&repo, RevsetExpression::all()).block_on()?;
    assert!(result.is_empty());
    Ok(())
}

#[test]
fn test_remove_descendants_linear_chain() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let repo = tx.repo_mut();
    let commit1 = write_random_commit(repo);
    let commit2 = write_random_commit_with_parents(repo, &[&commit1]);
    let commit3 = write_random_commit_with_parents(repo, &[&commit2]);
    let repo = tx.commit("test").block_on()?;

    assert_eq!(
        remove_descendants(
            &repo,
            &[
                commit1.id().clone(),
                commit2.id().clone(),
                commit3.id().clone(),
            ],
        )
        .block_on()?,
        HashSet::from([commit1.id().clone()])
    );
    assert_eq!(
        remove_descendants(&repo, &[commit1.id().clone(), commit2.id().clone(),],).block_on()?,
        HashSet::from([commit1.id().clone()])
    );
    assert_eq!(
        remove_descendants(&repo, &[commit1.id().clone()],).block_on()?,
        HashSet::from([commit1.id().clone()])
    );

    Ok(())
}

#[test]
fn test_find_divergent_changes_exactly_one_found() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root = repo.store().root_commit_id();
    let change_aa = make_change_id(&test_repo, 0xAA);

    let empty_tree = repo.store().empty_merged_tree();
    let author = Signature {
        name: "author1".to_string(),
        email: "author1".to_string(),
        timestamp: Timestamp::now(),
    };

    let commit_1 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "foo",
            Some(&change_aa),
        );
        tx.commit("tx1").block_on()?;
        commit
    };

    let commit_2 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "bar",
            Some(&change_aa),
        );
        tx.commit("tx2").block_on()?;
        commit
    };

    let repo = repo.reload_at_head().block_on()?;
    assert_eq!(
        find_divergent_changes(&repo, RevsetExpression::all()).block_on()?,
        HashMap::from([(
            change_aa.clone(),
            HashMap::from([
                (commit_1.id().clone(), commit_1.clone()),
                (commit_2.id().clone(), commit_2.clone()),
            ]),
        )])
    );

    Ok(())
}

#[test]
fn test_find_divergent_changes_two_found() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let root = repo.store().root_commit_id();
    let change_aa = make_change_id(&test_repo, 0xAA);
    let change_bb = make_change_id(&test_repo, 0xBB);

    let empty_tree = repo.store().empty_merged_tree();
    let author = Signature {
        name: "author1".to_string(),
        email: "author1".to_string(),
        timestamp: Timestamp::now(),
    };

    let commit_1 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "foo",
            Some(&change_aa),
        );
        tx.commit("tx1").block_on()?;
        commit
    };

    let commit_2 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "bar",
            Some(&change_aa),
        );
        tx.commit("tx2").block_on()?;
        commit
    };

    let commit_3 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "baz",
            Some(&change_bb),
        );
        tx.commit("tx3").block_on()?;
        commit
    };

    let commit_4 = {
        let mut tx = repo.start_transaction();
        let commit = create_commit(
            &mut tx,
            &[root],
            &empty_tree,
            &author,
            "qux",
            Some(&change_bb),
        );
        tx.commit("tx4").block_on()?;
        commit
    };

    let repo = repo.reload_at_head().block_on()?;
    drop(assert_divergent_changes(
        &repo,
        &[
            (&change_aa, &[commit_1.clone(), commit_2.clone()]),
            (&change_bb, &[commit_3.clone(), commit_4.clone()]),
        ],
    )?);
    Ok(())
}

#[test]
fn test_build_truncated_evolution_graph() -> TestResult {
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("tx1").block_on()?;

    let commit2 = {
        let mut tx = repo1.start_transaction();
        let commit2 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->foo")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on()?;
        tx.commit("tx2").block_on()?;
        commit2
    };

    let commit3 = {
        let mut tx = repo1.start_transaction();
        let commit3 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->bar")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on()?;
        tx.commit("tx3").block_on()?;
        commit3
    };

    let repo = repo1.reload_at_head().block_on()?;

    let divergent_commits = vec![commit2.clone(), commit3.clone()];
    let truncated_evolution_graph =
        TruncatedEvolutionGraph::new(repo, divergent_commits).block_on()?;
    assert_eq!(truncated_evolution_graph.change_id(), commit1.change_id());
    assert_eq!(
        truncated_evolution_graph.divergent_commit_ids(),
        &[commit2.id().clone(), commit3.id().clone()]
    );
    assert_eq!(
        truncated_evolution_graph
            .flow_graph
            .graph
            .adjacent_nodes(commit1.id())
            .unwrap()
            .collect::<Vec<_>>(),
        &[commit2.id(), commit3.id()]
    );
    assert!(
        truncated_evolution_graph
            .flow_graph
            .graph
            .adjacent_nodes(commit2.id())
            .unwrap()
            .collect::<Vec<_>>()
            .is_empty(),
    );
    assert!(
        truncated_evolution_graph
            .flow_graph
            .graph
            .adjacent_nodes(commit3.id())
            .unwrap()
            .collect::<Vec<_>>()
            .is_empty(),
    );

    Ok(())
}
