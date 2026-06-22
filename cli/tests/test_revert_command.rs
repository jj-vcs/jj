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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  fa26c55c301b d
    ○  3ba70189111d c
    ○  b2f482be4c08 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-ra", "-s"]);
    insta::assert_snapshot!(output, @"
    A a
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Reverting without a location is an error
    let output = work_dir.run_jj(["revert", "-ra"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    Usage: jj revert --revision <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Revert the commit with `--onto`
    let output = work_dir.run_jj(["revert", "-ra", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      wqnwkozp b0012bfd Revert "a"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  b0012bfd21de Revert "a"
    │
    │  This reverts commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53.
    @  fa26c55c301b d
    ○  3ba70189111d c
    ○  b2f482be4c08 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r@+"]);
    insta::assert_snapshot!(output, @"
    D a
    [EOF]
    ");

    // Revert the new reverted commit
    let output = work_dir.run_jj(["revert", "-r@+", "-d@+"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      nkmrtpmo 4deb57e3 Revert "Revert "a""
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  4deb57e3940a Revert "Revert "a""
    │
    │  This reverts commit b0012bfd21de6b73af1ca81b9e1a3dc6b605ab37.
    ○  b0012bfd21de Revert "a"
    │
    │  This reverts commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53.
    @  fa26c55c301b d
    ○  3ba70189111d c
    ○  b2f482be4c08 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r@++"]);
    insta::assert_snapshot!(output, @"
    A a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-after`
    let output = work_dir.run_jj(["revert", "-ra", "-Ab"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      nmzmmopx fc5e66c4 Revert "a"
    Rebased 2 descendant commits
    Working copy  (@) now at: truxwmqv bb9e400f d | (empty) d
    Parent commit (@-)      : ooyxmykx 5b8a9403 c | (empty) c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  bb9e400fb73e d
    ○  5b8a9403cdc5 c
    ○  fc5e66c47827 Revert "a"
    │
    │  This reverts commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53.
    ○  b2f482be4c08 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-rb+"]);
    insta::assert_snapshot!(output, @"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-before`
    let output = work_dir.run_jj(["revert", "-ra", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      pzsxstzt 8c22ca4f Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: truxwmqv 309be317 d | (empty) d
    Parent commit (@-)      : pzsxstzt 8c22ca4f Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  309be317775a d
    ○  8c22ca4f224e Revert "a"
    │
    │  This reverts commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53.
    ○  3ba70189111d c
    ○  b2f482be4c08 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-rd-"]);
    insta::assert_snapshot!(output, @"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert the commit with `--insert-after` and `--insert-before`
    let output = work_dir.run_jj(["revert", "-ra", "-Aa", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      oupztwtk d3f03fa4 Revert "a"
    Rebased 1 descendant commits
    Working copy  (@) now at: truxwmqv 9347ae05 d | (empty) d
    Parent commit (@-)      : ooyxmykx 3ba70189 c | (empty) c
    Parent commit (@-)      : oupztwtk d3f03fa4 Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @    9347ae05d47e d
    ├─╮
    │ ○  d3f03fa45cfa Revert "a"
    │ │
    │ │  This reverts commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53.
    ○ │  3ba70189111d c
    ○ │  b2f482be4c08 b
    ├─╯
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r", "a+ & d-"]);
    insta::assert_snapshot!(output, @"
    D a
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Revert nothing
    let output = work_dir.run_jj(["revert", "-r", "none()", "-d", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to revert.
    [EOF]
    ");
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  cf24eb81f036 e
    ○  73a51527a9fe d
    ○  38264d21e3d7 c
    ○  0b942411d277 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    ");

    // Revert multiple commits
    let output = work_dir.run_jj(["revert", "-rb", "-rc", "-re", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 3 commits as follows:
      wqnwkozp a6081eb6 Revert "e"
      mouksmqu 51cda3bf Revert "c"
      tqvpomtp a468f0ad Revert "b"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  a468f0ad978e Revert "b"
    │
    │  This reverts commit 0b942411d277054caabbaf1e9dc62f368d73bb66.
    ○  51cda3bfcc54 Revert "c"
    │
    │  This reverts commit 38264d21e3d79a70e31740bd1cd21201c4d6281f.
    ○  a6081eb6a3ab Revert "e"
    │
    │  This reverts commit cf24eb81f036ebed82802cd38b4ad9189208fa6a.
    @  cf24eb81f036 e
    ○  73a51527a9fe d
    ○  38264d21e3d7 c
    ○  0b942411d277 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    // View the output of each reverted commit
    let output = work_dir.run_jj(["show", "@+"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: a6081eb6a3abffb261f2f8eb6a700e34068a0ed8
    Change ID: wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "e"

        This reverts commit cf24eb81f036ebed82802cd38b4ad9189208fa6a.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 51cda3bfcc541a752f9001aac1ef15d33dc95f8d
    Change ID: mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "c"

        This reverts commit 38264d21e3d79a70e31740bd1cd21201c4d6281f.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: a468f0ad978eee5b2ae0c73f23c82c9480c0bce7
    Change ID: tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "b"

        This reverts commit 0b942411d277054caabbaf1e9dc62f368d73bb66.

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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @"
    A a
    [EOF]
    ");

    // Verify that message of reverted commit follows the template
    let output = work_dir.run_jj(["revert", "-r@", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      royxmykx 2fbadb7a Revert commit a1afb5834d8e "a"
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  2fbadb7a5a37 Revert commit a1afb5834d8e "a"
    @  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
}

#[test]
fn test_revert_with_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("a", "a\n")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("a", "a\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["b"], &[("a", "a\nb\nc\n")]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  3def7730ce49 c
    ○  0b942411d277 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    ");

    // Create a conflict by reverting B onto C
    let output = work_dir.run_jj(["revert", "-r=b", "--onto=c"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      yostqsxw db4fbaf3 (conflict) Revert "b"
    New conflicts appeared in 1 commits:
      yostqsxw db4fbaf3 (conflict) Revert "b"
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new yostqsxw
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ×  db4fbaf3663c Revert "b"
    │
    │  This reverts commit 0b942411d277054caabbaf1e9dc62f368d73bb66.
    @  3def7730ce49 c
    ○  0b942411d277 b
    ○  a1afb5834d8e a
    ◆  000000000000
    [EOF]
    "#);
    // Reverted commit should contain conflict markers
    let output = work_dir.run_jj(["file", "show", "-r=@+", "a"]);
    insta::assert_snapshot!(output, @r#"
    a
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: psuskuln 0b942411 "b" (reverted revision)
    \\\\\\\        to: ooyxmykx 3def7730 "c" (revert destination)
     b
    +c
    +++++++ ylvkpnrz a1afb583 "a" (parents of reverted revision)
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    "#);
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
