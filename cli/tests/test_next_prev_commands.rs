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
//

use std::path::Path;

use crate::common::force_interactive;
use crate::common::get_stderr_string;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;

#[test]
fn test_next_simple() {
    // Move from first => second.
    // first
    // |
    // second
    // |
    // third
    //
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    // Create a simple linear history, which we'll traverse.
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Move to `first`
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: vruxwmqv 0c7d7732 (empty) (no description set)
    Parent commit      : kkmpptxz 30056b0c (empty) third
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  vruxwmqvtpmx
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_multiple() {
    // Move from first => fourth.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@---"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  zsuskulnrvyr fourth
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // We should now be the child of the fourth commit.
    let output = test_env.run_jj_in(&repo_path, ["next", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: vruxwmqv 41cc776d (empty) (no description set)
    Parent commit      : zsuskuln 9d7e5e99 (empty) fourth
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  vruxwmqvtpmx
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_prev_simple() {
    // Move @- from third to second.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: royxmykx 6db74f64 (empty) (no description set)
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_prev_multiple_without_root() {
    // Move @- from fourth to second.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: yqosqzyt 794ffd20 (empty) (no description set)
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  yqosqzytrlsw
    │ ○  zsuskulnrvyr fourth
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_exceeding_history() {
    // Try to step beyond the current repos history.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["edit", "-r", "@--"])
        .success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // `jj next` beyond existing history fails.
    let output = test_env.run_jj_in(&repo_path, ["next", "3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No other descendant found 3 commit(s) forward from the working copy parent(s)
    Hint: Working copy parent: qpvuntsm fa15625b (empty) first
    [EOF]
    [exit status: 1]
    ");
}

// The working copy commit is a child of a "fork" with two children on each
// bookmark.
#[test]
fn test_next_parent_has_multiple_descendants() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    // Setup.
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "1"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m", "2"]).success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "3"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m", "4"]).success();
    test_env
        .run_jj_in(&repo_path, ["edit", "description(3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  mzvwutvlkqwt 4
    @  zsuskulnrvyr 3
    │ ○  kkmpptxzrspx 2
    │ ○  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: mzvwutvl 1b8531ce (empty) 4
    Parent commit      : zsuskuln b1394455 (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  mzvwutvlkqwt 4
    ○  zsuskulnrvyr 3
    │ ○  kkmpptxzrspx 2
    │ ○  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_with_merge_commit_parent() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    // Setup.
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "2"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["new", "description(1)", "description(2)", "-m", "3"],
        )
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m", "4"]).success();
    test_env.run_jj_in(&repo_path, ["prev", "0"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  mzvwutvlkqwt 4
    ├─╯
    ○    zsuskulnrvyr 3
    ├─╮
    │ ○  kkmpptxzrspx 2
    ○ │  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: vruxwmqv e2cefcb7 (empty) (no description set)
    Parent commit      : mzvwutvl b54bbdea (empty) 4
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  vruxwmqvtpmx
    ○  mzvwutvlkqwt 4
    ○    zsuskulnrvyr 3
    ├─╮
    │ ○  kkmpptxzrspx 2
    ○ │  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_on_merge_commit() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    // Setup.
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "2"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["new", "description(1)", "description(2)", "-m", "3"],
        )
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m", "4"]).success();
    test_env
        .run_jj_in(&repo_path, ["edit", "description(3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  mzvwutvlkqwt 4
    @    zsuskulnrvyr 3
    ├─╮
    │ ○  kkmpptxzrspx 2
    ○ │  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: mzvwutvl b54bbdea (empty) 4
    Parent commit      : zsuskuln 5542f0b4 (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  mzvwutvlkqwt 4
    ○    zsuskulnrvyr 3
    ├─╮
    │ ○  kkmpptxzrspx 2
    ○ │  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_fails_on_bookmarking_children_no_stdin() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  zsuskulnrvyr third
    ├─╯
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Try to advance the working copy commit.
    let assert = test_env.jj_cmd(&repo_path, &["next"]).assert().code(1);
    let stderr = test_env.normalize_output(get_stderr_string(&assert));
    insta::assert_snapshot!(stderr,@r"
    Error: Cannot prompt for input since the output is not connected to a terminal
    [EOF]
    ");
}

#[test]
fn test_next_fails_on_bookmarking_children_quit_prompt() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  zsuskulnrvyr third
    ├─╯
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Try to advance the working copy commit.
    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .arg("next")
            .write_stdin("q\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous next commit, choose one to target:
    1: zsuskuln 5f24490d (empty) third
    2: rlvkpnrz 9ed53a4a (empty) second
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Error: ambiguous target commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_next_choose_bookmarking_child() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    // Advance the working copy commit.
    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .arg("next")
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous next commit, choose one to target:
    1: royxmykx d00fe885 (empty) fourth
    2: zsuskuln 5f24490d (empty) third
    3: rlvkpnrz 9ed53a4a (empty) second
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Working copy now at: yostqsxw 5c8fa96d (empty) (no description set)
    Parent commit      : zsuskuln 5f24490d (empty) third
    [EOF]
    ");
}

#[test]
fn test_prev_on_merge_commit() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "c", "-r@", "left"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "c", "-r@", "right"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "left", "right"])
        .success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @    royxmykxtrkr
    ├─╮
    │ ○  zsuskulnrvyr right second
    ○ │  qpvuntsmwlqt left first
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: vruxwmqv 41658cf4 (empty) (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["undo"]).success();
    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .args(["prev", "--edit"])
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous prev commit, choose one to target:
    1: zsuskuln b0d21db3 right | (empty) second
    2: qpvuntsm fa15625b left | (empty) first
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Working copy now at: qpvuntsm fa15625b left | (empty) first
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_prev_on_merge_commit_with_parent_merge() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "x"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "y"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["new", "description(x)", "description(y)", "-m", "z"],
        )
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "1"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["new", "description(z)", "description(1)", "-m", "M"],
        )
        .success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @    royxmykxtrkr M
    ├─╮
    │ ○  mzvwutvlkqwt 1
    ○ │    zsuskulnrvyr z
    ├───╮
    │ │ ○  kkmpptxzrspx y
    │ ├─╯
    ○ │  qpvuntsmwlqt x
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .arg("prev")
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous prev commit, choose one to target:
    1: kkmpptxz 146d5c67 (empty) y
    2: qpvuntsm 6799aaa2 (empty) x
    3: zzzzzzzz 00000000 (empty) (no description set)
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Working copy now at: vruxwmqv e5a6794c (empty) (no description set)
    Parent commit      : qpvuntsm 6799aaa2 (empty) x
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["undo"]).success();
    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .args(["prev", "--edit"])
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous prev commit, choose one to target:
    1: mzvwutvl 89b8a355 (empty) 1
    2: zsuskuln a83fc061 (empty) z
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Working copy now at: zsuskuln a83fc061 (empty) z
    Parent commit      : qpvuntsm 6799aaa2 (empty) x
    Parent commit      : kkmpptxz 146d5c67 (empty) y
    [EOF]
    ");
}

#[test]
fn test_prev_prompts_on_multiple_parents() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@--"]).success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    // Create a merge commit, which has two parents.
    test_env
        .run_jj_in(&repo_path, ["new", "all:@--+"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "merge"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "merge+1"])
        .success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  yostqsxwqrlt
    ○  vruxwmqvtpmx merge+1
    ○      yqosqzytrlsw merge
    ├─┬─╮
    │ │ ○  qpvuntsmwlqt first
    │ ○ │  kkmpptxzrspx second
    │ ├─╯
    ○ │  mzvwutvlkqwt third
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Move @ backwards.
    let output = test_env.run_jj_with(|cmd| {
        force_interactive(cmd)
            .current_dir(&repo_path)
            .args(["prev", "2"])
            .write_stdin("3\n")
    });
    insta::assert_snapshot!(output, @r"
    ambiguous prev commit, choose one to target:
    1: mzvwutvl bc4f4fe3 (empty) third
    2: kkmpptxz b0d21db3 (empty) second
    3: qpvuntsm fa15625b (empty) first
    q: quit the prompt
    [EOF]
    ------- stderr -------
    enter the index of the commit you want to target: Working copy now at: kpqxywon ddac00b0 (empty) (no description set)
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kpqxywonksrl
    │ ○  vruxwmqvtpmx merge+1
    │ ○    yqosqzytrlsw merge
    ╭─┼─╮
    ○ │ │  qpvuntsmwlqt first
    │ │ ○  kkmpptxzrspx second
    ├───╯
    │ ○  mzvwutvlkqwt third
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["next"]).success();
    test_env.run_jj_in(&repo_path, ["edit", "@-"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  vruxwmqvtpmx merge+1
    @      yqosqzytrlsw merge
    ├─┬─╮
    │ │ ○  qpvuntsmwlqt first
    │ ○ │  kkmpptxzrspx second
    │ ├─╯
    ○ │  mzvwutvlkqwt third
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--no-edit"]);
    insta::assert_snapshot!(output,@r"
    ------- stderr -------
    Error: No other descendant found 1 commit(s) forward from the working copy parent(s)
    Hint: Working copy parent: mzvwutvl bc4f4fe3 (empty) third
    Hint: Working copy parent: kkmpptxz b0d21db3 (empty) second
    Hint: Working copy parent: qpvuntsm fa15625b (empty) first
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_prev_beyond_root_fails() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    // @- is at "fourth", and there is no parent 5 commits behind it.
    let output = test_env.run_jj_in(&repo_path, ["prev", "5"]);
    insta::assert_snapshot!(output,@r"
    ------- stderr -------
    Error: No ancestor found 5 commit(s) back from the working copy parents(s)
    Hint: Working copy parent: zsuskuln 9d7e5e99 (empty) fourth
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_prev_editing() {
    // Edit the third commit.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    // Edit the "fourth" commit, which becomes the leaf.
    test_env.run_jj_in(&repo_path, ["edit", "@-"]).success();
    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kkmpptxz 30056b0c (empty) third
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  zsuskulnrvyr fourth
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_editing() {
    // Edit the second commit.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    test_env.run_jj_in(&repo_path, ["edit", "@---"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kkmpptxz 30056b0c (empty) third
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  zsuskulnrvyr fourth
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_prev_conflict() {
    // Make the first commit our new parent.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("content.txt");
    std::fs::write(&file_path, "first").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    std::fs::write(&file_path, "second").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    // Create a conflict in the first commit, where we'll jump to.
    test_env
        .run_jj_in(&repo_path, ["edit", "description(first)"])
        .success();
    std::fs::write(&file_path, "first+1").unwrap();
    test_env
        .run_jj_in(&repo_path, ["new", "description(third)"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "fourth"])
        .success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  yqosqzytrlsw conflict
    ×  royxmykxtrkr conflict fourth
    ×  kkmpptxzrspx conflict third
    ×  rlvkpnrzqnoo conflict second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["prev", "--conflict"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  yostqsxwqrlt conflict
    │ ×  royxmykxtrkr conflict fourth
    ├─╯
    ×  kkmpptxzrspx conflict third
    ×  rlvkpnrzqnoo conflict second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_prev_conflict_editing() {
    // Edit the third commit.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("content.txt");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    std::fs::write(&file_path, "second").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    // Create a conflict in the third commit, where we'll jump to.
    test_env
        .run_jj_in(&repo_path, ["edit", "description(first)"])
        .success();
    std::fs::write(&file_path, "first text").unwrap();
    test_env
        .run_jj_in(&repo_path, ["new", "description(third)"])
        .success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr conflict
    ×  kkmpptxzrspx conflict third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["prev", "--conflict", "--edit"])
        .success();
    // We now should be editing the third commit.
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kkmpptxzrspx conflict third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_conflict() {
    // There is a conflict in the third commit, so after next it should be the new
    // parent.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("content.txt");
    std::fs::write(&file_path, "first").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    // Create a conflict in the third commit.
    std::fs::write(&file_path, "third").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "description(first)"])
        .success();
    std::fs::write(&file_path, "first v2").unwrap();
    test_env
        .run_jj_in(&repo_path, ["squash", "--into", "description(third)"])
        .success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ×  kkmpptxzrspx conflict third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["next", "--conflict"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  vruxwmqvtpmx conflict
    ×  kkmpptxzrspx conflict third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_conflict_editing() {
    // There is a conflict in the third commit, so after next it should be our
    // working copy.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("content.txt");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    std::fs::write(&file_path, "second").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    // Create a conflict in the third commit.
    std::fs::write(&file_path, "third").unwrap();
    test_env
        .run_jj_in(&repo_path, ["edit", "description(second)"])
        .success();
    std::fs::write(&file_path, "modified second").unwrap();
    test_env.run_jj_in(&repo_path, ["edit", "@-"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ×  kkmpptxzrspx conflict
    ○  rlvkpnrzqnoo second
    @  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["next", "--conflict", "--edit"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kkmpptxzrspx conflict
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_conflict_head() {
    // When editing a head with conflicts, `jj next --conflict [--edit]` errors out.
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("file");
    std::fs::write(&file_path, "first").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(&file_path, "second").unwrap();
    test_env.run_jj_in(&repo_path, ["abandon", "@-"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  rlvkpnrzqnoo conflict
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--conflict"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy parent(s) have no other descendants with conflicts
    Hint: Working copy parent: zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--conflict", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy has no descendants with conflicts
    Hint: Working copy: rlvkpnrz 0273eeab (conflict) (no description set)
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_movement_edit_mode_true() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = true"#);

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["prev"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: rlvkpnrz 9ed53a4a (empty) second
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: qpvuntsm fa15625b (empty) first
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    @  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: rlvkpnrz 9ed53a4a (empty) second
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kkmpptxz 30056b0c (empty) third
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: uyznsvlq 7ad57fb8 (empty) (no description set)
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  uyznsvlquzzm
    │ ○  kkmpptxzrspx third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: xtnwkqum 7ac7a1c4 (empty) (no description set)
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  xtnwkqumpolk
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No descendant found 1 commit(s) forward from the working copy
    Hint: Working copy: xtnwkqum 7ac7a1c4 (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_movement_edit_mode_false() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = false"#);

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "first"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "second"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "third"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["prev"]).success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  royxmykxtrkr
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: vruxwmqv 087a65b1 (empty) (no description set)
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  vruxwmqvtpmx
    │ ○  kkmpptxzrspx third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No ancestor found 3 commit(s) back from the working copy parents(s)
    Hint: Working copy parent: qpvuntsm fa15625b (empty) first
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kpqxywon d06750fb (empty) (no description set)
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kpqxywonksrl
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: wqnwkozp 10fa181f (empty) (no description set)
    Parent commit      : kkmpptxz 30056b0c (empty) third
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["prev", "--edit", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: rlvkpnrz 9ed53a4a (empty) second
    Parent commit      : qpvuntsm fa15625b (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kkmpptxz 30056b0c (empty) third
    Parent commit      : rlvkpnrz 9ed53a4a (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_offset_when_wc_has_descendants() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = false"#);

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "base"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "right-wc"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "right-1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "right-2"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "description(base)"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "left-wc"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "left-1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "left-2"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["edit", "description(right-wc)"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  zsuskulnrvyr right-2
    ○  kkmpptxzrspx right-1
    @  rlvkpnrzqnoo right-wc
    │ ○  vruxwmqvtpmx left-2
    │ ○  yqosqzytrlsw left-1
    │ ○  royxmykxtrkr left-wc
    ├─╯
    ○  qpvuntsmwlqt base
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["next", "2"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  kmkuslswpqwq
    │ ○  vruxwmqvtpmx left-2
    ├─╯
    ○  yqosqzytrlsw left-1
    ○  royxmykxtrkr left-wc
    │ ○  zsuskulnrvyr right-2
    │ ○  kkmpptxzrspx right-1
    │ ○  rlvkpnrzqnoo right-wc
    ├─╯
    ○  qpvuntsmwlqt base
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env
        .run_jj_in(&repo_path, ["edit", "description(left-wc)"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    ○  vruxwmqvtpmx left-2
    ○  yqosqzytrlsw left-1
    @  royxmykxtrkr left-wc
    │ ○  zsuskulnrvyr right-2
    │ ○  kkmpptxzrspx right-1
    │ ○  rlvkpnrzqnoo right-wc
    ├─╯
    ○  qpvuntsmwlqt base
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["next"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  nkmrtpmomlro
    │ ○  zsuskulnrvyr right-2
    │ ○  kkmpptxzrspx right-1
    ├─╯
    ○  rlvkpnrzqnoo right-wc
    │ ○  vruxwmqvtpmx left-2
    │ ○  yqosqzytrlsw left-1
    │ ○  royxmykxtrkr left-wc
    ├─╯
    ○  qpvuntsmwlqt base
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), local_bookmarks, if(conflict, "conflict"), description)"#;
    test_env.run_jj_in(cwd, ["log", "-T", template])
}
