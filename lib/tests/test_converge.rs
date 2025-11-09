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
use std::ops::Deref as _;
use std::slice;
use std::sync::Arc;
use std::sync::Mutex;

use assert_matches::assert_matches;
use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::backend::TreeId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergeResult;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::TruncatedEvolutionGraph;
use jj_lib::converge::apply_solution;
use jj_lib::converge::choose_change;
use jj_lib::converge::converge_change;
use jj_lib::converge::find_divergent_changes;
use jj_lib::converge::remove_descendants;
use jj_lib::evolution::walk_predecessors;
use jj_lib::merge::Merge;
use jj_lib::merge::MergeBuilder;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::commit_transactions;
use testutils::create_random_tree;
use testutils::create_tree_with;
use testutils::dump_tree;
use testutils::repo_path;
use testutils::repo_path_buf;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

struct MockConvergeUI {
    pub chosen_change: Option<ChangeId>,
    pub chosen_author: Option<Signature>,
    pub chosen_parents: Option<Vec<CommitId>>,
    pub merged_description: Option<String>,

    // Tracking for assertions
    pub choose_change_called: Mutex<bool>,
    pub choose_author_called: Mutex<bool>,
    pub choose_parents_called: Mutex<bool>,
    pub merge_description_called: Mutex<bool>,
}

impl MockConvergeUI {
    fn new() -> Self {
        Self {
            chosen_change: None,
            chosen_author: None,
            chosen_parents: None,
            merged_description: None,
            choose_change_called: Mutex::new(false),
            choose_author_called: Mutex::new(false),
            choose_parents_called: Mutex::new(false),
            merge_description_called: Mutex::new(false),
        }
    }
}

impl ConvergeUI for MockConvergeUI {
    fn choose_change<'a>(
        &self,
        divergent_changes: &'a CommitsByChangeId,
    ) -> Result<Option<&'a ChangeId>, ConvergeError> {
        *self.choose_change_called.lock().unwrap() = true;
        let Some(ref change_id) = self.chosen_change else {
            return Ok(None);
        };
        match divergent_changes.keys().find(|k| *k == change_id) {
            Some(change_id) => Ok(Some(change_id)),
            None => Err(ConvergeError::Other(
                format!("MockConvergeUI error: {change_id:.12} not in divergent changes").into(),
            )),
        }
    }

    fn choose_author(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Option<Signature>, ConvergeError> {
        *self.choose_author_called.lock().unwrap() = true;
        Ok(self.chosen_author.clone())
    }

    fn choose_parents(
        &self,
        _divergent_commits: &[Commit],
    ) -> Result<Option<Vec<CommitId>>, ConvergeError> {
        *self.choose_parents_called.lock().unwrap() = true;
        Ok(self.chosen_parents.clone())
    }

    fn merge_description(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Option<String>, ConvergeError> {
        *self.merge_description_called.lock().unwrap() = true;
        Ok(self.merged_description.clone())
    }
}

fn make_change_id(repo: &TestRepo, byte: u8) -> ChangeId {
    ChangeId::new(vec![byte; repo.repo.store().change_id_length()])
}

fn get_merged_tree_value(tree: &MergedTree, path: &str) -> Option<TreeValue> {
    tree.trees()
        .block_on()
        .unwrap()
        .into_resolved()
        .unwrap()
        .path_value(repo_path(path))
        .block_on()
        .unwrap()
}

#[allow(dead_code)]
fn tree_to_string(
    store: &Arc<Store>,
    tree_ids: &Merge<TreeId>,
    conflict_labels: &ConflictLabels,
) -> String {
    dump_tree(&MergedTree::new(
        store.clone(),
        tree_ids.clone(),
        conflict_labels.clone(),
    ))
}

fn get_predecessors(repo: &ReadonlyRepo, id: &CommitId) -> Vec<CommitId> {
    let entries: Vec<_> = walk_predecessors(repo, slice::from_ref(id))
        .try_collect()
        .expect("unreachable predecessors shouldn't be visited");
    let first = entries
        .first()
        .expect("specified commit should be reachable");
    first.predecessor_ids().to_vec()
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

pub fn create_simple_tree(repo: &Arc<ReadonlyRepo>, path: &str, content: &str) -> MergedTree {
    create_tree_with(repo, |builder| {
        builder.file(&repo_path_buf(path), content);
    })
}

fn create_merged_tree(terms: Vec<(MergedTree, String)>) -> MergedTree {
    MergedTree::merge(MergeBuilder::from_iter(terms).build())
        .block_on()
        .unwrap()
}

fn assert_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    expected: &[(&ChangeId, &[Commit])],
) -> Result<CommitsByChangeId, Box<dyn std::error::Error>> {
    let expected_divergent_commits: HashMap<ChangeId, HashSet<CommitId>> = expected
        .iter()
        .map(|(change_id, commits)| {
            (
                (*change_id).clone(),
                commits.iter().map(|c| c.id().clone()).collect(),
            )
        })
        .collect();
    let actual = find_divergent_changes(repo, RevsetExpression::all())?;
    let simplified: HashMap<ChangeId, HashSet<CommitId>> = actual
        .clone()
        .into_iter()
        .map(|(change_id, commits)| (change_id, commits.into_keys().collect::<HashSet<_>>()))
        .collect();
    assert_eq!(simplified, expected_divergent_commits);
    Ok(actual)
}

fn assert_heads(repo: &dyn Repo, expected: Vec<&CommitId>) {
    let expected = expected.iter().copied().cloned().collect();
    assert_eq!(*repo.view().heads(), expected);
}

#[test]
fn test_find_divergent_changes_none_found() -> Result<(), Box<dyn std::error::Error>> {
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
    let repo = tx.commit("test").block_on().unwrap();

    let result = find_divergent_changes(&repo, RevsetExpression::all())?;
    assert!(result.is_empty());
    Ok(())
}

#[test]
fn test_remove_descendants_linear_chain() -> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let repo = tx.repo_mut();
    let commit1 = write_random_commit(repo);
    let commit2 = write_random_commit_with_parents(repo, &[&commit1]);
    let commit3 = write_random_commit_with_parents(repo, &[&commit2]);
    let repo = tx.commit("test").block_on().unwrap();

    let result = remove_descendants(
        &repo,
        &[
            commit1.id().clone(),
            commit2.id().clone(),
            commit3.id().clone(),
        ],
    )?;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id(), commit1.id());

    let result = remove_descendants(&repo, &[commit1.id().clone(), commit2.id().clone()])?;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id(), commit1.id());

    let result = remove_descendants(&repo, &[commit1.id().clone()])?;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id(), commit1.id());

    Ok(())
}

#[test]
fn test_find_divergent_changes_exactly_one_found() -> Result<(), Box<dyn std::error::Error>> {
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
        tx.commit("tx1").block_on().unwrap();
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
        tx.commit("tx2").block_on().unwrap();
        commit
    };

    let repo = repo.reload_at_head().block_on().unwrap();
    let result = find_divergent_changes(&repo, RevsetExpression::all())?;
    let expected = HashMap::from([(
        change_aa.clone(),
        HashMap::from([
            (commit_1.id().clone(), commit_1.clone()),
            (commit_2.id().clone(), commit_2.clone()),
        ]),
    )]);
    assert_eq!(result.clone(), expected);

    // Since there is a single divergent change, choose_change() works without a
    // ConvergeUI.
    assert_eq!(choose_change(None, &result)?, Some(&change_aa));

    // It also works with a ConvergeUI, and the UI is not called since there is only
    // one option.
    let converge_ui = MockConvergeUI::new();
    let chosen_change = choose_change(Some(&converge_ui), &result)?;
    assert_eq!(chosen_change, Some(&change_aa));
    assert!(!*converge_ui.choose_author_called.lock().unwrap());

    Ok(())
}

#[test]
fn test_find_divergent_changes_two_found() -> Result<(), Box<dyn std::error::Error>> {
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
        tx.commit("tx1").block_on().unwrap();
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
        tx.commit("tx2").block_on().unwrap();
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
        tx.commit("tx3").block_on().unwrap();
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
        tx.commit("tx4").block_on().unwrap();
        commit
    };

    let repo = repo.reload_at_head().block_on().unwrap();
    let divergent_changes = assert_divergent_changes(
        &repo,
        &[
            (&change_aa, &[commit_1.clone(), commit_2.clone()]),
            (&change_bb, &[commit_3.clone(), commit_4.clone()]),
        ],
    )?;

    // Since there are multiple divergent change, choose_change() requires a
    // ConvergeUI.
    let chosen_change = choose_change(None, &divergent_changes)?;
    assert_eq!(chosen_change, None);

    // It does work with a ConvergeUI.
    let mut converge_ui = MockConvergeUI::new();
    converge_ui.chosen_change = Some(change_aa.clone());
    assert_eq!(
        choose_change(Some(&converge_ui), &divergent_changes)?,
        Some(&change_aa)
    );
    let ui_called = *converge_ui.choose_change_called.lock().unwrap();
    assert!(ui_called);

    // Simulate the case where the user aborts the UI by not choosing a change.
    let converge_ui = MockConvergeUI::new();
    let chosen_change = choose_change(Some(&converge_ui), &divergent_changes)?;
    assert_eq!(chosen_change, None);
    let ui_called = *converge_ui.choose_change_called.lock().unwrap();
    assert!(ui_called);

    Ok(())
}

#[test]
fn test_build_truncated_evolution_graph() -> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("tx1").block_on().unwrap();

    let commit2 = {
        let mut tx = repo1.start_transaction();
        let commit2 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->foo")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on().unwrap();
        tx.commit("tx2").block_on().unwrap();
        commit2
    };

    let commit3 = {
        let mut tx = repo1.start_transaction();
        let commit3 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->bar")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on().unwrap();
        tx.commit("tx3").block_on().unwrap();
        commit3
    };

    let repo = repo1.reload_at_head().block_on().unwrap();

    let divergent_commits = [commit2.clone(), commit3.clone()];
    let max_evolution_nodes = 100;
    let truncated_evolution_graph =
        TruncatedEvolutionGraph::new(repo.deref(), &divergent_commits, max_evolution_nodes)?;
    assert_eq!(truncated_evolution_graph.change_id(), commit1.change_id());
    assert_eq!(
        truncated_evolution_graph.divergent_commit_ids,
        vec![commit2.id().clone(), commit3.id().clone(),]
    );
    assert_eq!(
        truncated_evolution_graph.get_evolution_fork_point().id(),
        commit1.id()
    );
    assert_eq!(
        truncated_evolution_graph
            .commits
            .keys()
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from([
            commit1.id().clone(),
            commit2.id().clone(),
            commit3.id().clone()
        ])
    );
    assert_eq!(
        truncated_evolution_graph.commits[commit1.id()].commit.id(),
        commit1.id()
    );
    assert_eq!(
        truncated_evolution_graph.commits[commit2.id()].commit.id(),
        commit2.id()
    );
    assert_eq!(
        truncated_evolution_graph.commits[commit3.id()].commit.id(),
        commit3.id()
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

#[test]
fn test_simple_converge_description() -> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();

    let mut tx = test_repo.repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("tx1").block_on().unwrap();

    let commit2 = {
        let mut tx = repo1.start_transaction();
        let commit2 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->foo")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on().unwrap();
        tx.commit("tx2").block_on().unwrap();
        commit2
    };

    let commit3 = {
        let mut tx = repo1.start_transaction();
        let commit3 = tx
            .repo_mut()
            .rewrite_commit(&commit1)
            .set_description("rewritten->bar")
            .write_unwrap();
        tx.repo_mut().rebase_descendants().block_on().unwrap();
        tx.commit("tx3").block_on().unwrap();
        commit3
    };

    let repo = repo1.reload_at_head().block_on().unwrap();
    let divergent_commits = [commit2.clone(), commit3.clone()];
    let divergent_commit_ids = vec![commit2.id().clone(), commit3.id().clone()];
    let max_evolution_nodes = 100;
    {
        let converge_ui = None;
        let converge_result =
            converge_change(&repo, converge_ui, &divergent_commits, max_evolution_nodes)
                .block_on()?;

        assert_matches!(
            converge_result,
            ConvergeResult::NeedUserInput(msg) if msg == "cannot converge description automatically"
        );
    }

    let mut converge_ui = MockConvergeUI::new();
    converge_ui.merged_description = Some("merged_description".to_string());
    let converge_result = converge_change(
        &repo,
        Some(&converge_ui),
        &divergent_commits,
        max_evolution_nodes,
    )
    .block_on()?;

    match converge_result {
        ConvergeResult::Solution(ref solution) => {
            assert_eq!(solution.change_id, commit1.change_id().clone());
            assert_eq!(solution.divergent_commit_ids, divergent_commit_ids);
            assert_eq!(solution.author, commit1.author().clone());
            assert_eq!(solution.description, "merged_description".to_string());
            assert_eq!(solution.parents, commit1.parent_ids().to_vec());
            assert_eq!(solution.tree_ids, commit1.tree().tree_ids().clone());
            assert_eq!(solution.conflict_labels, commit1.tree().labels().clone());
        }
        _ => unreachable!("unexpected ConvergeResult"),
    }

    Ok(())
}

// Evolution (predecessors are below their successors):
//
// C4  C5
// |   |
// C2  C3
//  \  /
//   C1
//
// C1 is rewritten to C2 and C3 in parallel, and then in a single transaction C2
// is rewritten to C4 and C3 is rewritten to C5. The visible commits at the end
// are C4 and C5. The only thing changing throughout is the description.
//
// The ConvergeUI must be used to converge the description.
#[test]
fn test_manual_converge_description_concurrent_ops() -> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();
    let repo0 = test_repo.repo;

    let mut tx = repo0.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let repo1 = tx.commit("test").block_on().unwrap();

    let mut tx2 = repo1.start_transaction();
    let commit2 = tx2
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 2")
        .write_unwrap();
    tx2.repo_mut().rebase_descendants().block_on().unwrap();
    let mut tx3 = repo1.start_transaction();
    let commit3 = tx3
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("rewritten 3")
        .write_unwrap();
    tx3.repo_mut().rebase_descendants().block_on().unwrap();
    let repo4 = commit_transactions(vec![tx2, tx3]);

    let mut tx = repo4.start_transaction();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("rewritten 4")
        .write_unwrap();
    let commit5 = tx
        .repo_mut()
        .rewrite_commit(&commit3)
        .set_description("rewritten 5")
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on().unwrap();
    let repo5 = tx.commit("test").block_on().unwrap();

    let change_id = commit1.change_id().clone();
    assert_eq!(
        find_divergent_changes(&repo5, RevsetExpression::all())?,
        HashMap::from([(
            change_id.clone(),
            HashMap::from([
                (commit4.id().clone(), commit4.clone()),
                (commit5.id().clone(), commit5.clone()),
            ]),
        )])
    );

    let divergent_commits = [commit4.clone(), commit5.clone()];
    let divergent_commit_ids = vec![commit4.id().clone(), commit5.id().clone()];
    let max_evolution_nodes = 100;
    {
        let converge_ui = None;
        let converge_result =
            converge_change(&repo5, converge_ui, &divergent_commits, max_evolution_nodes)
                .block_on()?;
        assert_matches!(
            converge_result,
            ConvergeResult::NeedUserInput(msg) if msg == "cannot converge description automatically"
        );
    }

    let mut converge_ui = MockConvergeUI::new();
    converge_ui.merged_description = Some("merged_description".to_string());
    let converge_result = converge_change(
        &repo5,
        Some(&converge_ui),
        &divergent_commits,
        max_evolution_nodes,
    )
    .block_on()?;

    match converge_result {
        ConvergeResult::Solution(ref solution) => {
            assert_eq!(solution.change_id, change_id);
            assert_eq!(solution.divergent_commit_ids, divergent_commit_ids.clone());
            assert_eq!(solution.author, commit1.author().clone());
            assert_eq!(solution.description, "merged_description".to_string());
            assert_eq!(solution.parents, commit1.parent_ids().to_vec());
            assert_eq!(solution.tree_ids, commit1.tree().tree_ids().clone());
            assert_eq!(solution.conflict_labels, commit1.tree().labels().clone());
        }
        _ => unreachable!("unexpected ConvergeResult"),
    }
    assert!(*converge_ui.merge_description_called.lock().unwrap());

    Ok(())
}

// Evolution (predecessors are below their successors):
//
// C4("baz", parent_x)
//      |
// C2("bar", parent_y)      C3("bar", parent_x)
//      \                      /
//       C1("foo", parent_x)
//
// C1 is rewritten to C2 and C3 in parallel, and then C2 is rewritten to C4. The
// visible commits at the end are C3 and C4. converge is possible without user
// input.
//
// Expected result: Solution("baz", parent_x).
#[test]
fn test_automatic_converge_description_and_parent() -> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();

    // First create the parents.
    let mut tx = test_repo.repo.start_transaction();
    let parent_x = write_random_commit(tx.repo_mut()).id().clone();
    let parent_y = write_random_commit(tx.repo_mut()).id().clone();
    let repo0 = tx.commit("test").block_on().unwrap();

    let mut tx = repo0.start_transaction();
    let tree = create_random_tree(tx.repo_mut().base_repo());
    let commit1 = tx
        .repo_mut()
        .new_commit(vec![parent_x.clone()], tree)
        .set_description("foo".to_string())
        .write_unwrap();
    let repo1 = tx.commit("test").block_on().unwrap();

    let mut tx2 = repo1.start_transaction();
    let commit2 = tx2
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .set_parents(vec![parent_y.clone()])
        .write_unwrap();
    tx2.repo_mut().rebase_descendants().block_on().unwrap();
    let mut tx3 = repo1.start_transaction();
    let commit3 = tx3
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .write_unwrap();
    tx3.repo_mut().rebase_descendants().block_on().unwrap();
    let repo4 = commit_transactions(vec![tx2, tx3]);

    let mut tx = repo4.start_transaction();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("baz")
        .set_parents(vec![parent_x.clone()])
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on().unwrap();
    let repo5 = tx.commit("test").block_on().unwrap();

    let change_id = commit1.change_id().clone();
    let divergent_commits = [commit3.clone(), commit4.clone()];
    assert_divergent_changes(&repo5, &[(&change_id, &divergent_commits)])?;

    let divergent_commit_ids = vec![commit3.id().clone(), commit4.id().clone()];
    let max_evolution_nodes = 100;
    let converge_ui = None;
    let converge_result =
        converge_change(&repo5, converge_ui, &divergent_commits, max_evolution_nodes).block_on()?;

    match converge_result {
        ConvergeResult::Solution(ref solution) => {
            assert_eq!(solution.change_id, change_id);
            assert_eq!(solution.divergent_commit_ids, divergent_commit_ids.clone());
            assert_eq!(solution.author, commit1.author().clone());
            assert_eq!(solution.description, "baz".to_string());
            assert_eq!(solution.parents, vec![parent_x.clone()]);
            assert_eq!(solution.tree_ids, commit1.tree().tree_ids().clone());
            assert_eq!(solution.conflict_labels, commit1.tree().labels().clone());
        }
        _ => unreachable!("unexpected ConvergeResult"),
    }

    Ok(())
}

// Evolution (predecessors are below their successors):
//
// C4("baz", parent:X, file="content4")
//      |
// C2("bar", parent:Y, file="content2")
//      |
//      |                           C3("bar", parent:X,file="content3")
//       \                                    /
//       C1("foo", parent:X, file="content1")
//
// C1 is rewritten to C2 and C3 in parallel, and then C2 is rewritten to C4. The
// visible commits at the end are base,X,Y,C3,C4. converge is possible without
// user input.
//
// Commit graph:
//
// C1 C3 C4  C2
//  \ | /    |
//    X      Y
//     \    /
//      base
//
// Expected result (commit graph):
//
// Solution("baz")
//    |
//    X       Y
//     \     /
//      base
#[test]
fn test_automatic_converge_description_parent_and_trees() -> Result<(), Box<dyn std::error::Error>>
{
    let test_repo = TestRepo::init();
    let root = test_repo.repo.store().root_commit_id();
    let change_aa = make_change_id(&test_repo, 0xAA);
    let change_bb = make_change_id(&test_repo, 0xBB);
    let change_cc = make_change_id(&test_repo, 0xCC);

    let tree_base = create_simple_tree(&test_repo.repo, "otherfile", "content: otherfile");
    let tree_x = create_simple_tree(&test_repo.repo, "file", "content: X");
    let tree_y = create_simple_tree(&test_repo.repo, "file", "content: Y");
    let tree1 = create_simple_tree(&test_repo.repo, "file", "content1");
    let tree2 = create_simple_tree(&test_repo.repo, "file", "content2");
    let tree3 = create_simple_tree(&test_repo.repo, "file", "content3");
    let tree4 = create_simple_tree(&test_repo.repo, "file", "content4");

    // First create the parents.
    let mut tx = test_repo.repo.start_transaction();
    let base = tx
        .repo_mut()
        .new_commit(vec![root.clone()], tree_base.clone())
        .set_description("base".to_string())
        .write_unwrap()
        .id()
        .clone();
    let commit_x = tx
        .repo_mut()
        .new_commit(vec![base.clone()], tree_x.clone())
        .set_change_id(change_aa)
        .set_description("X".to_string())
        .write_unwrap();
    let commit_y = tx
        .repo_mut()
        .new_commit(vec![base.clone()], tree_y)
        .set_change_id(change_bb)
        .set_description("Y".to_string())
        .write_unwrap();
    let repo0 = tx.commit("test").block_on().unwrap();

    let mut tx = repo0.start_transaction();
    let commit1 = tx
        .repo_mut()
        .new_commit(vec![commit_x.id().clone()], tree1.clone())
        .set_change_id(change_cc.clone())
        .set_description("foo".to_string())
        .write_unwrap();
    let repo1 = tx.commit("test").block_on().unwrap();

    let mut tx2 = repo1.start_transaction();
    let commit2 = tx2
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .set_parents(vec![commit_y.id().clone()])
        .set_tree(tree2)
        .write_unwrap();
    tx2.repo_mut().rebase_descendants().block_on().unwrap();
    let mut tx3 = repo1.start_transaction();
    let commit3 = tx3
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .set_tree(tree3.clone())
        .write_unwrap();
    tx3.repo_mut().rebase_descendants().block_on().unwrap();
    let repo4 = commit_transactions(vec![tx2, tx3]);

    let mut tx = repo4.start_transaction();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("baz")
        .set_parents(vec![commit_x.id().clone()])
        .set_tree(tree4.clone())
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on().unwrap();
    let repo5 = tx.commit("test").block_on().unwrap();

    let change_id = commit1.change_id().clone();
    let divergent_commits = [commit3.clone(), commit4.clone()];
    assert_divergent_changes(&repo5, &[(&change_id, &divergent_commits)])?;

    let divergent_commit_ids = vec![commit3.id().clone(), commit4.id().clone()];
    let max_evolution_nodes = 100;
    let converge_ui = None;
    let converge_result =
        converge_change(&repo5, converge_ui, &divergent_commits, max_evolution_nodes).block_on()?;

    let expected_tree = create_merged_tree(vec![
        (
            commit1.tree().clone(),
            format!("converge base: {}", commit1.conflict_label()),
        ),
        (
            commit1.tree().clone(),
            format!("converge base: {}", commit1.conflict_label()),
        ),
        (
            commit3.tree().clone(),
            format!("divergent commit: {}", commit3.conflict_label()),
        ),
        (
            commit1.tree().clone(),
            format!("converge base: {}", commit1.conflict_label()),
        ),
        (
            commit4.tree().clone(),
            format!("divergent commit: {}", commit4.conflict_label()),
        ),
    ]);

    let ConvergeResult::Solution(ref solution) = converge_result else {
        unreachable!("unexpected ConvergeResult")
    };

    assert_eq!(solution.change_id, change_id);
    assert_eq!(solution.divergent_commit_ids, divergent_commit_ids.clone());
    assert_eq!(solution.author, commit1.author().clone());
    assert_eq!(solution.description, "baz".to_string());
    assert_eq!(solution.parents, vec![commit_x.id().clone()]);
    assert_eq!(solution.tree_ids, expected_tree.tree_ids().clone());
    assert_eq!(solution.conflict_labels, expected_tree.labels().clone());

    // TODO
    // assert_eq!(
    //     tree_to_string(
    //         test_repo.repo.store(),
    //         &solution.tree_ids,
    //         &solution.conflict_labels
    //     ),
    //     "xyz"
    // );

    let mut tx = repo5.start_transaction();
    let (applied, _) = apply_solution(solution.clone(), true, tx.repo_mut())?;
    let repo = tx.commit("apply solution").block_on().unwrap();
    assert_heads(repo.as_ref(), vec![applied.id(), commit_y.id()]);

    assert_eq!(applied.change_id(), &change_cc);
    assert_eq!(applied.description(), "baz");
    assert_eq!(applied.parent_ids(), &[commit_x.id().clone()]);

    assert_eq!(
        applied
            .tree()
            .path_value(repo_path("file"))
            .block_on()
            .unwrap(),
        Merge::from_removes_adds(
            vec![get_merged_tree_value(&tree1, "file")],
            vec![
                get_merged_tree_value(&tree3, "file"),
                get_merged_tree_value(&tree4, "file"),
            ],
        ),
    );
    assert_eq!(get_predecessors(&repo, applied.id()), divergent_commit_ids);
    Ok(())
}

// Evolution (predecessors are below their successors):
//
// C4("baz", parent:Y, file="content4")
//      |
// C2("bar", parent:Y, file="content2")
//      |
//      |                           C3("bar", parent:X,file="content3")
//       \                                    /
//       C1("foo", parent:X, file="content1")
//
// C1 is rewritten to C2 and C3 in parallel, and then C2 is rewritten to C4. The
// visible commits at the end are base,X,Y,C3,C4. converge is possible without
// user input.
//
// Commit graph:
//
// C1 C3  C2 C4
//  \ /    \ /
//   X      Y
//    \    /
//     base
//
// Expected result (commit graph):
//
//         Solution("baz")
//            |
//    X       Y
//     \     /
//      base
#[test]
fn test_automatic_converge_description_parent_and_trees_with_reparent()
-> Result<(), Box<dyn std::error::Error>> {
    let test_repo = TestRepo::init();
    let root = test_repo.repo.store().root_commit_id();
    let change_aa = make_change_id(&test_repo, 0xAA);
    let change_bb = make_change_id(&test_repo, 0xBB);
    let change_cc = make_change_id(&test_repo, 0xCC);

    let tree_base = create_simple_tree(&test_repo.repo, "otherfile", "content: otherfile");
    let tree_x = create_simple_tree(&test_repo.repo, "file", "content: X");
    let tree_y = create_simple_tree(&test_repo.repo, "file", "content: Y");
    let tree1 = create_simple_tree(&test_repo.repo, "file", "content1");
    let tree2 = create_simple_tree(&test_repo.repo, "file", "content2");
    let tree3 = create_simple_tree(&test_repo.repo, "file", "content3");
    let tree4 = create_simple_tree(&test_repo.repo, "file", "content4");

    // First create the parents.
    let mut tx = test_repo.repo.start_transaction();
    let base = tx
        .repo_mut()
        .new_commit(vec![root.clone()], tree_base.clone())
        .set_description("base".to_string())
        .write_unwrap()
        .id()
        .clone();
    let commit_x = tx
        .repo_mut()
        .new_commit(vec![base.clone()], tree_x.clone())
        .set_change_id(change_aa)
        .set_description("X".to_string())
        .write_unwrap();
    let commit_y = tx
        .repo_mut()
        .new_commit(vec![base.clone()], tree_y)
        .set_change_id(change_bb)
        .set_description("Y".to_string())
        .write_unwrap();
    let repo0 = tx.commit("test").block_on().unwrap();

    let mut tx = repo0.start_transaction();
    let commit1 = tx
        .repo_mut()
        .new_commit(vec![commit_x.id().clone()], tree1.clone())
        .set_change_id(change_cc.clone())
        .set_description("foo".to_string())
        .write_unwrap();
    let repo1 = tx.commit("test").block_on().unwrap();

    let mut tx2 = repo1.start_transaction();
    let commit2 = tx2
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .set_parents(vec![commit_y.id().clone()])
        .set_tree(tree2)
        .write_unwrap();
    tx2.repo_mut().rebase_descendants().block_on().unwrap();
    let mut tx3 = repo1.start_transaction();
    let commit3 = tx3
        .repo_mut()
        .rewrite_commit(&commit1)
        .set_description("bar")
        .set_parents(vec![commit_x.id().clone()])
        .set_tree(tree3.clone())
        .write_unwrap();
    tx3.repo_mut().rebase_descendants().block_on().unwrap();
    let repo4 = commit_transactions(vec![tx2, tx3]);

    let mut tx = repo4.start_transaction();
    let commit4 = tx
        .repo_mut()
        .rewrite_commit(&commit2)
        .set_description("baz")
        .set_parents(vec![commit_y.id().clone()])
        .set_tree(tree4.clone())
        .write_unwrap();
    tx.repo_mut().rebase_descendants().block_on().unwrap();
    let repo5 = tx.commit("test").block_on().unwrap();

    let change_id = commit1.change_id().clone();
    let divergent_commits = [commit3.clone(), commit4.clone()];
    assert_divergent_changes(&repo5, &[(&change_id, &divergent_commits)])?;

    let divergent_commit_ids = vec![commit3.id().clone(), commit4.id().clone()];
    let max_evolution_nodes = 100;
    let converge_ui = None;
    let converge_result =
        converge_change(&repo5, converge_ui, &divergent_commits, max_evolution_nodes).block_on()?;

    let rebased_tree1 = create_merged_tree(vec![
        (
            commit_y.tree().clone(),
            "converge solution parent(s)".to_string(),
        ),
        (
            commit_x.tree().clone(),
            format!(
                "(negated) nnnnnnnn {} \"{}\"",
                &commit_x.id().hex()[0..8],
                commit_x.description()
            ),
        ),
        (
            commit1.tree().clone(),
            format!(
                "nnnnnnnn {} \"{}\"",
                &commit1.id().hex()[0..8],
                commit1.description()
            ),
        ),
    ]);
    let rebased_tree3 = create_merged_tree(vec![
        (
            commit_y.tree().clone(),
            "converge solution parent(s)".to_string(),
        ),
        (
            commit_x.tree().clone(),
            format!(
                "(negated) nnnnnnnn {} \"{}\"",
                &commit_x.id().hex()[0..8],
                commit_x.description()
            ),
        ),
        (
            commit3.tree().clone(),
            format!(
                "nnnnnnnn {} \"{}\"",
                &commit3.id().hex()[0..8],
                commit3.description()
            ),
        ),
    ]);
    let rebased_tree4 = tree4.clone();

    let expected_tree = create_merged_tree(vec![
        (
            rebased_tree1.clone(),
            format!(
                "converge base: nnnnnnnn {} \"{}\"",
                &commit1.id().hex()[0..8],
                commit1.description()
            ),
        ),
        (
            rebased_tree1.clone(),
            format!(
                "converge base: nnnnnnnn {} \"{}\"",
                &commit1.id().hex()[0..8],
                commit1.description()
            ),
        ),
        (
            rebased_tree3.clone(),
            format!(
                "divergent commit: nnnnnnnn {} \"{}\"",
                &commit3.id().hex()[0..8],
                commit3.description()
            ),
        ),
        (
            rebased_tree1.clone(),
            format!(
                "converge base: nnnnnnnn {} \"{}\"",
                &commit1.id().hex()[0..8],
                commit1.description()
            ),
        ),
        (
            rebased_tree4.clone(),
            format!(
                "divergent commit: nnnnnnnn {} \"{}\"",
                &commit4.id().hex()[0..8],
                commit4.description()
            ),
        ),
    ]);

    let ConvergeResult::Solution(ref solution) = converge_result else {
        unreachable!("unexpected ConvergeResult")
    };

    assert_eq!(solution.change_id, change_id);
    assert_eq!(solution.divergent_commit_ids, divergent_commit_ids.clone());
    assert_eq!(solution.author, commit1.author().clone());
    assert_eq!(solution.description, "baz".to_string());
    assert_eq!(solution.parents, vec![commit_y.id().clone()]);
    assert_eq!(solution.tree_ids, expected_tree.tree_ids().clone());
    assert_eq!(solution.conflict_labels, expected_tree.labels().clone());

    // TODO
    // assert_eq!(
    //     tree_to_string(
    //         test_repo.repo.store(),
    //         &solution.tree_ids,
    //         &solution.conflict_labels
    //     ),
    //     "xyz"
    // );

    let mut tx = repo5.start_transaction();
    let (applied, _) = apply_solution(solution.clone(), true, tx.repo_mut())?;
    let repo = tx.commit("apply solution").block_on().unwrap();
    assert_heads(repo.as_ref(), vec![applied.id(), commit_x.id()]);

    assert_eq!(applied.change_id(), &change_cc);
    assert_eq!(applied.description(), "baz");
    assert_eq!(applied.parent_ids(), &[commit_y.id().clone()]);

    assert_eq!(
        applied
            .tree()
            .path_value(repo_path("file"))
            .block_on()
            .unwrap(),
        Merge::from_removes_adds(
            vec![get_merged_tree_value(&tree1, "file")],
            vec![
                get_merged_tree_value(&tree4, "file"),
                get_merged_tree_value(&tree3, "file"),
            ],
        ),
    );
    assert_eq!(get_predecessors(&repo, applied.id()), divergent_commit_ids);
    Ok(())
}
