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

use std::path::PathBuf;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_squash() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  382c9bad7d42 c
    ○  d5d59175b481 b
    ○  184ddbcce5a9 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // Squashes the working copy into the parent by default
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv f7bb78d8 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 59f44460 b c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  f7bb78d8da62 (empty)
    ○  59f4446070a0 b c
    ○  184ddbcce5a9 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");

    // Can squash a given commit into its parent
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 1d70f50a c | (no description set)
    Parent commit (@-)      : qpvuntsm 9146bcc8 a b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  1d70f50afa6d c
    ○  9146bcc8d996 a b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");

    // Cannot squash a merge commit (because it's unclear which parent it should go
    // into)
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["edit", "b"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();
    work_dir.write_file("file2", "d\n");
    work_dir.run_jj(["new", "c", "d"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "e"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    41219719ab5f e (empty)
    ├─╮
    │ ○  f86e2b3af3e3 d
    ○ │  382c9bad7d42 c
    ├─╯
    ○  d5d59175b481 b
    ○  184ddbcce5a9 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot squash merge commits without a specified destination
    Hint: Use `--into` to specify which parent to squash into
    [EOF]
    [exit status: 1]
    ");

    // Can squash into a merge commit
    work_dir.run_jj(["new", "e"]).success();
    work_dir.write_file("file1", "e\n");
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: xlzxqlsl b50b843d (empty) (no description set)
    Parent commit (@-)      : nmzmmopx 338cbc05 e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b50b843d8555 (empty)
    ○    338cbc05e4e6 e
    ├─╮
    │ ○  f86e2b3af3e3 d
    ○ │  382c9bad7d42 c
    ├─╯
    ○  d5d59175b481 b
    ○  184ddbcce5a9 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "e"]);
    insta::assert_snapshot!(output, @r"
    e
    [EOF]
    ");
}

#[test]
fn test_squash_partial() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    work_dir.write_file("file2", "c\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  a0b1a272ebc4 c
    ○  d117da276a0f b
    ○  54d3c1c0e9fd a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // If we don't make any changes in the diff-editor, the whole change is moved
    // into the parent
    std::fs::write(&edit_script, "dump JJ-INSTRUCTIONS instrs").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 3c633226 c | (no description set)
    Parent commit (@-)      : qpvuntsm 38ffd8b9 a b | (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are moving changes from: kkmpptxz d117da27 b | (no description set)
    into commit: qpvuntsm 54d3c1c0 a | (no description set)

    The left side of the diff shows the contents of the parent commit. The
    right side initially shows the contents of the commit you're moving
    changes from.

    Adjust the right side until the diff shows the changes you want to move
    to the destination. If you don't make any changes, then all the changes
    from the source will be moved into the destination.
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3c6332267ea8 c
    ○  38ffd8b98578 a b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");

    // Can squash only some changes in interactive mode
    work_dir.run_jj(["undo"]).success();
    std::fs::write(&edit_script, "reset file1").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 57c3cf20 c | (no description set)
    Parent commit (@-)      : kkmpptxz c4925e01 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  57c3cf20d0b1 c
    ○  c4925e01d298 b
    ○  1fc159063ed3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");

    // Can squash only some changes in non-interactive mode
    work_dir.run_jj(["undo"]).success();
    // Clear the script so we know it won't be used even without -i
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 64d7ad7c c | (no description set)
    Parent commit (@-)      : kkmpptxz 60a26452 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  64d7ad7c43c1 c
    ○  60a264527aee b
    ○  7314692d32e3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");

    // If we specify only a non-existent file, then nothing changes.
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "-r", "b", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // We get a warning if we pass a positional argument that looks like a revset
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "b"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: The argument "b" is being interpreted as a fileset expression. To specify a revset, pass -r "b" instead.
    Nothing changed.
    [EOF]
    "#);
}

#[test]
fn test_squash_keep_emptied() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    // Test the setup

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  382c9bad7d42 c
    ○  d5d59175b481 b
    ○  184ddbcce5a9 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    let output = work_dir.run_jj(["squash", "-r", "b", "--keep-emptied"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 7ee7f18a c | (no description set)
    Parent commit (@-)      : kkmpptxz 9490bd7f b | (empty) (no description set)
    [EOF]
    ");
    // With --keep-emptied, b remains even though it is now empty.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7ee7f18a5223 c
    ○  9490bd7f1e6a b (empty)
    ○  53bf93080518 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");
}

#[test]
fn test_squash_from_to() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    // F
    // |
    // E C
    // | |
    // D B
    // |/
    // A
    //
    // When moving changes between e.g. C and F, we should not get unrelated changes
    // from B and D.
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.write_file("file3", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file3", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    work_dir.run_jj(["edit", "a"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();
    work_dir.write_file("file3", "d\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "e"])
        .success();
    work_dir.write_file("file2", "e\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "f"])
        .success();
    work_dir.write_file("file2", "f\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  a847ab4967fe f
    ○  c2f9de87325d e
    ○  e0dac715116f d
    │ ○  59597b34a0d8 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // Errors out if source and destination are the same
    let output = work_dir.run_jj(["squash", "--into", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Source and destination cannot be the same
    [EOF]
    [exit status: 1]
    ");

    // Can squash from sibling, which results in the source being abandoned
    let output = work_dir.run_jj(["squash", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw b902d1dd f | (no description set)
    Parent commit (@-)      : znkkpsqq c2f9de87 e | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b902d1dd59d9 f
    ○  c2f9de87325d e
    ○  e0dac715116f d
    │ ○  12d6103dc0c8 b c
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    // File `file2`, which was not changed in source, is unchanged
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    f
    [EOF]
    ");

    // Can squash from ancestor
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from", "@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw cfc5eb87 f | (no description set)
    Parent commit (@-)      : znkkpsqq 4dc7c279 e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'd'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  cfc5eb876eb1 f
    ○  4dc7c27994bd e
    │ ○  59597b34a0d8 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a d
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied (the file contents were already
    // "f", as is typically the case when moving changes from an ancestor)
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    f
    [EOF]
    ");

    // Can squash from descendant
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from", "e", "--into", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kmkuslsw 6de62c22 f | (no description set)
    Parent commit (@-)      : vruxwmqv 32196a11 d e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  6de62c22fa07 f
    ○  32196a117ee3 d e
    │ ○  59597b34a0d8 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "d"]);
    insta::assert_snapshot!(output, @r"
    e
    [EOF]
    ");
}

#[test]
fn test_squash_from_to_partial() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    //   C
    //   |
    // D B
    // |/
    // A
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.write_file("file3", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file3", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    work_dir.write_file("file2", "c\n");
    work_dir.run_jj(["edit", "a"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();
    work_dir.write_file("file3", "d\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e0dac715116f d
    │ ○  087591be5a01 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // If we don't make any changes in the diff-editor, the whole change is moved
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 987bcfb2 d | (no description set)
    Parent commit (@-)      : qpvuntsm b7b76717 a | (no description set)
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  987bcfb2eb62 d
    │ ○  12d6103dc0c8 b c
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    // File `file3`, which was not changed in source, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @r"
    d
    [EOF]
    ");

    // Can squash only part of the change in interactive mode
    work_dir.run_jj(["undo"]).success();
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 576244e8 d | (no description set)
    Parent commit (@-)      : qpvuntsm b7b76717 a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  576244e87883 d
    │ ○  6f486f2f4539 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    // File `file3`, which was changed in source's parent, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @r"
    d
    [EOF]
    ");

    // Can squash only part of the change from a sibling in non-interactive mode
    work_dir.run_jj(["undo"]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "--from", "c", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 5b407c24 d | (no description set)
    Parent commit (@-)      : qpvuntsm b7b76717 a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5b407c249fa7 d
    │ ○  724d64da1487 c
    │ ○  12d6103dc0c8 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    // File `file3`, which was changed in source's parent, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @r"
    d
    [EOF]
    ");

    // Can squash only part of the change from a descendant in non-interactive mode
    work_dir.run_jj(["undo"]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "--from", "c", "--into", "b", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e0dac715116f d
    │ ○  d2a587ae205d c
    │ ○  a53394306362 b
    ├─╯
    ○  b7b767179c44 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");

    // If we specify only a non-existent file, then nothing changes.
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from", "c", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_squash_working_copy_restore_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    //   Y
    //   |
    // B X@
    // |/
    // A
    //
    // Each commit adds a file named the same as the commit
    let create_commit = |name: &str| {
        work_dir
            .run_jj(["bookmark", "create", "-r@", name])
            .success();
        work_dir.write_file(name, format!("test {name}\n"));
    };

    create_commit("a");
    work_dir.run_jj(["new"]).success();
    create_commit("b");
    work_dir.run_jj(["new", "a"]).success();
    create_commit("x");
    work_dir.run_jj(["new"]).success();
    create_commit("y");
    work_dir.run_jj(["edit", "x"]).success();

    let template = r#"separate(
        " ",
        commit_id.short(),
        bookmarks,
        description,
        if(empty, "(empty)")
    )"#;
    let run_log = || work_dir.run_jj(["log", "-r=::", "--summary", "-T", template]);

    // Verify the setup
    insta::assert_snapshot!(run_log(), @r"
    ○  c6d2eb64ab67 y
    │  A y
    @  adce7593bb1e x
    │  A x
    │ ○  a67b55091cc7 b
    ├─╯  A b
    ○  b73559961e51 a
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=a"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    a
    x
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=y"]);
    insta::assert_snapshot!(output, @r"
    a
    x
    y
    [EOF]
    ");

    let output = work_dir.run_jj(["squash", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits (while preserving their content)
    Working copy  (@) now at: kxryzmor 0d433c00 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 3e409b51 a x | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @  0d433c0038f4 (empty)
    │ ○  ba61d055a013 y
    ├─╯  A y
    │ ○  356e0d3a5041 b
    ├─╯  A b
    │    D x
    ○  3e409b51fdd1 a x
    │  A a
    │  A x
    ◆  000000000000 (empty)
    [EOF]
    ");

    let output = work_dir.run_jj(["diff", "--summary"]);
    //  The current commit becomes empty.
    insta::assert_snapshot!(output, @"");
    // Should coincide with the working copy commit before
    let output = work_dir.run_jj(["file", "list", "-r=a"]);
    insta::assert_snapshot!(output, @r"
    a
    x
    [EOF]
    ");
    // Commit b should be the same as before
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=y"]);
    insta::assert_snapshot!(output, @r"
    a
    x
    y
    [EOF]
    ");
}

#[test]
fn test_squash_from_to_restore_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    // F
    // |\
    // E C
    // | |
    // D B
    // |/
    // A
    //
    // Each commit adds a file named the same as the commit
    let create_commit = |name: &str| {
        work_dir
            .run_jj(["bookmark", "create", "-r@", name])
            .success();
        work_dir.write_file(name, format!("test {name}\n"));
    };

    create_commit("a");
    work_dir.run_jj(["new"]).success();
    create_commit("b");
    work_dir.run_jj(["new"]).success();
    create_commit("c");
    work_dir.run_jj(["new", "a"]).success();
    create_commit("d");
    work_dir.run_jj(["new"]).success();
    create_commit("e");
    work_dir.run_jj(["new", "e", "c"]).success();
    create_commit("f");

    let template = r#"separate(
        " ",
        commit_id.short(),
        bookmarks,
        description,
        if(empty, "(empty)")
    )"#;
    let run_log = || work_dir.run_jj(["log", "-r=::", "--summary", "-T", template]);

    // ========== Part 1 =========
    // Verify the setup
    insta::assert_snapshot!(run_log(), @r"
    @    669e39fae6d0 f
    ├─╮  A f
    │ ○  c5088cb5f49a c
    │ │  A c
    │ ○  a67b55091cc7 b
    │ │  A b
    ○ │  5d50c4afbd25 e
    │ │  A e
    ○ │  deaf82e70838 d
    ├─╯  A d
    ○  b73559961e51 a
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let beginning = work_dir.current_operation_id();
    test_env.advance_test_rng_seed_to_multiple_of(200_000);

    // Squash without --restore-descendants for comparison
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=b", "--into=d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits
    Working copy  (@) now at: kpqxywon 4cf82c6a f | (no description set)
    Parent commit (@-)      : yostqsxw 1c3a995c e | (no description set)
    Parent commit (@-)      : mzvwutvl 281dfbb6 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    4cf82c6a2a40 f
    ├─╮  A f
    │ ○  281dfbb64805 c
    │ │  A c
    ○ │  1c3a995ce262 e
    │ │  A e
    ○ │  107de126bf16 d
    ├─╯  A b
    │    A d
    ○  b73559961e51 a b
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=e"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    d
    e
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=f"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    d
    e
    f
    [EOF]
    ");

    // --restore-descendants
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=b", "--into=d", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon 14e71a32 f | (no description set)
    Parent commit (@-)      : yostqsxw 611a1a64 e | (no description set)
    Parent commit (@-)      : mzvwutvl 04318e1d c | (no description set)
    [EOF]
    ");
    //  `d`` becomes the same as in the above example,
    // but `c` does not lose file `b` and `e` still does not contain file `b`
    // regardless of what happened to their parents.
    insta::assert_snapshot!(run_log(), @r"
    @    14e71a32aefe f
    ├─╮  A f
    │ ○  04318e1d6e24 c
    │ │  A b
    │ │  A c
    ○ │  611a1a64daca e
    │ │  D b
    │ │  A e
    ○ │  0dc2f5a4e0f5 d
    ├─╯  A b
    │    A d
    ○  b73559961e51 a b
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=e"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    e
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=f"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    d
    e
    f
    [EOF]
    ");

    // --restore-descendants works with --keep-emptied, same result except for
    // leaving an empty commit
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj([
        "squash",
        "--from=b",
        "--into=d",
        "--restore-descendants",
        "--keep-emptied",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon 95a902be f | (no description set)
    Parent commit (@-)      : yostqsxw 90259340 e | (no description set)
    Parent commit (@-)      : mzvwutvl a374ec5a c | (no description set)
    [EOF]
    ");
    //  `d`` becomes the same as in the above example,
    // but `c` does not lose file `b` and `e` still does not contain file `b`
    // regardless of what happened to their parents.
    insta::assert_snapshot!(run_log(), @r"
    @    95a902beb718 f
    ├─╮  A f
    │ ○  a374ec5af343 c
    │ │  A b
    │ │  A c
    │ ○  ddb5691f9ef9 b (empty)
    ○ │  90259340e8cd e
    │ │  D b
    │ │  A e
    ○ │  a8bc17cfd6b0 d
    ├─╯  A b
    │    A d
    ○  b73559961e51 a
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=e"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    e
    [EOF]
    ");

    // ========== Part 2 =========
    // Reminder of the setup
    test_env.advance_test_rng_seed_to_multiple_of(200_000);
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    insta::assert_snapshot!(run_log(), @r"
    @    669e39fae6d0 f
    ├─╮  A f
    │ ○  c5088cb5f49a c
    │ │  A c
    │ ○  a67b55091cc7 b
    │ │  A b
    ○ │  5d50c4afbd25 e
    │ │  A e
    ○ │  deaf82e70838 d
    ├─╯  A d
    ○  b73559961e51 a
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");

    // --restore-descendants works when squashing from parent to child
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=a", "--into=b", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon f3face8d f | (no description set)
    Parent commit (@-)      : yostqsxw d00f972b e | (no description set)
    Parent commit (@-)      : mzvwutvl 84c37061 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    f3face8d3626 f
    ├─╮  A f
    │ ○  84c37061f88e c
    │ │  A c
    │ ○  3e7890f3e57a b
    │ │  A a
    │ │  A b
    ○ │  d00f972b8e57 e
    │ │  A e
    ○ │  99c7cbbf26f1 d
    ├─╯  A a
    │    A d
    ◆  000000000000 a (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");

    // --restore-descendants --keep-emptied works when squashing from parent to
    // child
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj([
        "squash",
        "--from=a",
        "--into=b",
        "--restore-descendants",
        "--keep-emptied",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon 7df3d054 f | (no description set)
    Parent commit (@-)      : yostqsxw 5a7c0319 e | (no description set)
    Parent commit (@-)      : mzvwutvl 0810c9f6 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    7df3d0546069 f
    ├─╮  A f
    │ ○  0810c9f6be14 c
    │ │  A c
    │ ○  12262f386fb3 b
    │ │  A a
    │ │  A b
    ○ │  5a7c031974ec e
    │ │  A e
    ○ │  07f2ffd3abca d
    ├─╯  A a
    │    A d
    ○  6d3799d50594 a (empty)
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");

    // --restore-descendants works when squashing from child to parent
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=b", "--into=a", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon e008b685 f | (no description set)
    Parent commit (@-)      : yostqsxw 915b0b35 e | (no description set)
    Parent commit (@-)      : mzvwutvl d4bfa8a6 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    e008b6854aad f
    ├─╮  A b
    │ │  A f
    │ ○  d4bfa8a660a4 c
    │ │  A c
    ○ │  915b0b359ff9 e
    │ │  A e
    ○ │  dd3e6a11e957 d
    ├─╯  D b
    │    A d
    ○  25fabe100a50 a b
    │  A a
    │  A b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");

    // same test, but with --keep-emptied
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj([
        "squash",
        "--from=b",
        "--into=a",
        "--keep-emptied",
        "--restore-descendants",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 5 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon a665ec55 f | (no description set)
    Parent commit (@-)      : yostqsxw aa2e7fda e | (no description set)
    Parent commit (@-)      : mzvwutvl d38c68b2 c | (no description set)
    [EOF]
    ");
    // BUG! b should now be empty!
    insta::assert_snapshot!(run_log(), @r"
    @    a665ec55feec f
    ├─╮  A b
    │ │  A f
    │ ○  d38c68b26a18 c
    │ │  A b
    │ │  A c
    │ ○  d9f8bd6ddc8d b
    │ │  D b
    ○ │  aa2e7fdab311 e
    │ │  A e
    ○ │  82253f121bbe d
    ├─╯  D b
    │    A d
    ○  fec1fccdaee4 a
    │  A a
    │  A b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=c"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");

    // ========== Part 3 =========
    // Reminder of the setup
    test_env.advance_test_rng_seed_to_multiple_of(200_000);
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    insta::assert_snapshot!(run_log(), @r"
    @    669e39fae6d0 f
    ├─╮  A f
    │ ○  c5088cb5f49a c
    │ │  A c
    │ ○  a67b55091cc7 b
    │ │  A b
    ○ │  5d50c4afbd25 e
    │ │  A e
    ○ │  deaf82e70838 d
    ├─╯  A d
    ○  b73559961e51 a
    │  A a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=f"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    d
    e
    f
    [EOF]
    ");

    // --restore-descendants works when squashing from grandchild to grandparent
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=e", "--into=a", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon fc56f622 f | (no description set)
    Parent commit (@-)      : yqosqzyt e5209795 d e | (no description set)
    Parent commit (@-)      : mzvwutvl 9560c92a c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    fc56f622d1a4 f
    ├─╮  A e
    │ │  A f
    │ ○  9560c92a929a c
    │ │  A c
    │ ○  46000a5f868f b
    │ │  A b
    │ │  D e
    ○ │  e520979539a9 d e
    ├─╯  A d
    │    D e
    ○  2fe83df3a80c a
    │  A a
    │  A e
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=f"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    d
    e
    f
    [EOF]
    ");

    // --restore-descendants works when squashing from grandparent to grandchild
    work_dir
        .run_jj(["operation", "restore", &beginning])
        .success();
    let output = work_dir.run_jj(["squash", "--from=a", "--into=e", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content)
    Working copy  (@) now at: kpqxywon 9d34b6cb f | (no description set)
    Parent commit (@-)      : yostqsxw e323559d e | (no description set)
    Parent commit (@-)      : mzvwutvl f0235d79 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(run_log(), @r"
    @    9d34b6cb2828 f
    ├─╮  A f
    │ ○  f0235d7969f6 c
    │ │  A c
    │ ○  f844dadb304c b
    │ │  A a
    │ │  A b
    ○ │  e323559d5244 e
    │ │  A e
    ○ │  31c7dae860c6 d
    ├─╯  A a
    │    A d
    ◆  000000000000 a (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=b"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=d"]);
    insta::assert_snapshot!(output, @r"
    a
    d
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list", "-r=f"]);
    insta::assert_snapshot!(output, @r"
    a
    b
    c
    d
    e
    f
    [EOF]
    ");
}

#[test]
fn test_squash_from_multiple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    //   F
    //   |
    //   E
    //  /|\
    // B C D
    //  \|/
    //   A
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file", "b\n");
    work_dir.run_jj(["new", "@-"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file", "c\n");
    work_dir.run_jj(["new", "@-"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();
    work_dir.write_file("file", "d\n");
    work_dir.run_jj(["new", "all:visible_heads()"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "e"])
        .success();
    work_dir.write_file("file", "e\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "f"])
        .success();
    work_dir.write_file("file", "f\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  94e57ecb8d4f f
    ○      78ed28eb87b8 e
    ├─┬─╮
    │ │ ○  35e764e4357c b
    │ ○ │  02a128cd4344 c
    │ ├─╯
    ○ │  aaf7b53a1b64 d
    ├─╯
    ○  3b1673b6370c a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // Squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b", "--from=c", "--into=d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon 7ea39167 f | (no description set)
    Parent commit (@-)      : yostqsxw acfbf2a0 e | (no description set)
    New conflicts appeared in 1 commits:
      yqosqzyt 4df3b215 d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by updating to it:
      jj new yqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7ea391676d52 f
    ○    acfbf2a0600d e
    ├─╮
    × │  4df3b2156c3d d
    ├─╯
    ○  3b1673b6370c a b c
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the sources have been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base #1 to side #1
    -a
    +d
    %%%%%%% Changes from base #2 to side #2
    -a
    +b
    +++++++ Contents of side #3
    c
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");

    // Squash a few commits up an down
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn 6a670d1a (empty) (no description set)
    Parent commit (@-)      : yostqsxw c1293ff7 e f | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  6a670d1ac76e (empty)
    ○    c1293ff7be51 e f
    ├─╮
    ○ │  aaf7b53a1b64 d
    ├─╯
    ○  3b1673b6370c a b c
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the sources have been applied to the destination
    let output = work_dir.run_jj(["file", "show", "-r=e", "file"]);
    insta::assert_snapshot!(output, @r"
    f
    [EOF]
    ");

    // Empty squash shouldn't crash
    let output = work_dir.run_jj(["squash", "--from=none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_squash_from_multiple_partial() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    //   F
    //   |
    //   E
    //  /|\
    // B C D
    //  \|/
    //   A
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["new", "@-"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    work_dir.write_file("file2", "c\n");
    work_dir.run_jj(["new", "@-"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();
    work_dir.write_file("file1", "d\n");
    work_dir.write_file("file2", "d\n");
    work_dir.run_jj(["new", "all:visible_heads()"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "e"])
        .success();
    work_dir.write_file("file1", "e\n");
    work_dir.write_file("file2", "e\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "f"])
        .success();
    work_dir.write_file("file1", "f\n");
    work_dir.write_file("file2", "f\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  30980b9045f7 f
    ○      5326a04aac1f e
    ├─┬─╮
    │ │ ○  d117da276a0f b
    │ ○ │  93a7bfff61e7 c
    │ ├─╯
    ○ │  763809ca0131 d
    ├─╯
    ○  54d3c1c0e9fd a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // Partially squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b|c", "--into=d", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon a8530305 f | (no description set)
    Parent commit (@-)      : yostqsxw 0a3637fc e | (no description set)
    New conflicts appeared in 1 commits:
      yqosqzyt 05a3ab3d d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by updating to it:
      jj new yqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  a8530305127c f
    ○      0a3637fca632 e
    ├─┬─╮
    │ │ ○  450d1499c1ae b
    │ ○ │  14b44bf0473c c
    │ ├─╯
    × │  05a3ab3dffc8 d
    ├─╯
    ○  54d3c1c0e9fd a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected changes have been removed from the sources
    let output = work_dir.run_jj(["file", "show", "-r=b", "file1"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=c", "file1"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    // The selected changes from the sources have been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file1"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base #1 to side #1
    -a
    +d
    %%%%%%% Changes from base #2 to side #2
    -a
    +b
    +++++++ Contents of side #3
    c
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
    // The unselected change from the sources have not been applied to the
    // destination
    let output = work_dir.run_jj(["file", "show", "-r=d", "file2"]);
    insta::assert_snapshot!(output, @r"
    d
    [EOF]
    ");

    // Partially squash a few commits up an down
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kpqxywon 3b7559b8 f | (no description set)
    Parent commit (@-)      : yostqsxw a3b1714c e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3b7559b89a57 f
    ○      a3b1714cdfb2 e
    ├─┬─╮
    │ │ ○  867efb38e801 b
    │ ○ │  84dcb3d4b3eb c
    │ ├─╯
    ○ │  763809ca0131 d
    ├─╯
    ○  54d3c1c0e9fd a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected changes have been removed from the sources
    let output = work_dir.run_jj(["file", "show", "-r=b", "file1"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=c", "file1"]);
    insta::assert_snapshot!(output, @r"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=f", "file1"]);
    insta::assert_snapshot!(output, @r"
    f
    [EOF]
    ");
    // The selected changes from the sources have been applied to the destination
    let output = work_dir.run_jj(["file", "show", "-r=e", "file1"]);
    insta::assert_snapshot!(output, @r"
    f
    [EOF]
    ");
    // The unselected changes from the sources have not been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file2"]);
    insta::assert_snapshot!(output, @r"
    d
    [EOF]
    ");
}

#[test]
fn test_squash_from_multiple_partial_no_op() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create history like this:
    // B C D
    //  \|/
    //   A
    work_dir.run_jj(["describe", "-m=a"]).success();
    work_dir.write_file("a", "a\n");
    work_dir.run_jj(["new", "-m=b"]).success();
    work_dir.write_file("b", "b\n");
    work_dir.run_jj(["new", "@-", "-m=c"]).success();
    work_dir.write_file("c", "c\n");
    work_dir.run_jj(["new", "@-", "-m=d"]).success();
    work_dir.write_file("d", "d\n");
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b37ca1ee3306 d
    │ ○  f40b442af3e8 c
    ├─╯
    │ ○  b73077b08c59 b
    ├─╯
    ○  2443ea76b0b1 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    // Source commits that didn't match the paths are not rewritten
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl e178068a d
    Parent commit (@-)      : qpvuntsm 2443ea76 a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e178068add8c d
    │ ○  f40b442af3e8 c
    ├─╯
    ○  2443ea76b0b1 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj([
        "evolog",
        "-T",
        r#"separate(" ", commit_id.short(), description)"#,
    ]);
    insta::assert_snapshot!(output, @r"
    @    e178068add8c d
    ├─╮
    │ ○  b73077b08c59 b
    │ ○  a786561e909f b
    ○  b37ca1ee3306 d
    ○  1d9eb34614c9 d
    [EOF]
    ");

    // If no source commits match the paths, then the whole operation is a no-op
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b37ca1ee3306 d
    │ ○  f40b442af3e8 c
    ├─╯
    │ ○  b73077b08c59 b
    ├─╯
    ○  2443ea76b0b1 a
    ◆  000000000000 (empty)
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(
        " ",
        commit_id.short(),
        bookmarks,
        description,
        if(empty, "(empty)")
    )"#;
    work_dir.run_jj(["log", "-T", template])
}

#[test]
fn test_squash_description() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(&edit_script, r#"fail"#).unwrap();

    // If both descriptions are empty, the resulting description is empty
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // If the destination's description is empty and the source's description is
    // non-empty, the resulting description is from the source
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    source
    [EOF]
    ");

    // If the destination description is non-empty and the source's description is
    // empty, the resulting description is from the destination
    work_dir.run_jj(["op", "restore", "@--"]).success();
    work_dir
        .run_jj(["describe", "@-", "-m", "destination"])
        .success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    destination
    [EOF]
    ");

    // An explicit description on the command-line overrides this
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["squash", "-m", "custom"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    custom
    [EOF]
    ");

    // If both descriptions were non-empty, we get asked for a combined description
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    std::fs::write(&edit_script, "dump editor0").unwrap();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    destination

    source
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r#"
    JJ: Enter a description for the combined commit.
    JJ: Description from the destination commit:
    destination

    JJ: Description from source commit:
    source

    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // An explicit description on the command-line overrides prevents launching an
    // editor
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["squash", "-m", "custom"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    custom
    [EOF]
    ");

    // If the source's *content* doesn't become empty, then the source remains and
    // both descriptions are unchanged
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["squash", "file1"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    destination
    [EOF]
    ");
    insta::assert_snapshot!(get_description(&work_dir, "@"), @r"
    source
    [EOF]
    ");
}

#[test]
fn test_squash_description_editor_avoids_unc() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir
        .run_jj(["describe", "@-", "-m", "destination"])
        .success();
    work_dir.run_jj(["describe", "-m", "source"]).success();

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["squash"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
}

#[test]
fn test_squash_empty() {
    let mut test_env = TestEnvironment::default();
    test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "parent"]).success();

    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz adece6e8 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 5076fc41 (empty) parent
    [EOF]
    ");
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    parent
    [EOF]
    ");

    work_dir.run_jj(["describe", "-m", "child"]).success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    parent

    child
    [EOF]
    ");
}

#[test]
fn test_squash_use_destination_message() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["commit", "-m=b"]).success();
    work_dir.run_jj(["describe", "-m=c"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  8aac283daeac c
    ○  017c7f689ed7 b
    ○  d8d5f980a897 a
    ◆  000000000000
    [EOF]
    ");

    // Squash the current revision using the short name for the option.
    work_dir.run_jj(["squash", "-u"]).success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  fd33e4bc332b
    ○  3a17aa5dcce9 b
    ○  d8d5f980a897 a
    ◆  000000000000
    [EOF]
    ");

    // Undo and squash again, but this time squash both "b" and "c" into "a".
    work_dir.run_jj(["undo"]).success();
    work_dir
        .run_jj([
            "squash",
            "--use-destination-message",
            "--from",
            "description(b)::",
            "--into",
            "description(a)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  7c832accbf60
    ○  688660377651 a
    ◆  000000000000
    [EOF]
    ");
}

// The --use-destination-message and --message options are incompatible.
#[test]
fn test_squash_use_destination_message_and_message_mutual_exclusion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["describe", "-m=b"]).success();
    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "--message=123",
        "--use-destination-message",
    ]), @r"
    ------- stderr -------
    error: the argument '--message <MESSAGE>' cannot be used with '--use-destination-message'

    Usage: jj squash --message <MESSAGE> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[must_use]
fn get_description(work_dir: &TestWorkDir, rev: &str) -> CommandOutput {
    work_dir.run_jj(["log", "--no-graph", "-T", "description", "-r", rev])
}

#[must_use]
fn get_log_output_with_description(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", commit_id.short(), description)"#;
    work_dir.run_jj(["log", "-T", template])
}
