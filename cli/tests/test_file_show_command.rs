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

#[cfg(unix)]
use testutils::TestResult;

use crate::common::TestEnvironment;

#[test]
fn test_show() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");
    work_dir.create_dir("dir");
    work_dir.write_file("dir/file2", "c\n");
    work_dir.write_file("file3", "d\n");

    // Can print the contents of a file in a commit
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "@-"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");

    // Defaults to printing the working-copy version
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");

    // Can print a file in a subdirectory
    let subdir_file = if cfg!(unix) {
        "dir/file2"
    } else {
        "dir\\file2"
    };
    let output = work_dir.run_jj(["file", "show", subdir_file]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");

    // Error if the path doesn't exist
    let output = work_dir.run_jj(["file", "show", "nonexistent"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No such path: nonexistent
    [EOF]
    [exit status: 1]
    ");

    // Can print files under the specified directory
    let output = work_dir.run_jj(["file", "show", "dir"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");

    // Can print a single file with template
    let template = r#""--- " ++ path ++ "\n""#;
    let output = work_dir.run_jj(["file", "show", "-T", template, "file1"]);
    insta::assert_snapshot!(output, @"
    --- file1
    b
    [EOF]
    ");

    // Can print multiple files with template
    let output = work_dir.run_jj(["file", "show", "-T", template, "."]);
    insta::assert_snapshot!(output, @"
    --- dir/file2
    c
    --- file1
    b
    --- file3
    d
    [EOF]
    ");

    // Can glob for multiple files too
    let output = work_dir.run_jj(["file", "show", "-T", template, "file*"]);
    insta::assert_snapshot!(output, @"
    --- file1
    b
    --- file3
    d
    [EOF]
    ");

    // Unmatched paths should generate warnings
    let output = work_dir.run_jj(["file", "show", "file1", "non-existent"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: non-existent
    [EOF]
    ");

    // Can print a conflict
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "c\n");
    work_dir
        .run_jj(["rebase", "-r", "@", "-o", "@--"])
        .success();
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: rlvkpnrz fc7b369e (parents of rebased revision)
    \\\\\\\        to: qpvuntsm eb7b8a1f (rebase destination)
    -b
    +a
    +++++++ kmkuslsw f74f80c5 (rebased revision)
    c
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_show_symlink() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.create_dir("dir");
    work_dir.write_file("dir/file2", "c\n");
    std::os::unix::fs::symlink("symlink1_target", work_dir.root().join("symlink1"))?;

    // Can print multiple files with template
    let template = r#""--- " ++ path ++ " [" ++ file_type ++ "]\n""#;
    let output = work_dir.run_jj(["file", "show", "-T", template, "."]);
    insta::assert_snapshot!(output, @"
    --- dir/file2 [file]
    c
    --- file1 [file]
    a
    --- symlink1 [symlink]
    [EOF]
    ------- stderr -------
    Warning: Path 'symlink1' exists but is not a file.
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_show_by_id() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj([
        "file",
        "list",
        "-r",
        "@-",
        "-T",
        r#"normal_value.object_id() ++ "\n""#,
        "file1",
    ]);
    insta::assert_snapshot!(output, @"
    78981922613b2afb6025042ff6bd878ac1994e85
    [EOF]
    ");

    // Can print the contents of a file by file ID
    let output = work_dir.run_jj([
        "file",
        "show",
        "--id",
        "78981922613b2afb6025042ff6bd878ac1994e85",
        "file1",
    ]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");

    // Error on invalid hex file ID
    let output = work_dir.run_jj(["file", "show", "--id", "not-hex", "file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Invalid hex file id: "not-hex"
    [EOF]
    [exit status: 1]
    "#);

    // Error on nonexistent file ID
    let output = work_dir.run_jj([
        "file",
        "show",
        "--id",
        "0000000000000000000000000000000000000000",
        "file1",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Object 0000000000000000000000000000000000000000 of type file not found
    [EOF]
    [exit status: 1]
    ");

    // Error when multiple paths are specified with --id
    let output = work_dir.run_jj([
        "file",
        "show",
        "--id",
        "78981922613b2afb6025042ff6bd878ac1994e85",
        "file1",
        "file2",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Exactly one path must be specified with --id
    [EOF]
    [exit status: 2]
    ");
}
