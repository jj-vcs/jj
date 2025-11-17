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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_parallelize_no_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for n in 1..6 {
        work_dir.run_jj(["commit", &format!("-m{n}")]).success();
    }
    work_dir.run_jj(["describe", "-m=6"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e12cca0818cd 6 parents: 5
    ○  44f4686efbe9 5 parents: 4
    ○  6858f6e16a6c 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  22b8a32d1949 6 parents:
    │ ○  436e81ced43f 5 parents:
    ├─╯
    │ ○  823bf930aefb 4 parents:
    ├─╯
    │ ○  3b6586259aa9 3 parents:
    ├─╯
    │ ○  dfd927ce07c0 2 parents:
    ├─╯
    │ ○  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

// Only the head commit has descendants.
#[test]
fn test_parallelize_with_descendants_simple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for n in 1..6 {
        work_dir.run_jj(["commit", &format!("-m{n}")]).success();
    }
    work_dir.run_jj(["describe", "-m=6"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e12cca0818cd 6 parents: 5
    ○  44f4686efbe9 5 parents: 4
    ○  6858f6e16a6c 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::subject(glob:4)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  75ac07d7dedc 6 parents: 5
    ○        39791a4c42c5 5 parents: 1 2 3 4
    ├─┬─┬─╮
    │ │ │ ○  823bf930aefb 4 parents:
    │ │ ○ │  3b6586259aa9 3 parents:
    │ │ ├─╯
    │ ○ │  dfd927ce07c0 2 parents:
    │ ├─╯
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

// One of the commits being parallelized has a child that isn't being
// parallelized. That child will become a merge of any ancestors which are being
// parallelized.
#[test]
fn test_parallelize_where_interior_has_non_target_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for n in 1..6 {
        work_dir.run_jj(["commit", &format!("-m{n}")]).success();
    }
    work_dir
        .run_jj(["new", "subject(glob:2)", "-m=2c"])
        .success();
    work_dir
        .run_jj(["new", "subject(glob:5)", "-m=6"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  9554e07afe42 6 parents: 5
    ○  44f4686efbe9 5 parents: 4
    ○  6858f6e16a6c 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    │ ○  a5a460ad9943 2c parents: 2
    ├─╯
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::subject(glob:4)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  8bbff9ba415a 6 parents: 5
    ○        3bfb6f7542f6 5 parents: 1 2 3 4
    ├─┬─┬─╮
    │ │ │ ○  486dfbb53401 4 parents:
    │ │ ○ │  71c114f0dd4d 3 parents:
    │ │ ├─╯
    │ │ │ ○  154d3801414a 2c parents: 1 2
    ╭─┬───╯
    │ ○ │  7c8f6e529b52 2 parents:
    │ ├─╯
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_where_root_has_non_target_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for n in 1..4 {
        work_dir.run_jj(["commit", &format!("-m{n}")]).success();
    }
    work_dir
        .run_jj(["new", "subject(glob:1)", "-m=1c"])
        .success();
    work_dir
        .run_jj(["new", "subject(glob:3)", "-m=4"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  4c392c2965f0 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    │ ○  2935e6f82e54 1c parents: 1
    ├─╯
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");
    work_dir
        .run_jj(["parallelize", "subject(glob:1)::subject(glob:3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      a5d4e90a54bf 4 parents: 1 2 3
    ├─┬─╮
    │ │ ○  1d9fa9e05929 3 parents:
    │ ○ │  f773cf087413 2 parents:
    │ ├─╯
    │ │ ○  2935e6f82e54 1c parents: 1
    ├───╯
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

// One of the commits being parallelized has a child that is a merge commit.
#[test]
fn test_parallelize_with_merge_commit_child() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "1"]).success();
    for n in 2..4 {
        work_dir.run_jj(["commit", "-m", &n.to_string()]).success();
    }
    work_dir.run_jj(["new", "root()", "-m", "a"]).success();
    work_dir
        .run_jj(["new", "subject(glob:2)", "subject(glob:a)", "-m", "2a-c"])
        .success();
    work_dir
        .run_jj(["new", "subject(glob:3)", "-m", "4"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e6a543fcc5d8 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    │ ○  af7ad8059bf1 2a-c parents: 2 a
    ╭─┤
    │ ○  8fa549442479 a parents:
    ○ │  320daf48ba58 2 parents: 1
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    // After this finishes, child-2a will have three parents: "1", "2", and "a".
    work_dir
        .run_jj(["parallelize", "subject(glob:1)::subject(glob:3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      431d5005bab0 4 parents: 1 2 3
    ├─┬─╮
    │ │ ○  3b6586259aa9 3 parents:
    │ │ │ ○  67b28b5cc688 2a-c parents: 1 2 a
    ╭─┬───┤
    │ │ │ ○  8fa549442479 a parents:
    │ │ ├─╯
    │ ○ │  dfd927ce07c0 2 parents:
    │ ├─╯
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_disconnected_target_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for n in 1..3 {
        work_dir.run_jj(["commit", &format!("-m{n}")]).success();
    }
    work_dir.run_jj(["describe", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(glob:1)", "subject(glob:3)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_head_is_a_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=0"]).success();
    work_dir.run_jj(["commit", "-m=1"]).success();
    work_dir.run_jj(["commit", "-m=2"]).success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["commit", "-m=b"]).success();
    work_dir
        .run_jj([
            "new",
            "subject(glob:2)",
            "subject(glob:b)",
            "-m=merged-head",
        ])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    c634925e6ac2 merged-head parents: 2 b
    ├─╮
    │ ○  448c6310957c b parents: a
    │ ○  07fb6466f0cd a parents:
    ○ │  1ae5c538c8ef 2 parents: 1
    ○ │  42fc76489fb1 1 parents: 0
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    f97b547cdca0 merged-head parents: 0 b
    ├─╮
    │ ○  448c6310957c b parents: a
    │ ○  07fb6466f0cd a parents:
    │ │ ○  b240f5a52f77 2 parents: 0
    ├───╯
    │ │ ○  42fc76489fb1 1 parents: 0
    ├───╯
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_interior_target_is_a_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=0"]).success();
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir.run_jj(["new", "root()", "-m=a"]).success();
    work_dir
        .run_jj(["new", "subject(glob:1)", "subject(glob:a)", "-m=2"])
        .success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  d84d604297c3 3 parents: 2
    ○    5684656729e6 2 parents: 1 a
    ├─╮
    │ ○  55fc07cbd79b a parents:
    ○ │  42fc76489fb1 1 parents: 0
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    d0dae190124d 3 parents: 0 a
    ├─╮
    │ │ ○  d5756d591190 2 parents: 0 a
    ╭─┬─╯
    │ ○  55fc07cbd79b a parents:
    │ │ ○  42fc76489fb1 1 parents: 0
    ├───╯
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_root_is_a_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["describe", "-m=y"]).success();
    work_dir.run_jj(["new", "root()", "-m=x"]).success();
    work_dir
        .run_jj(["new", "subject(glob:y)", "subject(glob:x)", "-m=1"])
        .success();
    work_dir.run_jj(["new", "-m=2"]).success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  ce0ec2d2d844 3 parents: 2
    ○  44681d919431 2 parents: 1
    ○    8a06bcc06aad 1 parents: y x
    ├─╮
    │ ○  2d5d6dbc7e1f x parents:
    ○ │  1ecf47f2262c y parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:1)::subject(glob:2)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    2949bc60f108 3 parents: 1 2
    ├─╮
    │ ○    bf222a0e51d4 2 parents: y x
    │ ├─╮
    ○ │ │  8a06bcc06aad 1 parents: y x
    ╰─┬─╮
      │ ○  2d5d6dbc7e1f x parents:
      ○ │  1ecf47f2262c y parents:
      ├─╯
      ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_multiple_heads() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=0"]).success();
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir
        .run_jj(["new", "subject(glob:0)", "-m=2"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  96d58e6cf428 2 parents: 0
    │ ○  42fc76489fb1 1 parents: 0
    ├─╯
    ○  fc8a812f1b99 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:0)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  fefea56b23ab 2 parents:
    │ ○  c4b1ea1106d1 1 parents:
    ├─╯
    │ ○  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

// All heads must have the same children as the other heads, but only if they
// have children. In this test only one head has children, so the command
// succeeds.
#[test]
fn test_parallelize_multiple_heads_with_and_without_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=0"]).success();
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir
        .run_jj(["new", "subject(glob:0)", "-m=2"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  96d58e6cf428 2 parents: 0
    │ ○  42fc76489fb1 1 parents: 0
    ├─╯
    ○  fc8a812f1b99 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:0)", "subject(glob:1)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  96d58e6cf428 2 parents: 0
    ○  fc8a812f1b99 0 parents:
    │ ○  c4b1ea1106d1 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_multiple_roots() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir.run_jj(["new", "root()", "-m=a"]).success();
    work_dir
        .run_jj(["new", "subject(glob:1)", "subject(glob:a)", "-m=2"])
        .success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  9653da1c76e9 3 parents: 2
    ○    248a57e1c968 2 parents: 1 a
    ├─╮
    │ ○  3ce82963438f a parents:
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    // Succeeds because the roots have the same parents.
    work_dir.run_jj(["parallelize", "root().."]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  0c77dac691b5 3 parents:
    │ ○  1a23775d87d5 2 parents:
    ├─╯
    │ ○  3ce82963438f a parents:
    ├─╯
    │ ○  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_multiple_heads_with_different_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=1"]).success();
    work_dir.run_jj(["commit", "-m=2"]).success();
    work_dir.run_jj(["commit", "-m=3"]).success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["commit", "-m=b"]).success();
    work_dir.run_jj(["commit", "-m=c"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  afa59494cb01 parents: c
    ○  8897bad1837f c parents: b
    ○  448c6310957c b parents: a
    ○  07fb6466f0cd a parents:
    │ ○  8cfb27e238c8 3 parents: 2
    │ ○  320daf48ba58 2 parents: 1
    │ ○  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj([
            "parallelize",
            "subject(glob:1)::subject(glob:2)",
            "subject(glob:a)::subject(glob:b)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7d3e76dfbc7b parents: c
    ○    6dbfcf648fad c parents: a b
    ├─╮
    │ ○  8e5c55acd419 b parents:
    ○ │  07fb6466f0cd a parents:
    ├─╯
    │ ○    abdef66ee7e9 3 parents: 1 2
    │ ├─╮
    │ │ ○  7c8f6e529b52 2 parents:
    ├───╯
    │ ○  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_multiple_roots_with_different_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=1"]).success();
    work_dir.run_jj(["commit", "-m=2"]).success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["commit", "-m=b"]).success();
    work_dir
        .run_jj([
            "new",
            "subject(glob:2)",
            "subject(glob:b)",
            "-m=merged-head",
        ])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    4e5f16f52b5e merged-head parents: 2 b
    ├─╮
    │ ○  7686d0ce4f97 b parents: a
    │ ○  331119737aad a parents:
    ○ │  320daf48ba58 2 parents: 1
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:2)::", "subject(glob:b)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    180840c2f967 merged-head parents: 1 a
    ├─╮
    │ │ ○  7686d0ce4f97 b parents: a
    │ ├─╯
    │ ○  331119737aad a parents:
    │ │ ○  320daf48ba58 2 parents: 1
    ├───╯
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_complex_nonlinear_target() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-m=0", "root()"]).success();
    work_dir
        .run_jj(["new", "-m=1", "subject(glob:0)"])
        .success();
    work_dir
        .run_jj(["new", "-m=2", "subject(glob:0)"])
        .success();
    work_dir
        .run_jj(["new", "-m=3", "subject(glob:0)"])
        .success();
    work_dir.run_jj(["new", "-m=4", "heads(..)"]).success();
    work_dir
        .run_jj(["new", "-m=1c", "subject(glob:1)"])
        .success();
    work_dir
        .run_jj(["new", "-m=2c", "subject(glob:2)"])
        .success();
    work_dir
        .run_jj(["new", "-m=3c", "subject(glob:3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5cca06f145b2 3c parents: 3
    │ ○    095be6de6f23 4 parents: 3 2 1
    ╭─┼─╮
    ○ │ │  46cc2c450bba 3 parents: 0
    │ │ │ ○  24113692de1e 2c parents: 2
    │ ├───╯
    │ ○ │  5664a1d6ac8f 2 parents: 0
    ├─╯ │
    │ ○ │  6d578e6cbc1a 1c parents: 1
    │ ├─╯
    │ ○  883b398bc1fd 1 parents: 0
    ├─╯
    ○  973f85cf2550 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(glob:0)::subject(glob:4)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: yostqsxw d6bb6520 (empty) 3c
    Parent commit (@-)      : rlvkpnrz 973f85cf (empty) 0
    Parent commit (@-)      : mzvwutvl 47ec86fe (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    d6bb652004e4 3c parents: 0 3
    ├─╮
    │ ○  47ec86fe7334 3 parents:
    │ │ ○  79e22ba7b736 2c parents: 0 2
    ╭───┤
    │ │ ○  9d6818f73e0d 2 parents:
    │ ├─╯
    │ │ ○  bbeb29b59bee 1c parents: 0 1
    ╭───┤
    │ │ ○  ea96e6d5bb04 1 parents:
    │ ├─╯
    ○ │  973f85cf2550 0 parents:
    ├─╯
    │ ○  0f9aae95edbe 4 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_immutable_base_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=x"]).success();
    work_dir.run_jj(["new", "-m=x1"]).success();
    work_dir.run_jj(["new", "-m=x2"]).success();
    work_dir.run_jj(["new", "-m=x3"]).success();

    work_dir.run_jj(["new", "root()", "-m=y"]).success();
    work_dir.run_jj(["new", "-m=y1"]).success();
    work_dir.run_jj(["new", "-m=y2"]).success();

    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "revset-aliases.'immutable_heads()'",
            "subject(glob:x) | subject(glob:y)",
        ])
        .success();
    work_dir
        .run_jj(["config", "set", "--repo", "revsets.log", "all()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  1a0f8336974a y2 parents: y1
    ○  0e07ea90229f y1 parents: y
    ◆  a0fb97fc193f y parents:
    │ ○  d6c30fecfe88 x3 parents: x2
    │ ○  6411b5818334 x2 parents: x1
    │ ○  6d01ab1fb731 x1 parents: x
    │ ◆  8ceb28e1dc31 x parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(glob:x*)", "subject(glob:y*)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  264da1b87cf3 y2 parents:
    │ ○  636008757721 y1 parents:
    ├─╯
    │ ○  db5717c9c093 x3 parents:
    ├─╯
    │ ○  3e5fc34764e8 x2 parents:
    ├─╯
    │ ○  71aeaa5e8891 x1 parents:
    ├─╯
    │ ◆  a0fb97fc193f y parents:
    ├─╯
    │ ◆  8ceb28e1dc31 x parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");
}

#[test]
fn test_parallelize_no_immutable_non_base_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=x"]).success();
    work_dir.run_jj(["new", "-m=x1"]).success();
    work_dir.run_jj(["new", "-m=x2"]).success();
    work_dir.run_jj(["new", "-m=x3"]).success();

    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "revset-aliases.'immutable_heads()'",
            "subject(glob:x1)",
        ])
        .success();
    work_dir
        .run_jj(["config", "set", "--repo", "revsets.log", "all()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  d6c30fecfe88 x3 parents: x2
    ○  6411b5818334 x2 parents: x1
    ◆  6d01ab1fb731 x1 parents: x
    ◆  8ceb28e1dc31 x parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(glob:x*)"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit 6d01ab1fb731 is immutable
    Hint: Could not modify commit: kkmpptxz 6d01ab1f (empty) x1
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
        commit_id.short(),
        description.first_line(),
        "parents:",
        parents.map(|c|c.description().first_line())
    )"#;
    work_dir.run_jj(["log", "-T", template])
}
