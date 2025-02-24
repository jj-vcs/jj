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

use std::path::PathBuf;

use crate::common::TestEnvironment;

fn set_up(trunk_name: &str) -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "origin"])
        .success();
    let origin_path = test_env.env_root().join("origin");
    let origin_git_repo_path = origin_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    test_env
        .run_jj_in(&origin_path, ["describe", "-m=description 1"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "create", "-r@", trunk_name])
        .success();
    test_env
        .run_jj_in(&origin_path, ["new", "root()", "-m=description 2"])
        .success();
    test_env
        .run_jj_in(
            &origin_path,
            ["bookmark", "create", "-r@", "unrelated_bookmark"],
        )
        .success();
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    test_env
        .run_jj_in(
            test_env.env_root(),
            [
                "git",
                "clone",
                "--config=git.auto-local-bookmark=true",
                origin_git_repo_path.to_str().unwrap(),
                "local",
            ],
        )
        .success();
    let workspace_root = test_env.env_root().join("local");
    (test_env, workspace_root)
}

#[test]
fn test_builtin_alias_trunk_matches_main() {
    let (test_env, workspace_root) = set_up("main");

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_master() {
    let (test_env, workspace_root) = set_up("master");

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 master d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_trunk() {
    let (test_env, workspace_root) = set_up("trunk");

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 trunk d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_exactly_one_commit() {
    let (test_env, workspace_root) = set_up("main");
    let origin_path = test_env.env_root().join("origin");
    test_env
        .run_jj_in(&origin_path, ["new", "root()", "-m=description 3"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "create", "-r@", "master"])
        .success();

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_override_alias() {
    let (test_env, workspace_root) = set_up("override-trunk");

    test_env.add_config(
        r#"revset-aliases.'trunk()' = 'latest(remote_bookmarks(exact:"override-trunk", exact:"origin"))'"#,
    );

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 override-trunk d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_no_match() {
    let (test_env, workspace_root) = set_up("no-match-trunk");

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_no_match_only_exact() {
    let (test_env, workspace_root) = set_up("maint");

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_builtin_user_redefines_builtin_immutable_heads() {
    let (test_env, workspace_root) = set_up("main");

    test_env.add_config(r#"revset-aliases.'builtin_immutable_heads()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'mutable()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'immutable()' = '@'"#);

    let output = test_env.run_jj_in(&workspace_root, ["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ○  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    [EOF]
    ------- stderr -------
    Warning: Redefining `revset-aliases.builtin_immutable_heads()` is not recommended; redefine `immutable_heads()` instead
    Warning: Redefining `revset-aliases.mutable()` is not recommended; redefine `immutable_heads()` instead
    Warning: Redefining `revset-aliases.immutable()` is not recommended; redefine `immutable_heads()` instead
    [EOF]
    ");
}
