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

use indoc::indoc;
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
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "add", "foo", "http://example.com/repo/foo"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "add", "bar", "http://example.com/repo/bar"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/bar
    foo http://example.com/repo/foo
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "remote", "remove", "foo"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/bar
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "remote", "remove", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'nonexistent'
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
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
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["git", "remote", "add", "foo", "http://example.com/repo/foo"])
        .success();
    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "foo",
        "http://example.com/repo/foo2",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'foo' already exists
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "add", "git", "http://example.com/repo/git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo http://example.com/repo/foo
    [EOF]
    ");
}

#[test]
fn test_git_remote_with_fetch_tags() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "remote", "add", "foo", "http://example.com/repo"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "foo-included",
        "http://example.com/repo",
        "--fetch-tags",
        "included",
    ]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "foo-all",
        "http://example.com/repo",
        "--fetch-tags",
        "all",
    ]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "foo-none",
        "http://example.com/repo",
        "--fetch-tags",
        "none",
    ]);
    insta::assert_snapshot!(output, @"");

    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "foo"]
    	url = http://example.com/repo
    	fetch = +refs/heads/*:refs/remotes/foo/*
    [remote "foo-included"]
    	url = http://example.com/repo
    	fetch = +refs/heads/*:refs/remotes/foo-included/*
    [remote "foo-all"]
    	url = http://example.com/repo
    	tagOpt = --tags
    	fetch = +refs/heads/*:refs/remotes/foo-all/*
    [remote "foo-none"]
    	url = http://example.com/repo
    	tagOpt = --no-tags
    	fetch = +refs/heads/*:refs/remotes/foo-none/*
    "#);
}

#[test]
fn test_git_remote_set_url() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["git", "remote", "add", "foo", "http://example.com/repo/foo"])
        .success();
    let output = work_dir.run_jj([
        "git",
        "remote",
        "set-url",
        "bar",
        "http://example.com/repo/bar",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'bar'
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj([
        "git",
        "remote",
        "set-url",
        "git",
        "http://example.com/repo/git",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj([
        "git",
        "remote",
        "set-url",
        "foo",
        "http://example.com/repo/bar",
    ]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo http://example.com/repo/bar
    [EOF]
    ");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
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
    let work_dir = test_env.work_dir("repo");

    // Relative path using OS-native separator
    let path = PathBuf::from_iter(["..", "native", "sep"]);
    work_dir
        .run_jj(["git", "remote", "add", "foo", path.to_str().unwrap()])
        .success();
    let output = work_dir.run_jj(["git", "remote", "list"]);
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
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    foo $TEST_ENV/unix/sep
    [EOF]
    ");
}

#[test]
fn test_git_remote_rename() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["git", "remote", "add", "foo", "http://example.com/repo/foo"])
        .success();
    work_dir
        .run_jj(["git", "remote", "add", "baz", "http://example.com/repo/baz"])
        .success();
    let output = work_dir.run_jj(["git", "remote", "rename", "bar", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'bar'
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "rename", "foo", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'baz' already exists
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "rename", "foo", "git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "rename", "foo", "bar"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/foo
    baz http://example.com/repo/baz
    [EOF]
    ");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
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
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    git::add_remote(work_dir.root(), "git", "http://example.com/repo/repo");
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();

    // The remote can be renamed.
    let output = work_dir.run_jj(["git", "remote", "rename", "git", "bar"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar http://example.com/repo/repo
    [EOF]
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    [EOF]
    ");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    [remote "bar"]
    	url = http://example.com/repo/repo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
    // @git bookmark shouldn't be renamed.
    let output = work_dir.run_jj(["log", "-rmain@git", "-Tbookmarks"]);
    insta::assert_snapshot!(output, @r"
    @  main
    │
    ~
    [EOF]
    ");

    // The remote cannot be renamed back by jj.
    let output = work_dir.run_jj(["git", "remote", "rename", "bar", "git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");

    // Reinitialize the repo with remote named 'git'.
    work_dir.remove_dir_all(".jj");
    git::rename_remote(work_dir.root(), "bar", "git");
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    [remote "git"]
    	url = http://example.com/repo/repo
    	fetch = +refs/heads/*:refs/remotes/git/*
    "#);

    // The remote can also be removed.
    let output = work_dir.run_jj(["git", "remote", "remove", "git"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = false
    	logallrefupdates = true
    "#);
    // @git bookmark shouldn't be removed.
    let output = work_dir.run_jj(["log", "-rmain@git", "-Tbookmarks"]);
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
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    git::add_remote(
        work_dir.root(),
        "slash/origin",
        "http://example.com/repo/repo",
    );
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();

    // Cannot add remote with a slash via `jj`
    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "another/origin",
        "http://examples.org/repo/repo",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: another/origin
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    slash/origin http://example.com/repo/repo
    [EOF]
    ");

    // The remote can be renamed.
    let output = work_dir.run_jj(["git", "remote", "rename", "slash/origin", "origin"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    origin http://example.com/repo/repo
    [EOF]
    ");

    // The remote cannot be renamed back by jj.
    let output = work_dir.run_jj(["git", "remote", "rename", "origin", "slash/origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    [EOF]
    [exit status: 1]
    ");

    // Reinitialize the repo with remote with slashes
    work_dir.remove_dir_all(".jj");
    git::rename_remote(work_dir.root(), "origin", "slash/origin");
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    // The remote can also be removed.
    let output = work_dir.run_jj(["git", "remote", "remove", "slash/origin"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @"");
    // @git bookmark shouldn't be removed.
    let output = work_dir.run_jj(["log", "-rmain@git", "-Tbookmarks"]);
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

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "remote", "add", "foo", "http://example.com/repo"]);
    insta::assert_snapshot!(output, @"");

    let mut config_file = fs::OpenOptions::new()
        .append(true)
        .open(work_dir.root().join(".jj/repo/store/git/config"))
        .unwrap();
    // `git clone` adds branch configuration like this.
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    write!(config_file, "[branch \"test\"]{eol}").unwrap();
    write!(config_file, "\tremote = foo{eol}").unwrap();
    write!(config_file, "\tmerge = refs/heads/test{eol}").unwrap();
    drop(config_file);

    let output = work_dir.run_jj(["git", "remote", "rename", "foo", "bar"]);
    insta::assert_snapshot!(output, @"");

    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
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

#[test]
fn test_git_remote_with_global_git_remote_config() {
    let mut test_env = TestEnvironment::default();
    test_env.work_dir("").write_file(
        "git-config",
        indoc! {r#"
            [remote "origin"]
                prune = true
            [remote "foo"]
                url = htps://example.com/repo/foo
                fetch = +refs/heads/*:refs/remotes/foo/*
        "#},
    );
    test_env.add_env_var("GIT_CONFIG_GLOBAL", test_env.env_root().join("git-config"));

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["git", "remote", "list"]);
    // Complete remotes from the global configuration are listed.
    //
    // `git remote -v` lists all remotes from the global configuration,
    // even incomplete ones like `origin`. This is inconsistent with
    // the other `git remote` commands, which ignore the global
    // configuration (even `git remote get-url`).
    insta::assert_snapshot!(output, @r"
    foo htps://example.com/repo/foo
    [EOF]
    ");

    let output = work_dir.run_jj(["git", "remote", "rename", "foo", "bar"]);
    // Divergence from Git: we read the remote from the global
    // configuration and write it back out. Git will use the global
    // configuration for commands like `git remote -v`, `git fetch`,
    // and `git push`, but `git remote rename`, `git remote remove`,
    // `git remote set-url`, etc., will ignore it.
    //
    // This behavior applies to `jj git remote remove` and
    // `jj git remote set-url` as well. It would be hard to change due
    // to gitoxide’s model, but hopefully it’s relatively harmless.
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "bar"]
    	url = htps://example.com/repo/foo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    "#);
    // This has the unfortunate consequence that the original remote
    // still exists after renaming.
    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar htps://example.com/repo/foo
    foo htps://example.com/repo/foo
    [EOF]
    ");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "origin",
        "http://example.com/repo/origin/1",
    ]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "set-url",
        "origin",
        "https://example.com/repo/origin/2",
    ]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["git", "remote", "list"]);
    insta::assert_snapshot!(output, @r"
    bar htps://example.com/repo/foo
    foo htps://example.com/repo/foo
    origin https://example.com/repo/origin/2
    [EOF]
    ");
    insta::assert_snapshot!(read_git_config(work_dir.root()), @r#"
    [core]
    	repositoryformatversion = 0
    	bare = true
    	logallrefupdates = false
    [remote "bar"]
    	url = htps://example.com/repo/foo
    	fetch = +refs/heads/*:refs/remotes/bar/*
    [remote "origin"]
    	url = https://example.com/repo/origin/2
    	fetch = +refs/heads/*:refs/remotes/origin/*
    "#);
}
