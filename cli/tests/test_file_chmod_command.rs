// Copyright 2023 The Jujutsu Authors
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
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["log", "-T", "bookmarks"])
}

#[test]
fn test_chmod_regular_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "n", &["base"], &[("file", "n\n")]);
    create_commit_with_files(&work_dir, "x", &["base"], &[("file", "x\n")]);
    // Test chmodding a file. The effect will be visible in the conflict below.
    work_dir
        .run_jj(["file", "chmod", "x", "file", "-r=x"])
        .success();
    create_commit_with_files(&work_dir, "conflict", &["x", "n"], &[]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    conflict
    ├─╮
    │ ○  n
    ○ │  x
    ├─╯
    ○  base
    ◆
    [EOF]
    ");
    let output = work_dir.run_jj(["debug", "tree"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("587be6b4c3f93f93c489c0111bba5596147a26cb"), executable: true, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("8ba3a16384aacc37d01564b28401755ce8053f51"), executable: false, copy_id: CopyId("") })]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +x
    +++++++ Contents of side #2
    n
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");

    // Test chmodding a conflict
    work_dir.run_jj(["file", "chmod", "x", "file"]).success();
    let output = work_dir.run_jj(["debug", "tree"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("587be6b4c3f93f93c489c0111bba5596147a26cb"), executable: true, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: true, copy_id: CopyId("") }), Some(File { id: FileId("8ba3a16384aacc37d01564b28401755ce8053f51"), executable: true, copy_id: CopyId("") })]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +x
    +++++++ Contents of side #2
    n
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
    work_dir.run_jj(["file", "chmod", "n", "file"]).success();
    let output = work_dir.run_jj(["debug", "tree"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("587be6b4c3f93f93c489c0111bba5596147a26cb"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("8ba3a16384aacc37d01564b28401755ce8053f51"), executable: false, copy_id: CopyId("") })]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +x
    +++++++ Contents of side #2
    n
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");

    // Unmatched paths should generate warnings
    let output = work_dir.run_jj(["file", "chmod", "x", "nonexistent", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Working copy  (@) now at: yostqsxw e5b18f6d conflict | (conflict) conflict
    Parent commit (@-)      : royxmykx 02247291 x | x
    Parent commit (@-)      : zsuskuln eb0ba805 n | n
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict including an executable
    [EOF]
    ");
}

// TODO: Test demonstrating that conflicts whose *base* is not a file are
// chmod-dable

#[test]
fn test_chmod_file_dir_deletion_conflicts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "file", &["base"], &[("file", "a\n")]);

    create_commit_with_files(&work_dir, "deletion", &["base"], &[]);
    work_dir.remove_file("file");

    create_commit_with_files(&work_dir, "dir", &["base"], &[]);
    work_dir.remove_file("file");
    work_dir.create_dir("file");
    // Without a placeholder file, `jj` ignores an empty directory
    work_dir.write_file("file/placeholder", "");

    // Create a file-dir conflict and a file-deletion conflict
    create_commit_with_files(&work_dir, "file_dir", &["file", "dir"], &[]);
    create_commit_with_files(&work_dir, "file_deletion", &["file", "deletion"], &[]);
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    file_deletion
    ├─╮
    │ ○  deletion
    │ │ ×  file_dir
    ╭───┤
    │ │ ○  dir
    │ ├─╯
    ○ │  file
    ├─╯
    ○  base
    ◆
    [EOF]
    ");

    // The file-dir conflict cannot be chmod-ed
    let output = work_dir.run_jj(["debug", "tree", "-r=file_dir"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("78981922613b2afb6025042ff6bd878ac1994e85"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false, copy_id: CopyId("") }), Some(Tree(TreeId("133bb38fc4e4bf6b551f1f04db7e48f04cac2877")))]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "-r=file_dir", "file"]);
    insta::assert_snapshot!(output, @r"
    Conflict:
      Removing file with id df967b96a579e45a18b8251732d16804b2e56a55
      Adding file with id 78981922613b2afb6025042ff6bd878ac1994e85
      Adding tree with id 133bb38fc4e4bf6b551f1f04db7e48f04cac2877
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "chmod", "x", "file", "-r=file_dir"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Some of the sides of the conflict are not files at 'file'.
    [EOF]
    [exit status: 1]
    ");

    // The file_deletion conflict can be chmod-ed
    let output = work_dir.run_jj(["debug", "tree", "-r=file_deletion"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("78981922613b2afb6025042ff6bd878ac1994e85"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false, copy_id: CopyId("") }), None]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "-r=file_deletion", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    +++++++ Contents of side #1
    a
    %%%%%%% Changes from base to side #2
    -base
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "chmod", "x", "file", "-r=file_deletion"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 849406ce file_deletion | (conflict) file_deletion
    Parent commit (@-)      : zsuskuln bc9cdea1 file | file
    Parent commit (@-)      : royxmykx d7d39332 deletion | deletion
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict including 1 deletion and an executable
    New conflicts appeared in 1 commits:
      kmkuslsw 849406ce file_deletion | (conflict) file_deletion
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new kmkuslsw
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    "###);
    let output = work_dir.run_jj(["debug", "tree", "-r=file_deletion"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("78981922613b2afb6025042ff6bd878ac1994e85"), executable: true, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: true, copy_id: CopyId("") }), None]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "-r=file_deletion", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    +++++++ Contents of side #1
    a
    %%%%%%% Changes from base to side #2
    -base
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}
