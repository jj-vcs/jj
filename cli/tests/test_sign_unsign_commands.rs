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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();
    work_dir.run_jj(["commit", "-m", "two"]).success();
    work_dir.run_jj(["commit", "-m", "three"]).success();

    let output = work_dir.run_jj(["log", "-r", "all()"]);
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

    let output = work_dir.run_jj(["sign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 4 commits:
      qpvuntsm 8174ec98 (empty) one
      rlvkpnrz 6500b275 (empty) two
      kkmpptxz bcfaa4c3 (empty) three
      zsuskuln 4947c6dd (empty) (no description set)
    Working copy  (@) now at: zsuskuln 4947c6dd (empty) (no description set)
    Parent commit (@-)      : kkmpptxz bcfaa4c3 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
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
    let output = work_dir.run_jj(["sign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 4 commits:
      qpvuntsm dabebf30 (empty) one
      rlvkpnrz 2085a464 (empty) two
      kkmpptxz 227f5e15 (empty) three
      zsuskuln 15d1b128 (empty) (no description set)
    Working copy  (@) now at: zsuskuln 15d1b128 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 227f5e15 (empty) three
    [EOF]
    ");
}

#[test]
fn test_sign_default_revset() {
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();

    test_env.add_config("revsets.sign = '@'");

    work_dir.run_jj(["sign"]).success();

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 8623fdf2 [✓︎]
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 876f4b7e
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_sign_with_key() {
    let test_env = TestEnvironment::default();

    test_env.add_config(
        r#"
[ui]
show-cryptographic-signatures = true

[signing]
behavior = "keep"
backend = "test"
key = "some-key"
"#,
    );

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();
    work_dir.run_jj(["commit", "-m", "two"]).success();

    work_dir.run_jj(["sign", "-r", "@-"]).success();
    work_dir
        .run_jj(["sign", "-r", "@--", "--key", "another-key"])
        .success();

    let output = work_dir.run_jj(["log", "-r", "@-|@--", "-Tbuiltin_log_detailed"]);
    insta::assert_snapshot!(output, @r"
    ○  Commit ID: f1a2b1ef76ee5b995ddbb1b13e8b54e4b0d32a12
    │  Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:11)
    │  Signature: good signature by test-display some-key
    │
    │      two
    │
    ○  Commit ID: d0e65e58aef1aca0ab92d3d42a9b00b82b7f76a6
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    ~  Author   : Test User <test.user@example.com> (2001-02-03 08:05:08)
       Committer: Test User <test.user@example.com> (2001-02-03 08:05:11)
       Signature: good signature by test-display another-key

           one

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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();
    work_dir.run_jj(["commit", "-m", "two"]).success();
    work_dir.run_jj(["commit", "-m", "three"]).success();

    work_dir
        .run_jj(&[
            "desc",
            "--author",
            "Someone Else <someone@else.com>",
            "--no-edit",
            "..@-",
        ])
        .success();
    let output = work_dir.run_jj(["sign", "-r", "..@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 3 commits:
      qpvuntsm 254d1a64 (empty) one
      rlvkpnrz caa78d30 (empty) two
      kkmpptxz c2bc0eb0 (empty) three
    Warning: 3 of these commits are not authored by you
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln ede04d15 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz c2bc0eb0 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "A"]).success();
    work_dir.run_jj(["commit", "-m", "B"]).success();
    work_dir.run_jj(["desc", "-m", "C"]).success();

    let output = work_dir.run_jj(["sign", "-r", "@|@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 2 commits:
      qpvuntsm 034b975d (empty) A
      kkmpptxz 29dc7928 (empty) C
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 29dc7928 (empty) C
    Parent commit (@-)      : rlvkpnrz 014c011c (empty) B
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["sign", "-r", "@-"]);

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No signing backend configured
    Hint: For configuring a signing backend, see https://jj-vcs.github.io/jj/latest/config/#commit-signing
    [EOF]
    [exit status: 1]
    ");
}

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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();
    work_dir.run_jj(["commit", "-m", "two"]).success();
    work_dir.run_jj(["commit", "-m", "three"]).success();

    work_dir.run_jj(["sign", "-r", "..@"]).success();

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:11 7aa7dcdf [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:11 0413d103 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:11 c8768375 [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 b90f5370 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["unsign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Unsigned 4 commits:
      qpvuntsm cb05440c (empty) one
      rlvkpnrz deb0db4b (empty) two
      kkmpptxz 7c11ee12 (empty) three
      zsuskuln be9daa4d (empty) (no description set)
    Working copy  (@) now at: zsuskuln be9daa4d (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 7c11ee12 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:13 be9daa4d
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:13 7c11ee12
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:13 deb0db4b
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 cb05440c
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "one"]).success();
    work_dir.run_jj(["commit", "-m", "two"]).success();
    work_dir.run_jj(["commit", "-m", "three"]).success();

    work_dir.run_jj(["sign", "-r", "..@"]).success();

    let run_jj_as_someone_else = |args: &[&str]| {
        let output = work_dir.run_jj_with(|cmd| {
            cmd.env("JJ_USER", "Someone Else")
                .env("JJ_EMAIL", "someone@else.com")
                .args(args)
        });
        (output.stdout, output.stderr)
    };

    let (_, stderr) = run_jj_as_someone_else(&["unsign", "-r", "..@"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned 4 commits:
      qpvuntsm 757aba72 (empty) one
      rlvkpnrz 49a6eeeb (empty) two
      kkmpptxz 8859969b (empty) three
      zsuskuln 8cea2d75 (empty) (no description set)
    Warning: 4 of these commits are not authored by you
    Working copy  (@) now at: zsuskuln 8cea2d75 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 8859969b (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 8cea2d75
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:12 8859969b
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:12 49a6eeeb
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 757aba72
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}
