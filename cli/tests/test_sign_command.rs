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

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "one"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "two"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "three"])
        .success();

    let output = test_env.run_jj_in(&repo_path, ["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:10 7acb64be
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 8bdfe4fb
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 b0e11728
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 876f4b7e
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["sign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 4 commits:
      qpvuntsm 8174ec98 (empty) one
      rlvkpnrz 6500b275 (empty) two
      kkmpptxz bcfaa4c3 (empty) three
      zsuskuln 4947c6dd (empty) (no description set)
    Working copy now at: zsuskuln 4947c6dd (empty) (no description set)
    Parent commit      : kkmpptxz bcfaa4c3 (empty) three
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 4947c6dd [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:12 bcfaa4c3 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:12 6500b275 [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 8174ec98 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Commits already always signed, even if they are already signed by me.
    // https://github.com/jj-vcs/jj/issues/5786
    let output = test_env.run_jj_in(&repo_path, ["sign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 4 commits:
      qpvuntsm dabebf30 (empty) one
      rlvkpnrz 2085a464 (empty) two
      kkmpptxz 227f5e15 (empty) three
      zsuskuln 15d1b128 (empty) (no description set)
    Working copy now at: zsuskuln 15d1b128 (empty) (no description set)
    Parent commit      : kkmpptxz 227f5e15 (empty) three
    [EOF]
    ");
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

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "one"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "two"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "three"])
        .success();

    test_env
        .run_jj_in(
            &repo_path,
            &[
                "desc",
                "--author",
                "Someone Else <someone@else.com>",
                "--no-edit",
                "..@-",
            ],
        )
        .success();
    let output = test_env.run_jj_in(&repo_path, ["sign", "-r", "..@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 3 commits:
      qpvuntsm 254d1a64 (empty) one
      rlvkpnrz caa78d30 (empty) two
      kkmpptxz c2bc0eb0 (empty) three
    Warning: 3 of these commits are not authored by you
    Rebased 1 descendant commits
    Working copy now at: zsuskuln ede04d15 (empty) (no description set)
    Parent commit      : kkmpptxz c2bc0eb0 (empty) three
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 ede04d15
    │  (empty) (no description set)
    ○  kkmpptxz someone@else.com 2001-02-03 08:05:12 c2bc0eb0 [✓︎]
    │  (empty) three
    ○  rlvkpnrz someone@else.com 2001-02-03 08:05:12 caa78d30 [✓︎]
    │  (empty) two
    ○  qpvuntsm someone@else.com 2001-02-03 08:05:12 254d1a64 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
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
behavior = "drop"
backend = "test"
"#,
    );

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "A"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "B"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["desc", "-m", "C"])
        .success();

    let output = test_env.run_jj_in(&repo_path, ["sign", "-r", "@|@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 2 commits:
      qpvuntsm 034b975d (empty) A
      kkmpptxz 29dc7928 (empty) C
    Rebased 1 descendant commits
    Working copy now at: kkmpptxz 29dc7928 (empty) C
    Parent commit      : rlvkpnrz 014c011c (empty) B
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:11 29dc7928 [✓︎]
    │  (empty) C
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:11 014c011c
    │  (empty) B
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 034b975d [✓︎]
    │  (empty) A
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_abort_with_error_if_no_signing_backend_is_configured() {
    let test_env = TestEnvironment::default();

    test_env.add_config(
        r#"
[signing]
backend = "none"
"#,
    );

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["sign", "-r", "@-"]);

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No signing backend configured
    Hint: For configuring a signing backend, see https://jj-vcs.github.io/jj/latest/config/#commit-signing
    [EOF]
    [exit status: 1]
    ");
}
