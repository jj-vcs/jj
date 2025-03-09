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

use std::path::Path;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;

fn create_commit(
    test_env: &TestEnvironment,
    repo_path: &Path,
    name: &str,
    parents: &[&str],
    files: &[(&str, &str)],
) {
    let parents = match parents {
        [] => &["root()"],
        parents => parents,
    };
    test_env
        .run_jj_with(|cmd| {
            cmd.current_dir(repo_path)
                .args(["new", "-m", name])
                .args(parents)
        })
        .success();
    for (name, contents) in files {
        std::fs::write(repo_path.join(name), contents).unwrap();
    }
    test_env
        .run_jj_in(repo_path, ["bookmark", "create", "-r@", name])
        .success();
}

#[test]
fn test_revert() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[], &[("a", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["a"], &[]);
    create_commit(&test_env, &repo_path, "c", &["b"], &[]);
    create_commit(&test_env, &repo_path, "d", &["c"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    @  cf3ddd838fd1 d
    ○  14e954305d4b c
    ○  aa48676d4a49 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-ra", "-s"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");
    let setup_opid = test_env.current_operation_id(&repo_path);

    // Reverting without a location is an error
    let output = test_env.run_jj_in(&repo_path, ["revert", "-ra"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    Usage: jj revert --revisions <REVSETS> <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    "#);

    // Revert the commit with `--destination`
    let output = test_env.run_jj_in(&repo_path, ["revert", "-ra", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    ○  f844336ef2a5 Revert "a"
    │
    │  This reverts commit 2443ea76b0b1c531326908326aab7020abab8e6c.
    @  cf3ddd838fd1 d
    ○  14e954305d4b c
    ○  aa48676d4a49 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-r@+"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");

    // Revert the new reverted commit
    let output = test_env.run_jj_in(&repo_path, ["revert", "-r@+", "-d@+"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    ○  57fb197b953f Revert "Revert "a""
    │
    │  This reverts commit f844336ef2a54f4499a5efefa1a9549451276316.
    ○  f844336ef2a5 Revert "a"
    │
    │  This reverts commit 2443ea76b0b1c531326908326aab7020abab8e6c.
    @  cf3ddd838fd1 d
    ○  14e954305d4b c
    ○  aa48676d4a49 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-r@++"]);
    insta::assert_snapshot!(output, @r#"
    A a
    [EOF]
    "#);
    test_env
        .run_jj_in(&repo_path, ["op", "restore", &setup_opid])
        .success();

    // Revert the commit with `--insert-after`
    let output = test_env.run_jj_in(&repo_path, ["revert", "-ra", "-Ab"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    Rebased 2 descendant commits
    Working copy now at: vruxwmqv 37ad0315 d | (empty) d
    Parent commit      : royxmykx ca80e93f c | (empty) c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    @  37ad03151aa7 d
    ○  ca80e93fdef9 c
    ○  ff67400f3e1f Revert "a"
    │
    │  This reverts commit 2443ea76b0b1c531326908326aab7020abab8e6c.
    ○  aa48676d4a49 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-rb+"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["op", "restore", &setup_opid])
        .success();

    // Revert the commit with `--insert-before`
    let output = test_env.run_jj_in(&repo_path, ["revert", "-ra", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    Rebased 1 descendant commits
    Working copy now at: vruxwmqv 3a8a8069 d | (empty) d
    Parent commit      : xlzxqlsl 0b91fe42 Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    @  3a8a80692ac3 d
    ○  0b91fe42616d Revert "a"
    │
    │  This reverts commit 2443ea76b0b1c531326908326aab7020abab8e6c.
    ○  14e954305d4b c
    ○  aa48676d4a49 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-rd-"]);
    insta::assert_snapshot!(output, @r"
    D a
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["op", "restore", &setup_opid])
        .success();

    // Revert the commit with `--insert-after` and `--insert-before`
    let output = test_env.run_jj_in(&repo_path, ["revert", "-ra", "-Aa", "-Bd"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    Rebased 1 descendant commits
    Working copy now at: vruxwmqv 3489c981 d | (empty) d
    Parent commit      : royxmykx 14e95430 c | (empty) c
    Parent commit      : pkstwlsy 29508aef Revert "a"
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    @    3489c98177aa d
    ├─╮
    │ ○  29508aefc220 Revert "a"
    │ │
    │ │  This reverts commit 2443ea76b0b1c531326908326aab7020abab8e6c.
    ○ │  14e954305d4b c
    ○ │  aa48676d4a49 b
    ├─╯
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-r", "a+ & d-"]);
    insta::assert_snapshot!(output, @r#"
    D a
    [EOF]
    "#);
    test_env
        .run_jj_in(&repo_path, ["op", "restore", &setup_opid])
        .success();
}

#[test]
fn test_revert_multiple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "a", &[], &[("a", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["a"], &[("a", "a\nb\n")]);
    create_commit(
        &test_env,
        &repo_path,
        "c",
        &["b"],
        &[("a", "a\nb\n"), ("b", "b\n")],
    );
    create_commit(&test_env, &repo_path, "d", &["c"], &[]);
    create_commit(&test_env, &repo_path, "e", &["d"], &[("a", "a\nb\nc\n")]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  208f8612074a e
    ○  ceeec03be46b d
    ○  413337bbd11f c
    ○  46cc97af6802 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    ");

    // Revert multiple commits
    let output = test_env.run_jj_in(&repo_path, ["revert", "-rb", "-rc", "-re", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 3 commits
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    ○  fb78f44decd2 Revert "b"
    │
    │  This reverts commit 46cc97af6802301d8db381386e8485ff3ff24ae6.
    ○  f5d9e8b20bd1 Revert "c"
    │
    │  This reverts commit 413337bbd11f7a6636c010d9e196acf801d8df2f.
    ○  05f5fa79161a Revert "e"
    │
    │  This reverts commit 208f8612074af4c219d06568a8e1f04f2e80dc25.
    @  208f8612074a e
    ○  ceeec03be46b d
    ○  413337bbd11f c
    ○  46cc97af6802 b
    ○  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
    // View the output of each reverted commit
    let output = test_env.run_jj_in(&repo_path, ["show", "@+"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: 05f5fa79161a41b9ed3dc11e156d18de8abc7907
    Change ID: wqnwkozpkustnxypnnntnykwrqrkrpvv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "e"

        This reverts commit 208f8612074af4c219d06568a8e1f04f2e80dc25.

    Modified regular file a:
       1    1: a
       2    2: b
       3     : c
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["show", "@++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: f5d9e8b20bd1c5c7485e8baab4b287759c717a52
    Change ID: mouksmquosnpvwqrpsvvxtxpywpnxlss
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "c"

        This reverts commit 413337bbd11f7a6636c010d9e196acf801d8df2f.

    Removed regular file b:
       1     : b
    [EOF]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["show", "@+++"]);
    insta::assert_snapshot!(output, @r#"
    Commit ID: fb78f44decd2082bc2a6940624744c90b20635a8
    Change ID: tqvpomtpwrqsylrpsxknultrymmqxmxv
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:19)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:19)

        Revert "b"

        This reverts commit 46cc97af6802301d8db381386e8485ff3ff24ae6.

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
    let repo_path = test_env.env_root().join("repo");
    create_commit(&test_env, &repo_path, "a", &[], &[("a", "a\n")]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    A a
    [EOF]
    ");

    // Verify that message of reverted commit follows the template
    let output = test_env.run_jj_in(&repo_path, ["revert", "-r@", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    ○  1db880a5204e Revert commit 2443ea76b0b1 "a"
    @  2443ea76b0b1 a
    ◆  000000000000
    [EOF]
    "#);
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    test_env.run_jj_in(cwd, ["log", "-T", template])
}
