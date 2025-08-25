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

use crate::common::TestEnvironment;

#[test]
fn test_can_only_redo_undo_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["redo"]), @r"
    ------- stderr -------
    Error: Nothing to redo.
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

    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "A");

    // create two adjacent redo-stacks
    work_dir.run_jj(["redo"]).success();
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
    Error: Nothing to redo.
    [EOF]
    [exit status: 1]
    ");
}
