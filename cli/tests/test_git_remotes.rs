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

use std::fs;
use std::path::PathBuf;

use crate::common::TestEnvironment;

#[test]
fn test_git_remotes() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "remote", "add", "foo", "http://example.com/repo/foo"],
    );
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "remote", "add", "bar", "http://example.com/repo/bar"],
    );
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/bar
    foo http://example.com/repo/foo
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "foo"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/bar
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'nonexistent'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_remote_add() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "foo", "http://example.com/repo/foo"],
        )
        .success();
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git",
            "remote",
            "add",
            "foo",
            "http://example.com/repo/foo2",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'foo' already exists
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "remote", "add", "git", "http://example.com/repo/git"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo http://example.com/repo/foo
    [EOF]
    ");
}

#[test]
fn test_git_remote_set_url() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "foo", "http://example.com/repo/foo"],
        )
        .success();
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git",
            "remote",
            "set-url",
            "bar",
            "http://example.com/repo/bar",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'bar'
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git",
            "remote",
            "set-url",
            "git",
            "http://example.com/repo/git",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git",
            "remote",
            "set-url",
            "foo",
            "http://example.com/repo/bar",
        ],
    );
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo http://example.com/repo/bar
    [EOF]
    ");
}

#[test]
fn test_git_remote_relative_path() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    // Relative path using OS-native separator
    let path = PathBuf::from_iter(["..", "native", "sep"]);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "foo", path.to_str().unwrap()],
        )
        .success();
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo $TEST_ENV/native/sep
    [EOF]
    ");

    // Relative path using UNIX separator
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["-Rrepo", "git", "remote", "set-url", "foo", "unix/sep"],
        )
        .success();
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo $TEST_ENV/unix/sep
    [EOF]
    ");
}

#[test]
fn test_git_remote_rename() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "foo", "http://example.com/repo/foo"],
        )
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "baz", "http://example.com/repo/baz"],
        )
        .success();
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "bar", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'bar'
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "foo", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'baz' already exists
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "foo", "git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "foo", "bar"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/foo
    baz http://example.com/repo/baz
    [EOF]
    ");
}

#[test]
fn test_git_remote_named_git() {
    let test_env = TestEnvironment::default();

    // Existing remote named 'git' shouldn't block the repo initialization.
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&repo_path).unwrap();
    git_repo
        .remote("git", "http://example.com/repo/repo")
        .unwrap();
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();

    // The remote can be renamed.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "git", "bar"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/repo
    [EOF]
    ");
    // @git bookmark shouldn't be renamed.
    let output = test_env.run_jj_in(&repo_path, ["log", "-rmain@git", "-Tbookmarks"]);
    insta::assert_snapshot!(output, @r"
    @  main
    │
    ~
    [EOF]
    ");

    // The remote cannot be renamed back by jj.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "rename", "bar", "git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");

    // Reinitialize the repo with remote named 'git'.
    fs::remove_dir_all(repo_path.join(".jj")).unwrap();
    git_repo.remote_rename("bar", "git").unwrap();
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();

    // The remote can also be removed.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "git"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r###"
    "###);
    // @git bookmark shouldn't be removed.
    let output = test_env.run_jj_in(&repo_path, ["log", "-rmain@git", "-Tbookmarks"]);
    insta::assert_snapshot!(output, @r"
    ○  main
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_git_remote_with_slashes() {
    let test_env = TestEnvironment::default();

    // Existing remote with slashes shouldn't block the repo initialization.
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::init(&repo_path).unwrap();
    git_repo
        .remote("slash/origin", "http://example.com/repo/repo")
        .unwrap();
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();

    // Cannot add remote with a slash via `jj`
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git",
            "remote",
            "add",
            "another/origin",
            "http://examples.org/repo/repo",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: another/origin
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    slash/origin http://example.com/repo/repo
    [EOF]
    ");

    // The remote can be renamed.
    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "remote", "rename", "slash/origin", "origin"],
    );
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    origin http://example.com/repo/repo
    [EOF]
    ");

    // The remote cannot be renamed back by jj.
    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "remote", "rename", "origin", "slash/origin"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    [EOF]
    [exit status: 1]
    ");

    // Reinitialize the repo with remote with slashes
    fs::remove_dir_all(repo_path.join(".jj")).unwrap();
    git_repo.remote_rename("origin", "slash/origin").unwrap();
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();

    // The remote can also be removed.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "slash/origin"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r###"
    "###);
    // @git bookmark shouldn't be removed.
    let output = test_env.run_jj_in(&repo_path, ["log", "-rmain@git", "-Tbookmarks"]);
    insta::assert_snapshot!(output, @r"
    ○  main
    │
    ~
    [EOF]
    ");
}
