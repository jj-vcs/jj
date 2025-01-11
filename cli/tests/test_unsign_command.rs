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
                         signature.status() ++ " " ++ signature.display(),
                         "no"
                      ) ++ " signature""#;

    let show_no_sig = test_env.jj_cmd_success(&repo_path, &["log", "-T", template, "-r", "all()"]);
    insta::assert_snapshot!(show_no_sig, @r"
    @  no signature
    ○  no signature
    ○  no signature
    ○  no signature
    ◆  no signature
    ");

    test_env.jj_cmd_ok(&repo_path, &["sign", "-r", "..@-"]);

    let show_with_sig =
        test_env.jj_cmd_success(&repo_path, &["log", "-T", template, "-r", "all()"]);
    insta::assert_snapshot!(show_with_sig, @r"
    @  no signature
    ○  good test-display signature
    ○  good test-display signature
    ○  good test-display signature
    ◆  no signature
    ");

    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["unsign", "-r", "..@-"]);
    insta::assert_snapshot!(stderr, @r"
    Unsigned the following commits:
      qpvuntsm hidden afde6e4b (empty) one
      rlvkpnrz hidden d49204af (empty) two
      kkmpptxz hidden ea6d9b6d (empty) three
    Rebased 1 descendant commits
    Working copy now at: zsuskuln 4029f2fc (empty) (no description set)
    Parent commit      : kkmpptxz ea6d9b6d (empty) three
    ");

    let show_with_sig =
        test_env.jj_cmd_success(&repo_path, &["log", "-T", template, "-r", "all()"]);
    insta::assert_snapshot!(show_with_sig, @r"
    @  no signature
    ○  no signature
    ○  no signature
    ○  no signature
    ◆  no signature
    ");
}

#[test]
#[should_panic]
fn test_warn_about_unsigning_commits_not_authored_by_me() {
    todo!()
}
