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

    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[
            ("file", "base\n"),
            ("dir/a", "a\n"),
            ("dir/b", "b\n"),
            ("notes.txt", "notes\n"),
            ("data.txt", "data\n"),
            ("keep", "keep\n"),
        ],
    );
    // The child doesn't touch `file`, so it inherits its deletion from `base`.
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);
    let setup_opid = work_dir.current_operation_id();

    // The file is deleted from the revision, and descendants are rebased.
    let output = work_dir.run_jj(["file", "delete", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln 69c03f6b child | child
    Parent commit (@-)      : rlvkpnrz 0f03ba3b base | base
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=base"]).normalize_backslash(), @r"
    data.txt
    dir/a
    dir/b
    keep
    notes.txt
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=child"]).normalize_backslash(), @r"
    data.txt
    dir/a
    dir/b
    keep
    notes.txt
    other
    [EOF]
    ");

    // With --restore-descendants, descendants keep their content.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "delete", "-r=base", "--restore-descendants", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content).
    Working copy  (@) now at: zsuskuln 178385e9 child | child
    Parent commit (@-)      : rlvkpnrz 53605aea base | base
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=base"]).normalize_backslash(), @r"
    data.txt
    dir/a
    dir/b
    keep
    notes.txt
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=child"]).normalize_backslash(), @r"
    data.txt
    dir/a
    dir/b
    file
    keep
    notes.txt
    other
    [EOF]
    ");

    // Multiple paths, directories, and filesets can be deleted at once
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "delete", "-r=base", "file", "dir", "glob:*.txt"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln ea53444d child | child
    Parent commit (@-)      : rlvkpnrz dce80445 base | base
    Added 0 files, modified 0 files, removed 5 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=base"]), @r"
    keep
    [EOF]
    ");

    // Unmatched paths are reported, but matched paths are still deleted
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["file", "delete", "-r=base", "file", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln 1ac20d8f child | child
    Parent commit (@-)      : rlvkpnrz 5676c4df base | base
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "list", "-r=base"]).normalize_backslash(), @r"
    data.txt
    dir/a
    dir/b
    keep
    notes.txt
    [EOF]
    ");

    // Nothing happens if no paths match
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    let output = work_dir.run_jj(["file", "delete", "-r=base", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);
}

#[test]
fn test_file_delete_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);
    work_dir.run_jj(["new", "conflict"]).success();

    // Deleting a conflicted file removes the conflict
    let output = work_dir.run_jj(["file", "delete", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: znkkpsqq 59c14e97 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 06df9ed5 conflict | conflict
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @"");
}
