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
use std::path::Path;

use test_case::test_case;

use crate::common::get_stderr_string;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;

fn add_commit_to_branch(git_repo: &git2::Repository, branch: &str) -> git2::Oid {
    let signature = git2_signature();
    let mut tree_builder = git_repo.treebuilder(None).unwrap();
    let file_oid = git_repo.blob(branch.as_bytes()).unwrap();
    tree_builder
        .insert("file", file_oid, git2::FileMode::Blob.into())
        .unwrap();
    let tree_oid = tree_builder.write().unwrap();
    let tree = git_repo.find_tree(tree_oid).unwrap();
    git_repo
        .commit(
            Some(&format!("refs/heads/{branch}")),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap()
}

fn git2_signature() -> git2::Signature<'static> {
    git2::Signature::new("Some One", "some.one@example.com", &git2::Time::new(0, 0)).unwrap()
}

/// Creates a remote Git repo containing a bookmark with the same name
fn init_git_remote(test_env: &TestEnvironment, remote: &str) -> git2::Repository {
    let git_repo_path = test_env.env_root().join(remote);
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    add_commit_to_branch(&git_repo, remote);

    git_repo
}

/// Add a remote containing a bookmark with the same name
fn add_git_remote(test_env: &TestEnvironment, repo_path: &Path, remote: &str) -> git2::Repository {
    let repo = init_git_remote(test_env, remote);
    test_env
        .run_jj_in(
            repo_path,
            ["git", "remote", "add", remote, &format!("../{remote}")],
        )
        .success();

    repo
}

#[must_use]
fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    test_env.run_jj_in(repo_path, ["bookmark", "list", "--all-remotes", "--quiet"])
}

fn create_commit(test_env: &TestEnvironment, repo_path: &Path, name: &str, parents: &[&str]) {
    let descr = format!("descr_for_{name}");
    let parents = match parents {
        [] => &["root()"],
        parents => parents,
    };
    test_env
        .run_jj_with(|cmd| {
            cmd.current_dir(repo_path)
                .args(["new", "-m", &descr])
                .args(parents)
        })
        .success();
    std::fs::write(repo_path.join(name), format!("{name}\n")).unwrap();
    test_env
        .run_jj_in(repo_path, ["bookmark", "create", "-r@", name])
        .success();
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, workspace_root: &Path) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description.first_line() ++ " " ++ bookmarks"#;
    test_env.run_jj_in(workspace_root, ["log", "-T", template, "-r", "all()"])
}

fn clone_git_remote_into(
    test_env: &TestEnvironment,
    upstream: &str,
    fork: &str,
) -> git2::Repository {
    let upstream_path = test_env.env_root().join(upstream);
    let fork_path = test_env.env_root().join(fork);
    let fork_repo = git2::Repository::init(fork_path).unwrap();
    {
        let mut upstream_remote = fork_repo
            .remote(upstream, upstream_path.to_str().unwrap())
            .unwrap();
        upstream_remote.fetch(&[upstream], None, None).unwrap();

        // create local branch mirroring the upstream
        let upstream_head = fork_repo
            .find_branch("upstream/upstream", git2::BranchType::Remote)
            .unwrap()
            .into_reference()
            .peel_to_commit()
            .unwrap();

        fork_repo.branch("upstream", &upstream_head, false).unwrap();
    }

    fork_repo
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_with_default_config(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin@origin: oputwtnw ffecd2d6 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_default_remote(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: oputwtnw ffecd2d6 message
      @origin: oputwtnw ffecd2d6 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_single_remote(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Hint: Fetching from the only existing remote: rem1
    bookmark: rem1@rem1 [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_single_remote_all_remotes_flag(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    test_env
        .jj_cmd(&repo_path, &["git", "fetch", "--all-remotes"])
        .assert()
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_single_remote_from_arg(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--remote", "rem1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_single_remote_from_config(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    test_env.add_config(r#"git.fetch = "rem1""#);

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_multiple_remotes(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");

    test_env
        .run_jj_in(
            &repo_path,
            ["git", "fetch", "--remote", "rem1", "--remote", "rem2"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    rem2: yszkquru 2497a8a0 message
      @rem2: yszkquru 2497a8a0 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_all_remotes(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");

    // add empty [remote "rem3"] section to .git/config, which should be ignored
    test_env
        .run_jj_in(&repo_path, ["git", "remote", "add", "rem3", "../unknown"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["git", "remote", "remove", "rem3"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--all-remotes"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    rem2: yszkquru 2497a8a0 message
      @rem2: yszkquru 2497a8a0 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_multiple_remotes_from_config(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");
    test_env.add_config(r#"git.fetch = ["rem1", "rem2"]"#);

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    rem2: yszkquru 2497a8a0 message
      @rem2: yszkquru 2497a8a0 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_nonexistent_remote(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    let output = test_env.run_jj_in(
        &repo_path,
        ["git", "fetch", "--remote", "rem1", "--remote", "rem2"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'rem2'
    [EOF]
    [exit status: 1]
    ");
    }
    insta::allow_duplicates! {
    // No remote should have been fetched as part of the failing transaction
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_nonexistent_remote_from_config(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");
    test_env.add_config(r#"git.fetch = ["rem1", "rem2"]"#);

    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No git remote named 'rem2'
    [EOF]
    [exit status: 1]
    ");
    }
    // No remote should have been fetched as part of the failing transaction
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_from_remote_named_git(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let repo_path = test_env.env_root().join("repo");
    init_git_remote(&test_env, "git");

    let git_repo = git2::Repository::init(&repo_path).unwrap();
    git_repo.remote("git", "../git").unwrap();

    // Existing remote named 'git' shouldn't block the repo initialization.
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();

    // Try fetching from the remote named 'git'.
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=git"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to import refs from underlying Git repo
    Caused by: Git remote named 'git' is reserved for local Git repository
    Hint: Run `jj git remote rename` to give different name.
    [EOF]
    [exit status: 1]
    ");
    }

    // Implicit import shouldn't fail because of the remote ref.
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "list", "--all-remotes"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @"");
    }

    // Explicit import is an error.
    // (This could be warning if we add mechanism to report ignored refs.)
    insta::allow_duplicates! {
    insta::assert_snapshot!(test_env.run_jj_in(&repo_path, ["git", "import"]), @r"
    ------- stderr -------
    Error: Failed to import refs from underlying Git repo
    Caused by: Git remote named 'git' is reserved for local Git repository
    Hint: Run `jj git remote rename` to give different name.
    [EOF]
    [exit status: 1]
    ");
    }

    // The remote can be renamed, and the ref can be imported.
    test_env
        .run_jj_in(&repo_path, ["git", "remote", "rename", "git", "bar"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "list", "--all-remotes"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    git: mrylzrtu 76fc7466 message
      @bar: mrylzrtu 76fc7466 message
      @git: mrylzrtu 76fc7466 message
    [EOF]
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_from_remote_with_slashes(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let repo_path = test_env.env_root().join("repo");
    init_git_remote(&test_env, "source");

    let git_repo = git2::Repository::init(&repo_path).unwrap();
    git_repo.remote("slash/origin", "../source").unwrap();

    // Existing remote with slash shouldn't block the repo initialization.
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo=."])
        .success();

    // Try fetching from the remote named 'git'.
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=slash/origin"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    Hint: Run `jj git remote rename` to give a different name.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_prune_before_updating_tips(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    let git_repo = add_git_remote(&test_env, &repo_path, "origin");
    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: oputwtnw ffecd2d6 message
      @origin: oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    // Remove origin bookmark in git repo and create origin/subname
    git_repo
        .find_branch("origin", git2::BranchType::Local)
        .unwrap()
        .rename("origin/subname", false)
        .unwrap();

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin/subname: oputwtnw ffecd2d6 message
      @origin: oputwtnw ffecd2d6 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_conflicting_bookmarks(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "rem1");

    // Create a rem1 bookmark locally
    test_env.run_jj_in(&repo_path, ["new", "root()"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "rem1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: kkmpptxz fcdbbd73 (empty) (no description set)
    [EOF]
    ");
    }

    test_env
        .run_jj_in(
            &repo_path,
            ["git", "fetch", "--remote", "rem1", "--branch", "glob:*"],
        )
        .success();
    // This should result in a CONFLICTED bookmark
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1 (conflicted):
      + kkmpptxz fcdbbd73 (empty) (no description set)
      + qxosxrvv 6a211027 message
      @rem1 (behind by 1 commits): qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_conflicting_bookmarks_colocated(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let repo_path = test_env.env_root().join("repo");
    let _git_repo = git2::Repository::init(&repo_path).unwrap();
    // create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &repo_path);
    test_env
        .run_jj_in(&repo_path, ["git", "init", "--git-repo", "."])
        .success();
    add_git_remote(&test_env, &repo_path, "rem1");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }

    // Create a rem1 bookmark locally
    test_env.run_jj_in(&repo_path, ["new", "root()"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "rem1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1: zsuskuln f652c321 (empty) (no description set)
      @git: zsuskuln f652c321 (empty) (no description set)
    [EOF]
    ");
    }

    test_env
        .run_jj_in(
            &repo_path,
            ["git", "fetch", "--remote", "rem1", "--branch", "rem1"],
        )
        .success();
    // This should result in a CONFLICTED bookmark
    // See https://github.com/jj-vcs/jj/pull/1146#discussion_r1112372340 for the bug this tests for.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    rem1 (conflicted):
      + zsuskuln f652c321 (empty) (no description set)
      + qxosxrvv 6a211027 message
      @git (behind by 1 commits): zsuskuln f652c321 (empty) (no description set)
      @rem1 (behind by 1 commits): qxosxrvv 6a211027 message
    [EOF]
    ");
    }
}

// Helper functions to test obtaining multiple bookmarks at once and changed
// bookmarks
fn create_colocated_repo_and_bookmarks_from_trunk1(
    test_env: &TestEnvironment,
    repo_path: &Path,
) -> String {
    // Create a colocated repo in `source` to populate it more easily
    test_env
        .run_jj_in(repo_path, ["git", "init", "--git-repo", "."])
        .success();
    create_commit(test_env, repo_path, "trunk1", &[]);
    create_commit(test_env, repo_path, "a1", &["trunk1"]);
    create_commit(test_env, repo_path, "a2", &["trunk1"]);
    create_commit(test_env, repo_path, "b", &["trunk1"]);
    format!(
        "   ===== Source git repo contents =====\n{}",
        get_log_output(test_env, repo_path)
    )
}

fn create_trunk2_and_rebase_bookmarks(test_env: &TestEnvironment, repo_path: &Path) -> String {
    create_commit(test_env, repo_path, "trunk2", &["trunk1"]);
    for br in ["a1", "a2", "b"] {
        test_env
            .run_jj_in(repo_path, ["rebase", "-b", br, "-d", "trunk2"])
            .success();
    }
    format!(
        "   ===== Source git repo contents =====\n{}",
        get_log_output(test_env, repo_path)
    )
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_all(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Nothing in our repo before the fetch
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @"");
    }
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin     [new] tracked
    bookmark: a2@origin     [new] tracked
    bookmark: b@origin      [new] tracked
    bookmark: trunk1@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    a1: nknoxmzm 359a9a02 descr_for_a1
      @origin: nknoxmzm 359a9a02 descr_for_a1
    a2: qkvnknrk decaa396 descr_for_a2
      @origin: qkvnknrk decaa396 descr_for_a2
    b: vpupmnsl c7d4bdcb descr_for_b
      @origin: vpupmnsl c7d4bdcb descr_for_b
    trunk1: zowqyktl ff36dc55 descr_for_trunk1
      @origin: zowqyktl ff36dc55 descr_for_trunk1
    [EOF]
    ");
        }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // ==== Change both repos ====
    // First, change the target repo:
    let source_log = create_trunk2_and_rebase_bookmarks(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    ○  babc49226c14 descr_for_b b
    │ ○  91e46b4b2653 descr_for_a2 a2
    ├─╯
    │ ○  0424f6dfc1ff descr_for_a1 a1
    ├─╯
    @  8f1f14fbbf42 descr_for_trunk2 trunk2
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }
    // Change a bookmark in the source repo as well, so that it becomes conflicted.
    test_env
        .run_jj_in(
            &target_jj_repo_path,
            ["describe", "b", "-m=new_descr_for_b_to_create_conflict"],
        )
        .success();

    // Our repo before and after fetch
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  061eddbb43ab new_descr_for_b_to_create_conflict b*
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    a1: nknoxmzm 359a9a02 descr_for_a1
      @origin: nknoxmzm 359a9a02 descr_for_a1
    a2: qkvnknrk decaa396 descr_for_a2
      @origin: qkvnknrk decaa396 descr_for_a2
    b: vpupmnsl 061eddbb new_descr_for_b_to_create_conflict
      @origin (ahead by 1 commits, behind by 1 commits): vpupmnsl hidden c7d4bdcb descr_for_b
    trunk1: zowqyktl ff36dc55 descr_for_trunk1
      @origin: zowqyktl ff36dc55 descr_for_trunk1
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin     [updated] tracked
    bookmark: a2@origin     [updated] tracked
    bookmark: b@origin      [updated] tracked
    bookmark: trunk2@origin [new] tracked
    Abandoned 2 commits that are no longer reachable.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    a1: quxllqov 0424f6df descr_for_a1
      @origin: quxllqov 0424f6df descr_for_a1
    a2: osusxwst 91e46b4b descr_for_a2
      @origin: osusxwst 91e46b4b descr_for_a2
    b (conflicted):
      - vpupmnsl hidden c7d4bdcb descr_for_b
      + vpupmnsl 061eddbb new_descr_for_b_to_create_conflict
      + vktnwlsu babc4922 descr_for_b
      @origin (behind by 1 commits): vktnwlsu babc4922 descr_for_b
    trunk1: zowqyktl ff36dc55 descr_for_trunk1
      @origin: zowqyktl ff36dc55 descr_for_trunk1
    trunk2: umznmzko 8f1f14fb descr_for_trunk2
      @origin: umznmzko 8f1f14fb descr_for_trunk2
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  babc49226c14 descr_for_b b?? b@origin
    │ │ ○  91e46b4b2653 descr_for_a2 a2
    │ ├─╯
    │ │ ○  0424f6dfc1ff descr_for_a1 a1
    │ ├─╯
    │ ○  8f1f14fbbf42 descr_for_trunk2 trunk2
    │ │ ○  061eddbb43ab new_descr_for_b_to_create_conflict b??
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_some_of_many_bookmarks(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Test an error message
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        ["git", "fetch", "--branch", "glob:^:a*"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Invalid branch pattern provided. When fetching, branch names and globs may not contain the characters `:`, `^`, `?`, `[`, `]`
    [EOF]
    [exit status: 1]
    ");
    }
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "a*"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Branch names may not include `*`.
    Hint: Prefix the pattern with `glob:` to expand `*` as a glob
    [EOF]
    [exit status: 1]
    ");
    }

    // Nothing in our repo before the fetch
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");
    }
    // Fetch one bookmark...
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "b"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: b@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    // ...check what the intermediate state looks like...
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    b: vpupmnsl c7d4bdcb descr_for_b
      @origin: vpupmnsl c7d4bdcb descr_for_b
    [EOF]
    ");
    }
    // ...then fetch two others with a glob.
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        ["git", "fetch", "--branch", "glob:a*"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin [new] tracked
    bookmark: a2@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  decaa3966c83 descr_for_a2 a2
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ │ ○  c7d4bdcbc215 descr_for_b b
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    // Fetching the same bookmark again
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "a1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  decaa3966c83 descr_for_a2 a2
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ │ ○  c7d4bdcbc215 descr_for_b b
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // ==== Change both repos ====
    // First, change the target repo:
    let source_log = create_trunk2_and_rebase_bookmarks(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    ○  01d115196c39 descr_for_b b
    │ ○  31c7d94b1f29 descr_for_a2 a2
    ├─╯
    │ ○  6df2d34cf0da descr_for_a1 a1
    ├─╯
    @  2bb3ebd2bba3 descr_for_trunk2 trunk2
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }
    // Change a bookmark in the source repo as well, so that it becomes conflicted.
    test_env
        .run_jj_in(
            &target_jj_repo_path,
            ["describe", "b", "-m=new_descr_for_b_to_create_conflict"],
        )
        .success();

    // Our repo before and after fetch of two bookmarks
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  6ebd41dc4f13 new_descr_for_b_to_create_conflict b*
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        ["git", "fetch", "--branch", "b", "--branch", "a1"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin [updated] tracked
    bookmark: b@origin  [updated] tracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  01d115196c39 descr_for_b b?? b@origin
    │ │ ○  6df2d34cf0da descr_for_a1 a1
    │ ├─╯
    │ ○  2bb3ebd2bba3 descr_for_trunk2
    │ │ ○  6ebd41dc4f13 new_descr_for_b_to_create_conflict b??
    │ ├─╯
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // We left a2 where it was before, let's see how `jj bookmark list` sees this.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    a1: ypowunwp 6df2d34c descr_for_a1
      @origin: ypowunwp 6df2d34c descr_for_a1
    a2: qkvnknrk decaa396 descr_for_a2
      @origin: qkvnknrk decaa396 descr_for_a2
    b (conflicted):
      - vpupmnsl hidden c7d4bdcb descr_for_b
      + vpupmnsl 6ebd41dc new_descr_for_b_to_create_conflict
      + nxrpswuq 01d11519 descr_for_b
      @origin (behind by 1 commits): nxrpswuq 01d11519 descr_for_b
    [EOF]
    ");
    }
    // Now, let's fetch a2 and double-check that fetching a1 and b again doesn't do
    // anything.
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        ["git", "fetch", "--branch", "b", "--branch", "glob:a*"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a2@origin [updated] tracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  31c7d94b1f29 descr_for_a2 a2
    │ │ ○  01d115196c39 descr_for_b b?? b@origin
    │ ├─╯
    │ │ ○  6df2d34cf0da descr_for_a1 a1
    │ ├─╯
    │ ○  2bb3ebd2bba3 descr_for_trunk2
    │ │ ○  6ebd41dc4f13 new_descr_for_b_to_create_conflict b??
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &target_jj_repo_path), @r"
    a1: ypowunwp 6df2d34c descr_for_a1
      @origin: ypowunwp 6df2d34c descr_for_a1
    a2: qrmzolkr 31c7d94b descr_for_a2
      @origin: qrmzolkr 31c7d94b descr_for_a2
    b (conflicted):
      - vpupmnsl hidden c7d4bdcb descr_for_b
      + vpupmnsl 6ebd41dc new_descr_for_b_to_create_conflict
      + nxrpswuq 01d11519 descr_for_b
      @origin (behind by 1 commits): nxrpswuq 01d11519 descr_for_b
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_bookmarks_some_missing(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");
    add_git_remote(&test_env, &repo_path, "rem1");
    add_git_remote(&test_env, &repo_path, "rem2");
    add_git_remote(&test_env, &repo_path, "rem3");

    // single missing bookmark, implicit remotes (@origin)
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--branch", "noexist"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No branch matching `noexist` found on any specified/configured remote
    Nothing changed.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }

    // multiple missing bookmarks, implicit remotes (@origin)
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git", "fetch", "--branch", "noexist1", "--branch", "noexist2",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No branch matching `noexist1` found on any specified/configured remote
    Warning: No branch matching `noexist2` found on any specified/configured remote
    Nothing changed.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }

    // single existing bookmark, implicit remotes (@origin)
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--branch", "origin"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: origin@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: oputwtnw ffecd2d6 message
      @origin: oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    // multiple existing bookmark, explicit remotes, each bookmark is only in one
    // remote.
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git", "fetch", "--branch", "rem1", "--branch", "rem2", "--branch", "rem3", "--remote",
            "rem1", "--remote", "rem2", "--remote", "rem3",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: rem1@rem1 [new] tracked
    bookmark: rem2@rem2 [new] tracked
    bookmark: rem3@rem3 [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
     insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
     origin: oputwtnw ffecd2d6 message
       @origin: oputwtnw ffecd2d6 message
     rem1: qxosxrvv 6a211027 message
       @rem1: qxosxrvv 6a211027 message
     rem2: yszkquru 2497a8a0 message
       @rem2: yszkquru 2497a8a0 message
     rem3: lvsrtwwm 4ffdff2b message
       @rem3: lvsrtwwm 4ffdff2b message
     [EOF]
     ");
    }

    // multiple bookmarks, one exists, one doesn't
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "git", "fetch", "--branch", "rem1", "--branch", "notexist", "--remote", "rem1",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No branch matching `notexist` found on any specified/configured remote
    Nothing changed.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: oputwtnw ffecd2d6 message
      @origin: oputwtnw ffecd2d6 message
    rem1: qxosxrvv 6a211027 message
      @rem1: qxosxrvv 6a211027 message
    rem2: yszkquru 2497a8a0 message
      @rem2: yszkquru 2497a8a0 message
    rem3: lvsrtwwm 4ffdff2b message
      @rem3: lvsrtwwm 4ffdff2b message
    [EOF]
    ");
    }
}

#[test]
fn test_git_fetch_bookmarks_missing_with_subprocess_localized_message() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    // "fatal: couldn't find remote ref %s" shouldn't be localized.
    let assert = test_env
        .jj_cmd(&repo_path, &["git", "fetch", "--branch=unknown"])
        // Initialize locale as "en_US" which is the most common.
        .env("LC_ALL", "en_US.UTF-8")
        // Set some other locale variables for testing.
        .env("LC_MESSAGES", "en_US.UTF-8")
        .env("LANG", "en_US.UTF-8")
        // GNU gettext prioritizes LANGUAGE if translation is enabled. It works
        // no matter if system locale exists or not.
        .env("LANGUAGE", "zh_TW")
        .assert()
        .success();
    let stderr = test_env.normalize_output(get_stderr_string(&assert));
    insta::assert_snapshot!(stderr, @r"
    Warning: No branch matching `unknown` found on any specified/configured remote
    Nothing changed.
    [EOF]
    ");
}

// See `test_undo_restore_commands.rs` for fetch-undo-push and fetch-undo-fetch
// of the same bookmarks for various kinds of undo.
#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_undo(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Fetch 2 bookmarks
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        ["git", "fetch", "--branch", "b", "--branch", "a1"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin [new] tracked
    bookmark: b@origin  [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&target_jj_repo_path, ["undo"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: eb2029853b02 (2001-02-03 08:05:18) fetch from git remote(s) origin
    [EOF]
    ");
    }
    // The undo works as expected
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    ◆  000000000000
    [EOF]
    ");
    }
    // Now try to fetch just one bookmark
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "b"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: b@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
}

// Compare to `test_git_import_undo` in test_git_import_export
// TODO: Explain why these behaviors are useful
#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_fetch_undo_what(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Initial state we will try to return to after `op restore`. There are no
    // bookmarks.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    }
    let base_operation_id = test_env.current_operation_id(&repo_path);

    // Fetch a bookmark
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--branch", "b"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: b@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    b: vpupmnsl c7d4bdcb descr_for_b
      @origin: vpupmnsl c7d4bdcb descr_for_b
    [EOF]
    ");
    }

    // We can undo the change in the repo without moving the remote-tracking
    // bookmark
    let output = test_env.run_jj_in(
        &repo_path,
        ["op", "restore", "--what", "repo", &base_operation_id],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    b (deleted)
      @origin: vpupmnsl hidden c7d4bdcb descr_for_b
    [EOF]
    ");
    }

    // Now, let's demo restoring just the remote-tracking bookmark. First, let's
    // change our local repo state...
    test_env
        .run_jj_in(&repo_path, ["bookmark", "c", "-r@", "newbookmark"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    b (deleted)
      @origin: vpupmnsl hidden c7d4bdcb descr_for_b
    newbookmark: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
    }
    // Restoring just the remote-tracking state will not affect `newbookmark`, but
    // will eliminate `b@origin`.
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "op",
            "restore",
            "--what",
            "remote-tracking",
            &base_operation_id,
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    newbookmark: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_remove_fetch(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
    }

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + oputwtnw ffecd2d6 message
      @origin (behind by 1 commits): oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    test_env
        .run_jj_in(&repo_path, ["git", "remote", "remove", "origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    test_env
        .run_jj_in(&repo_path, ["git", "remote", "add", "origin", "../origin"])
        .success();

    // Check that origin@origin is properly recreated
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: origin@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + oputwtnw ffecd2d6 message
      @origin (behind by 1 commits): oputwtnw ffecd2d6 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_rename_fetch(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    add_git_remote(&test_env, &repo_path, "origin");

    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
    }

    test_env.run_jj_in(&repo_path, ["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + oputwtnw ffecd2d6 message
      @origin (behind by 1 commits): oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "rename", "origin", "upstream"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    origin (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + oputwtnw ffecd2d6 message
      @upstream (behind by 1 commits): oputwtnw ffecd2d6 message
    [EOF]
    ");
    }

    // Check that jj indicates that nothing has changed
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote", "upstream"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_removed_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Fetch all bookmarks
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin     [new] tracked
    bookmark: a2@origin     [new] tracked
    bookmark: b@origin      [new] tracked
    bookmark: trunk1@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // Remove a2 bookmark in origin
    test_env
        .run_jj_in(
            &source_git_repo_path,
            ["bookmark", "forget", "--include-remotes", "a2"],
        )
        .success();

    // Fetch bookmark a1 from origin and check that a2 is still there
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "a1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // Fetch bookmarks a2 from origin, and check that it has been removed locally
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch", "--branch", "a2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a2@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_removed_parent_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let source_git_repo_path = test_env.env_root().join("source");
    let _git_repo = git2::Repository::init(source_git_repo_path.clone()).unwrap();

    // Clone an empty repo. The target repo is a normal `jj` repo, *not* colocated
    let output = test_env.run_jj_in(test_env.env_root(), ["git", "clone", "source", "target"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/target"
    Nothing changed.
    [EOF]
    "#);
    }
    let target_jj_repo_path = test_env.env_root().join("target");

    let source_log =
        create_colocated_repo_and_bookmarks_from_trunk1(&test_env, &source_git_repo_path);
    insta::allow_duplicates! {
    insta::assert_snapshot!(source_log, @r"
       ===== Source git repo contents =====
    @  c7d4bdcbc215 descr_for_b b
    │ ○  decaa3966c83 descr_for_a2 a2
    ├─╯
    │ ○  359a9a02457d descr_for_a1 a1
    ├─╯
    ○  ff36dc55760e descr_for_trunk1 trunk1
    ◆  000000000000
    [EOF]
    ");
    }

    // Fetch all bookmarks
    let output = test_env.run_jj_in(&target_jj_repo_path, ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin     [new] tracked
    bookmark: a2@origin     [new] tracked
    bookmark: b@origin      [new] tracked
    bookmark: trunk1@origin [new] tracked
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ │ ○  359a9a02457d descr_for_a1 a1
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1 trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }

    // Remove all bookmarks in origin.
    test_env
        .run_jj_in(
            &source_git_repo_path,
            ["bookmark", "forget", "--include-remotes", "glob:*"],
        )
        .success();

    // Fetch bookmarks master, trunk1 and a1 from origin and check that only those
    // bookmarks have been removed and that others were not rebased because of
    // abandoned commits.
    let output = test_env.run_jj_in(
        &target_jj_repo_path,
        [
            "git", "fetch", "--branch", "master", "--branch", "trunk1", "--branch", "a1",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a1@origin     [deleted] untracked
    bookmark: trunk1@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    Warning: No branch matching `master` found on any specified/configured remote
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &target_jj_repo_path), @r"
    @  230dd059e1b0
    │ ○  c7d4bdcbc215 descr_for_b b
    │ │ ○  decaa3966c83 descr_for_a2 a2
    │ ├─╯
    │ ○  ff36dc55760e descr_for_trunk1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_remote_only_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    // Create non-empty git repo to add as a remote
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    let signature = git2_signature();
    let mut tree_builder = git_repo.treebuilder(None).unwrap();
    let file_oid = git_repo.blob(b"content").unwrap();
    tree_builder
        .insert("file", file_oid, git2::FileMode::Blob.into())
        .unwrap();
    let tree_oid = tree_builder.write().unwrap();
    let tree = git_repo.find_tree(tree_oid).unwrap();
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();
    // Create a commit and a bookmark in the git repo
    git_repo
        .commit(
            Some("refs/heads/feature1"),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap();

    // Fetch using git.auto_local_bookmark = true
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    [EOF]
    ");
    }

    git_repo
        .commit(
            Some("refs/heads/feature2"),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap();

    // Fetch using git.auto_local_bookmark = false
    test_env.add_config("git.auto-local-bookmark = false");
    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  230dd059e1b0
    │ ◆  9f01a0e04879 message feature1 feature2@origin
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    feature2@origin: mzyxwzks 9f01a0e0 message
    [EOF]
    ");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_fetch_preserve_commits_across_repos(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    let upstream_repo = add_git_remote(&test_env, &repo_path, "upstream");

    let fork_path = test_env.env_root().join("fork");
    let fork_repo = clone_git_remote_into(&test_env, "upstream", "fork");
    test_env
        .run_jj_in(&repo_path, ["git", "remote", "add", "fork", "../fork"])
        .success();

    // add commit to fork remote in another branch
    add_commit_to_branch(&fork_repo, "feature");

    // fetch remote bookmarks
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "fetch", "--remote=fork", "--remote=upstream"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  230dd059e1b0
    │ ○  e386ce0e4690 message feature
    ├─╯
    │ ○  05ae9cbbe5c7 message upstream
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature: nwtolyry e386ce0e message
      @fork: nwtolyry e386ce0e message
    upstream: tzqqlonq 05ae9cbb message
      @fork: tzqqlonq 05ae9cbb message
      @upstream: tzqqlonq 05ae9cbb message
    [EOF]
    ");
    }

    // merge fork/feature into the upstream/upstream
    let mut fork_remote = upstream_repo
        .remote("fork", fork_path.to_str().unwrap())
        .unwrap();
    fork_remote.fetch(&["feature"], None, None).unwrap();
    let merge_base = upstream_repo
        .find_branch("upstream", git2::BranchType::Local)
        .unwrap()
        .into_reference()
        .peel_to_commit()
        .unwrap();
    let merge_target = upstream_repo
        .find_branch("fork/feature", git2::BranchType::Remote)
        .unwrap()
        .into_reference()
        .peel_to_commit()
        .unwrap();
    let merge_oid = upstream_repo.index().unwrap().write_tree().unwrap();
    let merge_tree = upstream_repo.find_tree(merge_oid).unwrap();
    let signature = git2_signature();
    upstream_repo
        .commit(
            Some("refs/heads/upstream"),
            &signature,
            &signature,
            "merge",
            &merge_tree,
            &[&merge_base, &merge_target],
        )
        .unwrap();

    // remove branch on the fork
    let mut branch = fork_repo
        .find_branch("feature", git2::BranchType::Local)
        .unwrap();
    branch.delete().unwrap();

    // fetch again on the jj repo, first looking at fork and then at upstream
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "fetch", "--remote=fork", "--remote=upstream"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  230dd059e1b0
    │ ○    407a9966fc22 merge upstream*
    │ ├─╮
    │ │ ○  e386ce0e4690 message
    ├───╯
    │ ○  05ae9cbbe5c7 message upstream@fork
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    upstream: qzsuxvvx 407a9966 merge
      @fork (behind by 2 commits): tzqqlonq 05ae9cbb message
      @upstream: qzsuxvvx 407a9966 merge
    [EOF]
    ");
    }
}
