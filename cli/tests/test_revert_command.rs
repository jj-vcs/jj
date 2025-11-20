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
      wqnwkozp 6e6f5aa8 Revert "a"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  6e6f5aa87473 Revert "a"
    │
    │  This reverts revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
      nkmrtpmo eab29308 Revert "Revert "a""
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  eab29308be1b Revert "Revert "a""
    │
    │  This reverts revision 6e6f5aa87473237e035f29771cb7bf2972cfef50.
    ○  6e6f5aa87473 Revert "a"
    │
    │  This reverts revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
      nmzmmopx 3cd5dfd6 Revert "a"
    Rebased 2 descendant commits
    Working copy  (@) now at: vruxwmqv c0f39512 d | (empty) d
    Parent commit (@-)      : royxmykx e00519a5 c | (empty) c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  c0f39512d531 d
    ○  e00519a59d32 c
    ○  3cd5dfd6277b Revert "a"
    │
    │  This reverts revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
      pzsxstzt 33a582b6 Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: vruxwmqv fbe75c92 d | (empty) d
    Parent commit (@-)      : pzsxstzt 33a582b6 Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  fbe75c92a250 d
    ○  33a582b633fd Revert "a"
    │
    │  This reverts revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
      oupztwtk ee35f73e Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: vruxwmqv e9edc92f d | (empty) d
    Parent commit (@-)      : royxmykx 96ff4227 c | (empty) c
    Parent commit (@-)      : oupztwtk ee35f73e Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @    e9edc92fe33e d
    ├─╮
    │ ○  ee35f73eff19 Revert "a"
    │ │
    │ │  This reverts revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
      wqnwkozp a9fa2bfe Revert "e"
      mouksmqu 03388660 Revert "c"
      tqvpomtp 5755f0c7 Revert "b"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  5755f0c7e371 Revert "b"
    │
    │  This reverts revision f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.
    ○  033886608b53 Revert "c"
    │
    │  This reverts revision 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.
    ○  a9fa2bfea793 Revert "e"
    │
    │  This reverts revision 51a01d6d8cc48a296cb87f8383b34ade3c050363.
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
    Revision ID: a9fa2bfea79396b6a61c7ecea5b1177e53b34ec2
    Change ID  : wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "e"

        This reverts revision 51a01d6d8cc48a296cb87f8383b34ade3c050363.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Revision ID: 033886608b533770592406e893c9fe5e60b853fc
    Change ID  : mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "c"

        This reverts revision 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Revision ID: 5755f0c7e371bde6f80dd2bbfd57db033951e832
    Change ID  : tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "b"

        This reverts revision f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.

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
