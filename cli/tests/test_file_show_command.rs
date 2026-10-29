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
use crate::common::create_commit_with_files;

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

#[test]
fn test_show_treeset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("f", "base\n")]);
    create_commit_with_files(&work_dir, "a", &["base"], &[("f", "a\n"), ("g", "a\n")]);
    create_commit_with_files(&work_dir, "b", &["base"], &[("f", "b\n")]);

    let output = work_dir.run_jj(["file", "show", "-r", "tree_merge(a | b)", "f", "g"]);
    insta::assert_snapshot!(output, @r#"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: rlvkpnrz 6d39a326 "base"
    \\\\\\\        to: royxmykx ae420c18 "b"
    -base
    +b
    +++++++ zsuskuln 8533f9de "a"
    a
    >>>>>>> conflict 1 of 1 ends
    a
    [EOF]
    "#);
    let output = work_dir.run_jj([
        "file",
        "show",
        "-r",
        "conflict_side(tree_merge(a | b), 1)",
        "f",
        "g",
    ]);
    insta::assert_snapshot!(output, @"
    b
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r", "tree_merge(a | b)"]);
    insta::assert_snapshot!(output, @"
    f
    g
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
