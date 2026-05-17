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

#[must_use]
fn get_log_output(work_dir: &crate::common::TestWorkDir) -> crate::common::CommandOutput {
    work_dir.run_jj(["log", "-T", "bookmarks"])
}

#[test]
fn test_file_edit() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file.txt", "original\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    // Verify editor sees the original content, then overwrite it.
    // \0 separates multiple instructions within a single editor invocation.
    std::fs::write(&edit_script, "expect\noriginal\n\0write\nnew contents\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=a", "file.txt"]);
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
fn test_file_edit_at_working_copy() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file.txt", "original\n");

    std::fs::write(&edit_script, "write\nedited\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "file.txt"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 317f7579 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "file.txt"]), @"
    edited
    [EOF]
    ");
}

#[test]
fn test_file_edit_uses_file_name_being_edited() {
    // The temp file opened in the editor should be the file's basename
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "a",
        &[],
        &[("foo.txt", "foo\n"), ("dir/bar.txt", "bar\n")],
    );

    std::fs::write(&edit_script, "dump-path path_dump").unwrap();
    work_dir
        .run_jj(["file", "edit", "-r=a", "foo.txt"])
        .success();
    let path = std::fs::read_to_string(test_env.env_root().join("path_dump"))
        .unwrap()
        .replace('\\', "/");
    assert_eq!(
        std::path::Path::new(&path).file_name().unwrap(),
        "foo.txt",
        "temp file for foo.txt should be named foo.txt, got: {path}"
    );

    std::fs::write(&edit_script, "dump-path path_dump").unwrap();
    work_dir
        .run_jj(["file", "edit", "-r=a", "dir/bar.txt"])
        .success();
    let path = std::fs::read_to_string(test_env.env_root().join("path_dump"))
        .unwrap()
        .replace('\\', "/");
    assert_eq!(
        std::path::Path::new(&path).file_name().unwrap(),
        "bar.txt",
        "temp file for dir/bar.txt should be named bar.txt (not dir/bar.txt), got: {path}"
    );
}

#[test]
fn test_file_edit_rebases_descendants() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The child commit only adds a new file, so editing `file` in `base` does
    // not produce a conflict on rebase.
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  child
    ○  base
    ◆
    [EOF]
    ");

    std::fs::write(&edit_script, "write\nmodified\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln aa7f70b8 child | child
    Parent commit (@-)      : rlvkpnrz 221f1897 base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The parent revision has the edited content.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @"
    modified
    [EOF]
    ");
    // The child revision has the new parent's content for `file` since it didn't
    // touch that file.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @"
    modified
    [EOF]
    ");
}

#[test]
fn test_file_edit_restore_descendants() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The child explicitly modifies `file`, so with --restore-descendants it
    // should keep "child\n" regardless of what happens in `base`.
    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("file", "child\n")]);
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  child
    ○  base
    ◆
    [EOF]
    ");

    std::fs::write(&edit_script, "write\nmodified\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=base", "--restore-descendants", "file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content)
    Working copy  (@) now at: zsuskuln f8c05ab2 child | child
    Parent commit (@-)      : rlvkpnrz 221f1897 base | base
    [EOF]
    ");

    // The parent has the edited content.
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
fn test_file_edit_no_change() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    // Editor leaves the file unchanged.
    std::fs::write(&edit_script, "").unwrap();
    let op_log_before = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);

    let output = work_dir.run_jj(["file", "edit", "-r=a", "file"]);
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
fn test_file_edit_immutable() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "main", &[], &[("file", "hello\n")]);

    let output = work_dir.run_jj(["file", "edit", "-r=main", "file"]);
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

    std::fs::write(&edit_script, "write\nedited\n").unwrap();
    let output = work_dir.run_jj(["--ignore-immutable", "file", "edit", "-r=main", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 5936de99 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz be0f8891 main | main
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=main", "file"]), @"
    edited
    [EOF]
    ");
}

#[test]
fn test_file_edit_new_file() {
    // Editing a path that doesn't exist in the revision opens with empty
    // content and creates the file on save.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("existing", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    std::fs::write(&edit_script, "write\nbrand new\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=a", "newfile"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 4af53614 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 05c2911c a | a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "newfile"]), @"
    brand new
    [EOF]
    ");
}

#[test]
fn test_file_edit_directory() {
    let mut test_env = TestEnvironment::default();
    test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("dir/file", "hello\n")]);

    let output = work_dir.run_jj(["file", "edit", "-r=a", "dir"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Path is a directory: dir
    [EOF]
    [exit status: 1]
    ");
}

/// Set up a simple 2-sided conflict: base="base\n", left="left\n",
/// right="right\n".
fn set_up_conflict(work_dir: &crate::common::TestWorkDir) {
    create_commit_with_files(work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(work_dir, "conflict", &["left", "right"], &[]);
}

#[test]
fn test_file_edit_conflict_shows_markers() {
    // The editor should receive the materialized conflict markers.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    set_up_conflict(&work_dir);

    // Dump what the editor sees into a file, leaving the conflict unchanged.
    std::fs::write(&edit_script, "dump conflict_dump").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Verify the editor saw conflict markers matching what `jj file show` emits.
    let dumped = std::fs::read_to_string(test_env.env_root().join("conflict_dump")).unwrap();
    let shown = work_dir
        .run_jj(["file", "show", "-r=conflict", "file"])
        .stdout
        .into_raw();
    assert_eq!(dumped, shown);
}

#[test]
fn test_file_edit_conflict_resolve() {
    // Editing a conflicted file and removing all conflict markers resolves it.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    set_up_conflict(&work_dir);
    work_dir.run_jj(["new", "-r=conflict"]).success();

    std::fs::write(&edit_script, "write\nresolved\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq 6e173dd9 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 4a4c9232 conflict | conflict
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The conflict should be resolved.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=conflict", "file"]), @"
    resolved
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @r#"
    file: Ok(Resolved(Some(File { id: FileId("2ab19ae607aabda796309682e0448237aab03047"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#);
}

#[test]
fn test_file_edit_conflict_update() {
    // Editing a conflicted file and keeping (modified) conflict markers keeps it
    // conflicted but updates the sides.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    set_up_conflict(&work_dir);

    // First, dump what the editor sees so we know the exact marker format.
    std::fs::write(&edit_script, "dump original_conflict").unwrap();
    work_dir
        .run_jj(["file", "edit", "-r=conflict", "file"])
        .success();
    let original = std::fs::read_to_string(test_env.env_root().join("original_conflict")).unwrap();

    // Replace "left" with "new-left" in the conflict markers and write it back.
    let modified = original.replace("left\n", "new-left\n");
    std::fs::write(&edit_script, format!("write\n{modified}")).unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv ad5baaa3 conflict | (conflict) conflict
    Parent commit (@-)      : zsuskuln c0778f46 left | left
    Parent commit (@-)      : royxmykx 386edb4d right | right
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    New conflicts appeared in 1 commits:
      vruxwmqv ad5baaa3 conflict | (conflict) conflict
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new vruxwmqv
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    // The file should still be conflicted but with the updated side content.
    let shown = work_dir
        .run_jj(["file", "show", "-r=conflict", "file"])
        .stdout
        .into_raw();
    assert!(
        shown.contains("new-left"),
        "expected new-left in conflict, got: {shown}"
    );
    assert!(
        !shown.contains("left\n") || shown.contains("new-left"),
        "expected old 'left' to be replaced"
    );
}

#[test]
fn test_file_edit_conflict_no_change() {
    // If the editor makes no changes to a conflict, "Nothing changed." is printed.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    set_up_conflict(&work_dir);

    let op_log_before = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);

    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    let op_log_after = work_dir.run_jj(["op", "log", "--no-graph", "-Tid.short()"]);
    assert_eq!(op_log_before, op_log_after);
}

#[test]
fn test_file_edit_editor_fail() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "hello\n")]);
    work_dir.run_jj(["new", "-r=a"]).success();

    std::fs::write(&edit_script, "fail").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=a", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Editor '/Users/stephen/src/github.com/jj-vcs/jj/target/debug/fake-editor' exited with exit status: 1
    [EOF]
    [exit status: 1]
    ");

    // The commit was not modified.
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=a", "file"]), @"
    hello
    [EOF]
    ");
}

#[test]
fn test_file_edit_preserves_executable() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
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

    std::fs::write(&edit_script, "write\n#!/bin/sh\necho hello\n").unwrap();
    let output = work_dir.run_jj(["file", "edit", "-r=a", "script.sh"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz f276cfce a | a
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // The executable bit must still be set after the edit.
    let debug_after = work_dir.run_jj(["debug", "tree", "-r=a"]);
    insta::assert_snapshot!(debug_after, @r#"
    script.sh: Ok(Resolved(Some(File { id: FileId("21ba682558a42264518f1e0ba55e8a5cd9d7db0a"), executable: true, copy_id: CopyId("") })))
    [EOF]
    "#);
}
