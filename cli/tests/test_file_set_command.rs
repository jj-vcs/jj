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

use crate::common::TestEnvironment;
use crate::common::create_commit_with_files;

#[test]
fn test_file_set() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file.txt", "original\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=a", "file.txt"])
            .write_stdin("new contents\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln c0a7b6ad (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 776d4585 a | a
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file.txt"]), @"
    new contents
    [EOF]
    ");
}

#[test]
fn test_file_set_rebases_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The child commit only adds a new file, so editing `file` in `base` does
    // not produce a conflict on rebase.
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=base", "file"])
            .write_stdin("modified\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 72a196e8 child | child
    Parent commit (@-)      : rlvkpnrz 7c5ffc58 base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The parent revision has the new content.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @"
    modified
    [EOF]
    ");
    // The child revision inherits the new content since it didn't modify `file`.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @"
    modified
    [EOF]
    ");
}

#[test]
fn test_file_set_restore_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The child explicitly modifies `file`, so with --restore-descendants it
    // should keep "child\n" regardless of what happens in `base`.
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("file", "child\n")]);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=base", "--restore-descendants", "file"])
            .write_stdin("modified\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content)
    Working copy  (@) now at: zsuskuln ce938357 child | child
    Parent commit (@-)      : rlvkpnrz 7c5ffc58 base | base
    [EOF]
    ");

    // The parent has the new content.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @"
    modified
    [EOF]
    ");
    // The child preserved its own content because of --restore-descendants.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @"
    child
    [EOF]
    ");
}

#[test]
fn test_file_set_no_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let op_log_before = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=a", "file"])
            .write_stdin("hello\n")
    });
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // No new operation was created.
    let op_log_after = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);
    assert_eq!(op_log_before, op_log_after);
}

#[test]
fn test_file_set_immutable() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "main", &[], &[("file", "hello\n")]);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=main", "file"])
            .write_stdin("content\n")
    });
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit b08080c83282 is immutable
    Hint: Could not modify commit: rlvkpnrz b08080c8 main | main
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://docs.jj-vcs.dev/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["--ignore-immutable", "file", "set", "-r=main", "file"])
            .write_stdin("content\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 371bd7b1 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 0e505733 main | main
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=main", "file"]), @"
    content
    [EOF]
    ");
}

#[test]
fn test_file_set_directory() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("dir/file", "hello\n")]);

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=a", "dir"])
            .write_stdin("content\n")
    });
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Path is a directory: dir
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_file_set_new_file() {
    // Setting a file that does not yet exist in the revision creates it.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("existing", "old\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=a", "newfile"])
            .write_stdin("brand new\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 702c5236 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 08838e71 a | a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "newfile"]), @"
    brand new
    [EOF]
    ");
}

#[test]
fn test_file_set_preserves_executable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("script.sh", "#!/bin/sh\n")]);
    work_dir
        .run_jj(["file", "chmod", "x", "-r=a", "script.sh"])
        .success();
    let debug_before = work_dir.run_jj(["debug", "tree", "-r=a"]);
    insta::assert_snapshot!(debug_before, @r#"
    script.sh: Ok(Resolved(Some(File { id: FileId("1a2485251c33a70432394c93fb89330ef214bfc9"), executable: true, copy_id: CopyId("") })))
    [EOF]
    "#);

    work_dir.run_jj(["new", "-r=a"]).success();
    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=a", "script.sh"])
            .write_stdin("#!/bin/sh\necho hello\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: royxmykx 8ddf4d1e (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz b4f8233a a | a
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The executable bit must still be set after the set.
    let debug_after = work_dir.run_jj(["debug", "tree", "-r=a"]);
    insta::assert_snapshot!(debug_after, @r#"
    script.sh: Ok(Resolved(Some(File { id: FileId("21ba682558a42264518f1e0ba55e8a5cd9d7db0a"), executable: true, copy_id: CopyId("") })))
    [EOF]
    "#);
}

#[test]
fn test_file_set_conflict() {
    // Setting a conflicted file replaces it with the provided content,
    // resolving the conflict.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);
    work_dir.run_jj(["new", "-r=conflict"]).success();

    // Confirm the file is indeed conflicted before we set it.
    let show_before = work_dir
        .run_jj(["file", "show", "-r=conflict", "file"])
        .stdout
        .into_raw();
    assert!(
        show_before.contains("<<<"),
        "expected conflict markers, got: {show_before}"
    );

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["file", "set", "-r=conflict", "file"])
            .write_stdin("resolved\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq 9474ffad (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 0613e382 conflict | conflict
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The conflict is now resolved.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=conflict", "file"]), @"
    resolved
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @r#"
    file: Ok(Resolved(Some(File { id: FileId("2ab19ae607aabda796309682e0448237aab03047"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#);
}
