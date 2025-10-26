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

use crate::common::TestEnvironment;

#[test]
fn test_undo_root_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 000000000000 root()
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot undo root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_merge_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new", "--at-op=@-"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Error: Cannot undo a merge operation
    Hint: Consider using `jj op restore` instead
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_jump_old_undo_stack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    for state in 'A'..='D' {
        work_dir.write_file("state", state.to_string());
        work_dir.run_jj(["debug", "snapshot"]).success();
    }
    assert_eq!(work_dir.read_file("state"), "D");

    // undo operations D and C, restoring the state of B
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");

    // create operations E and F
    work_dir.write_file("state", "E");
    work_dir.run_jj(["debug", "snapshot"]).success();
    work_dir.write_file("state", "F");
    work_dir.run_jj(["debug", "snapshot"]).success();
    assert_eq!(work_dir.read_file("state"), "F");

    // undo operations F, E and B, restoring the state of A while skipping the
    // undo-stack of C and D in the op log
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "E");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "A");
}

#[test]
fn test_op_revert_is_ignored() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    work_dir.write_file("state", "A");
    work_dir.run_jj(["debug", "snapshot"]).success();
    work_dir.write_file("state", "B");
    work_dir.run_jj(["debug", "snapshot"]).success();
    assert_eq!(work_dir.read_file("state"), "B");

    // `op revert` works the same way as `undo` initially, but running `undo`
    // afterwards will result in a no-op. `undo` does not recognize operations
    // created by `op revert` as undo-operations on which an undo-stack can
    // be grown.
    work_dir.run_jj(["op", "revert"]).success();
    assert_eq!(work_dir.read_file("state"), "A");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
}

#[test]
fn test_undo_with_rev_arg_falls_back_to_revert() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Reverted operation: 8f47435a3990 (2001-02-03 08:05:07) add workspace 'default'
    Rebased 1 descendant commits
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log", "-n1"]);
    insta::assert_snapshot!(output, @r"
    @  20c0ef5cef23 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  revert operation 8f47435a3990362feaf967ca6de2eb0a31c8b883dfcb66fba5c22200d12bbe61e3dc8bc855f1f6879285fcafaf85ac792f9a43bcc36e57d28737d18347d5e752
    │  args: jj undo @-
    [EOF]
    ");
}

#[test]
fn test_can_only_redo_undo_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["redo"]), @r"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_jump_over_old_redo_stack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    for state in 'A'..='D' {
        work_dir.write_file("state", state.to_string());
        work_dir.run_jj(["debug", "snapshot"]).success();
    }
    assert_eq!(work_dir.read_file("state"), "D");

    insta::assert_snapshot!(work_dir.run_jj(["undo", "--quiet"]), @"");
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "A");

    // create two adjacent redo-stacks
    insta::assert_snapshot!(work_dir.run_jj(["redo", "--quiet"]), @"");
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");

    // jump over two adjacent redo-stacks
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "D");

    // nothing left to redo
    insta::assert_snapshot!(work_dir.run_jj(["redo"]), @r"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
}
