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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e12cca0818cd 6 parents: 5
    ○  44f4686efbe9 5 parents: 4
    ○  6858f6e16a6c 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir.run_jj(["parallelize", "subject(1)::"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
        .run_jj(["parallelize", "subject(1)::subject(4)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
    work_dir.run_jj(["new", "subject(2)", "-m=2c"]).success();
    work_dir.run_jj(["new", "subject(5)", "-m=6"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  9554e07afe42 6 parents: 5
    ○  44f4686efbe9 5 parents: 4
    ○  6858f6e16a6c 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    │ ○  bb6f24b28785 2c parents: 2
    ├─╯
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(1)::subject(4)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  8bbff9ba415a 6 parents: 5
    ○        3bfb6f7542f6 5 parents: 1 2 3 4
    ├─┬─┬─╮
    │ │ │ ○  486dfbb53401 4 parents:
    │ │ ○ │  71c114f0dd4d 3 parents:
    │ │ ├─╯
    │ │ │ ○  f07fee340c0b 2c parents: 1 2
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
    work_dir.run_jj(["new", "subject(1)", "-m=1c"]).success();
    work_dir.run_jj(["new", "subject(3)", "-m=4"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  72aceb7fc062 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    │ ○  d0f2944abd65 1c parents: 1
    ├─╯
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");
    work_dir
        .run_jj(["parallelize", "subject(1)::subject(3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      bbe84ea27239 4 parents: 1 2 3
    ├─┬─╮
    │ │ ○  1d9fa9e05929 3 parents:
    │ ○ │  f773cf087413 2 parents:
    │ ├─╯
    │ │ ○  d0f2944abd65 1c parents: 1
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
        .run_jj(["new", "subject(2)", "subject(a)", "-m", "2a-c"])
        .success();
    work_dir.run_jj(["new", "subject(3)", "-m", "4"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e973fd6242d3 4 parents: 3
    ○  8cfb27e238c8 3 parents: 2
    │ ○  86ecf5b66397 2a-c parents: 2 a
    ╭─┤
    │ ○  f6b52d21d3b1 a parents:
    ○ │  320daf48ba58 2 parents: 1
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    // After this finishes, child-2a will have three parents: "1", "2", and "a".
    work_dir
        .run_jj(["parallelize", "subject(1)::subject(3)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      4d2b366781e1 4 parents: 1 2 3
    ├─┬─╮
    │ │ ○  3b6586259aa9 3 parents:
    │ │ │ ○  d653025aafc9 2a-c parents: 1 2 a
    ╭─┬───┤
    │ │ │ ○  f6b52d21d3b1 a parents:
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  8cfb27e238c8 3 parents: 2
    ○  320daf48ba58 2 parents: 1
    ○  884fe9b9c656 1 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(1)", "-r=subject(3)"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
        .run_jj(["new", "subject(2)", "subject(b)", "-m=merged-head"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    28a003dab4ae merged-head parents: 2 b
    ├─╮
    │ ○  61c599910b31 b parents: a
    │ ○  5d473ef2b320 a parents:
    ○ │  1ae5c538c8ef 2 parents: 1
    ○ │  42fc76489fb1 1 parents: 0
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir.run_jj(["parallelize", "subject(1)::"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    c00a7b308d15 merged-head parents: 0 b
    ├─╮
    │ ○  61c599910b31 b parents: a
    │ ○  5d473ef2b320 a parents:
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
        .run_jj(["new", "subject(1)", "subject(a)", "-m=2"])
        .success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37471d748332 3 parents: 2
    ○    9b3e0159d764 2 parents: 1 a
    ├─╮
    │ ○  fc6a3235e302 a parents:
    ○ │  42fc76489fb1 1 parents: 0
    ○ │  fc8a812f1b99 0 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir.run_jj(["parallelize", "subject(1)::"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    378ad6012f88 3 parents: 0 a
    ├─╮
    │ │ ○  ba2f5b931d15 2 parents: 0 a
    ╭─┬─╯
    │ ○  fc6a3235e302 a parents:
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
        .run_jj(["new", "subject(y)", "subject(x)", "-m=1"])
        .success();
    work_dir.run_jj(["new", "-m=2"]).success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  754cfe1ec718 3 parents: 2
    ○  63f067d42867 2 parents: 1
    ○    6086c98e22ad 1 parents: y x
    ├─╮
    │ ○  2d7c42f7b30e x parents:
    ○ │  1ecf47f2262c y parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(1)::subject(2)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d8da4317fbda 3 parents: 1 2
    ├─╮
    │ ○    2a7c5752a3bc 2 parents: y x
    │ ├─╮
    ○ │ │  6086c98e22ad 1 parents: y x
    ╰─┬─╮
      │ ○  2d7c42f7b30e x parents:
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
    work_dir.run_jj(["new", "subject(0)", "-m=2"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  771aef302228 2 parents: 0
    │ ○  42fc76489fb1 1 parents: 0
    ├─╯
    ○  fc8a812f1b99 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir.run_jj(["parallelize", "subject(0)::"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  09a639be16a8 2 parents:
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
    work_dir.run_jj(["new", "subject(0)", "-m=2"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  771aef302228 2 parents: 0
    │ ○  42fc76489fb1 1 parents: 0
    ├─╯
    ○  fc8a812f1b99 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "-r=subject(0)", "subject(1)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  771aef302228 2 parents: 0
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
        .run_jj(["new", "subject(1)", "subject(a)", "-m=2"])
        .success();
    work_dir.run_jj(["new", "-m=3"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  7995e40dff61 3 parents: 2
    ○    caf56efc50c4 2 parents: 1 a
    ├─╮
    │ ○  f407ec73f3df a parents:
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    // Succeeds because the roots have the same parents.
    work_dir.run_jj(["parallelize", "root().."]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f808db46609b 3 parents:
    │ ○  7e4bb64d08e1 2 parents:
    ├─╯
    │ ○  f407ec73f3df a parents:
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a08ec053a742 parents: c
    ○  f8da4b1f50a4 c parents: b
    ○  61c599910b31 b parents: a
    ○  5d473ef2b320 a parents:
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
            "subject(1)::subject(2)",
            "subject(a)::subject(b)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  06450adb3fa2 parents: c
    ○    cfca27dc8b42 c parents: a b
    ├─╮
    │ ○  8e5c55acd419 b parents:
    ○ │  5d473ef2b320 a parents:
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
        .run_jj(["new", "subject(2)", "subject(b)", "-m=merged-head"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    efcc8bb8ed49 merged-head parents: 2 b
    ├─╮
    │ ○  f981f9db15b1 b parents: a
    │ ○  613642a76679 a parents:
    ○ │  320daf48ba58 2 parents: 1
    ○ │  884fe9b9c656 1 parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(2)::", "subject(b)::"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    8bd22f2a0636 merged-head parents: 1 a
    ├─╮
    │ │ ○  f981f9db15b1 b parents: a
    │ ├─╯
    │ ○  613642a76679 a parents:
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
    work_dir.run_jj(["new", "-m=1", "subject(0)"]).success();
    work_dir.run_jj(["new", "-m=2", "subject(0)"]).success();
    work_dir.run_jj(["new", "-m=3", "subject(0)"]).success();
    work_dir.run_jj(["new", "-m=4", "heads(..)"]).success();
    work_dir.run_jj(["new", "-m=1c", "subject(1)"]).success();
    work_dir.run_jj(["new", "-m=2c", "subject(2)"]).success();
    work_dir.run_jj(["new", "-m=3c", "subject(3)"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  6f9e3f44440a 3c parents: 3
    │ ○    98c8cd92bef7 4 parents: 3 2 1
    ╭─┼─╮
    ○ │ │  25d0d46fa5dd 3 parents: 0
    │ │ │ ○  6fa118e0f9f8 2c parents: 2
    │ ├───╯
    │ ○ │  6f60f945406b 2 parents: 0
    ├─╯ │
    │ ○ │  e08f5bf4147a 1c parents: 1
    │ ├─╯
    │ ○  c2ba666f42b9 1 parents: 0
    ├─╯
    ○  ceba7ded0a6f 0 parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(0)::subject(4)"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: sostqsxw 5086906a (empty) 3c
    Parent commit (@-)      : ylvkpnrz ceba7ded (empty) 0
    Parent commit (@-)      : qzvwutvl 0054ae45 (empty) 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    5086906ab42f 3c parents: 0 3
    ├─╮
    │ ○  0054ae459415 3 parents:
    │ │ ○  7424f77d51de 2c parents: 0 2
    ╭───┤
    │ │ ○  ea19463dd1e8 2 parents:
    │ ├─╯
    │ │ ○  4bd394a36459 1c parents: 0 1
    ╭───┤
    │ │ ○  abcccef95e4b 1 parents:
    │ ├─╯
    ○ │  ceba7ded0a6f 0 parents:
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
            "subject(x) | subject(y)",
        ])
        .success();
    work_dir
        .run_jj(["config", "set", "--repo", "revsets.log", "all()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1fa61b6a7f42 y2 parents: y1
    ○  e2d8b15b2710 y1 parents: y
    ◆  a0fb97fc193f y parents:
    │ ○  fe953a05a00e x3 parents: x2
    │ ○  544c8880e4e7 x2 parents: x1
    │ ○  33676f246bb9 x1 parents: x
    │ ◆  b568176074ba x parents:
    ├─╯
    ◆  000000000000 parents:
    [EOF]
    ");

    work_dir
        .run_jj(["parallelize", "subject(x*)", "subject(y*)"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  83b8d2615c8a y2 parents:
    │ ○  c9dd4ef8bf8c y1 parents:
    ├─╯
    │ ○  177442cdd04a x3 parents:
    ├─╯
    │ ○  6514eaa1ab82 x2 parents:
    ├─╯
    │ ○  ab71546914bd x1 parents:
    ├─╯
    │ ◆  a0fb97fc193f y parents:
    ├─╯
    │ ◆  b568176074ba x parents:
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
            "subject(x1)",
        ])
        .success();
    work_dir
        .run_jj(["config", "set", "--repo", "revsets.log", "all()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  fe953a05a00e x3 parents: x2
    ○  544c8880e4e7 x2 parents: x1
    ◆  33676f246bb9 x1 parents: x
    ◆  b568176074ba x parents:
    ◆  000000000000 parents:
    [EOF]
    ");

    let output = work_dir.run_jj(["parallelize", "subject(x*)"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit 33676f246bb9 is immutable
    Hint: Could not modify commit: nkmpptxz 33676f24 (empty) x1
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://docs.jj-vcs.dev/latest/config/#set-of-immutable-commits
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
