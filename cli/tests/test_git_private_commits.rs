// Copyright 2024 The Jujutsu Authors
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
use crate::common::TestWorkDir;

fn set_up(test_env: &TestEnvironment) {
    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    origin_dir.run_jj(["describe", "-m=public 1"]).success();
    origin_dir.run_jj(["new", "-m=public 2"]).success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "clone",
                "--config=remotes.origin.auto-track-bookmarks='*'",
                origin_git_repo_path.to_str().unwrap(),
                "local",
            ],
        )
        .success();
}

fn set_up_remote_at_main(test_env: &TestEnvironment, work_dir: &TestWorkDir, remote_name: &str) {
    test_env
        .run_jj_in(".", ["git", "init", remote_name])
        .success();
    let other_path = test_env.env_root().join(remote_name);
    let other_git_repo_path = other_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            remote_name,
            other_git_repo_path.to_str().unwrap(),
        ])
        .success();
    work_dir
        .run_jj([
            "git",
            "push",
            "--allow-new",
            "--remote",
            remote_name,
            "-b=main",
        ])
        .success();
}

#[test]
fn test_git_private_commits_block_pushing() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir.run_jj(["new", "main", "-m=private 1"]).success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r@"])
        .success();

    // Will not push when a pushed commit is contained in git.private-commits
    test_env.add_config(r#"git.private-commits = "description('private*')""#);
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 7f665ca27d4e since it is private
    Hint: Rejected commit: yqosqzyt 7f665ca2 main* | (empty) private 1
    Hint: Configured git.private-commits: 'description('private*')'
    [EOF]
    [exit status: 1]
    ");

    // May push when the commit is removed from git.private-commits
    test_env.add_config(r#"git.private-commits = "none()""#);
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark main from 95cc152cd086 to 7f665ca27d4e
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy  (@) now at: znkkpsqq 8227d51b (empty) (no description set)
    Parent commit (@-)      : yqosqzyt 7f665ca2 main | (empty) private 1
    [EOF]
    ");
}

#[test]
fn test_git_private_commits_can_be_overridden() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir.run_jj(["new", "main", "-m=private 1"]).success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r@"])
        .success();

    // Will not push when a pushed commit is contained in git.private-commits
    test_env.add_config(r#"git.private-commits = "description('private*')""#);
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 7f665ca27d4e since it is private
    Hint: Rejected commit: yqosqzyt 7f665ca2 main* | (empty) private 1
    Hint: Configured git.private-commits: 'description('private*')'
    [EOF]
    [exit status: 1]
    ");

    // May push when the commit is removed from git.private-commits
    let output = work_dir.run_jj(["git", "push", "--all", "--allow-private"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark main from 95cc152cd086 to 7f665ca27d4e
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy  (@) now at: znkkpsqq 8227d51b (empty) (no description set)
    Parent commit (@-)      : yqosqzyt 7f665ca2 main | (empty) private 1
    [EOF]
    ");
}

#[test]
fn test_git_private_commits_are_not_checked_if_immutable() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir.run_jj(["new", "main", "-m=private 1"]).success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r@"])
        .success();

    test_env.add_config(r#"git.private-commits = "description('private*')""#);
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "all()""#);
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark main from 95cc152cd086 to 7f665ca27d4e
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy  (@) now at: yostqsxw 17947f20 (empty) (no description set)
    Parent commit (@-)      : yqosqzyt 7f665ca2 main | (empty) private 1
    [EOF]
    ");
}

#[test]
fn test_git_private_commits_not_directly_in_line_block_pushing() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    test_env.add_config("remotes.origin.auto-track-bookmarks = '*'");
    let work_dir = test_env.work_dir("local");

    // New private commit descended from root()
    work_dir.run_jj(["new", "root()", "-m=private 1"]).success();

    work_dir
        .run_jj(["new", "main", "@", "-m=public 3"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark1"])
        .success();

    test_env.add_config(r#"git.private-commits = "description('private*')""#);
    let output = work_dir.run_jj(["git", "push", "-b=bookmark1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 613114f44bdd since it is private
    Hint: Rejected commit: yqosqzyt 613114f4 (empty) private 1
    Hint: Configured git.private-commits: 'description('private*')'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_private_commits_descending_from_commits_pushed_do_not_block_pushing() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir.run_jj(["new", "main", "-m=public 3"]).success();
    work_dir
        .run_jj(["bookmark", "move", "main", "--to=@"])
        .success();
    work_dir.run_jj(["new", "-m=private 1"]).success();

    test_env.add_config(r#"git.private-commits = "description('private*')""#);
    let output = work_dir.run_jj(["git", "push", "-b=main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark main from 95cc152cd086 to f0291dea729d
    [EOF]
    ");
}

#[test]
fn test_git_private_commits_already_on_the_remote_do_not_block_push() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    test_env.add_config("remotes.origin.auto-track-bookmarks = '*'");
    let work_dir = test_env.work_dir("local");

    // Start a bookmark before a "private" commit lands in main
    work_dir
        .run_jj(["bookmark", "create", "bookmark1", "-r=main"])
        .success();

    // Push a commit that would become a private_root if it weren't already on
    // the remote
    work_dir.run_jj(["new", "main", "-m=private 1"]).success();
    work_dir.run_jj(["new", "-m=public 3"]).success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r@"])
        .success();
    let output = work_dir.run_jj(["git", "push", "-b=main", "-b=bookmark1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark1 to 95cc152cd086
      Move forward bookmark main from 95cc152cd086 to 03bc2bf271e0
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy  (@) now at: kpqxywon 5308110d (empty) (no description set)
    Parent commit (@-)      : yostqsxw 03bc2bf2 main | (empty) public 3
    [EOF]
    ");

    test_env.add_config(r#"git.private-commits = "subject('private*')""#);

    // Since "private 1" is already on the remote, pushing it should be allowed
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r=main"])
        .success();
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark1 from 95cc152cd086 to 03bc2bf271e0
    [EOF]
    ");

    // Ensure that the already-pushed commit doesn't block a new bookmark from
    // being pushed
    work_dir
        .run_jj(["new", "subject('private 1')", "-m=public 4"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark2"])
        .success();
    let output = work_dir.run_jj(["git", "push", "-b=bookmark2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark2 to 987ee765174d
    [EOF]
    ");
}

#[test]
fn test_git_private_commits_are_evaluated_separately_for_each_remote() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    set_up_remote_at_main(&test_env, &work_dir, "other");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);

    // Push a commit that would become a private_root if it weren't already on
    // the remote
    work_dir.run_jj(["new", "main", "-m=private 1"]).success();
    work_dir.run_jj(["new", "-m=public 3"]).success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r@"])
        .success();
    let output = work_dir.run_jj(["git", "push", "-b=main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark main from 95cc152cd086 to 7eb69d0eaf71
    [EOF]
    ");

    test_env.add_config(r#"git.private-commits = "description('private*')""#);

    // But pushing to a repo that doesn't have the private commit yet is still
    // blocked
    let output = work_dir.run_jj(["git", "push", "--remote=other", "-b=main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 469f044473ed since it is private
    Hint: Rejected commit: znkkpsqq 469f0444 (empty) private 1
    Hint: Configured git.private-commits: 'description('private*')'
    [EOF]
    [exit status: 1]
    ");
}
