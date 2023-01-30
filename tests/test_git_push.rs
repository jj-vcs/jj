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

use std::path::PathBuf;

use crate::common::TestEnvironment;

pub mod common;

fn set_up() -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "origin"]);
    let origin_path = test_env.env_root().join("origin");
    let origin_git_repo_path = origin_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    test_env.jj_cmd_success(&origin_path, &["describe", "-m=description 1"]);
    test_env.jj_cmd_success(&origin_path, &["branch", "create", "branch1"]);
    test_env.jj_cmd_success(&origin_path, &["new", "root", "-m=description 2"]);
    test_env.jj_cmd_success(&origin_path, &["branch", "create", "branch2"]);
    test_env.jj_cmd_success(&origin_path, &["git", "export"]);

    test_env.jj_cmd_success(
        test_env.env_root(),
        &[
            "git",
            "clone",
            origin_git_repo_path.to_str().unwrap(),
            "local",
        ],
    );
    let workspace_root = test_env.env_root().join("local");
    (test_env, workspace_root)
}

#[test]
fn test_git_push_nothing() {
    let (test_env, workspace_root) = set_up();
    // No branches to push yet
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stdout, @r###"
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_current_branch() {
    let (test_env, workspace_root) = set_up();
    // Update some branches. `branch1` is not a current branch, but `branch2` and
    // `my-branch` are.
    test_env.jj_cmd_success(
        &workspace_root,
        &["describe", "branch1", "-m", "modified branch1 commit"],
    );
    test_env.jj_cmd_success(&workspace_root, &["co", "branch2"]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "set", "branch2"]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "foo"]);
    // Check the setup
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    branch1: 19e00bf64429 modified branch1 commit
      @origin (ahead by 1 commits, behind by 1 commits): 45a3aa29e907 description 1
    branch2: 10ee3363b259 foo
      @origin (behind by 1 commits): 8476341eb395 description 2
    my-branch: 10ee3363b259 foo
    "###);
    // First dry-run. `branch1` should not get pushed.
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--dry-run"]);
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Move branch branch2 from 8476341eb395 to 10ee3363b259
      Add branch my-branch to 10ee3363b259
    Dry-run requested, not pushing.
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Move branch branch2 from 8476341eb395 to 10ee3363b259
      Add branch my-branch to 10ee3363b259
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    branch1: 19e00bf64429 modified branch1 commit
      @origin (ahead by 1 commits, behind by 1 commits): 45a3aa29e907 description 1
    branch2: 10ee3363b259 foo
    my-branch: 10ee3363b259 foo
    "###);
}

#[test]
fn test_git_push_parent_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["edit", "branch1"]);
    test_env.jj_cmd_success(
        &workspace_root,
        &["describe", "-m", "modified branch1 commit"],
    );
    test_env.jj_cmd_success(&workspace_root, &["new"]);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--dry-run"]);
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Force branch branch1 from 45a3aa29e907 to d47326d59ee1
    Dry-run requested, not pushing.
    "###);
}

#[test]
fn test_git_push_no_current_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["new"]);
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: No current branch.
    "###);
}

#[test]
fn test_git_push_current_branch_unchanged() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["co", "branch1"]);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push"]);
    insta::assert_snapshot!(stdout, @r###"
    Nothing changed.
    "###);
}

#[test]
fn test_git_push_multiple() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["branch", "delete", "branch1"]);
    test_env.jj_cmd_success(
        &workspace_root,
        &["branch", "set", "--allow-backwards", "branch2"],
    );
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "foo"]);
    // Check the setup
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    branch1 (deleted)
      @origin: 45a3aa29e907 description 1
    branch2: 15dcdaa4f12f foo
      @origin (ahead by 1 commits, behind by 1 commits): 8476341eb395 description 2
    my-branch: 15dcdaa4f12f foo
    "###);
    // First dry-run
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--all", "--dry-run"]);
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 45a3aa29e907
      Force branch branch2 from 8476341eb395 to 15dcdaa4f12f
      Add branch my-branch to 15dcdaa4f12f
    Dry-run requested, not pushing.
    "###);
    // Dry run requesting two specific branches
    let stdout = test_env.jj_cmd_success(
        &workspace_root,
        &["git", "push", "-b=branch1", "-b=my-branch", "--dry-run"],
    );
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 45a3aa29e907
      Add branch my-branch to 15dcdaa4f12f
    Dry-run requested, not pushing.
    "###);
    // Dry run requesting two specific branches twice
    let stdout = test_env.jj_cmd_success(
        &workspace_root,
        &[
            "git",
            "push",
            "-b=branch1",
            "-b=my-branch",
            "-b=branch1",
            "-b=my-branch",
            "--dry-run",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 45a3aa29e907
      Add branch my-branch to 15dcdaa4f12f
    Dry-run requested, not pushing.
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Delete branch branch1 from 45a3aa29e907
      Force branch branch2 from 8476341eb395 to 15dcdaa4f12f
      Add branch my-branch to 15dcdaa4f12f
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_root, &["branch", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    branch2: 15dcdaa4f12f foo
    my-branch: 15dcdaa4f12f foo
    "###);
}

#[test]
fn test_git_push_changes() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["new", "-m", "bar"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();

    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--change", "@"]);
    insta::assert_snapshot!(stdout, @r###"
    Creating branch push-1b76972398e6 for revision @
    Branch changes to push to origin:
      Add branch push-1b76972398e6 to 28d7620ea63a
    "###);
    // test pushing two changes at once
    std::fs::write(workspace_root.join("file"), "modified2").unwrap();
    let stdout = test_env.jj_cmd_success(
        &workspace_root,
        &["git", "push", "--change", "@", "--change", "@-"],
    );
    insta::assert_snapshot!(stdout, @r###"
    Creating branch push-19b790168e73 for revision @-
    Branch changes to push to origin:
      Force branch push-1b76972398e6 from 28d7620ea63a to 48d8c7948133
      Add branch push-19b790168e73 to fa16a14170fb
    "###);
    // specifying the same change twice doesn't break things
    std::fs::write(workspace_root.join("file"), "modified3").unwrap();
    let stdout = test_env.jj_cmd_success(
        &workspace_root,
        &["git", "push", "--change", "@", "--change", "@"],
    );
    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Force branch push-1b76972398e6 from 48d8c7948133 to b5f030322b1d
    "###);
}

#[test]
fn test_git_push_existing_long_branch() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(
        &workspace_root,
        &["branch", "create", "push-19b790168e73f7a73a98deae21e807c0"],
    );

    let stdout = test_env.jj_cmd_success(&workspace_root, &["git", "push", "--change=@"]);

    insta::assert_snapshot!(stdout, @r###"
    Branch changes to push to origin:
      Add branch push-19b790168e73f7a73a98deae21e807c0 to fa16a14170fb
    "###);
}

#[test]
fn test_git_push_unsnapshotted_change() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "foo"]);
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["git", "push", "--change", "@"]);
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["git", "push", "--change", "@"]);
}

#[test]
fn test_git_push_conflict() {
    let (test_env, workspace_root) = set_up();
    std::fs::write(workspace_root.join("file"), "first").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["commit", "-m", "first"]);
    std::fs::write(workspace_root.join("file"), "second").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["commit", "-m", "second"]);
    std::fs::write(workspace_root.join("file"), "third").unwrap();
    test_env.jj_cmd_success(&workspace_root, &["rebase", "-r", "@", "-d", "@--"]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "set", "my-branch"]);
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m", "third"]);
    let stderr = test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--all"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 3a1497bff04c since it has conflicts
    "###);
}

#[test]
fn test_git_push_no_description() {
    let (test_env, workspace_root) = set_up();
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "my-branch"]);
    test_env.jj_cmd_success(&workspace_root, &["describe", "-m="]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch", "my-branch"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 5b36783cd11c since it has no description
    "###);
}

#[test]
fn test_git_push_missing_author() {
    let (test_env, workspace_root) = set_up();
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .jj_cmd(&workspace_root, args)
            .env_remove(var)
            .assert()
            .success();
    };
    run_without_var("JJ_USER", &["checkout", "root", "-m=initial"]);
    run_without_var("JJ_USER", &["branch", "create", "missing-name"]);
    let stderr = test_env.jj_cmd_failure(
        &workspace_root,
        &["git", "push", "--branch", "missing-name"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 574dffd73428 since it has no author and/or committer set
    "###);
    run_without_var("JJ_EMAIL", &["checkout", "root", "-m=initial"]);
    run_without_var("JJ_EMAIL", &["branch", "create", "missing-email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit e6c50f13f197 since it has no author and/or committer set
    "###);
}

#[test]
fn test_git_push_missing_committer() {
    let (test_env, workspace_root) = set_up();
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .jj_cmd(&workspace_root, args)
            .env_remove(var)
            .assert()
            .success();
    };
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "missing-name"]);
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-name"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit e009726caa4a since it has no author and/or committer set
    "###);
    test_env.jj_cmd_success(&workspace_root, &["checkout", "root"]);
    test_env.jj_cmd_success(&workspace_root, &["branch", "create", "missing-email"]);
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit 27ec5f0793e6 since it has no author and/or committer set
    "###);

    // Test message when there are multiple reasons (missing committer and
    // description)
    run_without_var("JJ_EMAIL", &["describe", "-m=", "missing-email"]);
    let stderr =
        test_env.jj_cmd_failure(&workspace_root, &["git", "push", "--branch=missing-email"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Won't push commit f73024ee65ec since it has no description and it has no author and/or committer set
    "###);
}
