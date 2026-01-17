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
fn test_undo_push_operation() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "origin"])
        .success();
    test_env
        .run_jj_in(".", ["git", "clone", "origin", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("foo", "foo");
    work_dir.run_jj(["commit", "-mfoo"]).success();
    work_dir.run_jj(["git", "push", "-c@-"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Undoing a push operation often leads to conflicted bookmarks.
    Hint: To avoid this, run `jj redo` now.
    Restored to operation: 9e2ad53b06f1 (2001-02-03 08:05:09) commit 3850397cf31988d0657948307ad5bbe873d76a38
    [EOF]
    ");
}

#[test]
fn test_jump_over_old_undo_stack() {
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
fn test_undo_ignores_op_revert() {
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

    work_dir.run_jj(["new", "-m", "will be reverted"]).success();
    work_dir.run_jj(["new", "-m", "will remain"]).success();
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Reverted operation: bd38e657b210 (2001-02-03 08:05:08) new empty commit
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 48f21213 (empty) will remain
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log", "-n1"]);
    insta::assert_snapshot!(output, @"
    @  f46a6c31f65b test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  revert operation bd38e657b210a352a798e73366bfee01b120e6ecd023431be8ebc427c054b20a6fe56d6c33ee60152a35ef9fa45279a3301bb5fd16e9e4dfba95fa5b6ab27047
    │  args: jj undo @-
    [EOF]
    ");
}

#[test]
fn test_redo_non_undo_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m", "a"]).success();
    let output = work_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @r"
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
