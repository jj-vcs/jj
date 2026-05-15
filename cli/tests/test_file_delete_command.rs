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
fn test_file_delete() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj(["file", "delete", "-r=a", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 05587b47 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 73d7da45 a | (empty) a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // The file is gone from the revision.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_file_delete_multiple_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "a",
        &[],
        &[("file1", "one\n"), ("file2", "two\n"), ("keep", "keep\n")],
    );
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj(["file", "delete", "-r=a", "file1", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 381c0779 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 40dacdef a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    // Both deleted files are gone; the third is still present.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file1"]), @"
    ------- stderr -------
    Error: No such path: file1
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file2"]), @"
    ------- stderr -------
    Error: No such path: file2
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "keep"]), @"
    keep
    [EOF]
    ");
}

#[test]
fn test_file_delete_directory() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "a",
        &[],
        &[("dir/a", "a\n"), ("dir/b", "b\n"), ("keep", "keep\n")],
    );
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj(["file", "delete", "-r=a", "dir"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 381c0779 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 40dacdef a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    // All files under dir/ are gone.
    insta::assert_snapshot!(
        work_dir.run_jj(["file", "show", "-r=a", "dir/a"]).normalize_backslash(), @"
    ------- stderr -------
    Error: No such path: dir/a
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(
        work_dir.run_jj(["file", "show", "-r=a", "dir/b"]).normalize_backslash(), @"
    ------- stderr -------
    Error: No such path: dir/b
    [EOF]
    [exit status: 1]
    ");
    // The file outside the directory is untouched.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "keep"]), @"
    keep
    [EOF]
    ");
}

#[test]
fn test_file_delete_fileset() {
    // A jj fileset expression (glob:) can be used as the path argument.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "a",
        &[],
        &[
            ("readme.md", "readme\n"),
            ("notes.txt", "notes\n"),
            ("data.txt", "data\n"),
        ],
    );
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj(["file", "delete", "-r=a", "glob:*.txt"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 929a622c (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 2b863d10 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    // Only .txt files are deleted; the .md file is untouched.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "notes.txt"]), @"
    ------- stderr -------
    Error: No such path: notes.txt
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "data.txt"]), @"
    ------- stderr -------
    Error: No such path: data.txt
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "readme.md"]), @"
    readme
    [EOF]
    ");
}

#[test]
fn test_file_delete_rebases_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The child commit only touches a different file so the rebase is clean.
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);

    let output = work_dir.run_jj(["file", "delete", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 58951718 child | child
    Parent commit (@-)      : rlvkpnrz e39fdc42 base | (empty) base
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // File is gone from both the rewritten base and the rebased child.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_file_delete_no_match() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let op_log_before = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);

    let output = work_dir.run_jj(["file", "delete", "-r=a", "nonexistent"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
    [EOF]
    ");

    // No new operation was created.
    let op_log_after = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);
    assert_eq!(op_log_before, op_log_after);
}

#[test]
fn test_file_delete_partial_match() {
    // A warning is emitted for the unmatched path, but the matched path is
    // still deleted — the warning is informational, not fatal.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    let output = work_dir.run_jj(["file", "delete", "-r=a", "file", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 05587b47 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 73d7da45 a | (empty) a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // The matched file was deleted.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_file_delete_immutable() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "bookmarks(main)""#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "main", &[], &[("file", "hello\n")]);

    let output = work_dir.run_jj(["file", "delete", "-r=main", "file"]);
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

    let output = work_dir.run_jj(["--ignore-immutable", "file", "delete", "-r=main", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz f6eab8bc main | (empty) main
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=main", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_file_delete_conflict() {
    // Deleting a conflicted file removes it cleanly regardless of how many
    // conflict sides there are.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);
    work_dir.run_jj(["new", "-r=conflict"]).success();

    // Confirm the file is conflicted before deletion.
    let show_before = work_dir
        .run_jj(["file", "show", "-r=conflict", "file"])
        .stdout
        .into_raw();
    assert!(
        show_before.contains("<<<"),
        "expected conflict markers, got: {show_before}"
    );

    let output = work_dir.run_jj(["file", "delete", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq 068ccef1 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 8dd54404 conflict | conflict
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // The file is completely gone — no conflict entry remains.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=conflict", "file"]), @"
    ------- stderr -------
    Error: No such path: file
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @"");
}
