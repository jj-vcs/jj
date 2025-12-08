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

use testutils::git;

use crate::common::TestEnvironment;

#[test]
fn test_git_root_git_backend_noncolocated() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/repo/.jj/repo/store/git
    [EOF]
    ");
}

#[test]
fn test_git_root_git_backend_colocated() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/repo/.git
    [EOF]
    ");
}

#[test]
fn test_git_root_git_backend_external_git_dir() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("").create_dir("repo");
    let git_repo_work_dir = test_env.work_dir("git-repo");
    let git_repo = git::init(git_repo_work_dir.root());

    // Create an initial commit in Git
    let tree_id = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file",
        b"contents",
        "initial",
        &[],
    )
    .tree_id;
    git::checkout_tree_index(&git_repo, tree_id);
    assert_eq!(git_repo_work_dir.read_file("file"), b"contents");
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );

    work_dir
        .run_jj([
            "git",
            "init",
            "--git-repo",
            git_repo_work_dir.root().to_str().unwrap(),
        ])
        .success();

    let output = work_dir.run_jj(["git", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/git-repo/.git
    [EOF]
    ");
}

#[test]
fn test_git_root_simple_backend() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["debug", "init-simple", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "root"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The repo is not backed by a Git repo
    [EOF]
    [exit status: 1]
    ");
}
