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

use indexmap::IndexSet;
use indoc::indoc;
use jj_lib::backend::CommitId;
use jj_lib::backend::FileId;
use jj_lib::commit::Commit;
use jj_lib::fix::FileFixer;
use jj_lib::fix::FileToFix;
use jj_lib::fix::FixError;
use jj_lib::fix::LineRange;
use jj_lib::fix::ParallelFileFixer;
use jj_lib::fix::RegionsToFormat;
use jj_lib::fix::compute_changed_ranges;
use jj_lib::fix::fix_files;
use jj_lib::fix::get_base_commit_map;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::assert_tree_eq;
use testutils::create_tree;
use testutils::create_tree_with;
use testutils::read_file;
use testutils::repo_path;
use thiserror::Error;

type ReplacementKey = (RepoPathBuf, Option<Vec<u8>>, Vec<u8>);

#[derive(Clone, Debug)]
struct TestFileFixer {
    replacements: HashMap<ReplacementKey, Vec<u8>>,
}

impl TestFileFixer {
    fn new() -> Self {
        Self {
            replacements: HashMap::new(),
        }
    }

    fn add_replacement(
        &mut self,
        repo_path: &RepoPath,
        base_content: Option<&[u8]>,
        old_content: impl AsRef<[u8]>,
        new_content: impl AsRef<[u8]>,
    ) {
        self.replacements.insert(
            (
                repo_path.to_owned(),
                base_content.map(|c| c.to_vec()),
                old_content.as_ref().to_vec(),
            ),
            new_content.as_ref().to_vec(),
        );
    }

    fn fix_file(&mut self, store: &Store, file_to_fix: &FileToFix) -> Result<FileId, FixError> {
        let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);
        let base_content = file_to_fix
            .base_file_id
            .as_ref()
            .map(|base_file_id| read_file(store, &file_to_fix.repo_path, base_file_id));

        let key = (
            file_to_fix.repo_path.clone(),
            base_content,
            old_content.clone(),
        );
        let Some(new_content) = self.replacements.remove(&key) else {
            return Err(make_fix_content_error(&format!(
                indoc! {"
                    Unexpected fix request:
                    path: {}
                    old_content: {:?}
                "},
                file_to_fix.repo_path.as_internal_file_string(),
                String::from_utf8_lossy(&old_content)
            )));
        };

        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()
            .unwrap();
        Ok(new_file_id)
    }
}

impl FileFixer for TestFileFixer {
    fn fix_files<'a>(
        &mut self,
        store: &Store,
        files_to_fix: &'a HashSet<FileToFix>,
    ) -> Result<HashMap<&'a FileToFix, FileId>, FixError> {
        let mut changed_files = HashMap::new();
        for file_to_fix in files_to_fix {
            let new_file_id = self.fix_file(store, file_to_fix)?;
            changed_files.insert(file_to_fix, new_file_id);
        }
        assert!(self.replacements.is_empty());
        Ok(changed_files)
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
//
// This is used for testing `ParallelFileFixer`.
fn fix_file(store: &Store, file_to_fix: &FileToFix) -> Result<Option<FileId>, FixError> {
    let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);

    if let Some(rest) = old_content.strip_prefix(b"fixme:") {
        let new_content = rest.to_ascii_uppercase();
        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()?;
        Ok(Some(new_file_id))
    } else if let Some(rest) = old_content.strip_prefix(b"error:") {
        Err(make_fix_content_error(str::from_utf8(rest).unwrap()))
    } else {
        Ok(None)
    }
}

fn create_commit(tx: &mut Transaction, parents: Vec<CommitId>, tree: MergedTree) -> CommitId {
    tx.repo_mut()
        .new_commit(parents, tree)
        .write_unwrap()
        .id()
        .clone()
}

fn line_range(first: usize, last: usize) -> LineRange {
    LineRange::new(first, last)
}

#[test]
fn test_fix_added_and_modified_files() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");

    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path1, "modified")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;
    file_fixer.add_replacement(path1, None, b"unformatted", b"Formatted");
    file_fixer.add_replacement(path1, None, b"modified", b"Formatted");

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert!(summary.rewrites.contains_key(&commit_b));
    assert_eq!(summary.num_checked_commits, 2);
    assert_eq!(summary.num_fixed_commits, 2);

    let expected_tree = create_tree(repo, &[(path1, "Formatted")]);
    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_tree_eq!(new_commit_a.tree(), expected_tree);

    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_b).unwrap())
        .unwrap();
    assert_tree_eq!(new_commit_b.tree(), expected_tree);
    Ok(())
}

#[test]
fn test_fixer_does_not_change_content() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "Formatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;
    file_fixer.add_replacement(path1, None, b"Formatted", b"Formatted");

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
    Ok(())
}

#[test]
fn test_empty_commit() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let tree1 = create_tree(repo, &[]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
    Ok(())
}

#[test]
fn test_fix_empty_revset() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction();

    let mut file_fixer = TestFileFixer::new();
    let summary = fix_files(
        vec![],
        &EverythingMatcher,
        false,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert!(summary.rewrites.is_empty());
    Ok(())
}

#[test]
fn test_fixer_fails() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on();

    let error = result.err().unwrap();
    assert_eq!(
        error.to_string(),
        indoc! {r#"
            Forced failure: Unexpected fix request:
            path: file1
            old_content: "unformatted"
        "#}
    );
    Ok(())
}

#[test]
fn test_unchanged_file_is_not_fixed() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");

    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);

    let root_commits = vec![commit_b.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
    Ok(())
}

#[test]
fn test_fix_include_unchanged_files() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path_changed = repo_path("changed.txt");
    let path_unchanged = repo_path("unchanged.txt");
    let path_new = repo_path("newfile.txt");

    // c1: changed.txt, unchanged.txt
    let tree1 = create_tree(
        repo,
        &[(path_changed, "base"), (path_unchanged, "unformatted 2")],
    );
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: changed.txt, unchanged.txt, newfile.txt
    let tree2 = create_tree(
        repo,
        &[
            (path_changed, "unformatted 1"),
            (path_unchanged, "unformatted 2"),
            (path_new, "unformatted 3"),
        ],
    );
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on commit 2 with include_unchanged_files = true.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(
        path_changed,
        Some(b"base"),
        b"unformatted 1",
        b"Formatted 1",
    );
    file_fixer.add_replacement(
        path_unchanged,
        Some(b"unformatted 2"),
        b"unformatted 2",
        b"sorted includes\nunformatted 2",
    );
    file_fixer.add_replacement(path_new, None, b"unformatted 3", b"Formatted 3");

    let include_unchanged_files = true;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c2));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();

    let expected_tree = create_tree(
        repo,
        &[
            (path_changed, "Formatted 1"),
            (path_unchanged, "sorted includes\nunformatted 2"),
            (path_new, "Formatted 3"),
        ],
    );
    assert_tree_eq!(new_c2.tree(), expected_tree);
    Ok(())
}

/// If a descendant is already correctly formatted, it should still be rewritten
/// but its tree should be preserved.
#[test]
fn test_already_fixed_descendant() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");

    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path1, "Formatted")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.clone());

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = true;
    file_fixer.add_replacement(path1, None, b"unformatted", b"Formatted");
    file_fixer.add_replacement(path1, None, b"Formatted", b"Formatted");

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert!(summary.rewrites.contains_key(&commit_b));
    assert_eq!(summary.num_checked_commits, 2);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())?;
    assert_tree_eq!(new_commit_a.tree(), tree2);
    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())?;
    assert_tree_eq!(new_commit_b.tree(), tree2);
    Ok(())
}

#[test]
fn test_parallel_fixer_basic() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let mut parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on()?;

    let expected_tree_a = create_tree(repo, &[(path1, "CONTENT")]);
    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())?;
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
    Ok(())
}

#[test]
fn test_parallel_fixer_fixes_files() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let tree1 = create_tree_with(repo, |builder| {
        for i in 0..100 {
            builder.file(repo_path(&format!("file{i}")), format!("fixme:content{i}"));
        }
    });
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let mut parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on()?;

    let expected_tree_a = create_tree_with(repo, |builder| {
        for i in 0..100 {
            builder.file(repo_path(&format!("file{i}")), format!("CONTENT{i}"));
        }
    });

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&commit_a));
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 1);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())?;
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
    Ok(())
}

#[test]
fn test_parallel_fixer_does_not_change_content() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let tree1 = create_tree_with(repo, |builder| {
        for i in 0..100 {
            builder.file(repo_path(&format!("file{i}")), format!("content{i}"));
        }
    });
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let mut parallel_fixer = ParallelFileFixer::new(fix_file);

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on()?;

    assert!(summary.rewrites.is_empty());
    assert_eq!(summary.num_checked_commits, 1);
    assert_eq!(summary.num_fixed_commits, 0);
    Ok(())
}

#[test]
fn test_parallel_fixer_no_changes_upon_partial_failure() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let tree1 = create_tree_with(repo, |builder| {
        for i in 0..100 {
            let contents = if i == 7 {
                format!("error:boo{i}")
            } else if i % 3 == 0 {
                format!("fixme:content{i}")
            } else {
                format!("foobar:{i}")
            };

            builder.file(repo_path(&format!("file{i}")), &contents);
        }
    });
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let mut parallel_fixer = ParallelFileFixer::new(fix_file);

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on();
    let error = result.err().unwrap();
    assert_eq!(error.to_string(), "Forced failure: boo7");
    Ok(())
}

#[test]
fn test_fix_multiple_revisions() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // D
    // | C
    // | B
    // |/
    // A
    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let path2 = repo_path("file2");
    let tree2 = create_tree(repo, &[(path2, "unformatted")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);

    let path3 = repo_path("file3");
    let tree3 = create_tree(repo, &[(path3, "unformatted")]);
    let commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree3);

    let path4 = repo_path("file4");
    let tree4 = create_tree(repo, &[(path4, "unformatted")]);
    let commit_d = create_commit(&mut tx, vec![commit_a.clone()], tree4);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;
    file_fixer.add_replacement(path1, None, b"unformatted", b"Formatted");
    file_fixer.add_replacement(path2, None, b"unformatted", b"Formatted");
    file_fixer.add_replacement(path3, None, b"unformatted", b"Formatted");
    file_fixer.add_replacement(path4, None, b"unformatted", b"Formatted");

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()?;

    let expected_tree_a = create_tree(repo, &[(path1, "Formatted")]);
    let expected_tree_b = create_tree(repo, &[(path2, "Formatted")]);
    let expected_tree_c = create_tree(repo, &[(path3, "Formatted")]);
    let expected_tree_d = create_tree(repo, &[(path4, "Formatted")]);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_b).unwrap())
        .unwrap();
    let new_commit_c = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_c).unwrap())
        .unwrap();
    let new_commit_d = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_d).unwrap())
        .unwrap();

    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
    assert_tree_eq!(new_commit_b.tree(), expected_tree_b);
    assert_tree_eq!(new_commit_c.tree(), expected_tree_c);
    assert_tree_eq!(new_commit_d.tree(), expected_tree_d);
    Ok(())
}

#[test]
fn test_get_base_commit_map_chain() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // We have a chain of commits.
    //
    // D
    // |
    // C
    // |
    // B
    // |
    // A
    let mut tx = repo.start_transaction();
    let path = repo_path("file1");
    let tree1 = create_tree(repo, &[(path, "commit 1: content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path, "commit 2: content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);
    let tree3 = create_tree(repo, &[(path, "commit 3: content")]);
    let commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree3);
    let tree4 = create_tree(repo, &[(path, "commit 4: content")]);
    let commit_d = create_commit(&mut tx, vec![commit_c.clone()], tree4);

    let commit_b_obj = repo.store().get_commit(&commit_b).unwrap();
    let commit_c_obj = repo.store().get_commit(&commit_c).unwrap();
    let commit_d_obj = repo.store().get_commit(&commit_d).unwrap();

    // Commits are expected to be sorted in child to parent order.
    let commits: Vec<Commit> = vec![commit_d_obj, commit_c_obj, commit_b_obj];
    let base_commit_map = get_base_commit_map(&commits).block_on().unwrap();

    let parents_set = IndexSet::from([commit_a]);
    let expected_base_commit_map: HashMap<CommitId, IndexSet<CommitId>> = HashMap::from([
        (commit_d, parents_set.clone()),
        (commit_c, parents_set.clone()),
        (commit_b, parents_set.clone()),
    ]);

    assert_eq!(base_commit_map, expected_base_commit_map);
    Ok(())
}

#[test]
fn test_fix_complex_merge_with_base_map() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // We have a merge of commits
    //     E
    //    / \
    //   C   D
    //   | \ |
    //   A   B (roots)
    let mut tx = repo.start_transaction();
    let path = repo_path("file1");

    let tree1 = create_tree(repo, &[(path, "base A")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path, "base B")]);
    let commit_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree2);
    let tree3 = create_tree(repo, &[(path, "unformatted C")]);
    let commit_c = create_commit(&mut tx, vec![commit_a.clone(), commit_b.clone()], tree3);
    let tree4 = create_tree(repo, &[(path, "base D")]);
    let commit_d = create_commit(&mut tx, vec![commit_b.clone()], tree4);
    let tree5 = create_tree(repo, &[(path, "unformatted E")]);
    let commit_e = create_commit(&mut tx, vec![commit_c.clone(), commit_d.clone()], tree5);

    let commit_c_obj = repo.store().get_commit(&commit_c).unwrap();
    let commit_e_obj = repo.store().get_commit(&commit_e).unwrap();

    let commits: Vec<Commit> = vec![commit_e_obj, commit_c_obj];
    let base_commit_map = get_base_commit_map(&commits).block_on().unwrap();

    // Should be {e: {a, b, d}, c: {a, b}}
    let expected_base_commit_map: HashMap<CommitId, IndexSet<CommitId>> = HashMap::from([
        (
            commit_e.clone(),
            IndexSet::from([commit_a.clone(), commit_b.clone(), commit_d.clone()]),
        ),
        (
            commit_c.clone(),
            IndexSet::from([commit_a.clone(), commit_b.clone()]),
        ),
    ]);
    assert_eq!(base_commit_map, expected_base_commit_map);

    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"base A"), b"unformatted C", b"Formatted C");
    file_fixer.add_replacement(path, Some(b"base A"), b"unformatted E", b"Formatted E");

    let include_unchanged_files = false;
    let summary = fix_files(
        vec![commit_e.clone(), commit_c.clone()],
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    // C and E should be fixed.
    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&commit_c));
    assert!(summary.rewrites.contains_key(&commit_e));

    let new_commit_c = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_c).unwrap())
        .unwrap();
    let expected_tree_c = create_tree(repo, &[(path, "Formatted C")]);
    assert_tree_eq!(new_commit_c.tree(), expected_tree_c);

    let new_commit_e = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_e).unwrap())
        .unwrap();
    let expected_tree_e = create_tree(repo, &[(path, "Formatted E")]);
    assert_tree_eq!(new_commit_e.tree(), expected_tree_e);
    Ok(())
}

#[test]
fn test_fix_diamond_merge_with_base_map() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // We have a diamond merge:
    //
    //   E
    //  / \
    // C   D
    //  \ /
    //   B
    let mut tx = repo.start_transaction();
    let path = repo_path("file1");

    let tree1 = create_tree(repo, &[(path, "base B")]);
    let commit_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path, "unformatted C")]);
    let commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree2);
    let tree3 = create_tree(repo, &[(path, "base D")]);
    let commit_d = create_commit(&mut tx, vec![commit_b.clone()], tree3);
    let tree4 = create_tree(repo, &[(path, "unformatted E")]);
    let commit_e = create_commit(&mut tx, vec![commit_c.clone(), commit_d.clone()], tree4);

    let commit_c_obj = repo.store().get_commit(&commit_c).unwrap();
    let commit_e_obj = repo.store().get_commit(&commit_e).unwrap();

    // We are fixing e and c.
    let commits: Vec<Commit> = vec![commit_e_obj, commit_c_obj];
    let base_commit_map = get_base_commit_map(&commits).block_on().unwrap();

    // Should be {e: {b, d}, c: {b}}
    let expected_base_commit_map: HashMap<CommitId, IndexSet<CommitId>> = HashMap::from([
        (
            commit_e.clone(),
            IndexSet::from([commit_b.clone(), commit_d.clone()]),
        ),
        (commit_c.clone(), IndexSet::from([commit_b.clone()])),
    ]);
    assert_eq!(base_commit_map, expected_base_commit_map);

    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"base B"), b"unformatted C", b"Formatted C");
    file_fixer.add_replacement(path, Some(b"base D"), b"unformatted E", b"Formatted E");

    let include_unchanged_files = false;
    let summary = fix_files(
        vec![commit_e.clone(), commit_c.clone()],
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    // C and E should be fixed.
    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&commit_c));
    assert!(summary.rewrites.contains_key(&commit_e));

    let new_commit_c = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_c).unwrap())
        .unwrap();
    let expected_tree_c = create_tree(repo, &[(path, "Formatted C")]);
    assert_tree_eq!(new_commit_c.tree(), expected_tree_c);

    let new_commit_e = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_e).unwrap())
        .unwrap();
    let expected_tree_e = create_tree(repo, &[(path, "Formatted E")]);
    assert_tree_eq!(new_commit_e.tree(), expected_tree_e);
    Ok(())
}

#[test]
fn test_fix_sequence_formatted_from_base() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "base"
    let tree1 = create_tree(repo, &[(path, "base")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: "unformatted 1\n"
    let tree2 = create_tree(repo, &[(path, "unformatted 1")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "unformatted 1\nunformatted 2\n"
    let tree3 = create_tree(repo, &[(path, "unformatted 2")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"base"), b"unformatted 1", b"Formatted 1");
    file_fixer.add_replacement(path, Some(b"base"), b"unformatted 2", b"Formatted 2");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&c2));
    assert!(summary.rewrites.contains_key(&c3));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();
    let expected_tree_c2 = create_tree(repo, &[(path, "Formatted 1")]);
    assert_tree_eq!(new_c2.tree(), expected_tree_c2);

    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let expected_tree_c3 = create_tree(repo, &[(path, "Formatted 2")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);
    Ok(())
}

#[test]
fn test_fix_with_forking_commits() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // Two linear commits.
    // c1: "initial"
    let tree1 = create_tree(repo, &[(path, "initial")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: "base"
    let tree2 = create_tree(repo, &[(path, "base")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Forked commits.
    // c3: "unformatted 1"
    let tree3 = create_tree(repo, &[(path, "unformatted 1")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // c4: "unformatted 2"
    let tree4 = create_tree(repo, &[(path, "unformatted 2")]);
    let c4 = create_commit(&mut tx, vec![c2.clone()], tree4);

    // Run fix on c3 and c4 (i.e. the forked commits).
    let forked_commits = vec![c3.clone(), c4.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"base"), b"unformatted 1", b"Formatted 1");
    file_fixer.add_replacement(path, Some(b"base"), b"unformatted 2", b"Formatted 2");

    let include_unchanged_files = false;
    let summary = fix_files(
        forked_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&c3));
    assert!(summary.rewrites.contains_key(&c4));

    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let expected_tree_c3 = create_tree(repo, &[(path, "Formatted 1")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);

    let new_c4 = repo
        .store()
        .get_commit(summary.rewrites.get(&c4).unwrap())
        .unwrap();
    let expected_tree_c4 = create_tree(repo, &[(path, "Formatted 2")]);
    assert_tree_eq!(new_c4.tree(), expected_tree_c4);
    Ok(())
}

#[test]
fn test_fix_with_merging_commits() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "base"
    let tree1 = create_tree(repo, &[(path, "base")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Two forked commits.
    // c2: "left"
    let tree2 = create_tree(repo, &[(path, "left")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "right"
    let tree3 = create_tree(repo, &[(path, "right")]);
    let c3 = create_commit(&mut tx, vec![c1.clone()], tree3);

    // c4: "unformatted" (Merge c2, c3)
    let tree4 = create_tree(repo, &[(path, "unformatted")]);
    let c4 = create_commit(&mut tx, vec![c2.clone(), c3.clone()], tree4);

    // Run fix on c4 with merging base.
    let root_commits = vec![c4.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"left"), b"unformatted", b"Formatted");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c4));

    let new_c4 = repo
        .store()
        .get_commit(summary.rewrites.get(&c4).unwrap())
        .unwrap();
    let expected_tree_c4 = create_tree(repo, &[(path, "Formatted")]);
    assert_tree_eq!(new_c4.tree(), expected_tree_c4);
    Ok(())
}

#[test]
fn test_fix_conflicted_base_commit() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "base"
    let tree1 = create_tree(repo, &[(path, "base")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Left and right side of the conflict.
    // c2: "left"
    let tree2 = create_tree(repo, &[(path, "left")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "right"
    let tree3 = create_tree(repo, &[(path, "right")]);
    let c3 = create_commit(&mut tx, vec![c1.clone()], tree3);

    // c4: Merge(c2, c3). Creates conflict in "foo".
    let c4_tree = merge_commit_trees(
        tx.repo_mut(),
        &[
            repo.store().get_commit(&c2).unwrap(),
            repo.store().get_commit(&c3).unwrap(),
        ],
    )
    .block_on()
    .unwrap();
    let c4 = create_commit(&mut tx, vec![c2.clone(), c3.clone()], c4_tree);

    // c5: "unformatted"
    let tree5 = create_tree(repo, &[(path, "unformatted")]);
    let c5 = create_commit(&mut tx, vec![c4.clone()], tree5);

    // Run fix on c5.
    let root_commits = vec![c5.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"left"), b"unformatted", b"Formatted");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c5));

    let new_c5 = repo
        .store()
        .get_commit(summary.rewrites.get(&c5).unwrap())
        .unwrap();
    let expected_tree_c5 = create_tree(repo, &[(path, "Formatted")]);
    assert_tree_eq!(new_c5.tree(), expected_tree_c5);
    Ok(())
}

#[test]
fn test_fix_conflicted_current_commit() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "base"
    let tree1 = create_tree(repo, &[(path, "base")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Left and right side of the conflict.
    // c2_left: "left"
    let tree2_left = create_tree(repo, &[(path, "left")]);
    let c2_left = create_commit(&mut tx, vec![c1.clone()], tree2_left);

    // c2_right: "right"
    let tree2_right = create_tree(repo, &[(path, "right")]);
    let c2_right = create_commit(&mut tx, vec![c1.clone()], tree2_right);

    // c2: Merge(c2_left, c2_right) -> Conflict
    let c2_tree = merge_commit_trees(
        tx.repo_mut(),
        &[
            repo.store().get_commit(&c2_left).unwrap(),
            repo.store().get_commit(&c2_right).unwrap(),
        ],
    )
    .block_on()
    .unwrap();
    let c2 = create_commit(&mut tx, vec![c2_left.clone(), c2_right.clone()], c2_tree);

    // c3: "unformatted"
    let tree3 = create_tree(repo, &[(path, "unformatted")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // Run fix on c2 and c3.
    let root_commits = vec![c2.clone(), c3.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"left"), b"unformatted", b"Formatted");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    // c2 should not be rewritten because it matches the merge of its parents
    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c3));

    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let expected_tree_c3 = create_tree(repo, &[(path, "Formatted")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);
    Ok(())
}

#[test]
fn test_fix_reverts_commit_to_empty() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // Base commit: "Formatted\n"
    let tree1 = create_tree(repo, &[(path, "Formatted\n")]);
    let c1 = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree1.clone(),
    );

    // Child commit: "unformatted\n"
    let tree2 = create_tree(repo, &[(path, "unformatted\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"Formatted\n"), b"unformatted\n", b"Formatted\n");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c2));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();
    assert_tree_eq!(new_c2.tree(), tree1);
    Ok(())
}

#[test]
fn test_fix_renamed_file() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path_a = repo_path("file_a");
    let path_b = repo_path("file_b");

    // Base: file_a = "unformatted\n"
    let tree1 = create_tree(repo, &[(path_a, "unformatted\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Child: file_b = "unformatted\n"
    let tree2 = create_tree(repo, &[(path_b, "unformatted\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path_b, None, b"unformatted\n", b"Formatted\n");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 1);
    assert!(summary.rewrites.contains_key(&c2));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();

    // Since we are not using copy tracking right now, we are doing the diff against
    // an empty tree. Thus, we format the whole file.
    let expected_tree = create_tree(repo, &[(path_b, "Formatted\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);
    Ok(())
}

#[test]
fn test_fix_empty_file_not_formatted() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // Base: "content\n"
    let tree1 = create_tree(repo, &[(path, "content\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Child: ""
    let tree2 = create_tree(repo, &[(path, "")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"content\n"), b"", b"");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    // Empty files are not formatted when include_unchanged_files is false.
    assert!(summary.rewrites.is_empty());
    Ok(())
}

#[test]
fn test_fix_forking_commits_same_file_id_different_base_content() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // Base A: "base A"
    let tree_a = create_tree(repo, &[(path, "base A")]);
    let c_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree_a);

    // Base B: "base B"
    let tree_b = create_tree(repo, &[(path, "base B")]);
    let c_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree_b);

    // Same file content / FileId but different base commits
    let child_content = "unformatted";
    let tree_child = create_tree(repo, &[(path, child_content)]);

    // c1 has parent A
    let c1 = create_commit(&mut tx, vec![c_a.clone()], tree_child.clone());

    // c2 has parent B
    let c2 = create_commit(&mut tx, vec![c_b.clone()], tree_child.clone());

    // Run fix on c1 and c2.
    let root_commits = vec![c1.clone(), c2.clone()];
    let mut file_fixer = TestFileFixer::new();
    file_fixer.add_replacement(path, Some(b"base A"), b"unformatted", b"Formatted A");
    file_fixer.add_replacement(path, Some(b"base B"), b"unformatted", b"Formatted B");

    let include_unchanged_files = false;
    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
    .unwrap();

    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&c1));
    assert!(summary.rewrites.contains_key(&c2));

    let new_c1 = repo
        .store()
        .get_commit(summary.rewrites.get(&c1).unwrap())
        .unwrap();
    let expected_tree = create_tree(repo, &[(path, "Formatted A")]);
    assert_tree_eq!(new_c1.tree(), expected_tree);

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();

    let expected_tree = create_tree(repo, &[(path, "Formatted B")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);
    Ok(())
}

#[test]
fn test_compute_changed_line_ranges() -> TestResult {
    // Insert at end.
    assert_eq!(
        compute_changed_ranges(b"a\n", b"a\nb\n"),
        RegionsToFormat::LineRanges(vec![line_range(2, 2)])
    );

    // Delete in the middle.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nc\n"),
        RegionsToFormat::LineRanges(vec![])
    );

    // Modify in the middle.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nB\nc\n"),
        RegionsToFormat::LineRanges(vec![line_range(2, 2)])
    );

    // Modify multiple.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nB\nC\n"),
        RegionsToFormat::LineRanges(vec![line_range(2, 3)])
    );

    // Insert at start.
    assert_eq!(
        compute_changed_ranges(b"a\n", b"new\na\n"),
        RegionsToFormat::LineRanges(vec![line_range(1, 1)])
    );

    // Inserting new line at EOF.
    assert_eq!(
        compute_changed_ranges(b"a", b"a\n"),
        RegionsToFormat::LineRanges(vec![line_range(1, 1)])
    );

    // Insert at EOF but no newline.
    assert_eq!(
        compute_changed_ranges(b"a\n", b"a\nb"),
        RegionsToFormat::LineRanges(vec![line_range(2, 2)])
    );

    // Complex case with multiple modifications and insertions.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\nd\ne\nf\n", b"a\nB\nC\nd\ne\nF\n"),
        RegionsToFormat::LineRanges(vec![line_range(2, 3), line_range(6, 6)])
    );

    // Add line to empty file.
    assert_eq!(
        compute_changed_ranges(b"", b"a\n"),
        RegionsToFormat::LineRanges(vec![line_range(1, 1)])
    );

    // Remove last line from file.
    assert_eq!(
        compute_changed_ranges(b"a\n", b""),
        RegionsToFormat::LineRanges(vec![])
    );
    Ok(())
}
