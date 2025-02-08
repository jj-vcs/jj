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
[signing]
sign-all = false
backend = "test"
"#,
    );

    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "one"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "two"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "three"]);

    let template = r#"if(signature,
                         separate(" ", signature.status(), signature.display()),
                         "no"
                      ) ++ " signature""#;

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", template, "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  no signature
    ○  no signature
    ○  no signature
    ○  no signature
    ◆  no signature
    ");

    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Signed 3 commits:
      qpvuntsm hidden 8174ec98 (empty) one
      rlvkpnrz hidden 6500b275 (empty) two
      kkmpptxz hidden bcfaa4c3 (empty) three
    Working copy now at: zsuskuln eeb8c985 (empty) (no description set)
    Parent commit      : kkmpptxz bcfaa4c3 (empty) three
    ");

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", template, "-r", "all()"]);
    insta::assert_snapshot!(stdout, @r"
    @  no signature
    ○  good test-display signature
    ○  good test-display signature
    ○  good test-display signature
    ◆  no signature
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
[signing]
sign-all = false
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
    Working copy now at: zsuskuln 5a1d05b3 (empty) (no description set)
    Parent commit      : kkmpptxz 60618621 (empty) three
    ");
}

#[test]
#[should_panic]
fn test_abort_with_error_if_no_signing_backend_is_configured() {
    todo!()
}
