// Copyright 2026 The Jujutsu Authors
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
fn test_op_restore_warns_when_workspace_missing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "restore", "000000000000"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Restored to operation: 000000000000 root()
    Warning: The current workspace 'default' no longer exists after this operation. The working copy was left untouched.
    Hint: Restore to an operation that contains the workspace (e.g. `jj undo` or `jj redo`).
    [EOF]
    ");
}

#[test]
fn test_op_restore_to_valid_op_no_warning() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Restore to the current operation (@). This should not emit the
    // missing-workspace warning.
    let output = work_dir.run_jj(["op", "restore", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Restored to operation: e39dc288903d (2001-02-03 08:05:07) add workspace 'default'
    Nothing changed.
    [EOF]
    ");
}
