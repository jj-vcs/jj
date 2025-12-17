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

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use regex::Regex;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit_with_files;

/// Assert that a file's executable bit matches the expected value.
#[cfg(unix)]
#[track_caller]
fn assert_file_executable(path: &Path, expected: bool) {
    let perms = path.metadata().unwrap().permissions();
    let actual = (perms.mode() & 0o100) == 0o100;
    assert_eq!(actual, expected);
}

/// Set the executable bit of a file on the filesystem.
#[cfg(unix)]
#[track_caller]
pub fn set_file_executable(path: &Path, executable: bool) {
    let prev_mode = path.metadata().unwrap().permissions().mode();
    let is_executable = prev_mode & 0o100 != 0;
    assert_ne!(executable, is_executable, "why are you calling this?");
    let new_mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, PermissionsExt::from_mode(new_mode)).unwrap();
}

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
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from base to side #1
    -base
    +x
    +++++++ side #2
    n
    >>>>>>> conflict 1 of 1 ends
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
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from base to side #1
    -base
    +x
    +++++++ side #2
    n
    >>>>>>> conflict 1 of 1 ends
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
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from base to side #1
    -base
    +x
    +++++++ side #2
    n
    >>>>>>> conflict 1 of 1 ends
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
    <<<<<<< conflict 1 of 1
    +++++++ side #1
    a
    %%%%%%% diff from base to side #2
    -base
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "chmod", "x", "file", "-r=file_deletion"]);
    insta::assert_snapshot!(output, @r"
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
    ");
    let output = work_dir.run_jj(["debug", "tree", "-r=file_deletion"]);
    insta::assert_snapshot!(output, @r#"
    file: Ok(Conflicted([Some(File { id: FileId("78981922613b2afb6025042ff6bd878ac1994e85"), executable: true, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: true, copy_id: CopyId("") }), None]))
    [EOF]
    "#);
    let output = work_dir.run_jj(["file", "show", "-r=file_deletion", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< conflict 1 of 1
    +++++++ side #1
    a
    %%%%%%% diff from base to side #2
    -base
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_chmod_exec_bit_settings() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let path = &work_dir.root().join("file");

    // The timestamps in the `jj debug local-working-copy` output change, so we want
    // to remove them before asserting the snapshot
    let timestamp_regex = Regex::new(r"\b\d{10,}\b").unwrap();
    let redact_timestamp = |output: String| {
        let output = timestamp_regex.replace_all(&output, "<timestamp>");
        output.into_owned()
    };

    // Load with an explicit "auto" value to test the deserialization.
    test_env.add_config(r#"working-copy.exec-bit-change = "auto""#);
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);

    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_timestamp), @r#"
    Current operation: OperationId("8c58a72d1118aa7d8b1295949a7fa8c6fcda63a3c89813faf2b8ca599ceebf8adcfcbeb8f0bbb6439c86b47dd68b9cf85074c9e57214c3fb4b632e0c9e87ad65")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("6d5f482d15035cdd7733b1b551d1fead28d22592")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(false) }             5 <timestamp> None "file"
    [EOF]
    "#); // in-repo: false, on-disk: false (1/4)

    // 1. Start respecting the executable bit
    test_env.add_config(r#"working-copy.exec-bit-change = "respect""#);
    create_commit_with_files(&work_dir, "respect", &["base"], &[]);

    set_file_executable(path, true);
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_timestamp), @r#"
    Current operation: OperationId("3a6a78820e6892164ed55680b92fa679fbb4d6acd4135c7413d1b815bedcd2c24c85ac8f4f96c96f76012f33d31ffbf50473b938feadf36fcd9c92997789aeca")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("5201dbafb66dc1b28b029a262e1b206f6f93df1e")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(true) }             5 <timestamp> None "file"
    [EOF]
    "#); // in-repo: true, on-disk: true (2/4)

    work_dir.run_jj(["file", "chmod", "n", "file"]).success();
    assert_file_executable(path, false);

    work_dir.run_jj(["file", "chmod", "x", "file"]).success();
    assert_file_executable(path, true);

    // 2. Now ignore the executable bit
    create_commit_with_files(&work_dir, "ignore", &["base"], &[]);
    test_env.add_config(r#"working-copy.exec-bit-change = "ignore""#);
    set_file_executable(path, true);

    // chmod should affect the repo state, but not the on-disk file.
    work_dir.run_jj(["file", "chmod", "n", "file"]).success();
    assert_file_executable(path, true);
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_timestamp), @r#"
    Current operation: OperationId("cab1801e16b54d5b413f638bdf74388520b51232c88db6b314ef64b054607ab82ae6ef0b1f707b52aa8d2131511f6f48f8ca52e465621ff38c442b0ec893f309")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("6d5f482d15035cdd7733b1b551d1fead28d22592")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(true) }             5 <timestamp> None "file"
    [EOF]
    "#); // in-repo: false, on-disk: true (3/4)

    set_file_executable(path, false);
    work_dir.run_jj(["file", "chmod", "x", "file"]).success();
    assert_file_executable(path, false);
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_timestamp), @r#"
    Current operation: OperationId("def8ce6211dcff6d2784d5309d36079c1cb6eeb70821ae144982c76d38ed76fedc8b84e4daddaac70f6a0aae1c301ff5b60e1baa6ac371dabd77cec3537d2c39")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("5201dbafb66dc1b28b029a262e1b206f6f93df1e")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(false) }             5 <timestamp> None "file"
    [EOF]
    "#); // in-repo: true, on-disk: false (4/4) Yay! We've observed all possible states!

    // 3. Go back to respecting the executable bit
    test_env.add_config(r#"working-copy.exec-bit-change = "respect""#);
    work_dir.write_file("file", "update the file so respect notices the new state\n");

    assert_file_executable(path, false);
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy  (@) : znkkpsqq 71681768 ignore | ignore
    Parent commit (@-): rlvkpnrz 1792382a base | base
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "chmod", "x", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: znkkpsqq ef0a25b6 ignore | ignore
    Parent commit (@-)      : rlvkpnrz 1792382a base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    assert_file_executable(path, true);

    work_dir.run_jj(["new", "base"]).success();
    set_file_executable(path, true);
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_timestamp), @r#"
    Current operation: OperationId("0cce4e44f0b47cc4404f74fe164536aa57f67b8981726ce6ec88c39d79e266a2586a79d51a065906b6d8b284b39fe0ab023547f65571d1b61a97916f7f7cf4d8")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("5201dbafb66dc1b28b029a262e1b206f6f93df1e")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(true) }             5 <timestamp> None "file"
    [EOF]
    "#);
}

#[cfg(unix)]
#[test]
fn test_chmod_exec_bit_ignore() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let path = &work_dir.root().join("file");

    test_env.add_config(r#"working-copy.exec-bit-change = "ignore""#);

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    assert_file_executable(path, false);

    // 1. Reverting to "in-repo: true, on-disk: false" works.
    create_commit_with_files(&work_dir, "repo-x-disk-n", &["base"], &[]);
    work_dir.run_jj(["file", "chmod", "x", "file"]).success();
    assert_file_executable(path, false);

    // Commit, update the file, then reset the file.
    work_dir.run_jj(["new"]).success();
    work_dir.write_file(path, "something");
    work_dir.run_jj(["abandon"]).success();
    // The on-disk exec bit should remain false.
    assert_file_executable(path, false);

    // 2. Reverting to "in-repo: false, on-disk: true" works.
    create_commit_with_files(&work_dir, "repo-n-disk-x", &["base"], &[]);
    set_file_executable(path, true);
    work_dir.run_jj(["file", "chmod", "n", "file"]).success();
    assert_file_executable(path, true);

    // Commit, update the file, then reset the file.
    work_dir.run_jj(["new"]).success();
    work_dir.write_file(path, "something");
    work_dir.run_jj(["abandon"]).success();
    // The on-disk exec bit should remain true.
    assert_file_executable(path, true);
}

#[cfg(unix)]
#[test]
fn test_chmod_exec_bit_ignore_then_respect() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let path = &work_dir.root().join("file");

    // Start while ignoring executable bits.
    test_env.add_config(r#"working-copy.exec-bit-change = "ignore""#);
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);

    // Set the in-repo executable bit to true.
    let output = work_dir.run_jj(["file", "chmod", "x", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz cb3f99cb base | base
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    assert_file_executable(path, false);

    test_env.add_config(r#"working-copy.exec-bit-change = "respect""#);
    work_dir.write_file("file", "update the file so respect notices the new state\n");

    // This simultaneously snapshots and updates the executable bit.
    let output = work_dir.run_jj(["file", "chmod", "x", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 96872a96 base | base
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    assert_file_executable(path, true);
}
