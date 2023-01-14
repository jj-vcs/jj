// Copyright 2022 The Jujutsu Authors
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

use common::{get_stderr_string, get_stdout_string, TestEnvironment};

pub mod common;

#[test]
fn test_syntax_error() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "x &"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:4
      |
    1 | x &
      |    ^---
      |
      = expected dag_range_pre_op, range_pre_op, negate_op, or primary
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "x - y"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:3
      |
    1 | x - y
      |   ^
      |
      = '-' is not an infix operator (Did you mean '~' for difference?)
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "HEAD^"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:5
      |
    1 | HEAD^
      |     ^
      |
      = '^' is not a postfix operator (Did you mean '-' for parents?)
    "###);
}

#[test]
fn test_bad_function_call() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "all(or:nothing)"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:5
      |
    1 | all(or:nothing)
      |     ^--------^
      |
      = Invalid arguments to revset function "all": Expected 0 arguments
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "parents()"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:9
      |
    1 | parents()
      |         ^
      |
      = Invalid arguments to revset function "parents": Expected 1 argument
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "parents(foo, bar)"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:9
      |
    1 | parents(foo, bar)
      |         ^------^
      |
      = Invalid arguments to revset function "parents": Expected 1 argument
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "heads(foo, bar)"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:7
      |
    1 | heads(foo, bar)
      |       ^------^
      |
      = Invalid arguments to revset function "heads": Expected 0 or 1 arguments
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "file()"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:6
      |
    1 | file()
      |      ^
      |
      = Invalid arguments to revset function "file": Expected at least 1 argument
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "file(a, not:a-string)"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:9
      |
    1 | file(a, not:a-string)
      |         ^----------^
      |
      = Invalid arguments to revset function "file": Expected function argument of type string
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", r#"file(a, "../out")"#]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:9
      |
    1 | file(a, "../out")
      |         ^------^
      |
      = Invalid file pattern: Path "../out" is not in the repo
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "root:whatever()"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:6
      |
    1 | root:whatever()
      |      ^------^
      |
      = Revset function "whatever" doesn't exist
    "###);
}

#[test]
fn test_alias() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(
        br###"
    [revset-aliases]
    'my-root' = 'root'
    'syntax-error' = 'whatever &'
    'recurse' = 'recurse1'
    'recurse1' = 'recurse2()'
    'recurse2()' = 'recurse'
    'identity(x)' = 'x'
    'my_author(x)' = 'author(x)'
    "###,
    );

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "my-root"]);
    insta::assert_snapshot!(stdout, @r###"
    o 000000000000  1970-01-01 00:00:00.000 +00:00 000000000000
      (empty) (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-r", "identity(my-root)"]);
    insta::assert_snapshot!(stdout, @r###"
    o 000000000000  1970-01-01 00:00:00.000 +00:00 000000000000
      (empty) (no description set)
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "root & syntax-error"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:8
      |
    1 | root & syntax-error
      |        ^----------^
      |
      = Alias "syntax-error" cannot be expanded
     --> 1:11
      |
    1 | whatever &
      |           ^---
      |
      = expected dag_range_pre_op, range_pre_op, negate_op, or primary
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "identity()"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:10
      |
    1 | identity()
      |          ^
      |
      = Invalid arguments to revset function "identity": Expected 1 arguments
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "my_author(none())"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:1
      |
    1 | my_author(none())
      | ^---------------^
      |
      = Alias "my_author()" cannot be expanded
     --> 1:8
      |
    1 | author(x)
      |        ^
      |
      = Invalid arguments to revset function "author": Expected function argument of type string
    "###);

    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-r", "root & recurse"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse revset:  --> 1:8
      |
    1 | root & recurse
      |        ^-----^
      |
      = Alias "recurse" cannot be expanded
     --> 1:1
      |
    1 | recurse1
      | ^------^
      |
      = Alias "recurse1" cannot be expanded
     --> 1:1
      |
    1 | recurse2()
      | ^--------^
      |
      = Alias "recurse2()" cannot be expanded
     --> 1:1
      |
    1 | recurse
      | ^-----^
      |
      = Alias "recurse" expanded recursively
    "###);
}

#[test]
fn test_bad_alias_decl() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(
        br###"
    [revset-aliases]
    'my-root' = 'root'
    '"bad"' = 'root'
    'badfn(a, a)' = 'root'
    "###,
    );

    // Invalid declaration should be warned and ignored.
    let assert = test_env
        .jj_cmd(&repo_path, &["log", "-r", "my-root"])
        .assert()
        .success();
    insta::assert_snapshot!(get_stdout_string(&assert), @r###"
    o 000000000000  1970-01-01 00:00:00.000 +00:00 000000000000
      (empty) (no description set)
    "###);
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Failed to load "revset-aliases."bad"":  --> 1:1
      |
    1 | "bad"
      | ^---
      |
      = expected identifier or function_name
    Failed to load "revset-aliases.badfn(a, a)":  --> 1:7
      |
    1 | badfn(a, a)
      |       ^--^
      |
      = Redefinition of function parameter
    "###);
}
