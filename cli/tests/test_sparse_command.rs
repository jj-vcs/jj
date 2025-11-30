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

use std::io::Write as _;

use crate::common::TestEnvironment;

#[test]
fn test_sparse_manage_patterns() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Write some files to the working copy
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");

    // By default, all files are tracked
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");

    // Can stop tracking all files
    let output = work_dir.run_jj(["sparse", "set", "--remove", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    // The list is now empty
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"");
    // They're removed from the working copy
    assert!(!work_dir.root().join("file1").exists());
    assert!(!work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());
    // But they're still in the commit
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    file2
    file3
    [EOF]
    ");

    // Run commands in sub directory to ensure that patterns are parsed as
    // workspace-relative paths, not cwd-relative ones.
    let sub_dir = work_dir.create_dir("sub");

    // Not a workspace-relative path
    let output = sub_dir.run_jj(["sparse", "set", "--add=../file2"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: invalid value '../file2' for '--add <ADD>': Invalid component ".." in repo-relative path "../file2"

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    "#);

    // Can `--add` a few files
    let output = sub_dir.run_jj(["sparse", "set", "--add", "file2", "--add", "file3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    file3
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Can combine `--add` and `--remove`
    let output = sub_dir.run_jj([
        "sparse", "set", "--add", "file1", "--remove", "file2", "--remove", "file3",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    assert!(work_dir.root().join("file1").exists());
    assert!(!work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());

    // Can use `--clear` and `--add`
    let output = sub_dir.run_jj(["sparse", "set", "--clear", "--add", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());

    // Can reset back to all files
    let output = sub_dir.run_jj(["sparse", "reset"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");
    assert!(work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Can edit with editor
    let edit_patterns = |patterns: &[&str]| {
        let mut file = std::fs::File::create(&edit_script).unwrap();
        file.write_all(b"dump patterns0\0write\n").unwrap();
        for pattern in patterns {
            file.write_all(pattern.as_bytes()).unwrap();
            file.write_all(b"\n").unwrap();
        }
    };
    let read_patterns = || std::fs::read_to_string(test_env.env_root().join("patterns0")).unwrap();

    edit_patterns(&["file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @".");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");

    // Can edit with multiple files
    edit_patterns(&["file3", "file2", "file3"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @"file1");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    file3
    [EOF]
    ");

    // Invalid paths are rejected
    edit_patterns(&["./file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse sparse pattern: ./file1
    Caused by: Invalid component "." in repo-relative path "./file1"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_sparse_editor_avoids_unc() {
    use std::path::PathBuf;

    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["sparse", "edit"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
}
