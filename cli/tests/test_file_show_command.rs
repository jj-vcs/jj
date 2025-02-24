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
fn test_show() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["new"]);
    std::fs::write(repo_path.join("file1"), "b\n").unwrap();
    std::fs::create_dir(repo_path.join("dir")).unwrap();
    std::fs::write(repo_path.join("dir").join("file2"), "c\n").unwrap();

    // Can print the contents of a file in a commit
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file1", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");

    // Defaults to printing the working-copy version
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");

    // Can print a file in a subdirectory
    let subdir_file = if cfg!(unix) {
        "dir/file2"
    } else {
        "dir\\file2"
    };
    let output = test_env.run_jj_in(&repo_path, ["file", "show", subdir_file]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");

    // Error if the path doesn't exist
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such path: nonexistent
    [EOF]
    [exit status: 1]
    ");

    // Can print files under the specified directory
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "dir"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");

    // Can print multiple files
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "."]);
    insta::assert_snapshot!(output, @r"
    c
    b
    [EOF]
    ");

    // Unmatched paths should generate warnings
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file1", "non-existent"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: non-existent
    [EOF]
    ");

    // Can print a conflict
    test_env.jj_cmd_ok(&repo_path, &["new"]);
    std::fs::write(repo_path.join("file1"), "c\n").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["rebase", "-r", "@", "-d", "@--"]);
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -b
    +a
    +++++++ Contents of side #2
    c
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_show_symlink() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::create_dir(repo_path.join("dir")).unwrap();
    std::fs::write(repo_path.join("dir").join("file2"), "c\n").unwrap();
    std::os::unix::fs::symlink("symlink1_target", repo_path.join("symlink1")).unwrap();

    // Can print multiple files
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "."]);
    insta::assert_snapshot!(output, @r"
    c
    a
    [EOF]
    ------- stderr -------
    Warning: Path 'symlink1' exists but is not a file
    [EOF]
    ");
}
