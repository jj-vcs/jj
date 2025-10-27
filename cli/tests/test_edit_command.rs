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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_edit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "0");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["describe", "-m", "second"]).success();
    work_dir.write_file("file1", "1");

    // Errors out without argument
    let output = work_dir.run_jj(["edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <REVSET>

    Usage: jj edit <REVSET>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Makes the specified commit the working-copy commit
    let output = work_dir.run_jj(["edit", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 1f6994f8 first
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = get_log_output(&work_dir);
    insta::assert_snapshot!(output, @r"
    ○  b38b8e65163a second
    @  1f6994f8b95b first
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file1"), @"0");

    // Changes in the working copy are amended into the commit
    work_dir.write_file("file2", "0");
    let output = get_log_output(&work_dir);
    insta::assert_snapshot!(output, @r"
    ○  d5aea29cb4cb second
    @  2636584c21c0 first
    ◆  000000000000
    [EOF]
    ------- stderr -------
    Rebased 1 descendant commits onto updated working copy
    [EOF]
    ");
}

#[test]
fn test_edit_current() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Editing the current revision is a no-op
    let output = work_dir.run_jj(["edit", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Already editing that commit
    [EOF]
    ");

    // No operation created
    let output = work_dir.run_jj(["op", "log", "--limit=1"]);
    insta::assert_snapshot!(output, @r"
    @  8f47435a3990 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    [EOF]
    ");
}

#[test]
// Windows says "Access is denied" when trying to delete the object file.
#[cfg(unix)]
fn test_edit_current_wc_commit_missing() {
    use std::path::PathBuf;

    // Test that we get a reasonable error message when the current working-copy
    // commit is missing

    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["describe", "-m", "second"]).success();
    work_dir.run_jj(["edit", "@-"]).success();

    let wc_id = work_dir
        .run_jj(["log", "--no-graph", "-T=commit_id", "-r=@"])
        .success()
        .stdout
        .into_raw();
    let wc_child_id = work_dir
        .run_jj(["log", "--no-graph", "-T=commit_id", "-r=@+"])
        .success()
        .stdout
        .into_raw();
    // Make the Git backend fail to read the current working copy commit
    let commit_object_path = PathBuf::from_iter([
        ".jj",
        "repo",
        "store",
        "git",
        "objects",
        &wc_id[..2],
        &wc_id[2..],
    ]);
    work_dir.remove_file(commit_object_path);

    // Pass --ignore-working-copy to avoid triggering the error at snapshot time
    let output = work_dir.run_jj(["edit", "--ignore-working-copy", &wc_child_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: Failed to edit a commit
    Caused by:
    1: Current working-copy commit not found
    2: Object 68a505386f936fff6d718f55005e77ea72589bc1 of type commit not found
    3: An object with id 68a505386f936fff6d718f55005e77ea72589bc1 could not be found
    [EOF]
    [exit status: 255]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
