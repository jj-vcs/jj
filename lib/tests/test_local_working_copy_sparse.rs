// Copyright 2022 The Jujutsu Authors
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

use futures::StreamExt as _;
use itertools::Itertools as _;
use jj_lib::fileset::FilesetExpression;
use jj_lib::local_working_copy::LocalWorkingCopy;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::WorkingCopy as _;
use pollster::FutureExt as _;
use testutils::TestResult;
use testutils::TestWorkspace;
use testutils::commit_with_tree;
use testutils::create_tree;
use testutils::repo_path;

fn paths_to_fileset(paths: &[&RepoPath]) -> FilesetExpression {
    let prefix_exprs: Vec<_> = paths
        .iter()
        .map(|&path| FilesetExpression::prefix_path(path.to_owned()))
        .collect();
    FilesetExpression::union_all(prefix_exprs)
}

#[test]
fn test_sparse_checkout() -> TestResult {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let working_copy_path = test_workspace.workspace.workspace_root().to_owned();

    let root_file1_path = repo_path("file1");
    let root_file2_path = repo_path("file2");
    let dir1_path = repo_path("dir1");
    let dir1_file1_path = repo_path("dir1/file1");
    let dir1_file2_path = repo_path("dir1/file2");
    let dir1_subdir1_path = repo_path("dir1/subdir1");
    let dir1_subdir1_file1_path = repo_path("dir1/subdir1/file1");
    let dir2_path = repo_path("dir2");
    let dir2_file1_path = repo_path("dir2/file1");

    let tree = create_tree(
        repo,
        &[
            (root_file1_path, "contents"),
            (root_file2_path, "contents"),
            (dir1_file1_path, "contents"),
            (dir1_file2_path, "contents"),
            (dir1_subdir1_file1_path, "contents"),
            (dir2_file1_path, "contents"),
        ],
    );
    let commit = commit_with_tree(repo.store(), tree);

    test_workspace
        .workspace
        .check_out(repo.op_id().clone(), None, &commit)
        .block_on()?;
    let ws = &mut test_workspace.workspace;

    // Set sparse patterns to only dir1/
    let mut locked_ws = ws.start_working_copy_mutation().block_on()?;
    let sparse_expr = paths_to_fileset(&[dir1_path]);
    let stats = locked_ws
        .locked_wc()
        .set_sparse_patterns(sparse_expr.clone())
        .block_on()?;
    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 0,
            removed_files: 3,
            skipped_files: 0,
        }
    );
    assert_eq!(locked_ws.locked_wc().sparse_patterns()?, &sparse_expr);
    assert!(
        !root_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !root_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_subdir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir2_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );

    // Write the new state to disk
    locked_ws.finish(repo.op_id().clone()).block_on()?;
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert_eq!(
        wc.file_states()?.paths().collect_vec(),
        vec![dir1_file1_path, dir1_file2_path, dir1_subdir1_file1_path]
    );
    assert_eq!(wc.sparse_patterns()?, &sparse_expr);

    // Reload the state to check that it was persisted
    let wc = LocalWorkingCopy::load(
        repo.store().clone(),
        ws.workspace_root().to_path_buf(),
        wc.state_path().to_path_buf(),
        repo.settings(),
    )?;
    assert_eq!(
        wc.file_states()?.paths().collect_vec(),
        vec![dir1_file1_path, dir1_file2_path, dir1_subdir1_file1_path]
    );
    assert_eq!(wc.sparse_patterns()?, &sparse_expr);

    // Set sparse patterns to file2, dir1/subdir1/ and dir2/
    let mut locked_wc = wc.start_mutation().block_on()?;
    let sparse_expr = paths_to_fileset(&[root_file1_path, dir1_subdir1_path, dir2_path]);
    let stats = locked_wc
        .set_sparse_patterns(sparse_expr.clone())
        .block_on()?;
    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 2,
            removed_files: 2,
            skipped_files: 0,
        }
    );
    assert_eq!(locked_wc.sparse_patterns()?, &sparse_expr);
    assert!(
        root_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !root_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir1_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_subdir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir2_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    let wc = locked_wc.finish(repo.op_id().clone()).block_on()?;
    let wc: &LocalWorkingCopy = wc.downcast_ref().unwrap();
    assert_eq!(
        wc.file_states()?.paths().collect_vec(),
        vec![dir1_subdir1_file1_path, dir2_file1_path, root_file1_path]
    );
    Ok(())
}

/// Test that sparse patterns are respected on commit
#[test]
fn test_sparse_commit() -> TestResult {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let op_id = repo.op_id().clone();
    let working_copy_path = test_workspace.workspace.workspace_root().to_owned();

    let root_file1_path = repo_path("file1");
    let dir1_path = repo_path("dir1");
    let dir1_file1_path = repo_path("dir1/file1");
    let dir2_path = repo_path("dir2");
    let dir2_file1_path = repo_path("dir2/file1");

    let tree = create_tree(
        repo,
        &[
            (root_file1_path, "contents"),
            (dir1_file1_path, "contents"),
            (dir2_file1_path, "contents"),
        ],
    );

    let commit = commit_with_tree(repo.store(), tree.clone());
    test_workspace
        .workspace
        .check_out(repo.op_id().clone(), None, &commit)
        .block_on()?;

    // Set sparse patterns to only dir1/
    let mut locked_ws = test_workspace
        .workspace
        .start_working_copy_mutation()
        .block_on()?;
    let sparse_expr = paths_to_fileset(&[dir1_path]);
    locked_ws
        .locked_wc()
        .set_sparse_patterns(sparse_expr)
        .block_on()?;
    locked_ws.finish(repo.op_id().clone()).block_on()?;

    // Write modified version of all files, including files that are not in the
    // sparse patterns.
    std::fs::write(
        root_file1_path.to_fs_path_unchecked(&working_copy_path),
        "modified",
    )?;
    std::fs::write(
        dir1_file1_path.to_fs_path_unchecked(&working_copy_path),
        "modified",
    )?;
    std::fs::create_dir(dir2_path.to_fs_path_unchecked(&working_copy_path))?;
    std::fs::write(
        dir2_file1_path.to_fs_path_unchecked(&working_copy_path),
        "modified",
    )?;

    // Create a tree from the working copy. Only dir1/file1 should be updated in the
    // tree.
    let modified_tree = test_workspace.snapshot()?;
    let diff: Vec<_> = tree
        .diff_stream(&modified_tree, &EverythingMatcher)
        .collect()
        .block_on();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].path.as_ref(), dir1_file1_path);

    // Set sparse patterns to also include dir2/
    let mut locked_ws = test_workspace
        .workspace
        .start_working_copy_mutation()
        .block_on()?;
    let sparse_expr = paths_to_fileset(&[dir1_path, dir2_path]);
    locked_ws
        .locked_wc()
        .set_sparse_patterns(sparse_expr)
        .block_on()?;
    locked_ws.finish(op_id).block_on()?;

    // Create a tree from the working copy. Only dir1/file1 and dir2/file1 should be
    // updated in the tree.
    let modified_tree = test_workspace.snapshot()?;
    let diff: Vec<_> = tree
        .diff_stream(&modified_tree, &EverythingMatcher)
        .collect()
        .block_on();
    assert_eq!(diff.len(), 2);
    assert_eq!(diff[0].path.as_ref(), dir1_file1_path);
    assert_eq!(diff[1].path.as_ref(), dir2_file1_path);
    Ok(())
}

#[test]
fn test_sparse_commit_gitignore() -> TestResult {
    // Test that (untracked) .gitignore files in parent directories are respected
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let working_copy_path = test_workspace.workspace.workspace_root().to_owned();

    let dir1_path = repo_path("dir1");
    let dir1_file1_path = repo_path("dir1/file1");
    let dir1_file2_path = repo_path("dir1/file2");

    // Set sparse patterns to only dir1/
    let mut locked_ws = test_workspace
        .workspace
        .start_working_copy_mutation()
        .block_on()?;
    let sparse_expr = paths_to_fileset(&[dir1_path]);
    locked_ws
        .locked_wc()
        .set_sparse_patterns(sparse_expr)
        .block_on()?;
    locked_ws.finish(repo.op_id().clone()).block_on()?;

    // Write dir1/file1 and dir1/file2 and a .gitignore saying to ignore dir1/file1
    std::fs::write(working_copy_path.join(".gitignore"), "dir1/file1")?;
    std::fs::create_dir(dir1_path.to_fs_path_unchecked(&working_copy_path))?;
    std::fs::write(
        dir1_file1_path.to_fs_path_unchecked(&working_copy_path),
        "contents",
    )?;
    std::fs::write(
        dir1_file2_path.to_fs_path_unchecked(&working_copy_path),
        "contents",
    )?;

    // Create a tree from the working copy. Only dir1/file2 should be updated in the
    // tree because dir1/file1 is ignored.
    let modified_tree = test_workspace.snapshot()?;
    let entries = modified_tree.entries().collect_vec();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0.as_ref(), dir1_file2_path);
    Ok(())
}

#[test]
fn test_sparse_fileset_matching() -> TestResult {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let working_copy_path = test_workspace.workspace.workspace_root().to_owned();

    let root_file1_path = repo_path("file1.md");
    let dir1_file1_path = repo_path("dir1/file1.rs");
    let dir1_file2_path = repo_path("dir1/file2.rs");
    let dir1_file3_path = repo_path("dir1/file3.txt");
    let dir2_file1_path = repo_path("dir2/file1.rs");

    let tree = create_tree(
        repo,
        &[
            (root_file1_path, "contents"),
            (dir1_file1_path, "contents"),
            (dir1_file2_path, "contents"),
            (dir1_file3_path, "contents"),
            (dir2_file1_path, "contents"),
        ],
    );

    let commit = commit_with_tree(repo.store(), tree.clone());
    test_workspace
        .workspace
        .check_out(repo.op_id().clone(), None, &commit)
        .block_on()?;

    // 1. Match only dir1/*.rs (using glob)
    let mut locked_ws = test_workspace
        .workspace
        .start_working_copy_mutation()
        .block_on()?;

    // Construct fileset: root-glob:"dir1/*.rs"
    let glob_pattern = jj_lib::fileset::FilePattern::root_file_glob("dir1/*.rs").unwrap();
    let glob_expr = jj_lib::fileset::FilesetExpression::pattern(glob_pattern);

    let stats = locked_ws
        .locked_wc()
        .set_sparse_patterns(glob_expr.clone())
        .block_on()?;

    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 0,
            removed_files: 3, // file1.md, dir1/file3.txt, and dir2/file1.rs are removed
            skipped_files: 0,
        }
    );
    assert_eq!(locked_ws.locked_wc().sparse_patterns()?, &glob_expr);
    assert!(
        !root_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir1_file3_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir2_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );

    // 2. Exclude a specific file: dir1/ but NOT dir1/file2.rs (using Difference
    //    operator)
    locked_ws.finish(repo.op_id().clone()).block_on()?;
    let wc: &LocalWorkingCopy = test_workspace
        .workspace
        .working_copy()
        .downcast_ref()
        .unwrap();
    let mut locked_wc = wc.start_mutation().block_on()?;

    let dir1_expr = jj_lib::fileset::FilesetExpression::prefix_path(repo_path("dir1").to_owned());
    let file2_expr =
        jj_lib::fileset::FilesetExpression::file_path(repo_path("dir1/file2.rs").to_owned());
    let diff_expr = jj_lib::fileset::FilesetExpression::difference(dir1_expr, file2_expr);

    let stats = locked_wc
        .set_sparse_patterns(diff_expr.clone())
        .block_on()?;

    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 1,   // dir1/file3.txt is checked out
            removed_files: 1, // dir1/file2.rs is removed
            skipped_files: 0,
        }
    );
    assert_eq!(locked_wc.sparse_patterns()?, &diff_expr);
    assert!(
        !root_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir1_file2_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        dir1_file3_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );
    assert!(
        !dir2_file1_path
            .to_fs_path_unchecked(&working_copy_path)
            .exists()
    );

    Ok(())
}

#[test]
fn test_sparse_upgrade_path() -> TestResult {
    use prost::Message as _;
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let wc: &LocalWorkingCopy = test_workspace
        .workspace
        .working_copy()
        .downcast_ref()
        .unwrap();
    let state_path = wc.state_path().to_path_buf();

    // 1. Read the current tree state on disk
    let proto_path = state_path.join("tree_state");
    let buf = std::fs::read(&proto_path)?;
    let mut tree_state_proto = jj_lib::protos::local_working_copy::TreeState::decode(&*buf)?;

    // 2. Overwrite sparse_patterns with a legacy prefix-only proto
    let legacy_sparse = jj_lib::protos::local_working_copy::SparsePatterns {
        prefixes: vec!["dir1".to_owned(), "dir2".to_owned()],
        fileset_expression: "".to_owned(), // Empty to simulate old repo!
    };
    tree_state_proto.sparse_patterns = Some(legacy_sparse);

    // Write it back
    let mut buf = Vec::new();
    tree_state_proto.encode(&mut buf)?;
    std::fs::write(&proto_path, buf)?;

    // 3. Load the working copy (this runs our upgrade logic!)
    let wc = LocalWorkingCopy::load(
        repo.store().clone(),
        test_workspace.workspace.workspace_root().to_path_buf(),
        state_path.clone(),
        repo.settings(),
    )?;

    // 4. Verify that the patterns were successfully upgraded to a union fileset!
    let expected_expr = paths_to_fileset(&[repo_path("dir1"), repo_path("dir2")]);
    assert_eq!(wc.sparse_patterns()?, &expected_expr);

    // 5. Also test upgrading a legacy root prefix `[""]` to `all()`
    let mut tree_state_proto =
        jj_lib::protos::local_working_copy::TreeState::decode(&*std::fs::read(&proto_path)?)?;
    tree_state_proto.sparse_patterns = Some(jj_lib::protos::local_working_copy::SparsePatterns {
        prefixes: vec!["".to_owned()],
        fileset_expression: "".to_owned(),
    });
    let mut buf = Vec::new();
    tree_state_proto.encode(&mut buf)?;
    std::fs::write(&proto_path, buf)?;
    let wc = LocalWorkingCopy::load(
        repo.store().clone(),
        test_workspace.workspace.workspace_root().to_path_buf(),
        state_path,
        repo.settings(),
    )?;
    assert_eq!(*wc.sparse_patterns()?, FilesetExpression::all());

    Ok(())
}
