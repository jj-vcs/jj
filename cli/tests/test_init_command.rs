// Copyright 2020 The Jujutsu Authors
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
fn test_init_toy_disallowed() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["init", "repo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: --ui.allow-init-native is deprecated; use ui.allow-init-toy instead.
    Error: The toy backend is disallowed by default.
    Hint: Did you mean to call `jj git init`?
    Set `ui.allow-init-toy` to allow initializing a repo with the toy backend.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_init_toy() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.allow-init-toy = true"#);
    let output = test_env.run_jj_in(".", ["init", "repo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: --ui.allow-init-native is deprecated; use ui.allow-init-toy instead.
    Error: The toy backend is disallowed by default.
    Hint: Did you mean to call `jj git init`?
    Set `ui.allow-init-toy` to allow initializing a repo with the toy backend.
    [EOF]
    [exit status: 1]
    ");

    let workspace_root = test_env.env_root().join("repo");
    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(store_path.join("commits").is_dir());
    assert!(store_path.join("trees").is_dir());
    assert!(store_path.join("files").is_dir());
    assert!(store_path.join("symlinks").is_dir());
    assert!(store_path.join("conflicts").is_dir());

    let output = test_env.run_jj_in(".", ["init", "--ignore-working-copy", "repo2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(".", ["init", "--at-op=@-", "repo3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

// TODO: remove in jj 0.33+
#[test]
fn test_init_native_deprecation_warning() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.allow-init-native = true"#);
    let output = test_env.run_jj_in(".", ["init", "repo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Value not found for ui.allow-init-toy
    For help, see https://jj-vcs.github.io/jj/latest/config/.
    [EOF]
    [exit status: 1]
    ");
}
