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
use std::sync::Mutex;

use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::choose_change;
use jj_lib::converge::find_divergent_changes;
use jj_lib::merge::MergeBuilder;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::create_tree_with;
use testutils::repo_path;
use testutils::repo_path_buf;

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

#[expect(dead_code)]
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

#[expect(dead_code)]
pub fn create_simple_tree(repo: &Arc<ReadonlyRepo>, path: &str, content: &str) -> MergedTree {
    create_tree_with(repo, |builder| {
        builder.file(&repo_path_buf(path), content);
    })
}

#[expect(dead_code)]
fn create_merged_tree(terms: Vec<(MergedTree, String)>) -> MergedTree {
    MergedTree::merge(MergeBuilder::from_iter(terms).build())
        .block_on()
        .unwrap()
}

#[expect(dead_code)]
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
    let chosen_change = choose_change(None, &result)?;
    assert_eq!(chosen_change, Some(&change_aa));

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
    let result = find_divergent_changes(&repo, RevsetExpression::all())?;
    let expected = HashMap::from([
        (
            change_aa.clone(),
            HashMap::from([
                (commit_1.id().clone(), commit_1.clone()),
                (commit_2.id().clone(), commit_2.clone()),
            ]),
        ),
        (
            change_bb,
            HashMap::from([
                (commit_3.id().clone(), commit_3.clone()),
                (commit_4.id().clone(), commit_4.clone()),
            ]),
        ),
    ]);
    assert_eq!(result, expected);

    // Since there are multiple divergent change, choose_change() requires a
    // ConvergeUI.
    let chosen_change = choose_change(None, &result)?;
    assert_eq!(chosen_change, None);

    // It does work with a ConvergeUI.
    let mut converge_ui = MockConvergeUI::new();
    converge_ui.chosen_change = Some(change_aa.clone());
    let chosen_change = choose_change(Some(&converge_ui), &result)?;
    assert_eq!(chosen_change, Some(&change_aa));
    let ui_called = *converge_ui.choose_change_called.lock().unwrap();
    assert!(ui_called);

    // Simulate the case where the user aborts the UI by not choosing a change.
    let converge_ui = MockConvergeUI::new();
    let chosen_change = choose_change(Some(&converge_ui), &result)?;
    assert_eq!(chosen_change, None);
    let ui_called = *converge_ui.choose_change_called.lock().unwrap();
    assert!(ui_called);

    Ok(())
}
