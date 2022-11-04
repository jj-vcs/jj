// Copyright 2022 Google LLC
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

use crate::common::TestEnvironment;

pub mod common;

#[test]
fn test_git_clone() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(&git_repo_path).unwrap();

    // Clone an empty repo
    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "source", "empty"]);
    insta::assert_snapshot!(stdout, @r###"
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    "###);

    // Clone a non-empty repo
    let signature =
        git2::Signature::new("Some One", "some.one@example.com", &git2::Time::new(0, 0)).unwrap();
    let mut tree_builder = git_repo.treebuilder(None).unwrap();
    let file_oid = git_repo.blob(b"content").unwrap();
    tree_builder
        .insert("file", file_oid, git2::FileMode::Blob.into())
        .unwrap();
    let tree_oid = tree_builder.write().unwrap();
    let tree = git_repo.find_tree(tree_oid).unwrap();
    git_repo
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap();
    git_repo.set_head("refs/heads/main").unwrap();
    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(stdout, @r###"
    Fetching into new repo in "$TEST_ENV/clone"
    Working copy now at: 1f0b881a057d (no description set)
    Added 1 files, modified 0 files, removed 0 files
    "###);
    assert!(test_env.env_root().join("clone").join("file").exists());

    // Try cloning into an existing workspace
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);

    // Try cloning into an existing file
    std::fs::write(test_env.env_root().join("file"), "contents").unwrap();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "file"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);

    // Try cloning into non-empty, non-workspace directory
    std::fs::remove_dir_all(test_env.env_root().join("clone").join(".jj")).unwrap();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
}
