// Copyright 2022 Google LLC
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

pub mod common;

#[test]
fn test_restore() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "c\n").unwrap();
    std::fs::write(repo_path.join("file3"), "c\n").unwrap();

    // Restores from parent by default
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore"]);
    insta::assert_snapshot!(stdout, @r###"
    Created b05f8b84f2fc 
    Working copy now at: b05f8b84f2fc 
    Added 1 files, modified 1 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"");

    // Can restore from other revision
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "--from", "@--"]);
    insta::assert_snapshot!(stdout, @r###"
    Created 9cb58509136b 
    Working copy now at: 9cb58509136b 
    Added 1 files, modified 0 files, removed 2 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"R file2
");

    // Can restore into other revision
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "--to", "@-"]);
    insta::assert_snapshot!(stdout, @r###"
    Created 5ed06151e039 
    Rebased 1 descendant commits
    Working copy now at: ca6c95b68bd2 
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"");
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(stdout, @r###"
    R file1
    A file2
    A file3
    "###);

    // Can combine `--from` and `--to`
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "--from", "@", "--to", "@-"]);
    insta::assert_snapshot!(stdout, @r###"
    Created c83e17dc46fd 
    Rebased 1 descendant commits
    Working copy now at: df9fb6892f99 
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"");
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(stdout, @r###"
    R file1
    A file2
    A file3
    "###);

    // Can restore only specified paths
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "file2", "file3"]);
    insta::assert_snapshot!(stdout, @r###"
    Created 28647642d4a5 
    Working copy now at: 28647642d4a5 
    Added 0 files, modified 1 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"R file1
");
}

#[test]
fn test_restore_interactive() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();
    std::fs::write(repo_path.join("file3"), "b\n").unwrap();

    let edit_script = test_env.set_up_fake_diff_editor();

    // Nothing happens if we make no changes
    std::fs::write(&edit_script, "").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "-i"]);
    insta::assert_snapshot!(stdout, @"Nothing changed.
");
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @r###"
    R file1
    M file2
    A file3
    "###);

    // Nothing happens if the diff-editor exits with an error
    std::fs::write(&edit_script, "rm file2\0fail").unwrap();
    let stderr = test_env.jj_cmd_failure(&repo_path, &["restore", "-i"]);
    insta::assert_snapshot!(stderr, @"Error: Failed to edit diff: The diff tool exited with a non-zero code
");
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @r###"
    R file1
    M file2
    A file3
    "###);

    // Can restore changes to individual files
    std::fs::write(&edit_script, "reset file2\0reset file3").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "-i"]);
    insta::assert_snapshot!(stdout, @r###"
    Created abdbf6271a1c 
    Working copy now at: abdbf6271a1c 
    Added 0 files, modified 1 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @"R file1
");

    // Can make unrelated edits
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    std::fs::write(&edit_script, "write file3\nunrelated\n").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["restore", "-i"]);
    insta::assert_snapshot!(stdout, @r###"
    Created e31f7f33ad07 
    Working copy now at: e31f7f33ad07 
    Added 0 files, modified 1 files, removed 0 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "--git"]);
    insta::assert_snapshot!(stdout, @r###"
    diff --git a/file1 b/file1
    deleted file mode 100644
    index 7898192261..0000000000
    --- a/file1
    +++ /dev/null
    @@ -1,1 +1,0 @@
    -a
    diff --git a/file2 b/file2
    index 7898192261...6178079822 100644
    --- a/file2
    +++ b/file2
    @@ -1,1 +1,1 @@
    -a
    +b
    diff --git a/file3 b/file3
    new file mode 100644
    index 0000000000..c21c9352f7
    --- /dev/null
    +++ b/file3
    @@ -1,0 +1,1 @@
    +unrelated
    "###);

    // Combining paths with -i is not yet supported
    std::fs::write(&edit_script, "").unwrap();
    let stderr = test_env.jj_cmd_failure(&repo_path, &["restore", "-i", "file2"]);
    insta::assert_snapshot!(stderr.replace("jj.exe", "jj"), @r###"
    error: The argument '--interactive' cannot be used with '<PATHS>...'

    USAGE:
        jj restore --interactive

    For more information try --help
    "###);
}
