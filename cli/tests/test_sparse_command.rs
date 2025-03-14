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
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let edit_script = test_env.set_up_fake_editor();

    // Write some files to the working copy
    std::fs::write(repo_path.join("file1"), "contents").unwrap();
    std::fs::write(repo_path.join("file2"), "contents").unwrap();
    std::fs::write(repo_path.join("file3"), "contents").unwrap();

    // By default, all files are tracked
    let output = test_env.run_jj_in(&repo_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");

    // Can stop tracking all files
    let output = test_env.run_jj_in(&repo_path, ["sparse", "set", "--remove", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    // The list is now empty
    let output = test_env.run_jj_in(&repo_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @"");
    // They're removed from the working copy
    assert!(!repo_path.join("file1").exists());
    assert!(!repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());
    // But they're still in the commit
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    file2
    file3
    [EOF]
    ");

    // Run commands in sub directory to ensure that patterns are parsed as
    // workspace-relative paths, not cwd-relative ones.
    let sub_dir = repo_path.join("sub");
    std::fs::create_dir(&sub_dir).unwrap();

    // Not a workspace-relative path
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "set", "--add=../file2"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: invalid value '../file2' for '--add <ADD>': Invalid component ".." in repo-relative path "../file2"

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    "#);

    // Can `--add` a few files
    let output = test_env.run_jj_in(
        &sub_dir,
        ["sparse", "set", "--add", "file2", "--add", "file3"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    file3
    [EOF]
    ");
    assert!(!repo_path.join("file1").exists());
    assert!(repo_path.join("file2").exists());
    assert!(repo_path.join("file3").exists());

    // Can combine `--add` and `--remove`
    let output = test_env.run_jj_in(
        &sub_dir,
        [
            "sparse", "set", "--add", "file1", "--remove", "file2", "--remove", "file3",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    assert!(repo_path.join("file1").exists());
    assert!(!repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());

    // Can use `--clear` and `--add`
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "set", "--clear", "--add", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    [EOF]
    ");
    assert!(!repo_path.join("file1").exists());
    assert!(repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());

    // Can reset back to all files
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "reset"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");
    assert!(repo_path.join("file1").exists());
    assert!(repo_path.join("file2").exists());
    assert!(repo_path.join("file3").exists());

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
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @".");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");

    // Can edit with multiple files
    edit_patterns(&["file3", "file2", "file3"]);
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @"file1");
    let output = test_env.run_jj_in(&sub_dir, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    file2
    file3
    [EOF]
    ");
}

#[test]
fn test_sparse_editor_avoids_unc() {
    use std::path::PathBuf;

    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let edit_script = test_env.set_up_fake_editor();

    std::fs::write(edit_script, "dump-path path").unwrap();
    test_env.run_jj_in(&repo_path, ["sparse", "edit"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
}
