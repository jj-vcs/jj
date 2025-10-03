// Copyright 2024 The Jujutsu Authors
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
fn test_backout() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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

    // Backout the commit
    let output = work_dir.run_jj(["backout", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj backout` is deprecated; use `jj revert` instead
    Warning: `jj backout` will be removed in a future version, and this will be a hard error
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  2aec8d60fd26 Back out "a"
    │
    │  This backs out revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    @  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r", "@+"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");

    // Backout the new backed-out commit
    work_dir.run_jj(["edit", "@+"]).success();
    let output = work_dir.run_jj(["backout", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj backout` is deprecated; use `jj revert` instead
    Warning: `jj backout` will be removed in a future version, and this will be a hard error
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  e4a50561283b Back out "Back out "a""
    │
    │  This backs out revision 2aec8d60fd2632c0cfdd40fd55a13466500af6b2.
    @  2aec8d60fd26 Back out "a"
    │
    │  This backs out revision 7d980be7a1d499e4d316ab4c01242885032f7eaf.
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    let output = work_dir.run_jj(["diff", "-s", "-r", "@+"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");
}

#[test]
fn test_backout_multiple() {
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

    // Backout multiple commits
    let output = work_dir.run_jj(["backout", "-r", "b", "-r", "c", "-r", "e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj backout` is deprecated; use `jj revert` instead
    Warning: `jj backout` will be removed in a future version, and this will be a hard error
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    ○  b1a212f89ab2 Back out "b"
    │
    │  This backs out revision f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.
    ○  024899bba66a Back out "c"
    │
    │  This backs out revision 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.
    ○  8ab5f1ef5092 Back out "e"
    │
    │  This backs out revision 51a01d6d8cc48a296cb87f8383b34ade3c050363.
    @  51a01d6d8cc4 e
    ○  4b9d123d3b33 d
    ○  05e1f540476f c
    ○  f93a910dbdf0 b
    ○  7d980be7a1d4 a
    ◆  000000000000
    [EOF]
    "#);
    // View the output of each backed out commit
    let output = work_dir.run_jj(["show", "@+"]);
    insta::assert_snapshot!(output, @r#"
    Revision ID: 8ab5f1ef5092e346da1763441060d1cfd5ac9660
    Change ID  : wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "e"

        This backs out revision 51a01d6d8cc48a296cb87f8383b34ade3c050363.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Revision ID: 024899bba66ac12a43cc695d00d6fc238c2384ea
    Change ID  : mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "c"

        This backs out revision 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Revision ID: b1a212f89ab2f2ab0d427643cff523834914b995
    Change ID  : tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author     : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer  : Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "b"

        This backs out revision f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.

    Modified regular file a:
       1    1: a
       2     : b
    [EOF]
    "#);
}

#[test]
fn test_backout_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(
        r#"
        [templates]
        backout_description = '''
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

    // Verify that message of backed out commit follows the template
    let output = work_dir.run_jj(["backout", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj backout` is deprecated; use `jj revert` instead
    Warning: `jj backout` will be removed in a future version, and this will be a hard error
    [EOF]
    ");
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
