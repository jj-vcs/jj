// Copyright 2023 The Jujutsu Authors
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
use std::path::Path;

use crate::common::TestEnvironment;

pub mod common;

/// Add a remote containing a branch with the same name
fn add_git_remote(test_env: &TestEnvironment, repo_path: &Path, remote: &str) {
    let git_repo_path = test_env.env_root().join(remote);
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    let signature =
        git2::Signature::new("Some One", "some.one@example.com", &git2::Time::new(0, 0)).unwrap();
    let mut tree_builder = git_repo.treebuilder(None).unwrap();
    let file_oid = git_repo.blob(remote.as_bytes()).unwrap();
    tree_builder
        .insert("file", file_oid, git2::FileMode::Blob.into())
        .unwrap();
    let tree_oid = tree_builder.write().unwrap();
    let tree = git_repo.find_tree(tree_oid).unwrap();
    git_repo
        .commit(
            Some(&format!("refs/heads/{remote}")),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap();
    test_env.jj_cmd_success(
        repo_path,
        &["git", "remote", "add", remote, &format!("../{remote}")],
    );
}

fn get_branch_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["branch", "list"])
}

fn create_commit(test_env: &TestEnvironment, repo_path: &Path, name: &str, parents: &[&str]) {
    let descr = format!("descr_for_{name}");
    if parents.is_empty() {
        test_env.jj_cmd_success(repo_path, &["new", "root", "-m", &descr]);
    } else {
        let mut args = vec!["new", "-m", &descr];
        args.extend(parents);
        test_env.jj_cmd_success(repo_path, &args);
    }
    std::fs::write(repo_path.join(name), format!("{name}\n")).unwrap();
    test_env.jj_cmd_success(repo_path, &["branch", "create", name]);
}

fn get_log_output(test_env: &TestEnvironment, workspace_root: &Path) -> String {
    test_env.jj_cmd_success(
        workspace_root,
        &[
            "log",
            "-T",
            r#"commit_id.short() " " description.first_line() " " branches"#,
            "-r",
            "all()",
        ],
    )
}

#[test]
fn test_git_fetch_default_remote() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin: ffecd2d67827 message
    "###);
}

#[test]
fn test_git_fetch_single_remote() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    test_env.jj_cmd_success(&repo_path, &["git", "fetch", "--remote", "rem1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: 6a21102783e8 message
    "###);
}

#[test]
fn test_git_fetch_single_remote_from_config() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    test_env.add_config(r#"git.fetch = "rem1""#);

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: 6a21102783e8 message
    "###);
}

#[test]
fn test_git_fetch_multiple_remotes() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");

    test_env.jj_cmd_success(
        &repo_path,
        &["git", "fetch", "--remote", "rem1", "--remote", "rem2"],
    );
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: 6a21102783e8 message
    rem2: 2497a8a08f85 message
    "###);
}

#[test]
fn test_git_fetch_multiple_remotes_from_config() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");
    test_env.add_config(r#"git.fetch = ["rem1", "rem2"]"#);

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: 6a21102783e8 message
    rem2: 2497a8a08f85 message
    "###);
}

#[test]
fn test_git_fetch_nonexistent_remote() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    let stderr = &test_env.jj_cmd_failure(
        &repo_path,
        &["git", "fetch", "--remote", "rem1", "--remote", "rem2"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: No git remote named 'rem2'
    "###);
    // No remote should have been fetched as part of the failing transaction
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @"");
}

#[test]
fn test_git_fetch_nonexistent_remote_from_config() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    test_env.add_config(r#"git.fetch = ["rem1", "rem2"]"#);

    let stderr = &test_env.jj_cmd_failure(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: No git remote named 'rem2'
    "###);
    // No remote should have been fetched as part of the failing transaction
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @"");
}

#[test]
fn test_git_fetch_prune_before_updating_tips() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");
    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin: ffecd2d67827 message
    "###);

    // Remove origin branch in git repo and create origin/subname
    let git_repo = git2::Repository::open(test_env.env_root().join("origin")).unwrap();
    git_repo
        .find_branch("origin", git2::BranchType::Local)
        .unwrap()
        .rename("origin/subname", false)
        .unwrap();

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin/subname: ffecd2d67827 message
    "###);
}

#[test]
fn test_git_fetch_conflicting_branches() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    // Create a rem1 branch locally
    test_env.jj_cmd_success(&repo_path, &["new", "root"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "create", "rem1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: fcdbbd731496 (no description set)
    "###);

    test_env.jj_cmd_success(
        &repo_path,
        &["git", "fetch", "--remote", "rem1", "--branch", "*"],
    );
    // This should result in a CONFLICTED branch
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1 (conflicted):
      + fcdbbd731496 (no description set)
      + 6a21102783e8 message
      @rem1 (behind by 1 commits): 6a21102783e8 message
    "###);
}

#[test]
fn test_git_fetch_conflicting_branches_colocated() {
    let test_env = TestEnvironment::default();
    let repo_path = test_env.env_root().join("repo");
    let _git_repo = git2::Repository::init(&repo_path).unwrap();
    // create_colocated_repo_and_branches_from_trunk1(&test_env, &repo_path);
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo", "."]);
    add_git_remote(&test_env, &repo_path, "rem1");
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @"");

    // Create a rem1 branch locally
    test_env.jj_cmd_success(&repo_path, &["new", "root"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "create", "rem1"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1: f652c32197cf (no description set)
    "###);

    test_env.jj_cmd_success(
        &repo_path,
        &["git", "fetch", "--remote", "rem1", "--branch", "rem1"],
    );
    // This should result in a CONFLICTED branch
    // See https://github.com/martinvonz/jj/pull/1146#discussion_r1112372340 for the bug this tests for.
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    rem1 (conflicted):
      + f652c32197cf (no description set)
      + 6a21102783e8 message
      @rem1 (behind by 1 commits): 6a21102783e8 message
    "###);
}

// Helper functions to test obtaining multiple branches at once and changed
// branches
fn create_colocated_repo_and_branches_from_trunk1(
    test_env: &TestEnvironment,
    repo_path: &Path,
) -> String {
    // Create a colocated repo in `source` to populate it more easily
    test_env.jj_cmd_success(repo_path, &["init", "--git-repo", "."]);
    create_commit(test_env, repo_path, "trunk1", &[]);
    create_commit(test_env, repo_path, "a1", &["trunk1"]);
    create_commit(test_env, repo_path, "a2", &["trunk1"]);
    create_commit(test_env, repo_path, "b", &["trunk1"]);
    format!(
        "   ===== Source git repo contents =====\n{}",
        get_log_output(test_env, repo_path)
    )
}

fn create_trunk2_and_rebase_branches(test_env: &TestEnvironment, repo_path: &Path) -> String {
    create_commit(test_env, repo_path, "trunk2", &["trunk1"]);
    for br in ["a1", "a2", "b"] {
        test_env.jj_cmd_success(repo_path, &["rebase", "-b", br, "-d", "trunk2"]);
    }
    format!(
        "   ===== Source git repo contents =====\n{}",
        get_log_output(test_env, repo_path)
    )
}

#[test]
fn test_git_fetch_all() {
    let test_env = TestEnvironment::default();
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let stdout =
        test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "source", "target"]);
    insta::assert_snapshot!(stdout, @r###"
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    "###);
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_branches_from_trunk1(&test_env, &source_git_repo_path);
    insta::assert_snapshot!(source_log, @r###"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    o  000000000000
    "###);

    // Nothing in our repo before the fetch
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    @  230dd059e1b0
    o  000000000000
    "###);
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @"");
    insta::assert_snapshot!(test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch"]), @"");
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    a1: 359a9a02457d descr_for_a1
    a2: decaa3966c83 descr_for_a2
    b: c7d4bdcbc215 descr_for_b
    master: ff36dc55760e descr_for_trunk1
    trunk1: ff36dc55760e descr_for_trunk1
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  c7d4bdcbc215 descr_for_b b
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);

    // ==== Change both repos ====
    // First, change the target repo:
    let source_log = create_trunk2_and_rebase_branches(&test_env, &source_git_repo_path);
    insta::assert_snapshot!(source_log, @r###"
       ===== Source git repo contents =====
    o  babc49226c14 descr_for_b b
    │ o  91e46b4b2653 descr_for_a2 a2
    ├─╯
    │ o  0424f6dfc1ff descr_for_a1 a1
    ├─╯
    @  8f1f14fbbf42 descr_for_trunk2 trunk2
    o  ff36dc55760e descr_for_trunk1 master trunk1
    o  000000000000
    "###);
    // Change a branch in the source repo as well, so that it becomes conflicted.
    test_env.jj_cmd_success(
        &target_jj_repo_path,
        &["describe", "b", "-m=new_descr_for_b_to_create_conflict"],
    );

    // Our repo before and after fetch
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  061eddbb43ab new_descr_for_b_to_create_conflict b*
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    a1: 359a9a02457d descr_for_a1
    a2: decaa3966c83 descr_for_a2
    b: 061eddbb43ab new_descr_for_b_to_create_conflict
      @origin (ahead by 1 commits, behind by 1 commits): c7d4bdcbc215 descr_for_b
    master: ff36dc55760e descr_for_trunk1
    trunk1: ff36dc55760e descr_for_trunk1
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch"]), @"");
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    a1: 0424f6dfc1ff descr_for_a1
    a2: 91e46b4b2653 descr_for_a2
    b (conflicted):
      - c7d4bdcbc215 descr_for_b
      + 061eddbb43ab new_descr_for_b_to_create_conflict
      + babc49226c14 descr_for_b
      @origin (behind by 1 commits): babc49226c14 descr_for_b
    master: ff36dc55760e descr_for_trunk1
    trunk1: ff36dc55760e descr_for_trunk1
    trunk2: 8f1f14fbbf42 descr_for_trunk2
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  babc49226c14 descr_for_b b?? b@origin
    │ o  91e46b4b2653 descr_for_a2 a2
    ├─╯
    │ o  0424f6dfc1ff descr_for_a1 a1
    ├─╯
    o  8f1f14fbbf42 descr_for_trunk2 trunk2
    │ o  061eddbb43ab new_descr_for_b_to_create_conflict b??
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
}

#[test]
fn test_git_fetch_some_of_many_branches() {
    let test_env = TestEnvironment::default();
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let stdout =
        test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "source", "target"]);
    insta::assert_snapshot!(stdout, @r###"
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    "###);
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_branches_from_trunk1(&test_env, &source_git_repo_path);
    insta::assert_snapshot!(source_log, @r###"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    o  000000000000
    "###);

    // Test an error message
    let stderr =
        test_env.jj_cmd_failure(&target_jj_repo_path, &["git", "fetch", "--branch", "^:a*"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Invalid glob provided. Globs may not contain the characters `:` or `^`.
    "###);

    // Nothing in our repo before the fetch
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    @  230dd059e1b0
    o  000000000000
    "###);
    // Fetch one branch...
    let stdout = test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch", "--branch", "b"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  c7d4bdcbc215 descr_for_b b
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    // ...check what the intermediate state looks like...
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    b: c7d4bdcbc215 descr_for_b
    "###);
    // ...then fetch two others with a glob.
    let stdout = test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch", "--branch", "a*"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  decaa3966c83 descr_for_a2 a2
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    │ o  c7d4bdcbc215 descr_for_b b
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    // Fetching the same branch again
    let stdout = test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch", "--branch", "a1"]);
    insta::assert_snapshot!(stdout, @r###"
    Nothing changed.
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  decaa3966c83 descr_for_a2 a2
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    │ o  c7d4bdcbc215 descr_for_b b
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);

    // ==== Change both repos ====
    // First, change the target repo:
    let source_log = create_trunk2_and_rebase_branches(&test_env, &source_git_repo_path);
    insta::assert_snapshot!(source_log, @r###"
       ===== Source git repo contents =====
    o  13ac032802f1 descr_for_b b
    │ o  010977d69c5b descr_for_a2 a2
    ├─╯
    │ o  6f4e1c4dfe29 descr_for_a1 a1
    ├─╯
    @  09430ba04a82 descr_for_trunk2 trunk2
    o  ff36dc55760e descr_for_trunk1 master trunk1
    o  000000000000
    "###);
    // Change a branch in the source repo as well, so that it becomes conflicted.
    test_env.jj_cmd_success(
        &target_jj_repo_path,
        &["describe", "b", "-m=new_descr_for_b_to_create_conflict"],
    );

    // Our repo before and after fetch of two branches
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  2be688d8c664 new_descr_for_b_to_create_conflict b*
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    let stdout = test_env.jj_cmd_success(
        &target_jj_repo_path,
        &["git", "fetch", "--branch", "b", "--branch", "a1"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  13ac032802f1 descr_for_b b?? b@origin
    │ o  6f4e1c4dfe29 descr_for_a1 a1
    ├─╯
    o  09430ba04a82 descr_for_trunk2
    │ o  2be688d8c664 new_descr_for_b_to_create_conflict b??
    ├─╯
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);

    // We left a2 where it was before, let's see how `jj branch list` sees this.
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    a1: 6f4e1c4dfe29 descr_for_a1
    a2: decaa3966c83 descr_for_a2
    b (conflicted):
      - c7d4bdcbc215 descr_for_b
      + 2be688d8c664 new_descr_for_b_to_create_conflict
      + 13ac032802f1 descr_for_b
      @origin (behind by 1 commits): 13ac032802f1 descr_for_b
    "###);
    // Now, let's fetch a2 and double-check that fetching a1 and b again doesn't do
    // anything.
    let stdout = test_env.jj_cmd_success(
        &target_jj_repo_path,
        &["git", "fetch", "--branch", "b", "--branch", "a*"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  010977d69c5b descr_for_a2 a2
    │ o  13ac032802f1 descr_for_b b?? b@origin
    ├─╯
    │ o  6f4e1c4dfe29 descr_for_a1 a1
    ├─╯
    o  09430ba04a82 descr_for_trunk2
    │ o  2be688d8c664 new_descr_for_b_to_create_conflict b??
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    insta::assert_snapshot!(get_branch_output(&test_env, &target_jj_repo_path), @r###"
    a1: 6f4e1c4dfe29 descr_for_a1
    a2: 010977d69c5b descr_for_a2
    b (conflicted):
      - c7d4bdcbc215 descr_for_b
      + 2be688d8c664 new_descr_for_b_to_create_conflict
      + 13ac032802f1 descr_for_b
      @origin (behind by 1 commits): 13ac032802f1 descr_for_b
    "###);
}

// TODO: Fix the bug this test demonstrates. (https://github.com/martinvonz/jj/issues/1300)
// The issue likely stems from the fact that `jj undo` does not undo the fetch
// inside the git repo backing the `target` repo. It is unclear whether it
// should.
#[test]
fn test_git_fetch_undo() {
    let test_env = TestEnvironment::default();
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let stdout =
        test_env.jj_cmd_success(test_env.env_root(), &["git", "clone", "source", "target"]);
    insta::assert_snapshot!(stdout, @r###"
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    "###);
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_branches_from_trunk1(&test_env, &source_git_repo_path);
    insta::assert_snapshot!(source_log, @r###"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ o  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1 master trunk1
    o  000000000000
    "###);

    // Fetch 2 branches
    let stdout = test_env.jj_cmd_success(
        &target_jj_repo_path,
        &["git", "fetch", "--branch", "b", "--branch", "a1"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  c7d4bdcbc215 descr_for_b b
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&target_jj_repo_path, &["undo"]), @"");
    // The undo works as expected
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    @  230dd059e1b0
    o  000000000000
    "###);
    // Now try to fetch just one branch
    let stdout = test_env.jj_cmd_success(&target_jj_repo_path, &["git", "fetch", "--branch", "b"]);
    insta::assert_snapshot!(stdout, @"");
    // BUG: Both branches got fetched.
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r###"
    o  c7d4bdcbc215 descr_for_b b
    │ o  359a9a02457d descr_for_a1 a1
    ├─╯
    o  ff36dc55760e descr_for_trunk1
    │ @  230dd059e1b0
    ├─╯
    o  000000000000
    "###);
}

#[test]
fn test_git_fetch_remove_fetch() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env.jj_cmd_success(&repo_path, &["branch", "set", "origin"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin: 230dd059e1b0 (no description set)
    "###);

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin (conflicted):
      + 230dd059e1b0 (no description set)
      + ffecd2d67827 message
      @origin (behind by 1 commits): ffecd2d67827 message
    "###);

    test_env.jj_cmd_success(&repo_path, &["git", "remote", "remove", "origin"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin (conflicted):
      + 230dd059e1b0 (no description set)
      + ffecd2d67827 message
    "###);

    test_env.jj_cmd_success(&repo_path, &["git", "remote", "add", "origin", "../origin"]);

    // Check that origin@origin is properly recreated
    let stdout = test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin (conflicted):
      + 230dd059e1b0 (no description set)
      + ffecd2d67827 message
      @origin (behind by 1 commits): ffecd2d67827 message
    "###);
}

#[test]
fn test_git_fetch_rename_fetch() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env.jj_cmd_success(&repo_path, &["branch", "set", "origin"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin: 230dd059e1b0 (no description set)
    "###);

    test_env.jj_cmd_success(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin (conflicted):
      + 230dd059e1b0 (no description set)
      + ffecd2d67827 message
      @origin (behind by 1 commits): ffecd2d67827 message
    "###);

    test_env.jj_cmd_success(
        &repo_path,
        &["git", "remote", "rename", "origin", "upstream"],
    );
    insta::assert_snapshot!(get_branch_output(&test_env, &repo_path), @r###"
    origin (conflicted):
      + 230dd059e1b0 (no description set)
      + ffecd2d67827 message
      @upstream (behind by 1 commits): ffecd2d67827 message
    "###);

    // Check that jj indicates that nothing has changed
    let stdout = test_env.jj_cmd_success(&repo_path, &["git", "fetch", "--remote", "upstream"]);
    insta::assert_snapshot!(stdout, @r###"
    Nothing changed.
    "###);
}
