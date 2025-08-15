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
    // TODO: Adapt to future "undo" functionality: What happens if the user
    // progressively undoes everything all the way back to the root operation?

    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["undo", "000000000000"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Error: Cannot undo root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_merge_operation() {
    // TODO: What should future "undo" do with a merge operation? The
    // answer is probably not improbably not important, because users
    // who create merge operations will also be able to `op revert`
    // and `op restore` to achieve their goals on their own.
    // Possibilities:
    // - Fail and block any further attempt to undo anything.
    // - Restore directly to the fork point of the merge, ignoring any intermediate
    //   operations.
    // - Pick any path and walk only that backwards, ignoring the other paths.
    //   (Which path to pick?)
    // At first, it will be best to simply fail, before there is
    // agreement that doing anything else is not actively harmful.

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
    [EOF]
    [exit status: 1]
    ");
}
