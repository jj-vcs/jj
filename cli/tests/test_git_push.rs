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

use std::path::Path;
use std::path::PathBuf;

use test_case::test_case;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;

fn set_up() -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
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
        .run_jj_in(&origin_path, ["bookmark", "create", "-r@", "bookmark1"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["new", "root()", "-m=description 2"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "create", "-r@", "bookmark2"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    test_env
        .run_jj_in(
            ".",
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

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_nothing(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    // Show the setup. `insta` has trouble if this is done inside `set_up()`
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }
    // No bookmarks to push yet
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_current_bookmark(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Update some bookmarks. `bookmark1` is not a current bookmark, but
    // `bookmark2` and `my-bookmark` are.
    test_env
        .run_jj_in(
            &workspace_root,
            ["describe", "bookmark1", "-m", "modified bookmark1 commit"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark2"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark2", "-r@"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    // Check the setup
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: xtvrqkyv 0f8dc656 (empty) modified bookmark1 commit
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv hidden d13ecdbd (empty) description 1
    bookmark2: yostqsxw bc7610b6 (empty) foo
      @origin (behind by 1 commits): rlzusymt 8476341e (empty) description 2
    my-bookmark: yostqsxw bc7610b6 (empty) foo
    [EOF]
    ");
    }
    // First dry-run. `bookmark1` should not get pushed.
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to bc7610b65a91
      Add bookmark my-bookmark to bc7610b65a91
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to bc7610b65a91
      Add bookmark my-bookmark to bc7610b65a91
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: xtvrqkyv 0f8dc656 (empty) modified bookmark1 commit
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv hidden d13ecdbd (empty) description 1
    bookmark2: yostqsxw bc7610b6 (empty) foo
      @origin: yostqsxw bc7610b6 (empty) foo
    my-bookmark: yostqsxw bc7610b6 (empty) foo
      @origin: yostqsxw bc7610b6 (empty) foo
    [EOF]
    ");
    }

    // Try pushing backwards
    test_env
        .run_jj_in(
            &workspace_root,
            [
                "bookmark",
                "set",
                "bookmark2",
                "-rbookmark2-",
                "--allow-backwards",
            ],
        )
        .success();
    // This behavior is a strangeness of our definition of the default push revset.
    // We could consider changing it.
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
    // We can move a bookmark backwards
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-bbookmark2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move backward bookmark bookmark2 from bc7610b65a91 to 8476341eb395
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_parent_bookmark(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    test_env
        .run_jj_in(&workspace_root, ["edit", "bookmark1"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["describe", "-m", "modified bookmark1 commit"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "non-empty description"])
        .success();
    std::fs::write(workspace_root.join("file"), "file").unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to e612d524a5c6
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_matching_bookmark(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_matching_bookmark_unchanged(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark1"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
}

/// Test that `jj git push` without arguments pushes a bookmark to the specified
/// remote even if it's already up to date on another remote
/// (`remote_bookmarks(remote=<remote>)..@` vs. `remote_bookmarks()..@`).
#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_other_remote_has_bookmark(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Create another remote (but actually the same)
    let other_remote_path = test_env
        .env_root()
        .join("origin")
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");
    test_env
        .run_jj_in(
            &workspace_root,
            [
                "git",
                "remote",
                "add",
                "other",
                other_remote_path.to_str().unwrap(),
            ],
        )
        .success();
    // Modify bookmark1 and push it to `origin`
    test_env
        .run_jj_in(&workspace_root, ["edit", "bookmark1"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m=modified"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to a657f1b61b94
    [EOF]
    ");
    }
    // Since it's already pushed to origin, nothing will happen if push again
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
    // The bookmark was moved on the "other" remote as well (since it's actually the
    // same remote), but `jj` is not aware of that since it thinks this is a
    // different remote. So, the push should fail.
    //
    // But it succeeds! That's because the bookmark is created at the same location
    // as it is on the remote. This would also work for a descendant.
    //
    // TODO: Saner test?
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--remote=other"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to other:
      Add bookmark bookmark1 to a657f1b61b94
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_forward_unexpectedly_moved(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Move bookmark1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env
        .run_jj_in(&origin_path, ["new", "bookmark1", "-m=remote"])
        .success();
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "set", "bookmark1", "-r@"])
        .success();
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    // Move bookmark1 forward to another commit locally
    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark1", "-m=local"])
        .success();
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark1", "-r@"])
        .success();

    // Pushing should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark1 from d13ecdbda2a2 to 6750425ff51c
    Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
    Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_sideways_unexpectedly_moved(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Move bookmark1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env
        .run_jj_in(&origin_path, ["new", "bookmark1", "-m=remote"])
        .success();
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "set", "bookmark1", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &origin_path), @r"
    bookmark1: vruxwmqv 80284bec remote
      @git (behind by 1 commits): qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    // Move bookmark1 sideways to another commit locally
    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-m=local"])
        .success();
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "set", "bookmark1", "--allow-backwards", "-r@"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: kmkuslsw 0f8bf988 local
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to 0f8bf988588e
    Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
    Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
    [EOF]
    [exit status: 1]
    ");
    }
}

// This tests whether the push checks that the remote bookmarks are in expected
// positions.
#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deletion_unexpectedly_moved(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Move bookmark1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env
        .run_jj_in(&origin_path, ["new", "bookmark1", "-m=remote"])
        .success();
    std::fs::write(origin_path.join("remote"), "remote").unwrap();
    test_env
        .run_jj_in(&origin_path, ["bookmark", "set", "bookmark1", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &origin_path), @r"
    bookmark1: vruxwmqv 80284bec remote
      @git (behind by 1 commits): qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    // Delete bookmark1 locally
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1 (deleted)
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--bookmark", "bookmark1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
    Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
    Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_unexpectedly_deleted(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Delete bookmark1 forward on the remote
    let origin_path = test_env.env_root().join("origin");
    test_env
        .run_jj_in(&origin_path, ["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &origin_path), @r"
    bookmark1 (deleted)
      @git: qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    test_env
        .run_jj_in(&origin_path, ["git", "export"])
        .success();

    // Move bookmark1 sideways to another commit locally
    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-m=local"])
        .success();
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "set", "bookmark1", "--allow-backwards", "-r@"],
        )
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: kpqxywon 1ebe27ba local
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    // Pushing a moved bookmark fails if deleted on remote
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to 1ebe27ba04bf
    Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
    Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
    [EOF]
    [exit status: 1]
    ");
    }

    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1 (deleted)
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    if subprocess {
        // git does not allow to push a deleted bookmark if we expect it to exist even
        // though it was already deleted
        let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-bbookmark1"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        // Pushing a *deleted* bookmark succeeds if deleted on remote, even if we expect
        // bookmark1@origin to exist and point somewhere.
        let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-bbookmark1"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        [EOF]
        ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_creation_unexpectedly_already_exists(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Forget bookmark1 locally
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "forget", "--include-remotes", "bookmark1"],
        )
        .success();

    // Create a new branh1
    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-m=new bookmark1"])
        .success();
    std::fs::write(workspace_root.join("local"), "local").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: yostqsxw cb17dcdc new bookmark1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark1 to cb17dcdc74d5
    Error: Refusing to push a bookmark that unexpectedly moved on the remote. Affected refs: refs/heads/bookmark1
    Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_locally_created_and_rewritten(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    // Ensure that remote bookmarks aren't tracked automatically
    test_env.add_config("git.auto-local-bookmark = false");

    // Push locally-created bookmark
    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-mlocal 1"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "my"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refusing to create new remote bookmark my@origin
    Hint: Use --allow-new to push new bookmark. Use --remote to specify the remote to push to.
    Nothing changed.
    [EOF]
    ");
    }
    // Either --allow-new or git.push-new-bookmarks=true should work
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my to fcc999921ce9
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--config=git.push-new-bookmarks=true"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my to fcc999921ce9
    [EOF]
    ");
    }

    // Rewrite it and push again, which would fail if the pushed bookmark weren't
    // set to "tracking"
    test_env
        .run_jj_in(&workspace_root, ["describe", "-mlocal 2"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    my: vruxwmqv 423bb660 (empty) local 2
      @origin (ahead by 1 commits, behind by 1 commits): vruxwmqv hidden fcc99992 (empty) local 1
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark my from fcc999921ce9 to 423bb66069e7
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_multiple(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark1"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "set", "--allow-backwards", "bookmark2", "-r@"],
        )
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    // Check the setup
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1 (deleted)
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: yqosqzyt c4a3c310 (empty) foo
      @origin (ahead by 1 commits, behind by 1 commits): rlzusymt 8476341e (empty) description 2
    my-bookmark: yqosqzyt c4a3c310 (empty) foo
    [EOF]
    ");
    }
    // First dry-run
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
      Move sideways bookmark bookmark2 from 8476341eb395 to c4a3c3105d92
      Add bookmark my-bookmark to c4a3c3105d92
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Dry run requesting two specific bookmarks
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "-b=bookmark1",
            "-b=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
      Add bookmark my-bookmark to c4a3c3105d92
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Dry run requesting two specific bookmarks twice
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "-b=bookmark1",
            "-b=my-bookmark",
            "-b=bookmark1",
            "-b=glob:my-*",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
      Add bookmark my-bookmark to c4a3c3105d92
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Dry run with glob pattern
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "-b=glob:bookmark?", "--dry-run"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
      Move sideways bookmark bookmark2 from 8476341eb395 to c4a3c3105d92
    Dry-run requested, not pushing.
    [EOF]
    ");
    }

    // Unmatched bookmark name is error
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-b=foo"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: foo
    [EOF]
    [exit status: 1]
    ");
    }
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "-b=foo", "-b=glob:?bookmark"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching bookmarks for patterns: foo, ?bookmark
    [EOF]
    [exit status: 1]
    ");
    }

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
      Move sideways bookmark bookmark2 from 8476341eb395 to c4a3c3105d92
      Add bookmark my-bookmark to c4a3c3105d92
    [EOF]
    ");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark2: yqosqzyt c4a3c310 (empty) foo
      @origin: yqosqzyt c4a3c310 (empty) foo
    my-bookmark: yqosqzyt c4a3c310 (empty) foo
      @origin: yqosqzyt c4a3c310 (empty) foo
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["log", "-rall()"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:17 bookmark2 my-bookmark c4a3c310
    │  (empty) foo
    │ ○  rlzusymt test.user@example.com 2001-02-03 08:05:10 8476341e
    ├─╯  (empty) description 2
    │ ○  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 d13ecdbd
    ├─╯  (empty) description 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_changes(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "bar"])
        .success();
    std::fs::write(workspace_root.join("file"), "modified").unwrap();

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--change", "@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Creating bookmark push-yostqsxwqrlt for revision yostqsxwqrlt
    Changes to push to origin:
      Add bookmark push-yostqsxwqrlt to cf1a53a8800a
    [EOF]
    ");
    }
    // test pushing two changes at once
    std::fs::write(workspace_root.join("file"), "modified2").unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-c=(@|@-)"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `(@|@-)` resolved to more than one revision
    Hint: The revset `(@|@-)` resolved to these revisions:
      yostqsxw 16c16966 push-yostqsxwqrlt* | bar
      yqosqzyt a050abf4 foo
    Hint: Prefix the expression with `all:` to allow any number of revisions (i.e. `all:(@|@-)`).
    [EOF]
    [exit status: 1]
    ");
    }
    // test pushing two changes at once, part 2
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-c=all:(@|@-)"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Creating bookmark push-yqosqzytrlsw for revision yqosqzytrlsw
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from cf1a53a8800a to 16c169664e9f
      Add bookmark push-yqosqzytrlsw to a050abf4ff07
    [EOF]
    ");
    }
    // specifying the same change twice doesn't break things
    std::fs::write(workspace_root.join("file"), "modified3").unwrap();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "-c=all:(@|@)"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from 16c169664e9f to ef6313d50ac1
    [EOF]
    ");
    }

    // specifying the same bookmark with --change/--bookmark doesn't break things
    std::fs::write(workspace_root.join("file"), "modified4").unwrap();
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "-c=@", "-b=push-yostqsxwqrlt"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from ef6313d50ac1 to c1e65d3a64ce
    [EOF]
    ");
    }

    // try again with --change that moves the bookmark forward
    std::fs::write(workspace_root.join("file"), "modified5").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            [
                "bookmark",
                "set",
                "-r=@-",
                "--allow-backwards",
                "push-yostqsxwqrlt",
            ],
        )
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy : yostqsxw 38cb417c bar
    Parent commit: yqosqzyt a050abf4 push-yostqsxwqrlt* push-yqosqzytrlsw | foo
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "-c=@", "-b=push-yostqsxwqrlt"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from c1e65d3a64ce to 38cb417ce3a6
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy : yostqsxw 38cb417c push-yostqsxwqrlt | bar
    Parent commit: yqosqzyt a050abf4 push-yqosqzytrlsw | foo
    [EOF]
    ");
    }

    // Test changing `git.push-bookmark-prefix`. It causes us to push again.
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--config=git.push-bookmark-prefix=test-",
            "--change=@",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Creating bookmark test-yostqsxwqrlt for revision yostqsxwqrlt
    Changes to push to origin:
      Add bookmark test-yostqsxwqrlt to 38cb417ce3a6
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_revisions(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "bar"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "bookmark-1"])
        .success();
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "baz"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "bookmark-2a"],
        )
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "bookmark-2b"],
        )
        .success();
    std::fs::write(workspace_root.join("file"), "modified again").unwrap();

    // Push an empty set
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new", "-r=none()"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks point to the specified revisions: none()
    Nothing changed.
    [EOF]
    ");
    }
    // Push a revision with no bookmarks
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new", "-r=@--"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks point to the specified revisions: @--
    Nothing changed.
    [EOF]
    ");
    }
    // Push a revision with a single bookmark
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "-r=@-", "--dry-run"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark-1 to 5f432a855e59
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Push multiple revisions of which some have bookmarks
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "-r=@--", "-r=@-", "--dry-run"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks point to the specified revisions: @--
    Changes to push to origin:
      Add bookmark bookmark-1 to 5f432a855e59
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Push a revision with a multiple bookmarks
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "-r=@", "--dry-run"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark-2a to 84f499037f5c
      Add bookmark bookmark-2b to 84f499037f5c
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    // Repeating a commit doesn't result in repeated messages about the bookmark
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "-r=@-", "-r=@-", "--dry-run"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bookmark-1 to 5f432a855e59
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_mixed(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "bar"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "bookmark-1"])
        .success();
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "baz"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "bookmark-2a"],
        )
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "bookmark-2b"],
        )
        .success();
    std::fs::write(workspace_root.join("file"), "modified again").unwrap();

    // --allow-new is not implied for --bookmark=.. and -r=..
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--change=@--",
            "--bookmark=bookmark-1",
            "-r=@",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Creating bookmark push-yqosqzytrlsw for revision yqosqzytrlsw
    Error: Refusing to create new remote bookmark bookmark-1@origin
    Hint: Use --allow-new to push new bookmark. Use --remote to specify the remote to push to.
    [EOF]
    [exit status: 1]
    ");
    }

    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--change=@--",
            "--bookmark=bookmark-1",
            "-r=@",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Creating bookmark push-yqosqzytrlsw for revision yqosqzytrlsw
    Changes to push to origin:
      Add bookmark push-yqosqzytrlsw to a050abf4ff07
      Add bookmark bookmark-1 to 5f432a855e59
      Add bookmark bookmark-2a to 84f499037f5c
      Add bookmark bookmark-2b to 84f499037f5c
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_existing_long_bookmark(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            [
                "bookmark",
                "create",
                "-r@",
                "push-19b790168e73f7a73a98deae21e807c0",
            ],
        )
        .success();

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--change=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark push-19b790168e73f7a73a98deae21e807c0 to a050abf4ff07
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_unsnapshotted_change(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["git", "push", "--change", "@"])
        .success();
    std::fs::write(workspace_root.join("file"), "modified").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["git", "push", "--change", "@"])
        .success();
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_conflict(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    std::fs::write(workspace_root.join("file"), "first").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["commit", "-m", "first"])
        .success();
    std::fs::write(workspace_root.join("file"), "second").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["commit", "-m", "second"])
        .success();
    std::fs::write(workspace_root.join("file"), "third").unwrap();
    test_env
        .run_jj_in(&workspace_root, ["rebase", "-r", "@", "-d", "@--"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m", "third"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit e2221a796300 since it has conflicts
    Hint: Rejected commit: yostqsxw e2221a79 my-bookmark | (conflict) third
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_description(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m="])
        .success();
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark", "my-bookmark"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 5b36783cd11c since it has no description
    Hint: Rejected commit: yqosqzyt 5b36783c my-bookmark | (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
    }
    test_env
        .run_jj_in(
            &workspace_root,
            [
                "git",
                "push",
                "--allow-new",
                "--bookmark",
                "my-bookmark",
                "--allow-empty-description",
            ],
        )
        .success();
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_description_in_immutable(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "imm"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["describe", "-m="])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();

    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 5b36783cd11c since it has no description
    Hint: Rejected commit: yqosqzyt 5b36783c imm | (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
    }

    test_env.add_config(r#"revset-aliases."immutable_heads()" = "imm""#);
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my-bookmark to ea7373507ad9
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_author(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .run_jj_with(|cmd| cmd.current_dir(&workspace_root).args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["new", "root()", "-m=initial"]);
    run_without_var("JJ_USER", &["bookmark", "create", "-r@", "missing-name"]);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark", "missing-name"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 944313939bbd since it has no author and/or committer set
    Hint: Rejected commit: vruxwmqv 94431393 missing-name | (empty) initial
    [EOF]
    [exit status: 1]
    ");
    }
    run_without_var("JJ_EMAIL", &["new", "root()", "-m=initial"]);
    run_without_var("JJ_EMAIL", &["bookmark", "create", "-r@", "missing-email"]);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark=missing-email"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 59354714f789 since it has no author and/or committer set
    Hint: Rejected commit: kpqxywon 59354714 missing-email | (empty) initial
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_author_in_immutable(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .run_jj_with(|cmd| cmd.current_dir(&workspace_root).args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["new", "root()", "-m=no author name"]);
    run_without_var("JJ_EMAIL", &["new", "-m=no author email"]);
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "imm"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();

    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 011f740bf8b5 since it has no author and/or committer set
    Hint: Rejected commit: yostqsxw 011f740b imm | (empty) no author email
    [EOF]
    [exit status: 1]
    ");
    }

    test_env.add_config(r#"revset-aliases."immutable_heads()" = "imm""#);
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my-bookmark to 68fdae89de4f
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_committer(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .run_jj_with(|cmd| cmd.current_dir(&workspace_root).args(args).env_remove(var))
            .success();
    };
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "missing-name"],
        )
        .success();
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark=missing-name"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 4fd190283d1a since it has no author and/or committer set
    Hint: Rejected commit: yqosqzyt 4fd19028 missing-name | (empty) no committer name
    [EOF]
    [exit status: 1]
    ");
    }
    test_env
        .run_jj_in(&workspace_root, ["new", "root()"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "missing-email"],
        )
        .success();
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark=missing-email"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit eab97428a6ec since it has no author and/or committer set
    Hint: Rejected commit: kpqxywon eab97428 missing-email | (empty) no committer email
    [EOF]
    [exit status: 1]
    ");
    }

    // Test message when there are multiple reasons (missing committer and
    // description)
    run_without_var("JJ_EMAIL", &["describe", "-m=", "missing-email"]);
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark=missing-email"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 1143ed607f54 since it has no description and it has no author and/or committer set
    Hint: Rejected commit: kpqxywon 1143ed60 missing-email | (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_committer_in_immutable(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let run_without_var = |var: &str, args: &[&str]| {
        test_env
            .run_jj_with(|cmd| cmd.current_dir(&workspace_root).args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    test_env.run_jj_in(&workspace_root, ["new"]).success();
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "imm"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "foo"])
        .success();
    std::fs::write(workspace_root.join("file"), "contents").unwrap();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "create", "-r@", "my-bookmark"],
        )
        .success();

    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 7e61dc727a8f since it has no author and/or committer set
    Hint: Rejected commit: yostqsxw 7e61dc72 imm | (empty) no committer email
    [EOF]
    [exit status: 1]
    ");
    }

    test_env.add_config(r#"revset-aliases."immutable_heads()" = "imm""#);
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "git",
            "push",
            "--allow-new",
            "--bookmark=my-bookmark",
            "--dry-run",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my-bookmark to c79f85e90b4a
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deleted(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark1"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["log", "-rall()"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:13 5b36783c
    │  (empty) (no description set)
    │ ○  rlzusymt test.user@example.com 2001-02-03 08:05:10 bookmark2 8476341e
    ├─╯  (empty) description 2
    │ ○  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 d13ecdbd
    ├─╯  (empty) description 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_conflicting_bookmarks(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env.add_config("git.auto-local-bookmark = true");
    let git_repo = {
        let mut git_repo_path = workspace_root.clone();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git::open(&git_repo_path)
    };

    // Forget remote ref, move local ref, then fetch to create conflict.
    git_repo
        .find_reference("refs/remotes/origin/bookmark2")
        .unwrap()
        .delete()
        .unwrap();
    test_env
        .run_jj_in(&workspace_root, ["git", "import"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-m=description 3"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "bookmark2"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["git", "fetch"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2 (conflicted):
      + yostqsxw 8e670e2d (empty) description 3
      + rlzusymt 8476341e (empty) description 2
      @origin (behind by 1 commits): rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let bump_bookmark1 = || {
        test_env
            .run_jj_in(&workspace_root, ["new", "bookmark1", "-m=bump"])
            .success();
        test_env
            .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark1", "-r@"])
            .success();
    };

    // Conflicting bookmark at @
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Bookmark bookmark2 is conflicted
    Hint: Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up.
    Nothing changed.
    [EOF]
    ");
    }

    // --bookmark should be blocked by conflicting bookmark
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--allow-new", "--bookmark", "bookmark2"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark bookmark2 is conflicted
    Hint: Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up.
    [EOF]
    [exit status: 1]
    ");
    }

    // --all shouldn't be blocked by conflicting bookmark
    bump_bookmark1();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Bookmark bookmark2 is conflicted
    Hint: Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up.
    Changes to push to origin:
      Move forward bookmark bookmark1 from d13ecdbda2a2 to 8df52121b022
    [EOF]
    ");
    }

    // --revisions shouldn't be blocked by conflicting bookmark
    bump_bookmark1();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new", "-rall()"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Bookmark bookmark2 is conflicted
    Hint: Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up.
    Changes to push to origin:
      Move forward bookmark bookmark1 from 8df52121b022 to 345e1f64a64d
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deleted_untracked(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    // Absent local bookmark shouldn't be considered "deleted" compared to
    // non-tracking remote bookmark.
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark1"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--bookmark=bookmark1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: bookmark1
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_tracked_vs_all(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark1", "-mmoved bookmark1"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark1", "-r@"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark2", "-mmoved bookmark2"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "delete", "bookmark2"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "untrack", "bookmark1@origin"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "create", "-r@", "bookmark3"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: vruxwmqv db059e3f (empty) moved bookmark1
    bookmark1@origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2 (deleted)
      @origin: rlzusymt 8476341e (empty) description 2
    bookmark3: znkkpsqq 1aa4f1f2 (empty) moved bookmark2
    [EOF]
    ");
    }

    // At this point, only bookmark2 is still tracked. `jj git push --tracked` would
    // try to push it and no other bookmarks.
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--tracked", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark2 from 8476341eb395
    Dry-run requested, not pushing.
    [EOF]
    ");
    }

    // Untrack the last remaining tracked bookmark.
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "untrack", "bookmark2@origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&test_env, &workspace_root), @r"
    bookmark1: vruxwmqv db059e3f (empty) moved bookmark1
    bookmark1@origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2@origin: rlzusymt 8476341e (empty) description 2
    bookmark3: znkkpsqq 1aa4f1f2 (empty) moved bookmark2
    [EOF]
    ");
    }

    // Now, no bookmarks are tracked. --tracked does not push anything
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--tracked"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }

    // All bookmarks are still untracked.
    // - --all tries to push bookmark1, but fails because a bookmark with the same
    // name exist on the remote.
    // - --all succeeds in pushing bookmark3, since there is no bookmark of the same
    // name on the remote.
    // - It does not try to push bookmark2.
    //
    // TODO: Not trying to push bookmark2 could be considered correct, or perhaps
    // we want to consider this as a deletion of the bookmark that failed because
    // the bookmark was untracked. In the latter case, an error message should be
    // printed. Some considerations:
    // - Whatever we do should be consistent with what `jj bookmark list` does; it
    //   currently does *not* list bookmarks like bookmark2 as "about to be
    //   deleted", as can be seen above.
    // - We could consider showing some hint on `jj bookmark untrack
    //   bookmark2@origin` instead of showing an error here.
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Non-tracking remote bookmark bookmark1@origin exists
    Hint: Run `jj bookmark track bookmark1@origin` to import the remote bookmark.
    Changes to push to origin:
      Add bookmark bookmark3 to 1aa4f1f2ef7f
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_moved_forward_untracked(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    test_env
        .run_jj_in(&workspace_root, ["new", "bookmark1", "-mmoved bookmark1"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark1", "-r@"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Non-tracking remote bookmark bookmark1@origin exists
    Hint: Run `jj bookmark track bookmark1@origin` to import the remote bookmark.
    Nothing changed.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_moved_sideways_untracked(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    test_env
        .run_jj_in(&workspace_root, ["new", "root()", "-mmoved bookmark1"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["bookmark", "set", "--allow-backwards", "bookmark1", "-r@"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--allow-new"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Non-tracking remote bookmark bookmark1@origin exists
    Hint: Run `jj bookmark track bookmark1@origin` to import the remote bookmark.
    Nothing changed.
    [EOF]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_to_remote_named_git(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = {
        let mut git_repo_path = workspace_root.clone();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git_repo_path
    };
    git::rename_remote(&git_repo_path, "origin", "git");

    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--all", "--remote=git"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to git:
      Add bookmark bookmark1 to d13ecdbda2a2
      Add bookmark bookmark2 to 8476341eb395
    Error: Git remote named 'git' is reserved for local Git repository
    Hint: Run `jj git remote rename` to give a different name.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_to_remote_with_slashes(subprocess: bool) {
    let (test_env, workspace_root) = set_up();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = {
        let mut git_repo_path = workspace_root.clone();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git_repo_path
    };
    git::rename_remote(&git_repo_path, "origin", "slash/origin");

    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "push", "--all", "--remote=slash/origin"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to slash/origin:
      Add bookmark bookmark1 to d13ecdbda2a2
      Add bookmark bookmark2 to 8476341eb395
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    Hint: Run `jj git remote rename` to give a different name.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[test]
fn test_git_push_sign_on_push() {
    let (test_env, workspace_root) = set_up();
    let template = r#"
    separate("\n",
      description.first_line(),
      if(signature,
        separate(", ",
          "Signature: " ++ signature.display(),
          "Status: " ++ signature.status(),
          "Key: " ++ signature.key(),
        )
      )
    )
    "#;
    test_env
        .run_jj_in(
            &workspace_root,
            ["new", "bookmark2", "-m", "commit to be signed 1"],
        )
        .success();
    test_env
        .run_jj_in(&workspace_root, ["new", "-m", "commit to be signed 2"])
        .success();
    test_env
        .run_jj_in(&workspace_root, ["bookmark", "set", "bookmark2", "-r@"])
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["new", "-m", "commit which should not be signed 1"],
        )
        .success();
    test_env
        .run_jj_in(
            &workspace_root,
            ["new", "-m", "commit which should not be signed 2"],
        )
        .success();
    // There should be no signed commits initially
    let output = test_env.run_jj_in(&workspace_root, ["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  commit which should not be signed 2
    ○  commit which should not be signed 1
    ○  commit to be signed 2
    ○  commit to be signed 1
    ○  description 2
    │ ○  description 1
    ├─╯
    ◆
    [EOF]
    ");
    test_env.add_config(
        r#"
    signing.backend = "test"
    signing.key = "impeccable"
    git.sign-on-push = true
    "#,
    );
    let output = test_env.run_jj_in(&workspace_root, ["git", "push", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to 8710e91a14a1
    Dry-run requested, not pushing.
    [EOF]
    ");
    // There should be no signed commits after performing a dry run
    let output = test_env.run_jj_in(&workspace_root, ["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  commit which should not be signed 2
    ○  commit which should not be signed 1
    ○  commit to be signed 2
    ○  commit to be signed 1
    ○  description 2
    │ ○  description 1
    ├─╯
    ◆
    [EOF]
    ");
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Updated signatures of 2 commits
    Rebased 2 descendant commits
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to a6259c482040
    Working copy now at: kmkuslsw b5f47345 (empty) commit which should not be signed 2
    Parent commit      : kpqxywon 90df08d3 (empty) commit which should not be signed 1
    [EOF]
    ");
    // Only commits which are being pushed should be signed
    let output = test_env.run_jj_in(&workspace_root, ["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  commit which should not be signed 2
    ○  commit which should not be signed 1
    ○  commit to be signed 2
    │  Signature: test-display, Status: good, Key: impeccable
    ○  commit to be signed 1
    │  Signature: test-display, Status: good, Key: impeccable
    ○  description 2
    │ ○  description 1
    ├─╯
    ◆
    [EOF]
    ");

    // Immutable commits should not be signed
    let output = test_env.run_jj_in(
        &workspace_root,
        [
            "bookmark",
            "create",
            "bookmark3",
            "-r",
            "description('commit which should not be signed 1')",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to kpqxywon 90df08d3 bookmark3 | (empty) commit which should not be signed 1
    [EOF]
    ");
    let output = test_env.run_jj_in(
        &workspace_root,
        ["bookmark", "move", "bookmark2", "--to", "bookmark3"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kpqxywon 90df08d3 bookmark2* bookmark3 | (empty) commit which should not be signed 1
    [EOF]
    ");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "bookmark3""#);
    let output = test_env.run_jj_in(&workspace_root, ["git", "push"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refusing to create new remote bookmark bookmark3@origin
    Hint: Use --allow-new to push new bookmark. Use --remote to specify the remote to push to.
    Changes to push to origin:
      Move forward bookmark bookmark2 from a6259c482040 to 90df08d3d612
    [EOF]
    ");
    let output = test_env.run_jj_in(&workspace_root, ["log", "-T", template, "-r", "::"]);
    insta::assert_snapshot!(output, @r"
    @  commit which should not be signed 2
    ◆  commit which should not be signed 1
    ◆  commit to be signed 2
    │  Signature: test-display, Status: good, Key: impeccable
    ◆  commit to be signed 1
    │  Signature: test-display, Status: good, Key: impeccable
    ◆  description 2
    │ ○  description 1
    ├─╯
    ◆
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    test_env.run_jj_in(repo_path, &["bookmark", "list", "--all-remotes", "--quiet"])
}
