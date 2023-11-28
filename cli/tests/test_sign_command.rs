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
fn test_sign() {
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

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:10 7acb64be
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 8bdfe4fb
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 b0e11728
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 876f4b7e
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    ");

    // Sign single commit
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@"]);
    insta::assert_snapshot!(stderr, @r"
    Signed 1 commit
      zsuskuln hidden 772b634d (empty) (no description set)
    Working copy now at: zsuskuln 772b634d (empty) (no description set)
    Parent commit      : kkmpptxz 8bdfe4fb (empty) three
    ");

    // Sign multiple commits
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Signed 3 commits:
      qpvuntsm hidden e3ef5444 (empty) one
      rlvkpnrz hidden 892ec951 (empty) two
      kkmpptxz hidden 755a6764 (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln 3fd47ee5 (empty) (no description set)
    Parent commit      : kkmpptxz 755a6764 (empty) three
    ");

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:13 3fd47ee5 [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:13 755a6764 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:13 892ec951 [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 e3ef5444 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    ");

    // Don't resign commits, which are already signed by me.
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @"Nothing changed.");
}

#[test]
fn test_warn_about_signing_commits_not_authored_by_me() {
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
            "..@-",
        ],
    );
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Warning: Signed 3 commits not authored by you
      qpvuntsm hidden 82f99921 (empty) one
      rlvkpnrz hidden 715131ae (empty) two
      kkmpptxz hidden 60618621 (empty) three
    Signed 3 commits:
      qpvuntsm hidden 82f99921 (empty) one
      rlvkpnrz hidden 715131ae (empty) two
      kkmpptxz hidden 60618621 (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln 5a1d05b3 (empty) (no description set)
    Parent commit      : kkmpptxz 60618621 (empty) three
    ");
}

#[test]
fn test_keep_signatures_in_rebased_descendants() {
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

    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-"]);
    insta::assert_snapshot!(stderr, @r"
    Signed 1 commit
      kkmpptxz hidden 81608447 (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln 72143fcd (empty) (no description set)
    Parent commit      : kkmpptxz 81608447 (empty) three
    ");

    // sign "two", triggering automatic rebase
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@--"]);
    insta::assert_snapshot!(stderr, @r"
    Signed 1 commit
      rlvkpnrz hidden a7c1dfbb (empty) two
    Rebased 2 descendant commits
    Working copy now at: zsuskuln e9cbed15 (empty) (no description set)
    Parent commit      : kkmpptxz 627b6673 (empty) three
    ");

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 e9cbed15
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:12 627b6673 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:12 a7c1dfbb [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 876f4b7e
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    ");
}

#[test]
#[should_panic]
fn test_abort_with_error_if_no_signing_backend_is_configured() {
    todo!()
}
