// Copyright 2023 The Jujutsu Authors
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

#[test]
fn test_unsign() {
    let test_env = TestEnvironment::default();

    test_env.add_config(
        r#"
[ui]
show-cryptographic-signatures = true

[signing]
behavior = "keep"
backend = "test"
"#,
    );

    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "one"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "two"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "three"]);

    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@"]);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:11 7aa7dcdf [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:11 0413d103 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:11 c8768375 [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 b90f5370 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    ");

    // Unsign single commit
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["unsign", "-r", "@"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned 1 commit
      zsuskuln hidden 7a23b9bd (empty) (no description set)
    Working copy now at: zsuskuln 7a23b9bd (empty) (no description set)
    Parent commit      : kkmpptxz 0413d103 (empty) three
    ");

    // Unsign multiple commits
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["unsign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned 3 commits:
      qpvuntsm hidden afde6e4b (empty) one
      rlvkpnrz hidden d49204af (empty) two
      kkmpptxz hidden ea6d9b6d (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln 4029f2fc (empty) (no description set)
    Parent commit      : kkmpptxz ea6d9b6d (empty) three
    ");

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:14 4029f2fc
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:14 ea6d9b6d
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:14 d49204af
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:14 afde6e4b
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    ");
}

#[test]
fn test_warn_about_unsigning_commits_not_authored_by_me() {
    let test_env = TestEnvironment::default();

    test_env.add_config(
        r#"
[ui]
show-cryptographic-signatures = true

[signing]
behavior = "keep"
backend = "test"
"#,
    );

    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "one"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "two"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "three"]);

    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "desc",
            "--author",
            "Someone Else <someone@else.com>",
            "--no-edit",
            "..@",
        ],
    );
    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@"]);

    // Unsign single commit not authored by me
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["unsign", "-r", "@"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned 1 commit not authored by you
      zsuskuln hidden 6e279998 (empty) (no description set)
    Working copy now at: zsuskuln 6e279998 (empty) (no description set)
    Parent commit      : kkmpptxz 8b42bc9c (empty) three
    ");

    // Unsign multiple commits not authored by me
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["unsign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned 3 commits not authored by you:
      qpvuntsm hidden 247f09e0 (empty) one
      rlvkpnrz hidden 34791b11 (empty) two
      kkmpptxz hidden d40f7bed (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln afe0804d (empty) (no description set)
    Parent commit      : kkmpptxz d40f7bed (empty) three
    ");
}
