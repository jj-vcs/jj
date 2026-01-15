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
use jj_lib::backend::CommitId;
use jj_lib::backend::FileId;
use jj_lib::commit::Commit;
use jj_lib::fix::FileFixer;
use jj_lib::fix::FileToFix;
use jj_lib::fix::FixError;
use jj_lib::fix::ParallelFileFixer;
use jj_lib::fix::RegionsToFormat;
use jj_lib::fix::compute_changed_ranges;
use jj_lib::fix::fix_files;
use jj_lib::fix::get_base_commit_map;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::assert_tree_eq;
use testutils::create_tree;
use testutils::create_tree_with;
use testutils::read_file;
use testutils::repo_path;
use thiserror::Error;

#[derive(Clone, Debug)]
struct TestFileFixer {
    /// If true, the fixer will use `compute_changed_ranges` to determine
    /// which lines to fix, and only uppercase those lines. If false, it
    /// uppercases everything if it starts with "fixme:").
    fix_with_line_range: bool,
    formatter_call_count: u32,
}

impl TestFileFixer {
    fn new() -> Self {
        Self {
            fix_with_line_range: false,
            formatter_call_count: 0,
        }
    }

    fn new_with_line_ranges() -> Self {
        Self {
            fix_with_line_range: true,
            formatter_call_count: 0,
        }
    }

    fn fix_file(
        &mut self,
        store: &Store,
        file_to_fix: &FileToFix,
    ) -> Result<Option<FileId>, FixError> {
        if self.fix_with_line_range {
            let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);
            let base_content = file_to_fix
                .base_file_id
                .as_ref()
                .map(|base_file_id| read_file(store, &file_to_fix.repo_path, base_file_id));

            let ranges = match compute_changed_ranges(
                base_content.as_deref().unwrap_or_default(),
                &old_content,
            ) {
                RegionsToFormat::LineRanges(ranges) => ranges,
                RegionsToFormat::NoRegions => return Ok(None),
            };

            if ranges.is_empty() {
                let mut output = Vec::new();
                output.extend_from_slice(b"sort includes\n");
                output.extend_from_slice(&old_content);
                let new_file_id = store
                    .write_file(&file_to_fix.repo_path, &mut output.as_slice())
                    .block_on()
                    .unwrap();
                return Ok(Some(new_file_id));
            }

            let mut output = Vec::new();
            for (line_number, line_content) in
                old_content.split_inclusive(|b| *b == b'\n').enumerate()
            {
                let line_num = line_number + 1;
                let should_fix = ranges
                    .iter()
                    .any(|r| line_num >= r.first && line_num <= r.last);
                if should_fix {
                    if let Ok(s) = std::str::from_utf8(line_content) {
                        output.extend_from_slice(s.to_uppercase().as_bytes());
                    } else {
                        output.extend_from_slice(&line_content.to_ascii_uppercase());
                    }
                } else {
                    output.extend_from_slice(line_content);
                }
            }

            if output == old_content {
                return Ok(None);
            }

            let new_file_id = store
                .write_file(&file_to_fix.repo_path, &mut output.as_slice())
                .block_on()
                .unwrap();
            Ok(Some(new_file_id))
        } else {
            fix_file(store, file_to_fix)
        }
    }
}

impl FileFixer for TestFileFixer {
    fn fix_files<'a>(
        &mut self,
        store: &Store,
        files_to_fix: &'a HashSet<FileToFix>,
    ) -> Result<HashMap<&'a FileToFix, FileId>, FixError> {
        let mut changed_files: HashMap<&'a FileToFix, FileId> = HashMap::new();
        for file_to_fix in files_to_fix {
            self.formatter_call_count += 1;
            if let Some(new_file_id) = self.fix_file(store, file_to_fix)? {
                changed_files.insert(file_to_fix, new_file_id);
            }
        }
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
fn fix_file(store: &Store, file_to_fix: &FileToFix) -> Result<Option<FileId>, FixError> {
    let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);

    if let Some(rest) = old_content.strip_prefix(b"fixme:") {
        let new_content = rest.to_ascii_uppercase();
        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()
            .unwrap();
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

#[test]
fn test_fix_one_file() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
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
    .block_on()
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
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
}

#[test]
fn test_fixer_does_not_change_content() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "content")]);
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
    .block_on()
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
    .block_on()
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
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "error:boo")]);
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
    assert_eq!(error.to_string(), "Forced failure: boo");
}

#[test]
fn test_unchanged_file_is_not_fixed() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let tree2 = create_tree(repo, &[(path1, "fixme:content")]);
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
    .block_on()
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
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let tree2 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);

    let root_commits = vec![commit_b.clone()];
    let mut file_fixer = TestFileFixer::new();

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        true,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
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
    assert_tree_eq!(new_commit_b.tree(), expected_tree_b);
}

/// If a descendant is already correctly formatted, it should still be rewritten
/// but its tree should be preserved.
#[test]
fn test_already_fixed_descendant() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let tree2 = create_tree(repo, &[(path1, "CONTENT")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2.clone());

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();

    let summary = fix_files(
        root_commits,
        &EverythingMatcher,
        true,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on()
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
    assert_tree_eq!(new_commit_a.tree(), tree2);
    let new_commit_b = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_tree_eq!(new_commit_b.tree(), tree2);
}

#[test]
fn test_parallel_fixer_basic() {
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
    .block_on()
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
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
}

#[test]
fn test_parallel_fixer_fixes_files() {
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
    .block_on()
    .unwrap();

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
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
}

#[test]
fn test_parallel_fixer_does_not_change_content() {
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
    .block_on()
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
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:xyz")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let path2 = repo_path("file2");
    let tree2 = create_tree(repo, &[(path2, "content")]);
    let commit_b = create_commit(&mut tx, vec![commit_a.clone()], tree2);
    let path3 = repo_path("file3");
    let tree3 = create_tree(repo, &[(path3, "content")]);
    let _commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree3);
    let path4 = repo_path("file4");
    let tree4 = create_tree(repo, &[(path4, "content")]);
    let _commit_d = create_commit(&mut tx, vec![commit_a.clone()], tree4);

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
    .block_on()
    .unwrap();

    let expected_tree_a = create_tree(repo, &[(path1, "XYZ")]);

    let new_commit_a = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_a).unwrap())
        .unwrap();
    assert_tree_eq!(new_commit_a.tree(), expected_tree_a);
}

#[test]
fn test_get_base_commit_map_chain() {
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
    // A (root)
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
    let base_commit_map = get_base_commit_map(&commits).unwrap();

    let parents_set = IndexSet::from([commit_a]);
    let expected_base_commit_map: HashMap<CommitId, IndexSet<CommitId>> = HashMap::from([
        (commit_d, parents_set.clone()),
        (commit_c, parents_set.clone()),
        (commit_b, parents_set.clone()),
    ]);

    assert_eq!(base_commit_map, expected_base_commit_map);
}

#[test]
fn test_fix_complex_merge_with_base_map() {
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
    let tree1 = create_tree(repo, &[(path, "a\nb\nc\nd\ne\n")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path, "a\nb\nc\nd\ne\n")]);
    let commit_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree2);
    let tree3 = create_tree(repo, &[(path, "a-mod\nb\nc\nd\ne\n")]);
    let commit_c = create_commit(&mut tx, vec![commit_a.clone(), commit_b.clone()], tree3);
    let tree4 = create_tree(repo, &[(path, "a\nb\nc-mod\nd\ne\n")]);
    let commit_d = create_commit(&mut tx, vec![commit_b.clone()], tree4);
    let tree5 = create_tree(repo, &[(path, "a\nb\nc\nd\ne-mod\n")]);
    let commit_e = create_commit(&mut tx, vec![commit_c.clone(), commit_d.clone()], tree5);

    let commit_c_obj = repo.store().get_commit(&commit_c).unwrap();
    let commit_e_obj = repo.store().get_commit(&commit_e).unwrap();

    let commits: Vec<Commit> = vec![commit_e_obj, commit_c_obj];
    let base_commit_map = get_base_commit_map(&commits).unwrap();

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

    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let expected_tree_c = create_tree(repo, &[(path, "A-MOD\nb\nc\nd\ne\n")]);
    assert_tree_eq!(new_commit_c.tree(), expected_tree_c);

    let new_commit_e = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_e).unwrap())
        .unwrap();
    let expected_tree_e = create_tree(repo, &[(path, "a\nb\nC\nd\nE-MOD\n")]);
    assert_tree_eq!(new_commit_e.tree(), expected_tree_e);
}

#[test]
fn test_fix_diamond_merge_with_base_map() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // We have a diamond merge:
    //
    //   E
    //  / \
    // C   D
    //  \ /
    //   B (root)
    let mut tx = repo.start_transaction();
    let path = repo_path("file1");
    let tree1 = create_tree(repo, &[(path, "b\n\n\n\n")]);
    let commit_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);
    let tree2 = create_tree(repo, &[(path, "b\nc\n\n\n")]);
    let commit_c = create_commit(&mut tx, vec![commit_b.clone()], tree2);
    let tree3 = create_tree(repo, &[(path, "b\n\nd\n\n")]);
    let commit_d = create_commit(&mut tx, vec![commit_b.clone()], tree3);
    let tree4 = create_tree(repo, &[(path, "b\nc\nd\ne\n")]);
    let commit_e = create_commit(&mut tx, vec![commit_c.clone(), commit_d.clone()], tree4);

    let commit_c_obj = repo.store().get_commit(&commit_c).unwrap();
    let commit_e_obj = repo.store().get_commit(&commit_e).unwrap();

    // We are fixing e and c.
    let commits: Vec<Commit> = vec![commit_e_obj, commit_c_obj];
    let base_commit_map = get_base_commit_map(&commits).unwrap();

    // Should be {e: {b, d}, c: {b}}
    let expected_base_commit_map: HashMap<CommitId, IndexSet<CommitId>> = HashMap::from([
        (
            commit_e.clone(),
            IndexSet::from([commit_b.clone(), commit_d.clone()]),
        ),
        (commit_c.clone(), IndexSet::from([commit_b.clone()])),
    ]);

    assert_eq!(base_commit_map, expected_base_commit_map);

    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let expected_tree_c = create_tree(repo, &[(path, "b\nC\n\n\n")]);
    assert_tree_eq!(new_commit_c.tree(), expected_tree_c);

    let new_commit_e = repo
        .store()
        .get_commit(summary.rewrites.get(&commit_e).unwrap())
        .unwrap();
    let expected_tree_e = create_tree(repo, &[(path, "b\nC\nd\nE\n")]);
    assert_tree_eq!(new_commit_e.tree(), expected_tree_e);
}

#[test]
fn test_compute_changed_line_ranges() {
    // Insert & Delete & Modify.
    assert_eq!(
        compute_changed_ranges(b"a\n", b"a\nb\n"),
        RegionsToFormat::LineRanges(vec![(2..3).into()])
    );
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nc\n"),
        RegionsToFormat::LineRanges(vec![])
    );
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nB\nc\n"),
        RegionsToFormat::LineRanges(vec![(2..3).into()])
    );

    // Modify multiple & Insert at start.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\n", b"a\nB\nC\n"),
        RegionsToFormat::LineRanges(vec![(2..4).into()])
    );
    assert_eq!(
        compute_changed_ranges(b"a\n", b"new\na\n"),
        RegionsToFormat::LineRanges(vec![(1..2).into()])
    );

    // Inserting new line at EOF & Insert at EOF but no newline.
    assert_eq!(
        compute_changed_ranges(b"a", b"a\n"),
        RegionsToFormat::LineRanges(vec![(1..2).into()])
    );
    assert_eq!(
        compute_changed_ranges(b"a\n", b"a\nb"),
        RegionsToFormat::LineRanges(vec![(2..3).into()])
    );

    // Complex case with multiple modifications and insertions.
    assert_eq!(
        compute_changed_ranges(b"a\nb\nc\nd\ne\nf\n", b"a\nB\nC\nd\ne\nF\n"),
        RegionsToFormat::LineRanges(vec![(2..4).into(), (6..7).into()])
    );

    // Empty file.
    assert_eq!(
        compute_changed_ranges(b"", b"a\n"),
        RegionsToFormat::LineRanges(vec![(1..2).into()])
    );
    assert_eq!(
        compute_changed_ranges(b"a\n", b""),
        RegionsToFormat::NoRegions
    );
}

#[test]
fn test_fix_with_line_ranges_sequential_case() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "Foo\nBar\n"
    let tree1 = create_tree(repo, &[(path, "Foo\nBar\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: "Foo\nbar\n"
    let tree2 = create_tree(repo, &[(path, "Foo\nbar\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "foo\nbar\n"
    let tree3 = create_tree(repo, &[(path, "foo\nbar\n")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let expected_tree_c2 = create_tree(repo, &[(path, "Foo\nBAR\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree_c2);

    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let expected_tree_c3 = create_tree(repo, &[(path, "FOO\nBAR\n")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);
}

#[test]
fn test_fix_with_forking_commits() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // Two linear commits.
    // c1: "Foo\nBar\n"
    let tree1 = create_tree(repo, &[(path, "Foo\nBar\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: "Foo\nBar\nBaz\n"
    let tree2 = create_tree(repo, &[(path, "Foo\nBar\nBaz\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Forked commits.
    // c3: "Foo\nBar\nbaz\n"
    let tree3 = create_tree(repo, &[(path, "Foo\nBar\nbaz\n")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // c4: "Foo\nbar\nBaz\n"
    let tree4 = create_tree(repo, &[(path, "Foo\nbar\nBaz\n")]);
    let c4 = create_commit(&mut tx, vec![c2.clone()], tree4);

    // Run fix on c3 and c4 (i.e. the forked commits).
    let forked_commits = vec![c3.clone(), c4.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    // c3 should be fixed: "baz" -> "BAZ"
    // c4 should be fixed: "bar" -> "BAR"
    assert_eq!(summary.rewrites.len(), 2);
    assert!(summary.rewrites.contains_key(&c3));
    assert!(summary.rewrites.contains_key(&c4));

    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let expected_tree_c3 = create_tree(repo, &[(path, "Foo\nBar\nBAZ\n")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);

    let new_c4 = repo
        .store()
        .get_commit(summary.rewrites.get(&c4).unwrap())
        .unwrap();
    let expected_tree_c4 = create_tree(repo, &[(path, "Foo\nBAR\nBaz\n")]);
    assert_tree_eq!(new_c4.tree(), expected_tree_c4);
}

#[test]
fn test_fix_with_merging_commits() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "Foo\n"
    let tree1 = create_tree(repo, &[(path, "Foo\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Two forked commits.
    // c2: "Foo\nbar\n"
    let tree2 = create_tree(repo, &[(path, "Foo\nbar\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "Foo\nbaz\n"
    let tree3 = create_tree(repo, &[(path, "Foo\nbaz\n")]);
    let c3 = create_commit(&mut tx, vec![c1.clone()], tree3);

    // c4: "Foo\nBar\nBaz\n" (Merge c2, c3)
    let tree4 = create_tree(repo, &[(path, "Foo\nBar\nBaz\n")]);
    let c4 = create_commit(&mut tx, vec![c2.clone(), c3.clone()], tree4);

    // Run fix on c4 with merging base.
    let root_commits = vec![c4.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let expected_tree_c4 = create_tree(repo, &[(path, "Foo\nBAR\nBAZ\n")]);
    assert_tree_eq!(new_c4.tree(), expected_tree_c4);
}

#[test]
fn test_fix_with_line_ranges_conflicted_base_commit() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "line1\nline2\nline3\n"
    let tree1 = create_tree(repo, &[(path, "line1\nline2\nline3\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Left and right side of the conflict.
    // c2: "line1\nline2 left\nline3 change\n"
    let tree2 = create_tree(repo, &[(path, "line1\nline2 left\nline3 change\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // c3: "line1 change\nline2 right\nline3\n"
    let tree3 = create_tree(repo, &[(path, "line1 change\nline2 right\nline3\n")]);
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

    // c5: "line1\nline2 resolved\nline3\n"
    let tree5 = create_tree(repo, &[(path, "line1\nline2 resolved\nline3\n")]);
    let c5 = create_commit(&mut tx, vec![c4.clone()], tree5);

    // Run fix on c5.
    let root_commits = vec![c5.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    let expected_tree_c5 = create_tree(repo, &[(path, "line1\nLINE2 RESOLVED\nLINE3\n")]);
    assert_tree_eq!(new_c5.tree(), expected_tree_c5);
}

#[test]
fn test_fix_with_line_ranges_conflicted_current_commit() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("foo");

    // c1: "line1\nline2\nline3\n"
    let tree1 = create_tree(repo, &[(path, "line1\nline2\nline3\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Left and right side of the conflict.
    // c2_left: "line1\nline2 left\nline3 change\n"
    let tree2_left = create_tree(repo, &[(path, "line1\nline2 left\nline3 change\n")]);
    let c2_left = create_commit(&mut tx, vec![c1.clone()], tree2_left);

    // c2_right: "line1 change\nline2 right\nline3\n"
    let tree2_right = create_tree(repo, &[(path, "line1 change\nline2 right\nline3\n")]);
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

    // c3: "line1\nline2\nline3\n"
    let tree3 = create_tree(repo, &[(path, "line1\nline2\nline3\n")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree3);

    // Run fix on c2 and c3.
    let root_commits = vec![c2.clone(), c3.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    // c3: "line1\nLINE2\nLINE3\n"
    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();

    let expected_tree_c3 = create_tree(repo, &[(path, "line1\nLINE2\nLINE3\n")]);
    assert_tree_eq!(new_c3.tree(), expected_tree_c3);
}

#[test]
fn test_fix_reverts_commit_to_empty() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // Base commit: "UPPERCASE\n"
    let tree1 = create_tree(repo, &[(path, "UPPERCASE\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Child commit: "uppercase\n"
    let tree2 = create_tree(repo, &[(path, "uppercase\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    // Verify content is uppercased
    let expected_tree = create_tree(repo, &[(path, "UPPERCASE\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);

    // Verify it is empty (tree same as parent)
    let c1_obj = repo.store().get_commit(&c1).unwrap();
    assert_tree_eq!(new_c2.tree(), c1_obj.tree());
}

#[test]
fn test_fix_renamed_file() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path_a = repo_path("file_a");
    let path_b = repo_path("file_b");

    // Base: file_a = "line1\nline2\nline3\n"
    let tree1 = create_tree(repo, &[(path_a, "line1\nline2\nline3\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // Child: file_b = "line1\nline2 changed\nline3\n"
    let tree2 = create_tree(repo, &[(path_b, "line1\nline2 changed\nline3\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on c2.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let expected_tree = create_tree(repo, &[(path_b, "LINE1\nLINE2 CHANGED\nLINE3\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);
}

#[test]
fn test_fix_truncate_to_empty() {
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
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    // The fixer should not find any ranges to fix since the file is empty.
    assert!(summary.rewrites.is_empty());
}

#[test]
fn test_fix_utf8_content() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // "line 1: ¯\\_(ツ)_/¯\nline 2: 日\n"
    let tree1 = create_tree(repo, &[(path, "line 1: ¯\\_(ツ)_/¯\nline 2: 日\n")]);
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // "line 1: ¯\\_(ツ)_/¯\nline 2: 日 changed\n"
    let tree2 = create_tree(repo, &[(path, "line 1: ¯\\_(ツ)_/¯\nline 2: 日 changed\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();

    let expected_tree = create_tree(repo, &[(path, "line 1: ¯\\_(ツ)_/¯\nLINE 2: 日 CHANGED\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);
}

#[test]
fn test_fix_empty_revset() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let mut tx = repo.start_transaction();

    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
}

#[test]
fn test_fix_forking_commits_same_file_id_different_base_content() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // Base A: "foo\nbar\n"
    let tree_a = create_tree(repo, &[(path, "foo\nbar\n")]);
    let c_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree_a);

    // Base B: "bar\nfoo\n"
    let tree_b = create_tree(repo, &[(path, "bar\nfoo\n")]);
    let c_b = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree_b);

    // Same file content / FileId but different base commits
    let child_content = "foo\nbar\n";
    let tree_child = create_tree(repo, &[(path, child_content)]);

    // c1 has parent A
    let c1 = create_commit(&mut tx, vec![c_a.clone()], tree_child.clone());

    // c2 has parent B
    let c2 = create_commit(&mut tx, vec![c_b.clone()], tree_child.clone());

    // Run fix on c1 and c2.
    let root_commits = vec![c1.clone(), c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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
    assert!(!summary.rewrites.contains_key(&c1));
    assert!(summary.rewrites.contains_key(&c2));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();

    let expected_tree = create_tree(repo, &[(path, "foo\nBAR\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree);
}

#[test]
fn test_fix_formatter_caching() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path = repo_path("file");

    // c1: "foo\n"
    let tree_c1 = create_tree(repo, &[(path, "foo\n")]);
    let c1 = create_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        tree_c1,
    );

    // c2: "bar\n"
    let tree_c2 = create_tree(repo, &[(path, "bar\n")]);
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree_c2);

    // c3: "baz\n"
    let tree_c3 = create_tree(repo, &[(path, "baz\n")]);
    let c3 = create_commit(&mut tx, vec![c2.clone()], tree_c3);

    // c4: "bar\n"
    let tree_c4 = create_tree(repo, &[(path, "bar\n")]);
    let c4 = create_commit(&mut tx, vec![c3.clone()], tree_c4);

    // Run fix on c2, c3, c4.
    let root_commits = vec![c2.clone(), c3.clone(), c4.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
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

    assert_eq!(summary.rewrites.len(), 3);
    assert!(summary.rewrites.contains_key(&c2));
    assert!(summary.rewrites.contains_key(&c3));
    assert!(summary.rewrites.contains_key(&c4));

    let new_c2 = repo
        .store()
        .get_commit(summary.rewrites.get(&c2).unwrap())
        .unwrap();
    let new_c3 = repo
        .store()
        .get_commit(summary.rewrites.get(&c3).unwrap())
        .unwrap();
    let new_c4 = repo
        .store()
        .get_commit(summary.rewrites.get(&c4).unwrap())
        .unwrap();

    let expected_tree_bar = create_tree(repo, &[(path, "BAR\n")]);
    let expected_tree_baz = create_tree(repo, &[(path, "BAZ\n")]);
    assert_tree_eq!(new_c2.tree(), expected_tree_bar);
    assert_tree_eq!(new_c3.tree(), expected_tree_baz);
    assert_tree_eq!(new_c4.tree(), expected_tree_bar);

    // The formatter should be called twice. Once for the 'bar' content and once for
    // the 'baz' content.
    assert_eq!(file_fixer.formatter_call_count, 2);
}

#[test]
fn test_fix_with_line_ranges_and_include_unchanged_files() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path_changed = repo_path("changed.txt");
    let path_unchanged = repo_path("unchanged.txt");
    let path_new = repo_path("newfile.txt");

    // c1: changed.txt, unchanged.txt
    let tree1 = create_tree(
        repo,
        &[
            (path_changed, "Foo1\nFoo2\nFoo3\n"),
            (path_unchanged, "baz1\nbaz2\nbaz3\n"),
        ],
    );
    let c1 = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    // c2: changed.txt, unchanged.txt, newfile.txt
    let tree2 = create_tree(
        repo,
        &[
            (path_changed, "Foo1\nFoo2-Mod\nFoo3\n"),
            (path_unchanged, "baz1\nbaz2\nbaz3\n"),
            (path_new, "new\n"),
        ],
    );
    let c2 = create_commit(&mut tx, vec![c1.clone()], tree2);

    // Run fix on commit 2 with include_unchanged_files = true.
    let root_commits = vec![c2.clone()];
    let mut file_fixer = TestFileFixer::new_with_line_ranges();
    let include_unchanged_files = true;

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

    let expected_tree = create_tree(
        repo,
        &[
            (path_changed, "Foo1\nFOO2-MOD\nFoo3\n"),
            (path_unchanged, "sort includes\nbaz1\nbaz2\nbaz3\n"),
            (path_new, "NEW\n"),
        ],
    );
    assert_tree_eq!(new_c2.tree(), expected_tree);
}
