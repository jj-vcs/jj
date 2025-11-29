// Copyright 2025 The Jujutsu Authors
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

#[test]
fn test_revert() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("a", "a\n")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[]);
    create_commit_with_files(&work_dir, "c", &["b"], &[]);
    create_commit_with_files(&work_dir, "d", &["c"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  98fb6151f954 d
    ○  96ff42270bbc c
    ○  58aaf278bf58 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-ra", "-s"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Reverting without a location is an error
    let output = work_dir.run_jj(["revert", "-ra"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    Usage: jj revert --revisions <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Revert the commit with `--onto`
    let output = work_dir.run_jj(["revert", "-ra", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      wqnwkozp 64910788 Revert "a"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  64910788f8a5 Revert "a"
    │
    │  This reverts commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    @  98fb6151f954 d
    ○  96ff42270bbc c
    ○  58aaf278bf58 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r@+"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");

    // Revert the new reverted commit
    let output = work_dir.run_jj(["revert", "-r@+", "-d@+"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      nkmrtpmo 90d12316 Revert "Revert "a""
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  90d123162199 Revert "Revert "a""
    │
    │  This reverts commit 64910788f8a5d322739e1e38ef35f7d06ea4b38d.
    ○  64910788f8a5 Revert "a"
    │
    │  This reverts commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    @  98fb6151f954 d
    ○  96ff42270bbc c
    ○  58aaf278bf58 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r@++"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-after`
    let output = work_dir.run_jj(["revert", "-ra", "-Ab"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      nmzmmopx 9e7b8585 Revert "a"
    Rebased 2 descendant commits
    Working copy  (@) now at: vruxwmqv b1885396 d | (empty) d
    Parent commit (@-)      : royxmykx efc4bd83 c | (empty) c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  b18853966f79 d
    ○  efc4bd83159f c
    ○  9e7b85853718 Revert "a"
    │
    │  This reverts commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    ○  58aaf278bf58 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-rb+"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-before`
    let output = work_dir.run_jj(["revert", "-ra", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      pzsxstzt d51ea564 Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: vruxwmqv 5c5d60a6 d | (empty) d
    Parent commit (@-)      : pzsxstzt d51ea564 Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  5c5d60a69afd d
    ○  d51ea56444ce Revert "a"
    │
    │  This reverts commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    ○  96ff42270bbc c
    ○  58aaf278bf58 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-rd-"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-after` and `--insert-before`
    let output = work_dir.run_jj(["revert", "-ra", "-Aa", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      oupztwtk d311a8f0 Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: vruxwmqv 5b97d572 d | (empty) d
    Parent commit (@-)      : royxmykx 96ff4227 c | (empty) c
    Parent commit (@-)      : oupztwtk d311a8f0 Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @    5b97d572e457 d
    ├─╮
    │ ○  d311a8f0c13f Revert "a"
    │ │
    │ │  This reverts commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    ○ │  96ff42270bbc c
    ○ │  58aaf278bf58 b
    ├─╯
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r", "a+ & d-"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert nothing
    let output = work_dir.run_jj(["revert", "-r", "none()", "-d", "@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    No revisions to revert.
    [EOF]
    "#);
}

#[test]
fn test_revert_multiple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("a", "a\n")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("a", "a\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["b"], &[("a", "a\nb\n"), ("b", "b\n")]);
    create_commit_with_files(&work_dir, "d", &["c"], &[]);
    create_commit_with_files(&work_dir, "e", &["d"], &[("a", "a\nb\nc\n")]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  51a01d6d8cc4 e
    ○  4b9d123d3b33 d
    ○  05e1f540476f c
    ○  f93a910dbdf0 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    ");

    // Revert multiple commits
    let output = work_dir.run_jj(["revert", "-rb", "-rc", "-re", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 3 commits as follows:
      wqnwkozp 4329cf72 Revert "e"
      mouksmqu 092f722e Revert "c"
      tqvpomtp c90eef02 Revert "b"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  c90eef022369 Revert "b"
    │
    │  This reverts commit f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.
    ○  092f722e521f Revert "c"
    │
    │  This reverts commit 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.
    ○  4329cf7230d7 Revert "e"
    │
    │  This reverts commit 51a01d6d8cc48a296cb87f8383b34ade3c050363.
    @  51a01d6d8cc4 e
    ○  4b9d123d3b33 d
    ○  05e1f540476f c
    ○  f93a910dbdf0 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    // View the output of each reverted commit
    let output = work_dir.run_jj(["show", "@+"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 4329cf7230d7b8229e9c88087cfa2f8aa13a1317
    Change ID: wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "e"

        This reverts commit 51a01d6d8cc48a296cb87f8383b34ade3c050363.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 092f722e521fe49fde5a3830568fe1c51b8f2f5f
    Change ID: mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "c"

        This reverts commit 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: c90eef022369d43d5eae8303101b5889b4b73963
    Change ID: tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "b"

        This reverts commit f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.

    Modified regular file a:
       1    1: a
       2     : b
    [EOF]
    "#);
}

#[test]
fn test_revert_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(
        r#"
        [templates]
        revert_description = '''
        separate(" ",
          "Revert commit",
          commit_id.short(),
          '"' ++ description.first_line() ++ '"',
        )
        '''
        "#,
    );
    let work_dir = test_env.work_dir("repo");
    create_commit_with_files(&work_dir, "a", &[], &[("a", "a\n")]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");

    // Verify that message of reverted commit follows the template
    let output = work_dir.run_jj(["revert", "-r@", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      royxmykx 6bfb98a3 Revert commit 7d980be7a1d4 "a"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  6bfb98a33f58 Revert commit 7d980be7a1d4 "a"
    @  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
