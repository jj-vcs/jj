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

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::backend::MergedTreeId;
use jj_lib::fix::fix_files;
use jj_lib::fix::FileFixer;
use jj_lib::fix::FileToFix;
use jj_lib::fix::FixError;
use jj_lib::fix::FixResult;
use jj_lib::fix::ParallelFileFixer;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::create_tree;
use testutils::TestRepo;
use thiserror::Error;

struct TestFileFixer {}

impl TestFileFixer {
    fn new() -> Self {
        Self {}
    }
}

// A file fixer that changes files to uppercase if the file content starts with
// "fixme", returns an error if the content starts with "error", and otherwise
// leaves files unchanged.
impl FileFixer for TestFileFixer {
    fn fix_files<'a>(
        &self,
        store: &Store,
        files_to_fix: &'a HashSet<FileToFix>,
    ) -> Result<HashMap<&'a FileToFix, FixResult>, FixError> {
        let mut results = HashMap::new();
        for file_to_fix in files_to_fix {
            let result = fix_file(store, file_to_fix)?;
            results.insert(file_to_fix, result);
        }
        Ok(results)
    }
}

#[derive(Error, Debug)]
#[error("Forced failure: {0}")]
struct MyFixerError(String);

fn make_fix_content_error(message: &str) -> FixError {
    FixError::FixContent(Box::new(MyFixerError(message.into())))
}

// Reads the file from store. If the file starts with "fixme", its contents are
// changed to uppercase and the new file id is returned. If the file starts with
// "error", an error is raised. Otherwise returns None.
fn fix_file(store: &Store, file_to_fix: &FileToFix) -> Result<FixResult, FixError> {
    let mut old_content = vec![];
    let mut read = store
        .read_file(&file_to_fix.repo_path, &file_to_fix.file_id)
        .unwrap();
    read.read_to_end(&mut old_content).unwrap();

    if let Some(rest) = old_content.strip_prefix(b"fixme:") {
        let new_content = rest.to_ascii_uppercase();
        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()
            .unwrap();
        Ok(FixResult {
            file_id: Some(new_file_id),
            messages: vec![],
        })
    } else if let Some(rest) = old_content.strip_prefix(b"error:") {
        Err(make_fix_content_error(std::str::from_utf8(rest).unwrap()))
    } else {
        Ok(FixResult {
            file_id: None,
            messages: vec![],
        })
    }
}

fn create_tree_helper(
    repo: &Arc<ReadonlyRepo>,
    path_and_content: &[(String, String)],
) -> MergedTree {
    let content_map = path_and_content
        .iter()
        .map(|p| (RepoPath::from_internal_string(&p.0), p.1.as_str()))
        .collect_vec();
    create_tree(repo, &content_map)
}

fn create_commit(tx: &mut Transaction, parents: Vec<CommitId>, tree_id: MergedTreeId) -> CommitId {
    tx.repo_mut()
        .new_commit(parents, tree_id)
        .write()
        .unwrap()
        .id()
        .clone()
}

#[test]
fn test_fix_one_file() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    let expected_tree_a = create_tree(repo, &[(path1, "CONTENT")]);
    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_a.tree_id(), expected_tree_a.id());
}

#[test]
fn test_fixer_does_not_change_content() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
}

#[test]
fn test_empty_commit() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let tree1 = create_tree(repo, &[]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
}

#[test]
fn test_fixer_fails() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "error:boo")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    );

    let error = result.err().unwrap();
    assert_eq!(error.to_string(), "Forced failure: boo");
}

#[test]
fn test_unchanged_file_is_not_fixed() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let tree2 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.id());

    let root_commits = vec![commit_b.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
}

#[test]
fn test_unchanged_file_is_fixed() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let tree2 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.id());

    let root_commits = vec![commit_b.clone()];
    let file_fixer = TestFileFixer::new();

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        true,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    let expected_tree_b = create_tree(repo, &[(path1, "CONTENT")]);
    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_b));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_b).unwrap())
        .unwrap();
    assert_eq!(*new_commit_b.tree_id(), expected_tree_b.id());
}

/// If a descendant is already correctly formatted, it should still be rewritten
/// but its tree should be preserved.
#[test]
fn test_already_fixed_descendant() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let tree2 = create_tree(repo, &[(path1, "CONTENT")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.id());

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        true,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert!(summary.rewrites.contains_key(&commit_b));
    assert_eq!(summary.num_checked_commits, 2);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_a.tree_id(), tree2.id());
    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_b.tree_id(), tree2.id());
}

#[test]
fn test_parallel_fixer_basic() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &parallel_fixer,
    )
    .unwrap();

    let expected_tree_a = create_tree(repo, &[(path1, "CONTENT")]);
    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_a.tree_id(), expected_tree_a.id());
}

#[test]
fn test_parallel_fixer_fixes_files() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut path_contents1 = vec![];
    for i in 0..100 {
        let path = format!("file{i}");
        let content = format!("fixme:content{i}");
        path_contents1.push((path, content));
    }
    let tree1 = create_tree_helper(repo, &path_contents1);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &parallel_fixer,
    )
    .unwrap();

    let mut expected_path_contents = vec![];
    for i in 0..100 {
        let path = format!("file{i}");
        let content = format!("CONTENT{i}");
        expected_path_contents.push((path, content));
    }
    let expected_tree_a = create_tree_helper(repo, &expected_path_contents);

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_a.tree_id(), expected_tree_a.id());
}

#[test]
fn test_parallel_fixer_does_not_change_content() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut path_contents1 = vec![];
    for i in 0..100 {
        let path = format!("file{i}");
        let content = format!("content{i}");
        path_contents1.push((path, content));
    }
    let tree1 = create_tree_helper(repo, &path_contents1);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &parallel_fixer,
    )
    .unwrap();

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
}

#[test]
fn test_parallel_fixer_no_changes_upon_partial_failure() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let mut path_contents1 = vec![];
    for i in 0..100 {
        let path = format!("file{i}");
        let content = if i == 7 {
            format!("error:boo{i}")
        } else if i % 3 == 0 {
            format!("fixme:content{i}")
        } else {
            format!("foobar:{i}")
        };
        path_contents1.push((path, content));
    }
    let tree1 = create_tree_helper(repo, &path_contents1);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let parallel_fixer = ParallelFileFixer::new(fix_file);

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &parallel_fixer,
    );
    let error = result.err().unwrap();
    assert_eq!(error.to_string(), "Forced failure: boo7");
}

#[test]
fn test_fix_multiple_revisions() {
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
    let path1 = RepoPath::from_internal_string("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:xyz")]);
    let commit_a = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.id(),
    );
    let path2 = RepoPath::from_internal_string("file2");
    let tree2 = create_tree(repo, &[(path2, "content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.id());
    let path3 = RepoPath::from_internal_string("file3");
    let tree3 = create_tree(repo, &[(path3, "content")]);
    let _commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree3.id());
    let path4 = RepoPath::from_internal_string("file4");
    let tree4 = create_tree(repo, &[(path4, "content")]);
    let _commit_d = create_commit(&mut tx, vec![commit_a.clone()], tree4.id());

    let root_commits = vec![commit_a.clone()];
    let file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &file_fixer,
    )
    .unwrap();

    let expected_tree_a = create_tree(repo, &[(path1, "XYZ")]);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_eq!(*new_commit_a.tree_id(), expected_tree_a.id());
}
