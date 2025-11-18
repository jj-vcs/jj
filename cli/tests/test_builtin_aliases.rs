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

fn set_up(trunk_name: &str) -> TestEnvironment {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    origin_dir
        .run_jj(["describe", "-m=description 1"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", trunk_name])
        .success();
    origin_dir
        .run_jj(["new", "root()", "-m=description 2"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "unrelated_bookmark"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "clone",
                "--config=remotes.origin.auto-track-bookmarks='glob:*'",
                origin_git_repo_path.to_str().unwrap(),
                "local",
            ],
        )
        .success();
    test_env
}

#[test]
fn test_builtin_alias_trunk_matches_main() {
    let test_env = set_up("main");
    let work_dir = test_env.work_dir("local");

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:08 main 9b2e76de
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_master() {
    let test_env = set_up("master");
    let work_dir = test_env.work_dir("local");

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:08 master 9b2e76de
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_trunk() {
    let test_env = set_up("trunk");
    let work_dir = test_env.work_dir("local");

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:08 trunk 9b2e76de
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_matches_exactly_one_commit() {
    let test_env = set_up("main");
    let work_dir = test_env.work_dir("local");
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["new", "root()", "-m=description 3"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "master"])
        .success();

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:08 main 9b2e76de
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_override_alias() {
    let test_env = set_up("override-trunk");
    let work_dir = test_env.work_dir("local");

    test_env.add_config(
        r#"revset-aliases.'trunk()' = 'latest(remote_bookmarks(exact:"override-trunk", exact:"origin"))'"#,
    );

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:08 override-trunk 9b2e76de
    │  (empty) description 1
    ~
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_no_match() {
    let test_env = set_up("no-match-trunk");
    let work_dir = test_env.work_dir("local");

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_builtin_alias_trunk_no_match_only_exact() {
    let test_env = set_up("maint");
    let work_dir = test_env.work_dir("local");

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_builtin_user_redefines_builtin_immutable_heads() {
    let test_env = set_up("main");
    let work_dir = test_env.work_dir("local");

    test_env.add_config(r#"revset-aliases.'builtin_immutable_heads()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'mutable()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'immutable()' = '@'"#);

    let output = work_dir.run_jj(["log", "-r", "trunk()"]);
    insta::assert_snapshot!(output, @r"
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 main 9b2e76de
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
