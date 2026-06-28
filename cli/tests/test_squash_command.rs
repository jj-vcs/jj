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

use testutils::TestResult;

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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1cc10709fbc0 c
    ○  099a44efcab4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squashes the working copy into the parent by default
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 48829820 (empty) (no description set)
    Parent commit (@-)      : nkmpptxz 818fe366 b c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  48829820e303 (empty)
    ○  818fe3660cdf b c
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");

    // Can squash a given commit into its parent
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: pzvwutvl 6d920b60 c | (no description set)
    Parent commit (@-)      : qpvuntsm 105931bf a b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  6d920b60d0c3 c
    ○  105931bfedad a b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");

    // Cannot squash a merge commit (because it's unclear which parent it should go
    // into)
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    85b0f46cfb80 e (empty)
    ├─╮
    │ ○  f8340c58ec2d d
    ○ │  1cc10709fbc0 c
    ├─╯
    ○  099a44efcab4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: xlzxqlsl 48f45181 (empty) (no description set)
    Parent commit (@-)      : rmzmmopx e07a7b7e e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  48f451818309 (empty)
    ○    e07a7b7e496d e
    ├─╮
    │ ○  f8340c58ec2d d
    ○ │  1cc10709fbc0 c
    ├─╯
    ○  099a44efcab4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "e"]);
    insta::assert_snapshot!(output, @"
    e
    [EOF]
    ");
}

#[test]
fn test_squash_partial() -> TestResult {
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e385ba6aaff1 c
    ○  39826f7cef30 b
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    let start_op_id = work_dir.current_operation_id();

    // If we don't make any changes in the diff-editor, the whole change is moved
    // into the parent
    std::fs::write(&edit_script, "dump JJ-INSTRUCTIONS instrs")?;
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: pzvwutvl d79ac05a c | (no description set)
    Parent commit (@-)      : qpvuntsm 3141e675 a b | (no description set)
    [EOF]
    ");

    let instrs = std::fs::read_to_string(test_env.env_root().join("instrs"))?;
    insta::assert_snapshot!(
        instrs, @"
    You are moving changes from: nkmpptxz 39826f7c b | (no description set)
    into commit: qpvuntsm 64ea60be a | (no description set)

    The left side of the diff shows the contents of the parent commit. The
    right side initially shows the contents of the commit you're moving
    changes from.

    Adjust the right side until the diff shows the changes you want to move
    to the destination. If you don't make any changes, then all the changes
    from the source will be moved into the destination.
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d79ac05a29d6 c
    ○  3141e67514f6 a b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");

    // Can squash only some changes in interactive mode
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    std::fs::write(&edit_script, "reset file1")?;
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: pzvwutvl ecd03a71 c | (no description set)
    Parent commit (@-)      : nkmpptxz bb59a2d2 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  ecd03a711173 c
    ○  bb59a2d20bab b
    ○  3af17565155e a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "a"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");

    // Can squash only some changes in non-interactive mode
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    // Clear the script so we know it won't be used even without -i
    std::fs::write(&edit_script, "")?;
    let output = work_dir.run_jj(["squash", "-r", "b", "file2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: pzvwutvl 58a2bd33 c | (no description set)
    Parent commit (@-)      : nkmpptxz a643b929 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  58a2bd33e7b4 c
    ○  a643b929b068 b
    ○  cf083f1d9ccf a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "a"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    b
    [EOF]
    ");

    // If we specify only a non-existent file, then nothing changes.
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    let output = work_dir.run_jj(["squash", "-r", "b", "nonexistent"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
    [EOF]
    ");

    // We get a warning if we pass a positional argument that looks like a revset
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    let output = work_dir.run_jj(["squash", "b"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: No matching entries for paths: b
    Warning: The argument "b" is being interpreted as a fileset expression. To specify a revset, pass -r "b" instead.
    Nothing changed.
    [EOF]
    "#);

    // No warning if we pass a positional argument does not parse as a revset
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    let output = work_dir.run_jj(["squash", ".tmp"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: .tmp
    Nothing changed.
    [EOF]
    ");

    // we can use --interactive and fileset together
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    work_dir.write_file("file3", "foo\n");
    std::fs::write(&edit_script, "reset file1")?;
    let output = work_dir.run_jj(["squash", "-i", "file1", "file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: pzvwutvl 72ef6df7 c | (no description set)
    Parent commit (@-)      : nkmpptxz 4c315a39 b | (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["log", "-s"]);
    insta::assert_snapshot!(output, @"
    @  pzvwutvl test.user@example.com 2001-02-03 08:05:38 c 72ef6df7
    │  (no description set)
    │  M file1
    │  M file2
    ○  nkmpptxz test.user@example.com 2001-02-03 08:05:38 b 4c315a39
    │  (no description set)
    │  M file1
    │  M file2
    │  A file3
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:09 a 64ea60be
    │  (no description set)
    │  A file1
    │  A file2
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Error if no changes selected in interactive mode
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    std::fs::write(&edit_script, "reset file1\0reset file2")?;
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No changes selected
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e385ba6aaff1 c
    ○  39826f7cef30 b
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    Ok(())
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

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1cc10709fbc0 c
    ○  099a44efcab4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");

    let output = work_dir.run_jj(["squash", "-r", "b", "--keep-emptied"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: pzvwutvl 01e25431 c | (no description set)
    Parent commit (@-)      : nkmpptxz 0b7d47c7 b | (empty) (no description set)
    [EOF]
    ");
    // With --keep-emptied, b remains even though it is now empty.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  01e2543152dd c
    ○  0b7d47c7d9a8 b (empty)
    ○  2269fb3b12f5 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c3f05ef6d541 f
    ○  2d65a93c1fff e
    ○  1178bb01a390 d
    │ ○  2e0a6f0e80af c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // No-op if source and destination are the same
    let output = work_dir.run_jj(["squash", "--into", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Can squash from sibling, which results in the source being abandoned
    let output = work_dir.run_jj(["squash", "--from", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: xmkuslsw 4dcae49d f | (no description set)
    Parent commit (@-)      : onkkpsqq 2d65a93c e | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  4dcae49dd4b5 f
    ○  2d65a93c1fff e
    ○  1178bb01a390 d
    │ ○  7d81fc05a211 b c
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    // File `file2`, which was not changed in source, is unchanged
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");

    // Can squash from ancestor
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "@--"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: xmkuslsw fe225f90 f | (no description set)
    Parent commit (@-)      : onkkpsqq 8f1397ec e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'd'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  fe225f90bb8e f
    ○  8f1397ec3c64 e
    │ ○  2e0a6f0e80af c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a d
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied (the file contents were already
    // "f", as is typically the case when moving changes from an ancestor)
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");

    // Can squash from descendant
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "e", "--into", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: xmkuslsw d9c58db7 f | (no description set)
    Parent commit (@-)      : uruxwmqv 28db7434 d e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d9c58db7d356 f
    ○  28db743473ba d e
    │ ○  2e0a6f0e80af c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "d"]);
    insta::assert_snapshot!(output, @"
    e
    [EOF]
    ");

    // Can squash into the sources
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "e::f", "--into", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: pkstwlsy d9f582e2 (empty) (no description set)
    Parent commit (@-)      : uruxwmqv 09d81031 d e f | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d9f582e2ae5a (empty)
    ○  09d810311cd0 d e f
    │ ○  2e0a6f0e80af c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "d"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");
}

#[test]
fn test_squash_from_to_partial() -> TestResult {
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1178bb01a390 d
    │ ○  b602c842be32 c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // If we don't make any changes in the diff-editor, the whole change is moved
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: uruxwmqv caa38690 d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  caa3869056b3 d
    │ ○  7d81fc05a211 b c
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    // File `file3`, which was not changed in source, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @"
    d
    [EOF]
    ");

    // Can squash only part of the change in interactive mode
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "reset file2")?;
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: uruxwmqv d6d5cb6c d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d6d5cb6c8d88 d
    │ ○  79776eb0ce33 c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    // File `file3`, which was changed in source's parent, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @"
    d
    [EOF]
    ");

    // Can squash only part of the change from a sibling in non-interactive mode
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "")?;
    let output = work_dir.run_jj(["squash", "--from", "c", "file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: uruxwmqv 12b7c40d d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  12b7c40d7a38 d
    │ ○  79c8c3c1c41e c
    │ ○  7d81fc05a211 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    // File `file3`, which was changed in source's parent, is unchanged
    let output = work_dir.run_jj(["file", "show", "file3"]);
    insta::assert_snapshot!(output, @"
    d
    [EOF]
    ");

    // Can squash only part of the change from a descendant in non-interactive mode
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "")?;
    let output = work_dir.run_jj(["squash", "--from", "c", "--into", "b", "file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1178bb01a390 d
    │ ○  935243195478 c
    │ ○  b501bf0b84fb b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    c
    [EOF]
    ");
    // The unselected change from the source has not been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");

    // If we specify only a non-existent file, then nothing changes.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "c", "nonexistent"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
    [EOF]
    ");
    Ok(())
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
    work_dir.run_jj(["new", "visible_heads()"]).success();
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  753caba5242d f
    ○      ddd0b99531ee e
    ├─┬─╮
    │ │ ○  f6e7afb9574d b
    │ ○ │  d19fb4af3952 c
    │ ├─╯
    ○ │  fb329beb2b59 d
    ├─╯
    ○  e88768e65e67 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b", "--from=c", "--into=d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon 2b0355c6 f | (no description set)
    Parent commit (@-)      : tostqsxw 823fe121 e | (no description set)
    New conflicts appeared in 1 commits:
      oqosqzyt 8ac982e5 d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new oqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  2b0355c6b989 f
    ○    823fe1216e53 e
    ├─╮
    × │  8ac982e569d1 d
    ├─╯
    ○  e88768e65e67 a b c
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the sources have been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: qpvuntsm e88768e6 (parents of squashed revision)
    \\\\\\\        to: oqosqzyt fb329beb (squash destination)
    -a
    +d
    %%%%%%% diff from: qpvuntsm e88768e6 (parents of squashed revision)
    \\\\\\\        to: nkmpptxz f6e7afb9 (squashed revision)
    -a
    +b
    +++++++ pzvwutvl d19fb4af (squashed revision)
    c
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");

    // Squash a few commits up an down
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn 4df53997 (empty) (no description set)
    Parent commit (@-)      : tostqsxw 158ae8fa e f | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  4df539973717 (empty)
    ○    158ae8fad763 e f
    ├─╮
    ○ │  fb329beb2b59 d
    ├─╯
    ○  e88768e65e67 a b c
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The changes from the sources have been applied to the destination
    let output = work_dir.run_jj(["file", "show", "-r=e", "file"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");

    // Empty squash shouldn't crash
    let output = work_dir.run_jj(["squash", "--from=none()"]);
    insta::assert_snapshot!(output, @"
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
    work_dir.run_jj(["new", "visible_heads()"]).success();
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  db8e25ab40d6 f
    ○      a21546447dd4 e
    ├─┬─╮
    │ │ ○  39826f7cef30 b
    │ ○ │  7b9f5b9a1f25 c
    │ ├─╯
    ○ │  fa497fccf4b2 d
    ├─╯
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Partially squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b|c", "--into=d", "file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon 353aea0d f | (no description set)
    Parent commit (@-)      : tostqsxw f116f2cc e | (no description set)
    New conflicts appeared in 1 commits:
      oqosqzyt debdecf0 d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new oqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  353aea0d92ff f
    ○      f116f2cc2d71 e
    ├─┬─╮
    │ │ ○  a2848adee8db b
    │ ○ │  3008ab942232 c
    │ ├─╯
    × │  debdecf0b4f3 d
    ├─╯
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected changes have been removed from the sources
    let output = work_dir.run_jj(["file", "show", "-r=b", "file1"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=c", "file1"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    // The selected changes from the sources have been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file1"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: qpvuntsm 64ea60be (parents of squashed revision)
    \\\\\\\        to: oqosqzyt fa497fcc (squash destination)
    -a
    +d
    %%%%%%% diff from: qpvuntsm 64ea60be (parents of squashed revision)
    \\\\\\\        to: selected changes for squash (from nkmpptxz 39826f7c)
    -a
    +b
    +++++++ selected changes for squash (from pzvwutvl 7b9f5b9a)
    c
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");
    // The unselected change from the sources have not been applied to the
    // destination
    let output = work_dir.run_jj(["file", "show", "-r=d", "file2"]);
    insta::assert_snapshot!(output, @"
    d
    [EOF]
    ");

    // Partially squash a few commits up an down
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e", "file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kpqxywon 3cd8135b f | (no description set)
    Parent commit (@-)      : tostqsxw 2d22fc90 e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  3cd8135b107f f
    ○      2d22fc90cdbc e
    ├─┬─╮
    │ │ ○  0db6e33d88e4 b
    │ ○ │  e167a3c015bb c
    │ ├─╯
    ○ │  fa497fccf4b2 d
    ├─╯
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The selected changes have been removed from the sources
    let output = work_dir.run_jj(["file", "show", "-r=b", "file1"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=c", "file1"]);
    insta::assert_snapshot!(output, @"
    a
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "-r=f", "file1"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");
    // The selected changes from the sources have been applied to the destination
    let output = work_dir.run_jj(["file", "show", "-r=e", "file1"]);
    insta::assert_snapshot!(output, @"
    f
    [EOF]
    ");
    // The unselected changes from the sources have not been applied
    let output = work_dir.run_jj(["file", "show", "-r=d", "file2"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  110e903b7a41 d
    │ ○  f1ef0762d926 c
    ├─╯
    │ ○  892a3c5f3db8 b
    ├─╯
    ○  93d495c46d89 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Source commits that didn't match the paths are not rewritten
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: rzvwutvl e0415921 d
    Parent commit (@-)      : qpvuntsm 93d495c4 a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e04159212ff1 d
    │ ○  f1ef0762d926 c
    ├─╯
    ○  93d495c46d89 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj([
        "evolog",
        "-T",
        r#"separate(" ", commit.commit_id().short(), commit.description())"#,
    ]);
    insta::assert_snapshot!(output, @"
    @    e04159212ff1 d
    ├─╮
    │ ○  892a3c5f3db8 b
    │ ○  99ac3ad52a50 b
    ○  110e903b7a41 d
    ○  a8fb5d054651 d
    [EOF]
    ");

    // If no source commits match the paths, then the whole operation is a no-op
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  110e903b7a41 d
    │ ○  f1ef0762d926 c
    ├─╯
    │ ○  892a3c5f3db8 b
    ├─╯
    ○  93d495c46d89 a
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
fn test_squash_description() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(&edit_script, r#"fail"#)?;

    // If both descriptions are empty, the resulting description is empty
    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid1 = work_dir.current_operation_id();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // If the destination's description is empty and the source's description is
    // non-empty, the resulting description is from the source
    work_dir.run_jj(["op", "restore", &setup_opid1]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    source
    [EOF]
    ");

    // If the destination description is non-empty and the source's description is
    // empty, the resulting description is from the destination
    work_dir.run_jj(["op", "restore", &setup_opid1]).success();
    work_dir
        .run_jj(["describe", "@-", "-m", "destination"])
        .success();
    let setup_opid2 = work_dir.current_operation_id();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    destination
    [EOF]
    ");

    // An explicit description on the command-line overrides this
    work_dir.run_jj(["op", "restore", &setup_opid2]).success();
    work_dir.run_jj(["squash", "-m", "custom"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    custom
    [EOF]
    ");

    // If both descriptions were non-empty, we get asked for a combined description
    work_dir.run_jj(["op", "restore", &setup_opid2]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    let setup_opid3 = work_dir.current_operation_id();
    std::fs::write(&edit_script, "dump editor0")?;
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    destination

    source
    [EOF]
    ");
    let editor0 = std::fs::read_to_string(test_env.env_root().join("editor0"))?;
    insta::assert_snapshot!(
        editor0, @r#"
    JJ: Enter a description for the combined commit.
    JJ: Description from the destination commit:
    destination

    JJ: Description from source commit:
    source

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // An explicit description on the command-line overrides prevents launching an
    // editor
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir.run_jj(["squash", "-m", "custom"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    custom
    [EOF]
    ");

    // An explicit description on the command-line includes the trailers when
    // templates.commit_trailers is configured
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: " ++ committer.email()'"#,
            "-m",
            "custom",
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    custom

    CC: test.user@example.com
    [EOF]
    ");

    // If the source's *content* doesn't become empty, then the source remains and
    // both descriptions are unchanged
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir.run_jj(["squash", "file1"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    destination
    [EOF]
    ");
    insta::assert_snapshot!(get_description(&work_dir, "@"), @"
    source
    [EOF]
    ");

    // A combined description should only contain the trailers from the
    // commit_trailers template that were not in the squashed commits
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-m", "source\n\nfoo: bar"])
        .success();
    std::fs::write(&edit_script, "dump editor0")?;
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: alice@example.com\nfoo: bar"'"#,
        ])
        .success();
    let editor0 = std::fs::read_to_string(test_env.env_root().join("editor0"))?;
    insta::assert_snapshot!(
        editor0, @r#"
    JJ: Enter a description for the combined commit.
    JJ: Description from the destination commit:
    destination

    JJ: Description from source commit:
    source

    foo: bar

    JJ: Trailers not found in the squashed commits:
    CC: alice@example.com

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // No complaints when commits have identical trailers, order irrelevant
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-m", "source\n\nfoo: bar\nbaz: quux"])
        .success();
    std::fs::write(&edit_script, "dump editor0").unwrap();
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"baz: quux\nfoo: bar"'"#,
        ])
        .success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r#"
    JJ: Enter a description for the combined commit.
    JJ: Description from the destination commit:
    destination

    JJ: Description from source commit:
    source

    foo: bar
    baz: quux

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // If the destination description is non-empty and the source's description is
    // empty, the resulting description is from the destination, with additional
    // trailers if defined in the commit_trailers template
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir.run_jj(["describe", "-m", ""]).success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  80ec542962c1
    ○  e650dfcd7312 destination
    ◆  000000000000
    [EOF]
    ");
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: alice@example.com"'"#,
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    destination

    CC: alice@example.com
    [EOF]
    ");

    // If a single description is non-empty, the resulting description is
    // from the destination, with additional trailers if defined in the
    // commit_trailers template
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-r", "@-", "-m", ""])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  4b20b2b6c32e source
    ○  a01e1865957e
    ◆  000000000000
    [EOF]
    ");
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: alice@example.com"'"#,
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    source

    CC: alice@example.com
    [EOF]
    ");

    // squashing messages with empty descriptions shouldn't add any trailer
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-r", "..", "-m", ""])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  f7af8ee727c1
    ○  2560c9e5ef72
    ◆  000000000000
    [EOF]
    ");
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: bob@example.com"'"#,
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // squashing messages with --use-destination-message on a commit with an
    // empty description shouldn't add any trailer
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-r", "@-", "-m", ""])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  b3f1ffd7d003 source
    ○  ecf32ca3d742
    ◆  000000000000
    [EOF]
    ");
    work_dir
        .run_jj([
            "squash",
            "--use-destination-message",
            "--config",
            r#"templates.commit_trailers='"CC: bob@example.com"'"#,
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // squashing with an empty message on the command line shouldn't add
    // any trailer
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  da682fda7323 source
    ○  e650dfcd7312 destination
    ◆  000000000000
    [EOF]
    ");
    work_dir
        .run_jj([
            "squash",
            "--message",
            "",
            "--config",
            r#"templates.commit_trailers='"CC: bob@example.com"'"#,
        ])
        .success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // Invalid trailer content
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir.write_file("data.txt", b"\xff\n");
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  57c9368a52f6 source
    ○  e650dfcd7312 destination
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj([
        "squash",
        "--config",
        r#"templates.commit_trailers='indent("Content: ", diff.git())'"#,
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Trailers should be valid utf-8
    [EOF]
    [exit status: 1]
    ");
    Ok(())
}

#[test]
fn test_squash_description_editor_avoids_unc() -> TestResult {
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

    std::fs::write(edit_script, "dump-path path")?;
    work_dir.run_jj(["squash"]).success();

    let edited_path = PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path"))?);
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
    Ok(())
}

#[test]
fn test_squash_empty() {
    let mut test_env = TestEnvironment::default();
    test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "parent"]).success();

    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz db7ad962 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 771da191 (empty) parent
    [EOF]
    ");
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    parent
    [EOF]
    ");

    work_dir.run_jj(["describe", "-m", "child"]).success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
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
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  cf388db088f7 c
    ○  e412ddda5587 b
    ○  b86e28cd6862 a
    ◆  000000000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squash the current revision using the short name for the option.
    work_dir.run_jj(["squash", "-u"]).success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  70c0f74e4486
    ○  44c1701e4ef8 b
    ○  b86e28cd6862 a
    ◆  000000000000
    [EOF]
    ");

    // Undo and squash again, but this time squash both "b" and "c" into "a".
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "squash",
            "--use-destination-message",
            "--from=subject(b)::",
            "--into=subject(a)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @"
    @  e5a16e0e6a46
    ○  6e47254e0803 a
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_squash_option_exclusion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["describe", "-m=b"]).success();

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "--message=123",
        "--use-destination-message",
    ]), @"
    ------- stderr -------
    error: the argument '--message <MESSAGE>' cannot be used with '--use-destination-message'

    Usage: jj squash --message <MESSAGE> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "-r@",
        "--into=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--revision <REVSET>' cannot be used with '--into <REVSET>'

    Usage: jj squash --revision <REVSET> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "-r@",
        "--onto=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--revision <REVSET>' cannot be used with '--onto <REVSETS>'

    Usage: jj squash --revision <REVSET> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "-r@",
        "--after=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--revision <REVSET>' cannot be used with '--insert-after <REVSETS>'

    Usage: jj squash --revision <REVSET> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "-r@",
        "--before=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--revision <REVSET>' cannot be used with '--insert-before <REVSETS>'

    Usage: jj squash --revision <REVSET> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "--onto=@",
        "--into=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--onto <REVSETS>' cannot be used with '--into <REVSET>'

    Usage: jj squash --onto <REVSETS> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "--after=@",
        "--into=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--insert-after <REVSETS>' cannot be used with '--into <REVSET>'

    Usage: jj squash --insert-after <REVSETS> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    insta::assert_snapshot!(work_dir.run_jj([
        "squash",
        "--before=@-",
        "--into=@-"
    ]), @"
    ------- stderr -------
    error: the argument '--insert-before <REVSETS>' cannot be used with '--into <REVSET>'

    Usage: jj squash --insert-before <REVSETS> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_squash_to_new_commit() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "file1\n");
    work_dir.run_jj(["commit", "-m", "file1"]).success();
    work_dir.write_file("file2", "file2\n");
    work_dir.run_jj(["commit", "-m", "file2"]).success();
    work_dir.write_file("file3", "file3\n");
    work_dir.run_jj(["commit", "-m", "file3"]).success();
    work_dir.write_file("file4", "file4\n");
    work_dir.run_jj(["commit", "-m", "file4"]).success();
    work_dir
        .run_jj(["bookmark", "create", "bm4", "-r", "@-"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "bm3", "-r", "bm4-"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "bm2", "-r", "bm3-"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "bm1", "-r", "bm2-"])
        .success();
    let setup_opid = work_dir.current_operation_id();

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr bm4 file4
    │  A file4
    ○  kkmpptxzrspx bm3 file3
    │  A file3
    ○  rlvkpnrzqnoo bm2 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit before a commit
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-before",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit kpqxywon f87660dd file 3&4
    Rebased 2 descendant commits
    Working copy  (@) now at: vzqnnsmr 35736bd0 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 4c273fe2 bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  vzqnnsmrxxkw
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  kpqxywonksrl file 3&4
    │  A file3
    │  A file4
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit after a commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-after",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit lylxulpl 43cf43eb file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: rsllmpnm 42df41a1 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 3a4b2e8a bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  rsllmpnmslon
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  lylxulplsnyw file 3&4
    │  A file3
    │  A file4
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit onto another
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--onto",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit uyznsvlq fd4b7c85 file 3&4
    Working copy  (@) now at: uuqyqztp c2d532a4 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 27974c44 bm2 bm3 bm4 | file2
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  uuqyqztpptml
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    │ ○  uyznsvlquzzm file 3&4
    ├─╯  A file3
    │    A file4
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit after the source commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-after",
        "zsuskulnrvyr",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit nmzmmopx c7558c47 file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: pwyqokvy 525d9441 (empty) (no description set)
    Parent commit (@-)      : nmzmmopx c7558c47 file 3&4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  pwyqokvyvunr
    ○  nmzmmopxokps file 3&4
    │  A file3
    │  A file4
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit before the source commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-before",
        "zsuskulnrvyr",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit nlrtlrxv c5286eac bm4 | file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: plymsszl faf348e7 (empty) (no description set)
    Parent commit (@-)      : nlrtlrxv c5286eac bm4 | file 3&4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  plymsszllttm
    ○  nlrtlrxvuusk bm4 file 3&4
    │  A file3
    │  A file4
    ○  rlvkpnrzqnoo bm2 bm3 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // double destination with a commit that will disappear
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--onto",
        "rlvkpnrzqnoo",
        "--onto",
        "kkmpptxzrspx",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit uuuvxpvw 2ad3f239 file 3&4
    Working copy  (@) now at: nmpuuozl 95ef3451 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 27974c44 bm2 bm3 bm4 | file2
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  nmpuuozlonyz
    │ ○  uuuvxpvwspwr file 3&4
    ├─╯  A file3
    │    A file4
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // creating a new commit should open the editor to write the commit message
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(edit_script, ["dump editor1", "write\nfile 3&4"].join("\0"))?;
    let output = work_dir.run_jj([
        "squash",
        "-f",
        "kkmpptxzrspx::zsuskulnrvyr",
        "--insert-before",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit pkstwlsy 41510a56 file 3&4
    Rebased 3 descendant commits
    Working copy  (@) now at: mzvwutvl 3d11750f (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 8ed73d06 bm2 bm3 bm4 | file2
    [EOF]
    ");

    let editor1 = std::fs::read_to_string(test_env.env_root().join("editor1"))?;
    insta::assert_snapshot!(
        editor1, @r#"
    JJ: Enter a description for the combined commit.

    JJ: Description from source commit:
    file3

    JJ: Description from source commit:
    file4

    JJ: Change ID: pkstwlsy
    JJ: This commit contains the following changes:
    JJ:     A file3
    JJ:     A file4
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  pkstwlsyuyku file 3&4
    │  A file3
    │  A file4
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-r", "pkstwlsyuyku"]);
    insta::assert_snapshot!(output, @"
    ○    pkstwlsy test.user@example.com 2001-02-03 08:05:35 41510a56
    ├─╮  file 3&4
    │ │  -- operation 3d6e647dccb7 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    │ ○  zsuskuln/0 test.user@example.com 2001-02-03 08:05:35 a5bc761f (hidden)
    │ │  file4
    │ │  -- operation 3d6e647dccb7 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    │ ○  zsuskuln/4 test.user@example.com 2001-02-03 08:05:11 38778966 (hidden)
    │ │  file4
    │ │  -- operation 9ca0f4771551 commit 89a30a7539466ed176c1ef122a020fd9cb15848e
    │ ○  zsuskuln/5 test.user@example.com 2001-02-03 08:05:11 89a30a75 (hidden)
    │ │  (no description set)
    │ │  -- operation 26b29f385618 snapshot working copy
    │ ○  zsuskuln/6 test.user@example.com 2001-02-03 08:05:10 bbf04d26 (hidden)
    │    (empty) (no description set)
    │    -- operation fc5403fc3984 commit c23c424826221bc4fdee9487926595324e50ee95
    ○  kkmpptxz/0 test.user@example.com 2001-02-03 08:05:35 ce3b0a58 (hidden)
    │  file3
    │  -- operation 3d6e647dccb7 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    ○  kkmpptxz/3 test.user@example.com 2001-02-03 08:05:10 0d254956 (hidden)
    │  file3
    │  -- operation fc5403fc3984 commit c23c424826221bc4fdee9487926595324e50ee95
    ○  kkmpptxz/4 test.user@example.com 2001-02-03 08:05:10 c23c4248 (hidden)
    │  (no description set)
    │  -- operation c5043d80534d snapshot working copy
    ○  kkmpptxz/5 test.user@example.com 2001-02-03 08:05:09 c1272e87 (hidden)
       (empty) (no description set)
       -- operation 4eb3252f7fd7 commit cb58ff1c6f1af92f827661e7275941ceb4d910c5
    [EOF]
    ");

    // creating a new commit with --use-destination-message shouldn't open the
    // editor
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-f",
        "kkmpptxzrspx::zsuskulnrvyr",
        "--insert-before",
        "qpvuntsmwlqt",
        "--use-destination-message",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit soqnvnyz bf2d3b81 (no description set)
    Rebased 3 descendant commits
    Working copy  (@) now at: mzvwutvl 9ebad59e (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 0c770834 bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  soqnvnyzoxuuA file3
    │  A file4
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // squashing 0 sources should create an empty commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-f",
        "none()",
        "--insert-before",
        "qpvuntsmwlqt",
        "--use-destination-message",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit nsrwusvy c2183685 (empty) (no description set)
    Rebased 5 descendant commits
    Working copy  (@) now at: mzvwutvl cb96ecf9 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 97edce13 bm4 | file4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr bm4 file4
    │  A file4
    ○  kkmpptxzrspx bm3 file3
    │  A file3
    ○  rlvkpnrzqnoo bm2 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  nsrwusvynpoy
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-r", "nsrwusvynpoy"]);
    insta::assert_snapshot!(output, @"
    ○  nsrwusvy test.user@example.com 2001-02-03 08:05:42 c2183685
       (empty) (no description set)
       -- operation 29b5af212226 squash 0 commits
    [EOF]
    ");

    // squashing empty changes should create an empty commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-f",
        "kkmpptxzrspx::zsuskulnrvyr",
        "--insert-before",
        "qpvuntsmwlqt",
        "--use-destination-message",
        "no file",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: no file
    Created new commit wtlqussy 7eff41c8 (empty) (no description set)
    Rebased 5 descendant commits
    Working copy  (@) now at: mzvwutvl cf8a98c4 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 461e8bb9 bm4 | file4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr bm4 file4
    │  A file4
    ○  kkmpptxzrspx bm3 file3
    │  A file3
    ○  rlvkpnrzqnoo bm2 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  wtlqussytxur
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-r", "wtlqussytxur"]);
    insta::assert_snapshot!(output, @"
    ○  wtlqussy test.user@example.com 2001-02-03 08:05:46 7eff41c8
       (empty) (no description set)
       -- operation f5c12bfd0cbc squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    [EOF]
    ");

    // squashing from an empty commit should produce an empty commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["new", "--no-edit", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit ozrrkvty 2d2b113d (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj([
        "squash",
        "-f",
        "ozrrkvty",
        "--onto",
        "root()",
        "--use-destination-message",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit pyoswmwk 991d0644 (empty) (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  mzvwutvlkqwt
    ○  zsuskulnrvyr bm4 file4
    │  A file4
    ○  kkmpptxzrspx bm3 file3
    │  A file3
    ○  rlvkpnrzqnoo bm2 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    │ ○  pyoswmwkkqyt
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-r", "pyoswmwkkqyt"]);
    insta::assert_snapshot!(output, @"
    ○  pyoswmwk test.user@example.com 2001-02-03 08:05:50 991d0644
    │  (empty) (no description set)
    │  -- operation 65e6a8ee34b6 squash commit 2d2b113dbb76af23695f11dca69cc8b1be571b60
    ○  ozrrkvty/0 test.user@example.com 2001-02-03 08:05:50 2d2b113d (hidden)
       (empty) (no description set)
       -- operation 2177f5bc2539 new empty commit
    [EOF]
    ");

    // --before and --after together
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-after",
        "root()",
        "--insert-before",
        "rlvkpnrzqnoo",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit vkywoywq 96673776 file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: rsmzzqvr e37e3e4a (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 8044909c bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  rsmzzqvrnpvn
    ○    rlvkpnrzqnoo bm2 bm3 bm4 file2
    ├─╮  A file2
    │ ○  vkywoywqymtr file 3&4
    │ │  A file3
    │ │  A file4
    ○ │  qpvuntsmwlqt bm1 file1
    ├─╯  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // squash-moves can use the current commit too
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir.run_jj(["edit", "bm3"]).success();
    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    ○  zsuskulnrvyr bm4 file4
    │  A file4
    @  kkmpptxzrspx bm3 file3
    │  A file3
    ○  rlvkpnrzqnoo bm2 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["squash", "--before", "rlvkpnrzqnoo"]), @"
    ------- stderr -------
    Created new commit wxzmtyol bc0392dc file3
    Rebased 2 descendant commits
    Working copy  (@) now at: musouqkq 1841cc81 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 264a43af bm2 bm3 | file2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_with_summary(&work_dir), @"
    @  musouqkqsmll
    │ ○  zsuskulnrvyr bm4 file4
    ├─╯  A file4
    ○  rlvkpnrzqnoo bm2 bm3 file2
    │  A file2
    ○  wxzmtyollrwl file3
    │  A file3
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_squash_with_editor_combine_messages() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "destination"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new", "-m", "source"]).success();
    work_dir.write_file("file1", "b\n");

    // Both source and destination have descriptions, so editor will open to combine
    // them; the --editor was superfluous.
    std::fs::write(
        &edit_script,
        ["dump editor", "write\nfinal description from editor"].join("\0"),
    )?;
    work_dir.run_jj(["squash", "--editor"]).success();

    // Verify editor was opened once with combined messages
    let editor = std::fs::read_to_string(test_env.env_root().join("editor"))?;
    insta::assert_snapshot!(
        editor, @r#"
    JJ: Enter a description for the combined commit.
    JJ: Description from the destination commit:
    destination

    JJ: Description from source commit:
    source

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    final description from editor
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_squash_with_editor_and_message_args() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "destination"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");

    std::fs::write(&edit_script, "dump editor")?;
    work_dir
        .run_jj(["squash", "-m", "message from command line", "--editor"])
        .success();

    // Verify editor was opened with the message from command line
    let editor = std::fs::read_to_string(test_env.env_root().join("editor"))?;
    insta::assert_snapshot!(
        editor, @r#"
    message from command line

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    message from command line
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_squash_with_editor_and_empty_message() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "destination"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");

    // Use --editor with an empty message. The trailers should be added because
    // the editor will be opened.
    std::fs::write(&edit_script, "dump editor")?;
    work_dir
        .run_jj([
            "squash",
            "-m",
            "",
            "--editor",
            "--config",
            r#"templates.commit_trailers='"Trailer: value"'"#,
        ])
        .success();

    // Verify editor was opened with trailers added to the empty message
    let editor = std::fs::read_to_string(test_env.env_root().join("editor"))?;
    insta::assert_snapshot!(
        editor, @r#"


    Trailer: value

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"
    Trailer: value
    [EOF]
    ");
    Ok(())
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

#[must_use]
fn get_log_with_summary(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), local_bookmarks, description)"#;
    work_dir.run_jj(["log", "-T", template, "--summary"])
}
