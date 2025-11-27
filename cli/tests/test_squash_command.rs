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
    @  22be6c4e01da c
    ○  75591b1896b4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squashes the working copy into the parent by default
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 2cf02eb8 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 9422c8d6 b c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  2cf02eb82d82 (empty)
    ○  9422c8d6f294 b c
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    c
    [EOF]
    ");

    // Can squash a given commit into its parent
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 441a7a3a c | (no description set)
    Parent commit (@-)      : qpvuntsm 105931bf a b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  441a7a3a17b0 c
    ○  105931bfedad a b
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    e05d4caaf6ce e (empty)
    ├─╮
    │ ○  9bb7863cfc78 d
    ○ │  22be6c4e01da c
    ├─╯
    ○  75591b1896b4 b
    ○  e6086990958c a
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
    Working copy  (@) now at: xlzxqlsl 91a81249 (empty) (no description set)
    Parent commit (@-)      : nmzmmopx 9155baf5 e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  91a81249954f (empty)
    ○    9155baf5ced1 e
    ├─╮
    │ ○  9bb7863cfc78 d
    ○ │  22be6c4e01da c
    ├─╯
    ○  75591b1896b4 b
    ○  e6086990958c a
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
    @  87059ac9657b c
    ○  f2c9709f39e9 b
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");

    let start_op_id = work_dir.current_operation_id();

    // If we don't make any changes in the diff-editor, the whole change is moved
    // into the parent
    std::fs::write(&edit_script, "dump JJ-INSTRUCTIONS instrs").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 34484d82 c | (no description set)
    Parent commit (@-)      : qpvuntsm 3141e675 a b | (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are moving changes from: kkmpptxz f2c9709f b | (no description set)
    into commit: qpvuntsm 64ea60be a | (no description set)

    The left side of the diff shows the contents of the parent commit. The
    right side initially shows the contents of the commit you're moving
    changes from.

    Adjust the right side until the diff shows the changes you want to move
    to the destination. If you don't make any changes, then all the changes
    from the source will be moved into the destination.
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  34484d825f47 c
    ○  3141e67514f6 a b
    ◆  000000000000 (empty)
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    b
    [EOF]
    ");

    // Can squash only some changes in interactive mode
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    std::fs::write(&edit_script, "reset file1").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "-i"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 37e1a0ef c | (no description set)
    Parent commit (@-)      : kkmpptxz b41e789d b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  37e1a0ef57ff c
    ○  b41e789df71c b
    ○  3af17565155e a
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
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    // Clear the script so we know it won't be used even without -i
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "-r", "b", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 72ff256c c | (no description set)
    Parent commit (@-)      : kkmpptxz dd056a92 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  72ff256cd290 c
    ○  dd056a925eb3 b
    ○  cf083f1d9ccf a
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
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    let output = work_dir.run_jj(["squash", "-r", "b", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
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

    // we can use --interactive and fileset together
    work_dir.run_jj(["op", "restore", &start_op_id]).success();
    work_dir.write_file("file3", "foo\n");
    std::fs::write(&edit_script, "reset file1").unwrap();
    let output = work_dir.run_jj(["squash", "-i", "file1", "file3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 69c58f86 c | (no description set)
    Parent commit (@-)      : kkmpptxz 0f38c564 b | (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["log", "-s"]);
    insta::assert_snapshot!(output, @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:36 c 69c58f86
    │  (no description set)
    │  M file1
    │  M file2
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:36 b 0f38c564
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
    @  22be6c4e01da c
    ○  75591b1896b4 b
    ○  e6086990958c a
    ◆  000000000000 (empty)
    [EOF]
    ");

    let output = work_dir.run_jj(["squash", "-r", "b", "--keep-emptied"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: mzvwutvl 093590e0 c | (no description set)
    Parent commit (@-)      : kkmpptxz 357946cf b | (empty) (no description set)
    [EOF]
    ");
    // With --keep-emptied, b remains even though it is now empty.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  093590e044bd c
    ○  357946cf85df b (empty)
    ○  2269fb3b12f5 a
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
    @  0fac1124d1ad f
    ○  4ebe104a0e4e e
    ○  dc71a460d5d6 d
    │ ○  ee0b260ffc44 c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // No-op if source and destination are the same
    let output = work_dir.run_jj(["squash", "--into", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Can squash from sibling, which results in the source being abandoned
    let output = work_dir.run_jj(["squash", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 941ab024 f | (no description set)
    Parent commit (@-)      : znkkpsqq 4ebe104a e | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  941ab024b3f8 f
    ○  4ebe104a0e4e e
    ○  dc71a460d5d6 d
    │ ○  e31bf988d7c9 b c
    ├─╯
    ○  e3e04beaf7d3 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw c102d2c4 f | (no description set)
    Parent commit (@-)      : znkkpsqq beb7c033 e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'd'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c102d2c4e165 f
    ○  beb7c0338f7c e
    │ ○  ee0b260ffc44 c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a d
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "e", "--into", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kmkuslsw 1bc21d4e f | (no description set)
    Parent commit (@-)      : vruxwmqv 8b6b080a d e | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  1bc21d4e92d6 f
    ○  8b6b080ab587 d e
    │ ○  ee0b260ffc44 c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "d"]);
    insta::assert_snapshot!(output, @r"
    e
    [EOF]
    ");

    // Can squash into the sources
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "e::f", "--into", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: pkstwlsy 76baa567 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 415e4069 d e f | (no description set)
    [EOF]
    ");
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  76baa567ed0a (empty)
    ○  415e40694e88 d e f
    │ ○  ee0b260ffc44 c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    // The change from the source has been applied
    let output = work_dir.run_jj(["file", "show", "file2", "-r", "d"]);
    insta::assert_snapshot!(output, @r"
    f
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
    @  dc71a460d5d6 d
    │ ○  499d601f6046 c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // If we don't make any changes in the diff-editor, the whole change is moved
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 85589465 d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  85589465a5f7 d
    │ ○  e31bf988d7c9 b c
    ├─╯
    ○  e3e04beaf7d3 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = work_dir.run_jj(["squash", "-i", "--from", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 62bd5cd9 d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  62bd5cd9f413 d
    │ ○  2748f30463ed c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "--from", "c", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 76bf6139 d | (no description set)
    Parent commit (@-)      : qpvuntsm e3e04bea a | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  76bf613928cf d
    │ ○  9d4418d4828e c
    │ ○  e31bf988d7c9 b
    ├─╯
    ○  e3e04beaf7d3 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["squash", "--from", "c", "--into", "b", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  dc71a460d5d6 d
    │ ○  f964ce4bca71 c
    │ ○  e12c895adba6 b
    ├─╯
    ○  e3e04beaf7d3 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from", "c", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  65e53f39b4d6 f
    ○      7dc592781647 e
    ├─┬─╮
    │ │ ○  fed4d1a2e491 b
    │ ○ │  d7e94ec7e73e c
    │ ├─╯
    ○ │  8acbb71558d5 d
    ├─╯
    ○  e88768e65e67 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b", "--from=c", "--into=d"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon 0b695306 f | (no description set)
    Parent commit (@-)      : yostqsxw ff064d52 e | (no description set)
    New conflicts appeared in 1 commits:
      yqosqzyt 61130da4 d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new yqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    "###);
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  0b6953066ee0 f
    ○    ff064d529578 e
    ├─╮
    × │  61130da4e714 d
    ├─╯
    ○  e88768e65e67 a b c
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn ec32238b (empty) (no description set)
    Parent commit (@-)      : yostqsxw 5298eef6 e f | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  ec32238b2be5 (empty)
    ○    5298eef6bca5 e f
    ├─╮
    ○ │  8acbb71558d5 d
    ├─╯
    ○  e88768e65e67 a b c
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  4558bd852475 f
    ○      e2db96b2e57a e
    ├─┬─╮
    │ │ ○  f2c9709f39e9 b
    │ ○ │  aa908686a197 c
    │ ├─╯
    ○ │  f6812ff8db35 d
    ├─╯
    ○  64ea60be8d77 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Partially squash a few commits sideways
    let output = work_dir.run_jj(["squash", "--from=b|c", "--into=d", "file1"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kpqxywon a724910c f | (no description set)
    Parent commit (@-)      : yostqsxw 1bc405e1 e | (no description set)
    New conflicts appeared in 1 commits:
      yqosqzyt 7ddfe685 d | (conflict) (no description set)
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new yqosqzyt
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    "###);
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  a724910cd361 f
    ○      1bc405e12b68 e
    ├─┬─╮
    │ │ ○  e9db15b956c4 b
    │ ○ │  83cbe51db94d c
    │ ├─╯
    × │  7ddfe6857387 d
    ├─╯
    ○  64ea60be8d77 a
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=b|c|f", "--into=e", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kpqxywon b5a40c15 f | (no description set)
    Parent commit (@-)      : yostqsxw 5dea187c e | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b5a40c154611 f
    ○      5dea187c414d e
    ├─┬─╮
    │ │ ○  8b9afc05ca07 b
    │ ○ │  5630471a8fd5 c
    │ ├─╯
    ○ │  f6812ff8db35 d
    ├─╯
    ○  64ea60be8d77 a
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
    @  fdb92bc249a0 d
    │ ○  0dc8cb72859d c
    ├─╯
    │ ○  b1a17f79a1a5 b
    ├─╯
    ○  93d495c46d89 a
    ◆  000000000000 (empty)
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Source commits that didn't match the paths are not rewritten
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 6dfc239e d
    Parent commit (@-)      : qpvuntsm 93d495c4 a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  6dfc239e2ba3 d
    │ ○  0dc8cb72859d c
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
    insta::assert_snapshot!(output, @r"
    @    6dfc239e2ba3 d
    ├─╮
    │ ○  b1a17f79a1a5 b
    │ ○  d8b7d57239ca b
    ○  fdb92bc249a0 d
    ○  af709ccc1ca9 d
    [EOF]
    ");

    // If no source commits match the paths, then the whole operation is a no-op
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["squash", "--from=@-+ ~ @", "--into=@", "-m=d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  fdb92bc249a0 d
    │ ○  0dc8cb72859d c
    ├─╯
    │ ○  b1a17f79a1a5 b
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
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid1 = work_dir.current_operation_id();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @"");

    // If the destination's description is empty and the source's description is
    // non-empty, the resulting description is from the source
    work_dir.run_jj(["op", "restore", &setup_opid1]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
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
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    destination
    [EOF]
    ");

    // An explicit description on the command-line overrides this
    work_dir.run_jj(["op", "restore", &setup_opid2]).success();
    work_dir.run_jj(["squash", "-m", "custom"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    custom
    [EOF]
    ");

    // If both descriptions were non-empty, we get asked for a combined description
    work_dir.run_jj(["op", "restore", &setup_opid2]).success();
    work_dir.run_jj(["describe", "-m", "source"]).success();
    let setup_opid3 = work_dir.current_operation_id();
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
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
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
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    custom

    CC: test.user@example.com
    [EOF]
    ");

    // If the source's *content* doesn't become empty, then the source remains and
    // both descriptions are unchanged
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir.run_jj(["squash", "file1"]).success();
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    destination
    [EOF]
    ");
    insta::assert_snapshot!(get_description(&work_dir, "@"), @r"
    source
    [EOF]
    ");

    // A combined description should only contain the trailers from the
    // commit_trailers template that were not in the squashed commits
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-m", "source\n\nfoo: bar"])
        .success();
    std::fs::write(&edit_script, "dump editor0").unwrap();
    work_dir
        .run_jj([
            "squash",
            "--config",
            r#"templates.commit_trailers='"CC: alice@example.com\nfoo: bar"'"#,
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

    JJ: Trailers not found in the squashed commits:
    CC: alice@example.com

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
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  97f34efed913
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
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
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
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  40d544a674fa source
    ○  0cd53b79bf29
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
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    source

    CC: alice@example.com
    [EOF]
    ");

    // squashing messages with empty descriptions shouldn't add any trailer
    work_dir.run_jj(["op", "restore", &setup_opid3]).success();
    work_dir
        .run_jj(["describe", "-r", "..", "-m", ""])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  40803d8a1ce7
    ○  dd5a6e210d71
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
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  7ce5b3a58427 source
    ○  b9442a4ce005
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
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
    @  2a79b102bf46 source
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
    Working copy  (@) now at: kkmpptxz db7ad962 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 771da191 (empty) parent
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
    @  cf388db088f7 c
    ○  e412ddda5587 b
    ○  b86e28cd6862 a
    ◆  000000000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Squash the current revision using the short name for the option.
    work_dir.run_jj(["squash", "-u"]).success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
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
            "--from=subject(glob:b)::",
            "--into=subject(glob:a)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output_with_description(&work_dir), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
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
    ]), @r"
    ------- stderr -------
    error: the argument '--insert-before <REVSETS>' cannot be used with '--into <REVSET>'

    Usage: jj squash --insert-before <REVSETS> [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_squash_to_new_commit() {
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

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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

    // insert the commit before the source commit
    let output = work_dir.run_jj([
        "squash",
        "-m",
        "file 3&4",
        "-f",
        "kkmpptxzrspx::",
        "--insert-before",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit kpqxywon f87660dd file 3&4
    Rebased 2 descendant commits
    Working copy  (@) now at: vzqnnsmr 35736bd0 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 4c273fe2 bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit lylxulpl 43cf43eb file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: rsllmpnm 42df41a1 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 3a4b2e8a bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit uyznsvlq fd4b7c85 file 3&4
    Working copy  (@) now at: uuqyqztp c2d532a4 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 27974c44 bm2 bm3 bm4 | file2
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit nmzmmopx c7558c47 file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: pwyqokvy 525d9441 (empty) (no description set)
    Parent commit (@-)      : nmzmmopx c7558c47 file 3&4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit nlrtlrxv c5286eac bm4 | file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: plymsszl faf348e7 (empty) (no description set)
    Parent commit (@-)      : nlrtlrxv c5286eac bm4 | file 3&4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit uuuvxpvw 2ad3f239 file 3&4
    Working copy  (@) now at: nmpuuozl 95ef3451 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 27974c44 bm2 bm3 bm4 | file2
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    std::fs::write(edit_script, ["dump editor1", "write\nfile 3&4"].join("\0")).unwrap();
    let output = work_dir.run_jj([
        "squash",
        "-f",
        "kkmpptxzrspx::zsuskulnrvyr",
        "--insert-before",
        "qpvuntsmwlqt",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit pkstwlsy 41510a56 file 3&4
    Rebased 3 descendant commits
    Working copy  (@) now at: mzvwutvl 3d11750f (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 8ed73d06 bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
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

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ○    pkstwlsy test.user@example.com 2001-02-03 08:05:35 41510a56
    ├─╮  file 3&4
    │ │  -- operation ad1aa66374f6 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:35 a5bc761f
    │ │  file4
    │ │  -- operation ad1aa66374f6 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:11 38778966
    │ │  file4
    │ │  -- operation 83489d186f66 commit 89a30a7539466ed176c1ef122a020fd9cb15848e
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:11 89a30a75
    │ │  (no description set)
    │ │  -- operation e23fd04aab50 snapshot working copy
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:10 bbf04d26
    │    (empty) (no description set)
    │    -- operation 19d57874b952 commit c23c424826221bc4fdee9487926595324e50ee95
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:35 ce3b0a58
    │  file3
    │  -- operation ad1aa66374f6 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 0d254956
    │  file3
    │  -- operation 19d57874b952 commit c23c424826221bc4fdee9487926595324e50ee95
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 c23c4248
    │  (no description set)
    │  -- operation d19ad3734aa6 snapshot working copy
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 c1272e87
       (empty) (no description set)
       -- operation fdee458ae5f2 commit cb58ff1c6f1af92f827661e7275941ceb4d910c5
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit soqnvnyz bf2d3b81 (no description set)
    Rebased 3 descendant commits
    Working copy  (@) now at: mzvwutvl 9ebad59e (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 0c770834 bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  mzvwutvlkqwt
    ○  rlvkpnrzqnoo bm2 bm3 bm4 file2
    │  A file2
    ○  qpvuntsmwlqt bm1 file1
    │  A file1
    ○  soqnvnyzoxuu
    │  A file3
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit nsrwusvy c2183685 (empty) (no description set)
    Rebased 5 descendant commits
    Working copy  (@) now at: mzvwutvl cb96ecf9 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 97edce13 bm4 | file4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ○  nsrwusvy test.user@example.com 2001-02-03 08:05:42 c2183685
       (empty) (no description set)
       -- operation a656ab530912 squash 0 commits
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: no file
    Created new commit wtlqussy 7eff41c8 (empty) (no description set)
    Rebased 5 descendant commits
    Working copy  (@) now at: mzvwutvl cf8a98c4 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 461e8bb9 bm4 | file4
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ○  wtlqussy test.user@example.com 2001-02-03 08:05:46 7eff41c8
       (empty) (no description set)
       -- operation 1661e2cea988 squash commit 0d254956d33ed5bb11d93eb795c5e514aadc81b5 and 1 more
    [EOF]
    ");

    // squashing from an empty commit should produce an empty commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["new", "--no-edit", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit szrrkvty f5e47d01 (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj([
        "squash",
        "-f",
        "szrrkvty",
        "--onto",
        "root()",
        "--use-destination-message",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit pyoswmwk 991d0644 (empty) (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ○  pyoswmwk test.user@example.com 2001-02-03 08:05:50 991d0644
    │  (empty) (no description set)
    │  -- operation 60d056329b43 squash commit f5e47d019271a392eb7f92a6b2e9f8cf41d97049
    ○  szrrkvty hidden test.user@example.com 2001-02-03 08:05:50 f5e47d01
       (empty) (no description set)
       -- operation 31e408225067 new empty commit
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit vkywoywq 96673776 file 3&4
    Rebased 1 descendant commits
    Working copy  (@) now at: rsmzzqvr e37e3e4a (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 8044909c bm2 bm3 bm4 | file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
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
}

#[test]
fn test_squash_with_editor_combine_messages() {
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
    )
    .unwrap();
    work_dir.run_jj(["squash", "--editor"]).success();

    // Verify editor was opened once with combined messages
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
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

    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    final description from editor
    [EOF]
    ");
}

#[test]
fn test_squash_with_editor_and_message_args() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "destination"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "b\n");

    std::fs::write(&edit_script, "dump editor").unwrap();
    work_dir
        .run_jj(["squash", "-m", "message from command line", "--editor"])
        .success();

    // Verify editor was opened with the message from command line
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    message from command line

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"
    message from command line
    [EOF]
    ");
}

#[test]
fn test_squash_with_editor_and_empty_message() {
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
    std::fs::write(&edit_script, "dump editor").unwrap();
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
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"

    Trailer: value

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_description(&work_dir, "@-"), @r"

    Trailer: value
    [EOF]
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

#[must_use]
fn get_log_with_summary(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), local_bookmarks, description)"#;
    work_dir.run_jj(["log", "-T", template, "--summary"])
}
