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
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "init"]);

    let template = r#"if(signature,
                         signature.status() ++ " " ++ signature.display(),
                         "no"
                      ) ++ " signature""#;

    let show_no_sig = test_env.jj_cmd_success(&repo_path, &["show", "-T", template, "-r", "@-"]);
    insta::assert_snapshot!(show_no_sig, @"no signature");

    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-"]);
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: rlvkpnrz 1c141424 (empty) (no description set)
    Parent commit      : qpvuntsm a9cc7c27 (empty) init
    Commit was signed: qpvuntsm a9cc7c27 (empty) init
    ");

    let show_with_sig = test_env.jj_cmd_success(&repo_path, &["show", "-T", template, "-r", "@-"]);
    insta::assert_snapshot!(show_with_sig, @r"good test-display signature");
}

#[test]
fn test_sig_drop() {
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
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "init"]);

    let template = r#"if(signature,
                         signature.status() ++ " " ++ signature.display(),
                         "no"
                      ) ++ " signature""#;

    let show_no_sig = test_env.jj_cmd_success(&repo_path, &["show", "-T", template, "-r", "@-"]);
    insta::assert_snapshot!(show_no_sig, @"no signature");

    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-"]);

    let show_with_sig = test_env.jj_cmd_success(&repo_path, &["show", "-T", template, "-r", "@-"]);
    insta::assert_snapshot!(show_with_sig, @"good test-display signature");

    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-", "--drop"]);
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: rlvkpnrz be42f24a (empty) (no description set)
    Parent commit      : qpvuntsm 755eae8e (empty) init
    Signature was dropped: qpvuntsm 755eae8e (empty) init
    ");

    let show_with_sig = test_env.jj_cmd_success(&repo_path, &["show", "-T", template, "-r", "@-"]);
    insta::assert_snapshot!(show_with_sig, @"no signature");
}

#[test]
fn test_commit_already_signed() {
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
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "init"]);

    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-"]);
    let stderr = test_env.jj_cmd_failure(&repo_path, &["sign", "-r", "@-"]);
    insta::assert_snapshot!(stderr, @"Error: Commit is already signed, use --force to sign anyway");
}

#[test]
fn test_force_sign() {
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
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "init"]);

    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "@-"]);
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["sign", "--force", "-r", "@-"]);
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: rlvkpnrz 1c141424 (empty) (no description set)
    Parent commit      : qpvuntsm a9cc7c27 (empty) init
    Commit was signed: qpvuntsm a9cc7c27 (empty) init
    ");
}

#[test]
fn test_different_author() {
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
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "init"]);

    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "desc",
            "--author",
            "Someone Else <someone@else.com>",
            "-m",
            "init",
            "@-",
        ],
    );
    let stderr = test_env.jj_cmd_failure(&repo_path, &["sign", "-r", "@-"]);
    insta::assert_snapshot!(stderr, @"Error: Commit is not authored by you, use --force to sign anyway");
}
