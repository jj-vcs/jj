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
    @  zsuskuln test.user@example.com 2001-02-03 08:05:10 fbef508b
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 8c63f712
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 26a2c4cb
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 401ea16f
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["sign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Signed 4 commits:
      qpvuntsm 7fb98da0 (empty) one
      rlvkpnrz 062a3c5a (empty) two
      kkmpptxz d2174a79 (empty) three
      zsuskuln 8d7bc037 (empty) (no description set)
    Working copy  (@) now at: zsuskuln 8d7bc037 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz d2174a79 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 8d7bc037 [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:12 d2174a79 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:12 062a3c5a [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 7fb98da0 [✓︎]
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
      qpvuntsm a57217b4 (empty) one
      rlvkpnrz e0c0e7ad (empty) two
      kkmpptxz fc827eb8 (empty) three
      zsuskuln 66574289 (empty) (no description set)
    Working copy  (@) now at: zsuskuln 66574289 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz fc827eb8 (empty) three
    [EOF]
    ");

    // Signing nothing is a valid no-op.
    let output = work_dir.run_jj(["sign", "-r", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
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
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 72a53d81 [✓︎]
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 401ea16f
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
    ○  Commit ID: 810ff318afe002ce54260260e4d4f7071eb476ed
    │  Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:11)
    │  Signature: good signature by test-display some-key
    │
    │      two
    │
    ○  Commit ID: eec44cafe0dc853b67cc7e14ca4fe3b80d3687f1
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
      qpvuntsm 2c0b7924 (empty) one
      rlvkpnrz 0e054ee0 (empty) two
      kkmpptxz ed55e398 (empty) three
    Warning: 3 of these commits are not authored by you
    Rebased 1 descendant commits
    Working copy  (@) now at: zsuskuln 1b3596cb (empty) (no description set)
    Parent commit (@-)      : kkmpptxz ed55e398 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 1b3596cb
    │  (empty) (no description set)
    ○  kkmpptxz someone@else.com 2001-02-03 08:05:12 ed55e398 [✓︎]
    │  (empty) three
    ○  rlvkpnrz someone@else.com 2001-02-03 08:05:12 0e054ee0 [✓︎]
    │  (empty) two
    ○  qpvuntsm someone@else.com 2001-02-03 08:05:12 2c0b7924 [✓︎]
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
      qpvuntsm 0e149d92 (empty) A
      kkmpptxz ab7e21e9 (empty) C
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz ab7e21e9 (empty) C
    Parent commit (@-)      : rlvkpnrz 3981b3e4 (empty) B
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:11 ab7e21e9 [✓︎]
    │  (empty) C
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:11 3981b3e4
    │  (empty) B
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 0e149d92 [✓︎]
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
    Hint: For configuring a signing backend, see https://docs.jj-vcs.dev/latest/config/#commit-signing
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
    @  zsuskuln test.user@example.com 2001-02-03 08:05:11 be4609e2 [✓︎]
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:11 7b6ad8e6 [✓︎]
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:11 8dc06170 [✓︎]
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 fbef1f02 [✓︎]
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["unsign", "-r", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Unsigned 4 commits:
      qpvuntsm c08b67cb (empty) one
      rlvkpnrz 3081d203 (empty) two
      kkmpptxz 8c2dc912 (empty) three
      zsuskuln 9aec4578 (empty) (no description set)
    Working copy  (@) now at: zsuskuln 9aec4578 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 8c2dc912 (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:13 9aec4578
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:13 8c2dc912
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:13 3081d203
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 c08b67cb
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Unsigning nothing is a valid no-op.
    let output = work_dir.run_jj(["unsign", "-r", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
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
      qpvuntsm 4430b844 (empty) one
      rlvkpnrz 65d9cdf7 (empty) two
      kkmpptxz f6eb4a7e (empty) three
      zsuskuln 0fda7ce2 (empty) (no description set)
    Warning: 4 of these commits are not authored by you
    Working copy  (@) now at: zsuskuln 0fda7ce2 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz f6eb4a7e (empty) three
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  zsuskuln test.user@example.com 2001-02-03 08:05:12 0fda7ce2
    │  (empty) (no description set)
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:12 f6eb4a7e
    │  (empty) three
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:12 65d9cdf7
    │  (empty) two
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 4430b844
    │  (empty) one
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}
