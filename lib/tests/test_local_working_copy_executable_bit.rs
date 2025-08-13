// Copyright 2025 The Jujutsu Authors
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

#![cfg(unix)] // Nothing to test on Windows

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::store::Store;
use testutils::TestTreeBuilder;
use testutils::TestWorkspace;
use testutils::repo_path;

/// Assert that a file's executable bit matches the expected value.
#[track_caller]
fn assert_file_executable(path: &Path, expected: bool) {
    let perms = path.metadata().unwrap().permissions();
    let actual = (perms.mode() & 0o100) == 0o100;
    assert_eq!(actual, expected);
}

/// Set the executable bit of a file on the filesystem.
#[track_caller]
fn set_file_executable(path: &Path, executable: bool) {
    let prev_mode = path.metadata().unwrap().permissions().mode();
    let is_executable = prev_mode & 0o100 != 0;
    assert_ne!(executable, is_executable, "why are you calling this?");
    let new_mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, PermissionsExt::from_mode(new_mode)).unwrap();
}

/// Assert that a tree value's executable bit matches the expected value.
#[track_caller]
fn assert_tree_executable(tree_val: TreeValue, expected: bool) {
    if let TreeValue::File { executable, .. } = tree_val {
        assert_eq!(executable, expected);
    } else {
        panic!()
    }
}

/// Create a tree with an empty file having the given executable bit. Returns
/// the new tree id.
#[track_caller]
fn create_tree_executable(
    store: &Arc<Store>,
    repo_path: &RepoPath,
    executable: bool,
) -> MergedTree {
    let mut tree_builder = TestTreeBuilder::new(store.clone());
    tree_builder.file(repo_path, "").executable(executable);
    tree_builder.write_merged_tree()
}

/// Build two commits that write the executable bit of a file as true/false.
#[track_caller]
fn prepare_exec_commits(ws: &TestWorkspace, repo_path: &RepoPath) -> (Commit, Commit) {
    let store = ws.repo.store();
    let tree_exec = create_tree_executable(store, repo_path, true);
    let tree_no_exec = create_tree_executable(store, repo_path, false);

    let commit_with_id = |id| testutils::commit_with_tree(ws.repo.store(), id);
    let commit_exec = commit_with_id(tree_exec);
    let commit_no_exec = commit_with_id(tree_no_exec);
    assert_ne!(commit_exec, commit_no_exec);

    (commit_exec, commit_no_exec)
}

/// Test that checking out a tree writes the correct executable bit to the
/// filesystem.
#[test]
fn test_exec_bit_checkout() {
    let mut ws = TestWorkspace::init();
    let path = &ws.workspace.workspace_root().join("file");
    let repo_path = repo_path("file");

    let (exec, no_exec) = prepare_exec_commits(&ws, repo_path);
    let mut checkout_exec_commit = |executable| {
        let commit = if executable { &exec } else { &no_exec };
        let op_id = ws.repo.op_id().clone();
        ws.workspace.check_out(op_id, None, commit).unwrap();
    };

    // Checkout commits and ensure the filesystem is updated correctly.
    assert!(!fs::exists(path).unwrap());
    for exec in [true, false, true] {
        checkout_exec_commit(exec);
        assert_file_executable(path, exec);
    }
}

/// Test that snapshotting files stores the correct executable bit in the tree.
#[test]
fn test_exec_bit_snapshot() {
    let mut ws = TestWorkspace::init();
    let path = &ws.workspace.workspace_root().join("file");
    let repo_path = repo_path("file");

    // Snapshot, then assert the tree has the expected executable bit.
    let mut snapshot_assert_exec_bit = |expected| {
        let merged_tree_val = ws.snapshot().unwrap().path_value(repo_path).unwrap();
        let tree_val = merged_tree_val.into_resolved().unwrap().unwrap();
        assert_tree_executable(tree_val, expected);
    };

    // Snapshot tree values when the file is/isn't executable.
    fs::write(path, "initial content").unwrap();
    snapshot_assert_exec_bit(false);

    fs::write(path, "first change").unwrap();
    snapshot_assert_exec_bit(false);

    set_file_executable(path, true);
    snapshot_assert_exec_bit(true);

    fs::write(path, "second change").unwrap();
    snapshot_assert_exec_bit(true);

    // Back to the same contents as before, but different exec bit.
    fs::write(path, "first change").unwrap();
    set_file_executable(path, false);
    snapshot_assert_exec_bit(false);

    set_file_executable(path, true);
    snapshot_assert_exec_bit(true);
}
