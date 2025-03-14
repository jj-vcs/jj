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
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use testutils::git;

use crate::common::TestEnvironment;

fn read_git_config(repo_path: &Path) -> String {
    let git_config = fs::read_to_string(repo_path.join(".jj/repo/store/git/config"))
        .or_else(|_| fs::read_to_string(repo_path.join(".git/config")))
        .unwrap();
    git_config
        .split_inclusive('\n')
        .filter(|line| {
            // Filter out non‐portable values.
            [
                "\tfilemode =",
                "\tsymlinks =",
                "\tignorecase =",
                "\tprecomposeunicode =",
            ]
            .iter()
            .all(|prefix| !line.to_ascii_lowercase().starts_with(prefix))
        })
        .collect()
}

#[test]
fn test_git_remotes() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "bar"]
    	url = http://example.com/repo/bar
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
}

#[test]
fn test_git_remote_add() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "foo"]
    	url = http://example.com/repo/bar
    	fetch = +refs/heads/*:refs/remotes/foo/*
    "#);
}

#[test]
fn test_git_remote_relative_path() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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
            ".",
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
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
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "baz"]
    	url = http://example.com/repo/baz
    	fetch = +refs/heads/*:refs/remotes/baz/*
    [remote "bar"]
    	url = http://example.com/repo/foo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
}

#[test]
fn test_git_remote_named_git() {
    let test_env = TestEnvironment::default();

    // Existing remote named 'git' shouldn't block the repo initialization.
    let repo_path = test_env.env_root().join("repo");
    git::init(&repo_path);
    git::add_remote(&repo_path, "git", "http://example.com/repo/repo");
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
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    [remote "bar"]
    	url = http://example.com/repo/repo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
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
    git::rename_remote(&repo_path, "bar", "git");
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    [remote "git"]
    	url = http://example.com/repo/repo
    	fetch = +refs/heads/*:refs/remotes/git/*
    "#);

    // The remote can also be removed.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "git"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    "#);
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
    git::init(&repo_path);
    git::add_remote(&repo_path, "slash/origin", "http://example.com/repo/repo");
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
    git::rename_remote(&repo_path, "origin", "slash/origin");
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();

    // The remote can also be removed.
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "remove", "slash/origin"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
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
fn test_git_remote_with_branch_config() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(test_env.env_root(), &["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(
        &repo_path,
        &["git", "remote", "add", "foo", "http://example.com/repo"],
    );
    insta::assert_snapshot!(output, @"");

    let mut config_file = fs::OpenOptions::new()
        .append(true)
        .open(repo_path.join(".jj/repo/store/git/config"))
        .unwrap();
    // `git clone` adds branch configuration like this.
    writeln!(config_file, "[branch \"test\"]").unwrap();
    writeln!(config_file, "\tremote = foo").unwrap();
    writeln!(config_file, "\tmerge = refs/heads/test").unwrap();
    drop(config_file);

    let output = test_env.run_jj_in(&repo_path, &["git", "remote", "rename", "foo", "bar"]);
    insta::assert_snapshot!(output, @"");

    insta::assert_snapshot!(read_git_config(&repo_path), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [branch "test"]
    	remote = bar
    	merge = refs/heads/test
    [remote "bar"]
    	url = http://example.com/repo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
}
