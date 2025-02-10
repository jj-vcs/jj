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
fn test_init_local_disallowed() {
    let test_env = TestEnvironment::default();
    let stdout = test_env.jj_cmd_failure(test_env.env_root(), &["init", "repo"]);
    insta::assert_snapshot!(stdout, @r"
    Error: The native backend is disallowed by default.
    Hint: Did you mean to call `jj git init`?
    Set `ui.allow-init-native` to allow initializing a repo with the native backend.
    ");
}

#[test]
fn test_init_local() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"ui.allow-init-native = true"#);
    let (stdout, stderr) = test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r#"Initialized repo in "repo""#);

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

    let stderr = test_env.jj_cmd_cli_error(
        test_env.env_root(),
        &["init", "--ignore-working-copy", "repo2"],
    );
    insta::assert_snapshot!(stderr, @"Error: --ignore-working-copy is not respected");

    let stderr = test_env.jj_cmd_cli_error(test_env.env_root(), &["init", "--at-op=@-", "repo3"]);
    insta::assert_snapshot!(stderr, @"Error: --at-op is not respected");
}
