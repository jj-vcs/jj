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

use std::io::Write;

use crate::common::TestEnvironment;

pub mod common;

#[test]
fn test_sparse_manage_patterns() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let edit_script = test_env.set_up_fake_editor();

    // Write some files to the working copy
    std::fs::write(repo_path.join("file1"), "contents").unwrap();
    std::fs::write(repo_path.join("file2"), "contents").unwrap();
    std::fs::write(repo_path.join("file3"), "contents").unwrap();

    // By default, all files are tracked
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    .
    "###);

    // Can stop tracking all files
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "set", "--remove", "."]);
    insta::assert_snapshot!(stdout, @r###"
    Added 0 files, modified 0 files, removed 3 files
    "###);
    // The list is now empty
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @"");
    // They're removed from the working copy
    assert!(!repo_path.join("file1").exists());
    assert!(!repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());
    // But they're still in the commit
    let stdout = test_env.jj_cmd_success(&repo_path, &["files"]);
    insta::assert_snapshot!(stdout, @r###"
    file1
    file2
    file3
    "###);

    // Can `--add` a few files
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["sparse", "set", "--add", "file2", "--add", "file3"],
    );
    insta::assert_snapshot!(stdout, @r###"
    Added 2 files, modified 0 files, removed 0 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file2
    file3
    "###);
    assert!(!repo_path.join("file1").exists());
    assert!(repo_path.join("file2").exists());
    assert!(repo_path.join("file3").exists());

    // Can combine `--add` and `--remove`
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "sparse", "set", "--add", "file1", "--remove", "file2", "--remove", "file3",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    Added 1 files, modified 0 files, removed 2 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file1
    "###);
    assert!(repo_path.join("file1").exists());
    assert!(!repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());

    // Can use `--clear` and `--add`
    let stdout =
        test_env.jj_cmd_success(&repo_path, &["sparse", "set", "--clear", "--add", "file2"]);
    insta::assert_snapshot!(stdout, @r###"
    Added 1 files, modified 0 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file2
    "###);
    assert!(!repo_path.join("file1").exists());
    assert!(repo_path.join("file2").exists());
    assert!(!repo_path.join("file3").exists());

    // Can reset back to all files
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "set", "--reset"]);
    insta::assert_snapshot!(stdout, @r###"
    Added 2 files, modified 0 files, removed 0 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    .
    "###);
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
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "set", "--edit"]);
    insta::assert_snapshot!(stdout, @r###"
    Added 0 files, modified 0 files, removed 2 files
    "###);
    insta::assert_snapshot!(read_patterns(), @".");
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file1
    "###);

    // Can edit with `--clear` and `--add`
    edit_patterns(&["file2"]);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["sparse", "set", "--edit", "--clear", "--add", "file1"],
    );
    insta::assert_snapshot!(stdout, @r###"
    Added 1 files, modified 0 files, removed 1 files
    "###);
    insta::assert_snapshot!(read_patterns(), @"file1");
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file2
    "###);

    // Can edit with multiple files
    edit_patterns(&["file2", "file3"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "set", "--clear", "--edit"]);
    insta::assert_snapshot!(stdout, @r###"
    Added 1 files, modified 0 files, removed 0 files
    "###);
    insta::assert_snapshot!(read_patterns(), @"");
    let stdout = test_env.jj_cmd_success(&repo_path, &["sparse", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    file2
    file3
    "###);
}
