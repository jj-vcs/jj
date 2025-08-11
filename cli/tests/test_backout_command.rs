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
    ○  b8c8e82a19bc Back out "a"
    │
    │  This backs out commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
    ○  812d823be175 Back out "Back out "a""
    │
    │  This backs out commit b8c8e82a19bcf6217e065d1aff9a5f0ba807b565.
    @  b8c8e82a19bc Back out "a"
    │
    │  This backs out commit 7d980be7a1d499e4d316ab4c01242885032f7eaf.
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
    ○  036128aa5f6e Back out "b"
    │
    │  This backs out commit f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.
    ○  156974608bed Back out "c"
    │
    │  This backs out commit 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.
    ○  3f72017241e0 Back out "e"
    │
    │  This backs out commit 51a01d6d8cc48a296cb87f8383b34ade3c050363.
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
    Commit ID: 3f72017241e0a32ab837ae929061cdc05ff04f5b
    Change ID: wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "e"

        This backs out commit 51a01d6d8cc48a296cb87f8383b34ade3c050363.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 156974608bed539fb98d89c1f6995d962123cdbd
    Change ID: mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "c"

        This backs out commit 05e1f540476f8c4207ff44febbe2ce6e6696dc4b.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = work_dir.run_jj(["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 036128aa5f6eb3770cc8284c0dbe198b2c9a5f62
    Change ID: tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Back out "b"

        This backs out commit f93a910dbdf0f841e6cf2bc0ab0ba4c336d6f436.

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
