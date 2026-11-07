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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit_with_files;

/// Prints the path and content of every file in the revision.
#[must_use]
fn get_file_contents(work_dir: &TestWorkDir, rev: &str) -> CommandOutput {
    work_dir
        .run_jj([
            "file",
            "show",
            &format!("-r={rev}"),
            "-T",
            r#"path ++ ": ""#,
            "all()",
        ])
        .normalize_backslash()
}

#[test]
fn test_file_move() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[("file", "base\n"), ("dir/a", "a\n"), ("dir/b", "b\n")],
    );
    create_commit_with_files(
        &work_dir,
        "child",
        &["base"],
        &[("file", "child\n"), ("dir/a", "child a\n")],
    );
    create_commit_with_files(&work_dir, "grandchild", &["child"], &[("other", "other\n")]);
    let setup_opid = work_dir.current_operation_id();

    // The file is moved in the revision and all its descendants, which keep
    // their own content.
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'renamed' in 3 commits.
    Working copy  (@) now at: royxmykx b0ffa2b2 grandchild | grandchild
    Parent commit (@-)      : zsuskuln 3b950c7a child | child
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    dir/a: a
    dir/b: b
    renamed: base
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/a: child a
    dir/b: b
    renamed: child
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "grandchild"), @r"
    dir/a: child a
    dir/b: b
    other: other
    renamed: child
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["diff", "--summary", "-r=child"]), @r"
    M dir/a
    M renamed
    [EOF]
    ");

    // A directory can be moved into a new parent directory
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "dir", "new/sub"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'dir' to 'new/sub' in 3 commits.
    Working copy  (@) now at: royxmykx 76e71260 grandchild | grandchild
    Parent commit (@-)      : zsuskuln d15fba50 child | child
    Added 2 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    file: base
    new/sub/a: a
    new/sub/b: b
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "grandchild"), @r"
    file: child
    new/sub/a: child a
    new/sub/b: b
    other: other
    [EOF]
    ");

    // Ancestors of the source revision are left untouched
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=child", "file", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'renamed' in 2 commits.
    Working copy  (@) now at: royxmykx 470b1c68 grandchild | grandchild
    Parent commit (@-)      : zsuskuln 413d260f child | child
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    dir/a: a
    dir/b: b
    file: base
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/a: child a
    dir/b: b
    renamed: child
    [EOF]
    ");

    // The source defaults to the working-copy revision
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "file", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'renamed' in 1 commits.
    Working copy  (@) now at: royxmykx f4a89385 grandchild | grandchild
    Parent commit (@-)      : zsuskuln 04e52562 child | child
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/a: child a
    dir/b: b
    file: child
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "grandchild"), @r"
    dir/a: child a
    dir/b: b
    other: other
    renamed: child
    [EOF]
    ");

    // A file can be moved into an existing directory
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "dir/file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'dir/file' in 3 commits.
    Working copy  (@) now at: royxmykx be33aa3a grandchild | grandchild
    Parent commit (@-)      : zsuskuln 878a3eaa child | child
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/a: child a
    dir/b: b
    dir/file: child
    [EOF]
    ");

    // Moving a path onto itself does nothing
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);

    // A file can be moved below its own path
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "file/file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'file/file' in 3 commits.
    Working copy  (@) now at: royxmykx b897ca2e grandchild | grandchild
    Parent commit (@-)      : zsuskuln 960fed79 child | child
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/a: child a
    dir/b: b
    file/file: child
    [EOF]
    ");

    // A directory can be moved into its own subdirectory
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "dir", "dir/sub"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'dir' to 'dir/sub' in 3 commits.
    Working copy  (@) now at: royxmykx f548bf5e grandchild | grandchild
    Parent commit (@-)      : zsuskuln 79fd4a4d child | child
    Added 2 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir/sub/a: child a
    dir/sub/b: b
    file: child
    [EOF]
    ");

    // A path can replace its parent directory, overwriting its siblings
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "dir/a", "dir"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Overwrote existing 'dir' in these commits:
      rlvkpnrz 760c323d base | base
      zsuskuln 79e280e2 child | child
      royxmykx e9087ac2 grandchild | grandchild
    Hint: If this was not intentional, run `jj undo` to restore the previous state.
    Moved 'dir/a' to 'dir' in 3 commits.
    Working copy  (@) now at: royxmykx e9087ac2 grandchild | grandchild
    Parent commit (@-)      : zsuskuln 79e280e2 child | child
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    dir: child a
    file: child
    [EOF]
    ");
}

#[test]
fn test_file_move_missing_source() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("keep", "keep\n")]);
    create_commit_with_files(&work_dir, "child", &["base"], &[("file", "child\n")]);
    let setup_opid = work_dir.current_operation_id();

    // Revisions without the path are reported, and the path is moved in the
    // other revisions
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: 'file' does not exist in these commits:
      rlvkpnrz 93ad9c98 base | base
    Moved 'file' to 'renamed' in 1 commits.
    Working copy  (@) now at: zsuskuln 4c44d2bb child | child
    Parent commit (@-)      : rlvkpnrz 93ad9c98 base | base
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    keep: keep
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    keep: keep
    renamed: child
    [EOF]
    ");

    // Nothing happens if the path doesn't exist in any revision
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    let output = work_dir.run_jj(["file", "move", "-s=base", "nonexistent", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: 'nonexistent' does not exist in these commits:
      rlvkpnrz 93ad9c98 base | base
      zsuskuln 3c06a88e child | child
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);
}

#[test]
fn test_file_move_overwrite() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[
            ("file", "base\n"),
            ("dir/a", "a\n"),
            ("blocker", "blocker\n"),
        ],
    );
    create_commit_with_files(&work_dir, "child", &["base"], &[("target", "target\n")]);
    let setup_opid = work_dir.current_operation_id();

    // Only revisions in which the destination exists are reported
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "target"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Overwrote existing 'target' in these commits:
      zsuskuln acd01b91 child | (empty) child
    Hint: If this was not intentional, run `jj undo` to restore the previous state.
    Moved 'file' to 'target' in 2 commits.
    Working copy  (@) now at: zsuskuln acd01b91 child | (empty) child
    Parent commit (@-)      : rlvkpnrz 7c0a078c base | base
    Added 0 files, modified 1 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    blocker: blocker
    dir/a: a
    target: base
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "child"), @r"
    blocker: blocker
    dir/a: a
    target: base
    [EOF]
    ");

    // An existing directory is replaced entirely
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "dir"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Overwrote existing 'dir' in these commits:
      rlvkpnrz c5c69211 base | base
      zsuskuln 8c6a7391 child | child
    Hint: If this was not intentional, run `jj undo` to restore the previous state.
    Moved 'file' to 'dir' in 2 commits.
    Working copy  (@) now at: zsuskuln 8c6a7391 child | child
    Parent commit (@-)      : rlvkpnrz c5c69211 base | base
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    blocker: blocker
    dir: base
    [EOF]
    ");

    // A file in a parent position is replaced with a directory
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "move", "-s=base", "dir", "blocker/nested"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Overwrote existing 'blocker' in these commits:
      rlvkpnrz 3204f5fc base | base
      zsuskuln 6d523e9c child | child
    Hint: If this was not intentional, run `jj undo` to restore the previous state.
    Moved 'dir' to 'blocker/nested' in 2 commits.
    Working copy  (@) now at: zsuskuln 6d523e9c child | child
    Parent commit (@-)      : rlvkpnrz 3204f5fc base | base
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_file_contents(&work_dir, "base"), @r"
    blocker/nested/a: a
    file: base
    [EOF]
    ");
}

#[test]
fn test_file_move_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);

    // The conflict moves with the file, and no new conflicts are created
    let output = work_dir.run_jj(["file", "move", "-s=base", "file", "renamed"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 'file' to 'renamed' in 4 commits.
    Working copy  (@) now at: vruxwmqv bc24f5eb conflict | (conflict) (empty) conflict
    Parent commit (@-)      : zsuskuln 99071469 left | left
    Parent commit (@-)      : royxmykx 867531e0 right | right
    Added 1 files, modified 0 files, removed 1 files
    Warning: There are unresolved conflicts at these paths:
    renamed    2-sided conflict
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @r#"
    renamed: Ok(Conflicted([Some(File { id: FileId("45cf141ba67d59203f02a54f03162f3fcef57830"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false, copy_id: CopyId("") }), Some(File { id: FileId("c376d892e8b105bd712d06ec5162b5f31ce949c3"), executable: false, copy_id: CopyId("") })]))
    [EOF]
    "#);
    let template = r#"separate(" ", description.first_line(), if(conflict, "(conflict)")) ++ "\n""#;
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", template]), @r"
    @    conflict (conflict)
    ├─╮
    │ ○  right
    ○ │  left
    ├─╯
    ○  base
    ◆
    [EOF]
    ");
}
