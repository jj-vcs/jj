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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit_with_files;

#[must_use]
fn run_file_set(work_dir: &TestWorkDir, args: &[&str], stdin: &str) -> CommandOutput {
    work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "--stdin"])
            .args(args)
            .write_stdin(stdin)
    })
}

#[test]
fn test_file_set() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[
            ("file", "base\n"),
            ("dir/file", "dir\n"),
            ("script.sh", "#!/bin/sh\n"),
        ],
    );
    work_dir
        .run_jj(["file", "chmod", "x", "-r=base", "script.sh"])
        .success();
    // The child doesn't touch `file`, so it inherits changes made to `base`.
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);
    let setup_opid = work_dir.current_operation_id();

    // The file is updated in the revision, and descendants are rebased.
    let output = run_file_set(&work_dir, &["-r=base", "file"], "modified\n");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl d2c70bd0 child | child
    Parent commit (@-)      : rlvkpnrz 6b6a8e3b base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @r"
    modified
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @r"
    modified
    [EOF]
    ");

    // With --restore-descendants, descendants keep their content.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = run_file_set(
        &work_dir,
        &["-r=base", "--restore-descendants", "file"],
        "modified\n",
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content).
    Working copy  (@) now at: mzvwutvl 960b31ff child | child
    Parent commit (@-)      : rlvkpnrz 0f101435 base | base
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @r"
    modified
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @r"
    base
    [EOF]
    ");

    // The executable bit is preserved
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    run_file_set(
        &work_dir,
        &["-r=base", "script.sh"],
        "#!/bin/sh\necho hello\n",
    )
    .success();
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=base", "script.sh"]), @r#"
    script.sh: Ok(Resolved(Some(File { id: FileId("21ba682558a42264518f1e0ba55e8a5cd9d7db0a"), executable: true, copy_id: CopyId("") })))
    [EOF]
    "#);

    // A path that doesn't exist in the revision is created
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = run_file_set(&work_dir, &["-r=base", "new_file"], "brand new\n");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl 504ed615 child | child
    Parent commit (@-)      : rlvkpnrz b6a559fa base | base
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "new_file"]), @r"
    brand new
    [EOF]
    ");

    // Nothing happens if the content is unchanged
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    let output = run_file_set(&work_dir, &["-r=base", "file"], "base\n");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);

    // Directories can't be set
    let output = run_file_set(&work_dir, &["-r=base", "dir"], "content\n");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Path is a directory: dir
    [EOF]
    [exit status: 1]
    ");

    // --stdin is required
    let output = work_dir.run_jj(["file", "set", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      --stdin

    Usage: jj file set --stdin --revision <REVSET> <FILE>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_file_set_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);
    work_dir.run_jj(["new", "conflict"]).success();

    // Setting a conflicted file replaces the conflict with the new content
    let output = run_file_set(&work_dir, &["-r=conflict", "file"], "resolved\n");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: znkkpsqq 6e173dd9 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 4a4c9232 conflict | conflict
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @r#"
    file: Ok(Resolved(Some(File { id: FileId("2ab19ae607aabda796309682e0448237aab03047"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#);
}
