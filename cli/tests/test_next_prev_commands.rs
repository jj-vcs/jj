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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::force_interactive;

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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Create a simple linear history, which we'll traverse.
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Move to `first`
    work_dir.run_jj(["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  royxmykxtrkr
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 01f06d39 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 7576de42 (empty) third
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    work_dir.run_jj(["new", "@---"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["next", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv a53dd783 (empty) (no description set)
    Parent commit (@-)      : zsuskuln c5025ce1 (empty) fourth
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: royxmykx 539f176b (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: yqosqzyt a4d3accb (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["new", "-r", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  mzvwutvlkqwt
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // `jj next` beyond existing history fails.
    let output = work_dir.run_jj(["next", "3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No other descendant found 3 commit(s) forward from the working copy parent(s)
    Hint: Working copy parent: rlvkpnrz 9439bf06 (empty) second
    [EOF]
    [exit status: 1]
    ");
}

// The working copy commit is a child of a "fork" with two children on each
// bookmark.
#[test]
fn test_next_parent_has_multiple_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Setup.
    work_dir.run_jj(["desc", "-m", "1"]).success();
    work_dir.run_jj(["new", "-m", "2"]).success();
    work_dir.run_jj(["new", "root()", "-m", "3"]).success();
    work_dir.run_jj(["new", "-m", "4"]).success();
    work_dir.run_jj(["edit", "subject(3)"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  mzvwutvlkqwt 4
    @  zsuskulnrvyr 3
    │ ○  kkmpptxzrspx 2
    │ ○  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl e5543950 (empty) 4
    Parent commit (@-)      : zsuskuln 83df6e43 (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Setup.
    work_dir.run_jj(["desc", "-m", "1"]).success();
    work_dir.run_jj(["new", "root()", "-m", "2"]).success();
    work_dir
        .run_jj(["new", "subject(1)", "subject(2)", "-m", "3"])
        .success();
    work_dir.run_jj(["new", "-m", "4"]).success();
    work_dir.run_jj(["prev", "0"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 7a09c355 (empty) (no description set)
    Parent commit (@-)      : mzvwutvl f02c921e (empty) 4
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Setup.
    work_dir.run_jj(["desc", "-m", "1"]).success();
    work_dir.run_jj(["new", "root()", "-m", "2"]).success();
    work_dir
        .run_jj(["new", "subject(1)", "subject(2)", "-m", "3"])
        .success();
    work_dir.run_jj(["new", "-m", "4"]).success();
    work_dir.run_jj(["edit", "subject(3)"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  mzvwutvlkqwt 4
    @    zsuskulnrvyr 3
    ├─╮
    │ ○  kkmpptxzrspx 2
    ○ │  qpvuntsmwlqt 1
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl f02c921e (empty) 4
    Parent commit (@-)      : zsuskuln d2500577 (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous next commit, choose one to target:
    1: zsuskuln 6fc6af46 (empty) third
    2: rlvkpnrz 9439bf06 (empty) second
    q: quit the prompt
    Error: Cannot prompt for input since the output is not connected to a terminal
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_next_fails_on_bookmarking_children_quit_prompt() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["new", "@--"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj_with(|cmd| force_interactive(cmd).arg("next").write_stdin("q\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous next commit, choose one to target:
    1: zsuskuln 6fc6af46 (empty) third
    2: rlvkpnrz 9439bf06 (empty) second
    q: quit the prompt
    enter the index of the commit you want to target: Error: ambiguous target commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_next_choose_bookmarking_child() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    // Advance the working copy commit.
    let output = work_dir.run_jj_with(|cmd| force_interactive(cmd).arg("next").write_stdin("2\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous next commit, choose one to target:
    1: royxmykx 3522c887 (empty) fourth
    2: zsuskuln 6fc6af46 (empty) third
    3: rlvkpnrz 9439bf06 (empty) second
    q: quit the prompt
    enter the index of the commit you want to target: Working copy  (@) now at: yostqsxw 683938a4 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 6fc6af46 (empty) third
    [EOF]
    ");
}

#[test]
fn test_prev_on_merge_commit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["desc", "-m", "first"]).success();
    work_dir.run_jj(["bookmark", "c", "-r@", "left"]).success();
    work_dir.run_jj(["new", "root()", "-m", "second"]).success();
    work_dir.run_jj(["bookmark", "c", "-r@", "right"]).success();
    work_dir.run_jj(["new", "left", "right"]).success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    royxmykxtrkr
    ├─╮
    │ ○  zsuskulnrvyr right second
    ○ │  qpvuntsmwlqt left first
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv b64f323d (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj_with(|cmd| {
        force_interactive(cmd)
            .args(["prev", "--edit"])
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous prev commit, choose one to target:
    1: zsuskuln 22a08bc0 right | (empty) second
    2: qpvuntsm 68a50538 left | (empty) first
    q: quit the prompt
    enter the index of the commit you want to target: Working copy  (@) now at: qpvuntsm 68a50538 left | (empty) first
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_prev_on_merge_commit_with_parent_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["desc", "-m", "x"]).success();
    work_dir.run_jj(["new", "root()", "-m", "y"]).success();
    work_dir
        .run_jj(["new", "subject(x)", "subject(y)", "-m", "z"])
        .success();
    work_dir.run_jj(["new", "root()", "-m", "1"]).success();
    work_dir
        .run_jj(["new", "subject(z)", "subject(1)", "-m", "M"])
        .success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj_with(|cmd| force_interactive(cmd).arg("prev").write_stdin("2\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous prev commit, choose one to target:
    1: kkmpptxz ab947132 (empty) y
    2: qpvuntsm 007e88d2 (empty) x
    3: zzzzzzzz 00000000 (empty) (no description set)
    q: quit the prompt
    enter the index of the commit you want to target: Working copy  (@) now at: vruxwmqv ff8922ba (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 007e88d2 (empty) x
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj_with(|cmd| {
        force_interactive(cmd)
            .args(["prev", "--edit"])
            .write_stdin("2\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous prev commit, choose one to target:
    1: mzvwutvl 0e8fc48c (empty) 1
    2: zsuskuln e7382ca4 (empty) z
    q: quit the prompt
    enter the index of the commit you want to target: Working copy  (@) now at: zsuskuln e7382ca4 (empty) z
    Parent commit (@-)      : qpvuntsm 007e88d2 (empty) x
    Parent commit (@-)      : kkmpptxz ab947132 (empty) y
    [EOF]
    ");
}

#[test]
fn test_prev_prompts_on_multiple_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["new", "@--"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    // Create a merge commit, which has two parents.
    work_dir.run_jj(["new", "@--+"]).success();
    work_dir.run_jj(["commit", "-m", "merge"]).success();
    work_dir.run_jj(["commit", "-m", "merge+1"]).success();

    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj_with(|cmd| {
        force_interactive(cmd)
            .args(["prev", "2"])
            .write_stdin("3\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    ambiguous prev commit, choose one to target:
    1: mzvwutvl 5ec63817 (empty) third
    2: kkmpptxz e8959fbd (empty) second
    3: qpvuntsm 68a50538 (empty) first
    q: quit the prompt
    enter the index of the commit you want to target: Working copy  (@) now at: kpqxywon 5448803a (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    work_dir.run_jj(["next"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  wqnwkozpkust
    │ ○  vruxwmqvtpmx merge+1
    ├─╯
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
}

#[test]
fn test_prev_beyond_root_fails() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    // @- is at "fourth", and there is no parent 5 commits behind it.
    let output = work_dir.run_jj(["prev", "5"]);
    insta::assert_snapshot!(output,@r"
    ------- stderr -------
    Error: No ancestor found 5 commit(s) back from the working copy parents(s)
    Hint: Working copy parent: zsuskuln c5025ce1 (empty) fourth
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_prev_editing() {
    // Edit the third commit.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    // Edit the "fourth" commit, which becomes the leaf.
    work_dir.run_jj(["edit", "@-"]).success();
    // Check that the graph looks the way we expect.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 7576de42 (empty) third
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    work_dir.run_jj(["edit", "@---"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  zsuskulnrvyr fourth
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 7576de42 (empty) third
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("content.txt", "first");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.write_file("content.txt", "second");
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    // Create a conflict in the first commit, where we'll jump to.
    work_dir.run_jj(["edit", "subject(first)"]).success();
    work_dir.write_file("content.txt", "first+1");
    work_dir.run_jj(["new", "subject(third)"]).success();
    work_dir.run_jj(["commit", "-m", "fourth"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  yqosqzytrlsw conflict
    ×  royxmykxtrkr conflict fourth
    ×  kkmpptxzrspx conflict third
    ×  rlvkpnrzqnoo conflict second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    work_dir.run_jj(["prev", "--conflict"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.write_file("content.txt", "second");
    work_dir.run_jj(["commit", "-m", "third"]).success();
    // Create a conflict in the third commit, where we'll jump to.
    work_dir.run_jj(["edit", "subject(first)"]).success();
    work_dir.write_file("content.txt", "first text");
    work_dir.run_jj(["new", "subject(third)"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  royxmykxtrkr conflict
    ×  kkmpptxzrspx conflict third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    work_dir.run_jj(["prev", "--conflict", "--edit"]).success();
    // We now should be editing the third commit.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("content.txt", "first");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    // Create a conflict in the third commit.
    work_dir.write_file("content.txt", "third");
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir.run_jj(["new", "subject(first)"]).success();
    work_dir.write_file("content.txt", "first v2");
    work_dir
        .run_jj(["squash", "--into", "subject(third)"])
        .success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  royxmykxtrkr
    │ ×  kkmpptxzrspx conflict third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    work_dir.run_jj(["next", "--conflict"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.write_file("content.txt", "second");
    work_dir.run_jj(["commit", "-m", "second"]).success();
    // Create a conflict in the third commit.
    work_dir.write_file("content.txt", "third");
    work_dir.run_jj(["edit", "subject(second)"]).success();
    work_dir.write_file("content.txt", "modified second");
    work_dir.run_jj(["edit", "@-"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ×  kkmpptxzrspx conflict
    ○  rlvkpnrzqnoo second
    @  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    work_dir.run_jj(["next", "--conflict", "--edit"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "first");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "second");
    work_dir.run_jj(["abandon", "@-"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  rlvkpnrzqnoo conflict
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--conflict"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy parent(s) have no other descendants with conflicts
    Hint: Working copy parent: zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["next", "--conflict", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy has no descendants with conflicts
    Hint: Working copy: rlvkpnrz 09d24b1f (conflict) (no description set)
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_movement_edit_mode_true() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = true"#);

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    work_dir.run_jj(["prev"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 9439bf06 (empty) second
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 68a50538 (empty) first
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    @  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 9439bf06 (empty) second
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 7576de42 (empty) third
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: uyznsvlq 1062d305 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  uyznsvlquzzm
    │ ○  kkmpptxzrspx third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: xtnwkqum ccbc5bff (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  xtnwkqumpolk
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No descendant found 1 commit(s) forward from the working copy
    Hint: Working copy: xtnwkqum ccbc5bff (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_movement_edit_mode_false() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = false"#);

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr
    ○  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    work_dir.run_jj(["prev"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  royxmykxtrkr
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 1a9aa547 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  vruxwmqvtpmx
    │ ○  kkmpptxzrspx third
    │ ○  rlvkpnrzqnoo second
    ├─╯
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No ancestor found 3 commit(s) back from the working copy parents(s)
    Hint: Working copy parent: qpvuntsm 68a50538 (empty) first
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["next"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kpqxywon 97dd6a5a (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kpqxywonksrl
    │ ○  kkmpptxzrspx third
    ├─╯
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: wqnwkozp 525e0f84 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 7576de42 (empty) third
    [EOF]
    ");

    let output = work_dir.run_jj(["prev", "--edit", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 9439bf06 (empty) second
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx third
    @  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "--edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 7576de42 (empty) third
    Parent commit (@-)      : rlvkpnrz 9439bf06 (empty) second
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kkmpptxzrspx third
    ○  rlvkpnrzqnoo second
    ○  qpvuntsmwlqt first
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_next_when_wc_has_descendants() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.movement.edit = false"#);

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "base"]).success();
    work_dir.run_jj(["commit", "-m", "right-wc"]).success();
    work_dir.run_jj(["commit", "-m", "right-1"]).success();
    work_dir.run_jj(["commit", "-m", "right-2"]).success();
    work_dir.run_jj(["edit", "subject(right-wc)"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  zsuskulnrvyr right-2
    ○  kkmpptxzrspx right-1
    @  rlvkpnrzqnoo right-wc
    ○  qpvuntsmwlqt base
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["next", "2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy must not have any children
    Hint: Create a new commit on top of this one or use `--edit`
    [EOF]
    [exit status: 1]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), local_bookmarks, if(conflict, "conflict"), description)"#;
    work_dir.run_jj(["log", "-T", template])
}
