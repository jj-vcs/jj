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

use std::sync::Mutex;

use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::commit::Commit;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::propose_divergence_solution;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;

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
    ) -> Result<&'a ChangeId, ConvergeError> {
        *self.choose_change_called.lock().unwrap() = true;
        let Some(ref change_id) = self.chosen_change else {
            return Err(ConvergeError::UserAborted());
        };
        match divergent_changes.keys().find(|k| *k == change_id) {
            Some(change_id) => Ok(change_id),
            None => Err(ConvergeError::Other(
                format!("MockConvergeUI error: {change_id:.12} not in divergent changes").into(),
            )),
        }
    }

    fn choose_author(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Signature, ConvergeError> {
        *self.choose_author_called.lock().unwrap() = true;
        self.chosen_author
            .clone()
            .ok_or(ConvergeError::NeedUserInput())
    }

    fn choose_parents(
        &self,
        _divergent_commits: &[Commit],
    ) -> Result<Vec<CommitId>, ConvergeError> {
        *self.choose_parents_called.lock().unwrap() = true;
        self.chosen_parents
            .clone()
            .ok_or(ConvergeError::NeedUserInput())
    }

    fn merge_description(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<String, ConvergeError> {
        *self.merge_description_called.lock().unwrap() = true;
        self.merged_description
            .clone()
            .ok_or(ConvergeError::NeedUserInput())
    }
}

#[test]
fn test_no_divergent_changes() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let store = repo.store();
    let empty_tree = store.empty_merged_tree();
    let mut tx = repo.start_transaction();

    let author = Signature {
        name: "author1".to_string(),
        email: "author1".to_string(),
        timestamp: Timestamp::now(),
    };

    let mut create_commit = |parents, tree: MergedTree, author, desc, change_id| {
        let builder = tx
            .repo_mut()
            .new_commit(parents, tree.clone())
            .set_author(author)
            .set_description(desc)
            .set_tree(tree);
        match change_id {
            Some(change_id) => builder.set_change_id(change_id),
            None => builder,
        }
        .write_unwrap()
    };

    let _commit_1 = create_commit(
        vec![store.root_commit_id().clone()],
        empty_tree.clone(),
        author.clone(),
        "commit 1",
        None,
    );
    let _commit_2 = create_commit(
        vec![store.root_commit_id().clone()],
        empty_tree.clone(),
        author.clone(),
        "commit 2",
        None,
    );

    let repo = tx.commit("test").block_on().unwrap();

    let ui = MockConvergeUI::new();
    let result = propose_divergence_solution(&repo, &ui, RevsetExpression::all(), 100).block_on();
    assert!(matches!(result, Err(ConvergeError::NoDivergentChanges())));
}

// fn setup_simple_divergence(
//     description1: &str,
//     description2: &str,
//     use_author1_for_both: bool,
// ) -> DivergenceSetup {
//     let test_repo = TestRepo::init();
//     let mut tx = test_repo.repo.start_transaction();

//     let author1 = Signature {
//         name: "author1".to_string(),
//         email: "author1".to_string(),
//         timestamp: tx.repo_mut().op_id().timestamp.clone(),
//     };
//     let author2 = Signature {
//         name: "author2".to_string(),
//         email: "author2".to_string(),
//         timestamp: tx.repo_mut().op_id().timestamp.clone(),
//     };

//     let fork_point = test_repo.commit_with_description(tx.repo_mut(),
// "fork_point");     tx.repo_mut()
//         .edit(fork_point.id(), &WorkspaceId::default())
//         .unwrap();

//     let mut divergent1_builder = tx
//         .repo_mut()
//         .rewrite_commit(&fork_point)
//         .set_author(author1.clone())
//         .set_description(description1.to_string());
//     let divergent1 = divergent1_builder.write().unwrap();

//     let author_for_2 = if use_author1_for_both {
//         author1.clone()
//     } else {
//         author2.clone()
//     };
//     let mut divergent2_builder = tx
//         .repo_mut()
//         .rewrite_commit(&fork_point)
//         .set_author(author_for_2)
//         .set_description(description2.to_string());
//     let divergent2 = divergent2_builder.write().unwrap();

//     let repo = tx.commit("test");
//     DivergenceSetup {
//         repo,
//         test_repo,
//         author1,
//         author2,
//         fork_point,
//         divergent1,
//         divergent2,
//     }
// }

// #[test]
// fn test_simple_divergence_auto_resolve_all() {
//     let setup = setup_simple_divergence("description", "description", true);
//     let workspace = Workspace::for_test(&setup.repo, "@", "test").unwrap();
//     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset = setup
//         .repo
//         .evaluate_revset(revset_expression.as_ref())
//         .unwrap();

//     let ui = MockConvergeUI::new();
//     let result = propose_divergence_solution(&setup.repo, &ui, revset,
// 100).block_on();

//     let solution = result.unwrap();
//     assert_eq!(solution.change_id, *setup.divergent1.change_id());
//     assert_eq!(
//         solution.divergent_commit_ids,
//         vec![setup.divergent1.id().clone(), setup.divergent2.id().clone()]
//     );
//     assert_eq!(solution.author, *setup.divergent1.author());
//     assert_eq!(solution.description, *setup.divergent1.description());

//     assert!(!*ui.choose_change_called.lock().unwrap());
//     assert!(!*ui.choose_author_called.lock().unwrap());
//     assert!(!*ui.choose_parents_called.lock().unwrap());
//     assert!(!*ui.merge_description_called.lock().unwrap());
// }

// #[test]
// fn test_simple_divergence_ui_resolves_description() {
//     let setup = setup_simple_divergence("description1", "description2",
// true);     let workspace = Workspace::for_test(&setup.repo, "@",
// "test").unwrap();     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset = setup
//         .repo
//         .evaluate_revset(revset_expression.as_ref())
//         .unwrap();

//     let mut ui = MockConvergeUI::new();
//     ui.merged_description = Some("merged_description".to_string());
//     let result = propose_divergence_solution(&setup.repo, &ui, revset,
// 100).block_on();

//     let solution = result.unwrap();
//     assert_eq!(solution.description, "merged_description");

//     assert!(!*ui.choose_change_called.lock().unwrap());
//     assert!(!*ui.choose_author_called.lock().unwrap());
//     assert!(!*ui.choose_parents_called.lock().unwrap());
//     assert!(*ui.merge_description_called.lock().unwrap());
// }

// #[test]
// fn test_simple_divergence_ui_resolves_author() {
//     let setup = setup_simple_divergence("description", "description", false);
//     let workspace = Workspace::for_test(&setup.repo, "@", "test").unwrap();
//     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset = setup
//         .repo
//         .evaluate_revset(revset_expression.as_ref())
//         .unwrap();

//     let mut ui = MockConvergeUI::new();
//     let op = setup.repo.operation();
//     let chosen_author = Signature {
//         name: "chosen".to_string(),
//         email: "chosen".to_string(),
//         timestamp: op.id().timestamp.clone(),
//     };
//     ui.chosen_author = Some(chosen_author.clone());
//     let result = propose_divergence_solution(&setup.repo, &ui, revset,
// 100).block_on();

//     let solution = result.unwrap();
//     assert_eq!(solution.author, chosen_author);

//     assert!(!*ui.choose_change_called.lock().unwrap());
//     assert!(*ui.choose_author_called.lock().unwrap());
//     assert!(!*ui.choose_parents_called.lock().unwrap());
//     assert!(!*ui.merge_description_called.lock().unwrap());
// }

// #[test]
// fn test_multiple_divergent_changes() {
//     let setup1 = setup_simple_divergence("description", "description", true);
//     let mut tx = setup1.repo.start_transaction();
//     let setup2_fork = setup1
//         .test_repo
//         .commit_with_description(tx.repo_mut(), "fork_point2");
//     tx.repo_mut()
//         .edit(setup2_fork.id(), &WorkspaceId::default())
//         .unwrap();
//     let _divergent3 = tx
//         .repo_mut()
//         .rewrite_commit(&setup2_fork)
//         .set_description("description3".to_string())
//         .write()
//         .unwrap();
//     let _divergent4 = tx
//         .repo_mut()
//         .rewrite_commit(&setup2_fork)
//         .set_description("description4".to_string())
//         .write()
//         .unwrap();
//     let repo = tx.commit("test");

//     let workspace = Workspace::for_test(&repo, "@", "test").unwrap();
//     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset =
// repo.evaluate_revset(revset_expression.as_ref()).unwrap();

//     let mut ui = MockConvergeUI::new();
//     ui.chosen_change = Some(setup1.divergent1.change_id().clone());
//     let result = propose_divergence_solution(&repo, &ui, revset,
// 100).block_on();

//     let solution = result.unwrap();
//     assert_eq!(solution.change_id, *setup1.divergent1.change_id());

//     assert!(*ui.choose_change_called.lock().unwrap());
// }

// #[test]
// fn test_safety_limit() {
//     let setup = setup_simple_divergence("description", "description", true);
//     let workspace = Workspace::for_test(&setup.repo, "@", "test").unwrap();
//     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset = setup
//         .repo
//         .evaluate_revset(revset_expression.as_ref())
//         .unwrap();

//     let ui = MockConvergeUI::new();
//     // There are 3 commits in the history of the divergent change
// (fork_point,     // divergent1, divergent2).
//     let result = propose_divergence_solution(&setup.repo, &ui, revset,
// 2).block_on();

//     assert!(matches!(
//         result,
//         Err(ConvergeError::TooManyCommitsInChangeEvolution())
//     ));
// }

// #[test]
// fn test_tree_merging() {
//     let test_repo = TestRepo::init();
//     let mut tx = test_repo.repo.start_transaction();

//     let fork_point = test_repo.commit_with_description(tx.repo_mut(),
// "fork_point");     tx.repo_mut()
//         .edit(fork_point.id(), &WorkspaceId::default())
//         .unwrap();

//     let mut divergent1_builder = tx.repo_mut().rewrite_commit(&fork_point);
//     let tree1 = test_repo
//         .repo
//         .store()
//         .tree_builder(fork_point.tree_id().clone());
//     let mut tree1 = tree1
//         .write_tree_at(vec!["a".into()], test_repo.repo.store())
//         .unwrap();
//     tree1
//         .set_file_at("a/file".into(), "content1".into())
//         .unwrap();
//     let tree1_id = tree1.write_to_repo(test_repo.repo.store()).unwrap();
//     divergent1_builder.set_tree_id(tree1_id);
//     let _divergent1 = divergent1_builder.write().unwrap();

//     let mut divergent2_builder = tx.repo_mut().rewrite_commit(&fork_point);
//     let tree2 = test_repo
//         .repo
//         .store()
//         .tree_builder(fork_point.tree_id().clone());
//     let mut tree2 = tree2
//         .write_tree_at(vec!["b".into()], test_repo.repo.store())
//         .unwrap();
//     tree2
//         .set_file_at("b/file".into(), "content2".into())
//         .unwrap();
//     let tree2_id = tree2.write_to_repo(test_repo.repo.store()).unwrap();
//     divergent2_builder.set_tree_id(tree2_id);
//     let _divergent2 = divergent2_builder.write().unwrap();

//     let repo = tx.commit("test");

//     let workspace = Workspace::for_test(&repo, "@", "test").unwrap();
//     let context = RevsetParseContext {
//         workspace: Some(&workspace),
//         user_email: "test".to_string(),
//     };
//     let mut diagnostics = RevsetDiagnostics::new();
//     let revset_expression = revset::parse(&mut diagnostics, "all()",
// &context).unwrap();     let revset =
// repo.evaluate_revset(revset_expression.as_ref()).unwrap();

//     let ui = MockConvergeUI::new();
//     let result = propose_divergence_solution(&repo, &ui, revset,
// 100).block_on();

//     let solution = result.unwrap();
//     let merged_tree = solution.tree;
//     assert_eq!(
//         test_repo.read_file(&merged_tree, "a/file"),
//         Some("content1".into())
//     );
//     assert_eq!(
//         test_repo.read_file(&merged_tree, "b/file"),
//         Some("content2".into())
//     );
// }
