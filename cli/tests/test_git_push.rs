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

use test_case::test_case;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

fn git_repo_dir_for_jj_repo(work_dir: &TestWorkDir<'_>) -> std::path::PathBuf {
    work_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git")
}

fn set_up(test_env: &TestEnvironment) {
    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = git_repo_dir_for_jj_repo(&origin_dir);

    origin_dir
        .run_jj(["describe", "-m=description 1"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark1"])
        .success();
    origin_dir
        .run_jj(["new", "root()", "-m=description 2"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark2"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

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
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_nothing(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    // Show the setup. `insta` has trouble if this is done inside `set_up()`
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }
    // No bookmarks to push yet
    let output = work_dir.run_jj(["git", "push", "--all"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_current_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Update some bookmarks. `bookmark1` is not a current bookmark, but
    // `bookmark2` and `my-bookmark` are.
    work_dir
        .run_jj(["describe", "bookmark1", "-m", "modified bookmark1 commit"])
        .success();
    work_dir.run_jj(["new", "bookmark2"]).success();
    work_dir
        .run_jj(["bookmark", "set", "bookmark2", "-r@"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    // Check the setup
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: xtvrqkyv 0f8dc656 (empty) modified bookmark1 commit
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv hidden d13ecdbd (empty) description 1
    bookmark2: yostqsxw bc7610b6 (empty) foo
      @origin (behind by 1 commits): rlzusymt 8476341e (empty) description 2
    my-bookmark: yostqsxw bc7610b6 (empty) foo
    [EOF]
    ");
    }
    // First dry-run. `bookmark1` should not get pushed.
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--dry-run"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new"]);
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
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
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
    work_dir
        .run_jj([
            "bookmark",
            "set",
            "bookmark2",
            "-rbookmark2-",
            "--allow-backwards",
        ])
        .success();
    // This behavior is a strangeness of our definition of the default push revset.
    // We could consider changing it.
    let output = work_dir.run_jj(["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
    // We can move a bookmark backwards
    let output = work_dir.run_jj(["git", "push", "-bbookmark2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move backward bookmark bookmark2 from bc7610b65a91 to 8476341eb395
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_parent_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    work_dir.run_jj(["edit", "bookmark1"]).success();
    work_dir
        .run_jj(["describe", "-m", "modified bookmark1 commit"])
        .success();
    work_dir
        .run_jj(["new", "-m", "non-empty description"])
        .success();
    work_dir.write_file("file", "file");
    let output = work_dir.run_jj(["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to e612d524a5c6
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_matching_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks found in the default push revset: remote_bookmarks(remote=origin)..@
    Nothing changed.
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_matching_bookmark_unchanged(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["new", "bookmark1"]).success();
    let output = work_dir.run_jj(["git", "push"]);
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
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_other_remote_has_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    // Create another remote (but actually the same)
    let other_remote_path = test_env
        .env_root()
        .join("origin")
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "other",
            other_remote_path.to_str().unwrap(),
        ])
        .success();
    // Modify bookmark1 and push it to `origin`
    work_dir.run_jj(["edit", "bookmark1"]).success();
    work_dir.run_jj(["describe", "-m=modified"]).success();
    let output = work_dir.run_jj(["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark bookmark1 from d13ecdbda2a2 to a657f1b61b94
    [EOF]
    ");
    }
    // Since it's already pushed to origin, nothing will happen if push again
    let output = work_dir.run_jj(["git", "push"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--remote=other"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to other:
      Add bookmark bookmark1 to a657f1b61b94
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_forward_unexpectedly_moved(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Move bookmark1 forward on the remote
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["new", "bookmark1", "-m=remote"])
        .success();
    origin_dir.write_file("remote", "remote");
    origin_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

    // Move bookmark1 forward to another commit locally
    work_dir.run_jj(["new", "bookmark1", "-m=local"]).success();
    work_dir.write_file("local", "local");
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();

    // Pushing should fail
    let output = work_dir.run_jj(["git", "push"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move forward bookmark bookmark1 from d13ecdbda2a2 to 6750425ff51c
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move forward bookmark bookmark1 from d13ecdbda2a2 to 6750425ff51c
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_sideways_unexpectedly_moved(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Move bookmark1 forward on the remote
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["new", "bookmark1", "-m=remote"])
        .success();
    origin_dir.write_file("remote", "remote");
    origin_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&origin_dir), @r"
    bookmark1: vruxwmqv 80284bec remote
      @git (behind by 1 commits): qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    origin_dir.run_jj(["git", "export"]).success();

    // Move bookmark1 sideways to another commit locally
    work_dir.run_jj(["new", "root()", "-m=local"]).success();
    work_dir.write_file("local", "local");
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "--allow-backwards", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: kmkuslsw 0f8bf988 local
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = work_dir.run_jj(["git", "push"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move sideways bookmark bookmark1 from d13ecdbda2a2 to 0f8bf988588e
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move sideways bookmark bookmark1 from d13ecdbda2a2 to 0f8bf988588e
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }
}

// This tests whether the push checks that the remote bookmarks are in expected
// positions.
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deletion_unexpectedly_moved(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Move bookmark1 forward on the remote
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["new", "bookmark1", "-m=remote"])
        .success();
    origin_dir.write_file("remote", "remote");
    origin_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&origin_dir), @r"
    bookmark1: vruxwmqv 80284bec remote
      @git (behind by 1 commits): qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    origin_dir.run_jj(["git", "export"]).success();

    // Delete bookmark1 locally
    work_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1 (deleted)
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = work_dir.run_jj(["git", "push", "--bookmark", "bookmark1"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_unexpectedly_deleted(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Delete bookmark1 forward on the remote
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&origin_dir), @r"
    bookmark1 (deleted)
      @git: qpvuntsm d13ecdbd (empty) description 1
    bookmark2: zsuskuln 8476341e (empty) description 2
      @git: zsuskuln 8476341e (empty) description 2
    [EOF]
    ");
    }
    origin_dir.run_jj(["git", "export"]).success();

    // Move bookmark1 sideways to another commit locally
    work_dir.run_jj(["new", "root()", "-m=local"]).success();
    work_dir.write_file("local", "local");
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "--allow-backwards", "-r@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: kpqxywon 1ebe27ba local
      @origin (ahead by 1 commits, behind by 1 commits): xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    // Pushing a moved bookmark fails if deleted on remote
    let output = work_dir.run_jj(["git", "push"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move sideways bookmark bookmark1 from d13ecdbda2a2 to 1ebe27ba04bf
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move sideways bookmark bookmark1 from d13ecdbda2a2 to 1ebe27ba04bf
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }

    work_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
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
        let output = work_dir.run_jj(["git", "push", "-bbookmark1"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        // Pushing a *deleted* bookmark succeeds if deleted on remote, even if we expect
        // bookmark1@origin to exist and point somewhere.
        let output = work_dir.run_jj(["git", "push", "-bbookmark1"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Delete bookmark bookmark1 from d13ecdbda2a2
        [EOF]
        ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_creation_unexpectedly_already_exists(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Forget bookmark1 locally
    work_dir
        .run_jj(["bookmark", "forget", "--include-remotes", "bookmark1"])
        .success();

    // Create a new branh1
    work_dir
        .run_jj(["new", "root()", "-m=new bookmark1"])
        .success();
    work_dir.write_file("local", "local");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: yostqsxw cb17dcdc new bookmark1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");
    }

    let output = work_dir.run_jj(["git", "push", "--allow-new"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark bookmark1 to cb17dcdc74d5
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark bookmark1 to cb17dcdc74d5
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/bookmark1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_locally_created_and_rewritten(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    // Ensure that remote bookmarks aren't tracked automatically
    test_env.add_config("git.auto-local-bookmark = false");

    // Push locally-created bookmark
    work_dir.run_jj(["new", "root()", "-mlocal 1"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my"])
        .success();
    let output = work_dir.run_jj(["git", "push"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark my to fcc999921ce9
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--config=git.push-new-bookmarks=true"]);
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
    work_dir.run_jj(["describe", "-mlocal 2"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    my: vruxwmqv 423bb660 (empty) local 2
      @origin (ahead by 1 commits, behind by 1 commits): vruxwmqv hidden fcc99992 (empty) local 1
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark my from fcc999921ce9 to 423bb66069e7
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_multiple(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "--allow-backwards", "bookmark2", "-r@"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    // Check the setup
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1 (deleted)
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: yqosqzyt c4a3c310 (empty) foo
      @origin (ahead by 1 commits, behind by 1 commits): rlzusymt 8476341e (empty) description 2
    my-bookmark: yqosqzyt c4a3c310 (empty) foo
    [EOF]
    ");
    }
    // First dry-run
    let output = work_dir.run_jj(["git", "push", "--all", "--deleted", "--dry-run"]);
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
    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "-b=bookmark1",
        "-b=my-bookmark",
        "--dry-run",
    ]);
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
    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "-b=bookmark1",
        "-b=my-bookmark",
        "-b=bookmark1",
        "-b=glob:my-*",
        "--dry-run",
    ]);
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
    let output = work_dir.run_jj(["git", "push", "-b=glob:bookmark?", "--dry-run"]);
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
    let output = work_dir.run_jj(["git", "push", "-b=foo"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: foo
    [EOF]
    [exit status: 1]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "-b=foo", "-b=glob:?bookmark"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching bookmarks for patterns: foo, ?bookmark
    [EOF]
    [exit status: 1]
    ");
    }

    // --deleted is required to push deleted bookmarks even with --all
    let output = work_dir.run_jj(["git", "push", "--all", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refusing to push deleted bookmark bookmark1
    Hint: Push deleted bookmarks with --deleted or forget the bookmark to suppress this warning.
    Changes to push to origin:
      Move sideways bookmark bookmark2 from 8476341eb395 to c4a3c3105d92
      Add bookmark my-bookmark to c4a3c3105d92
    Dry-run requested, not pushing.
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--all", "--deleted", "--dry-run"]);
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

    let output = work_dir.run_jj(["git", "push", "--all", "--deleted"]);
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
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark2: yqosqzyt c4a3c310 (empty) foo
      @origin: yqosqzyt c4a3c310 (empty) foo
    my-bookmark: yqosqzyt c4a3c310 (empty) foo
      @origin: yqosqzyt c4a3c310 (empty) foo
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["log", "-rall()"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_changes(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["new", "-m", "bar"]).success();
    work_dir.write_file("file", "modified");

    let output = work_dir.run_jj(["git", "push", "--change", "@"]);
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
    work_dir.write_file("file", "modified2");
    let output = work_dir.run_jj(["git", "push", "-c=(@|@-)"]);
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
    let output = work_dir.run_jj(["git", "push", "-c=all:(@|@-)"]);
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
    work_dir.write_file("file", "modified3");
    let output = work_dir.run_jj(["git", "push", "-c=all:(@|@)"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from 16c169664e9f to ef6313d50ac1
    [EOF]
    ");
    }

    // specifying the same bookmark with --change/--bookmark doesn't break things
    work_dir.write_file("file", "modified4");
    let output = work_dir.run_jj(["git", "push", "-c=@", "-b=push-yostqsxwqrlt"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move sideways bookmark push-yostqsxwqrlt from ef6313d50ac1 to c1e65d3a64ce
    [EOF]
    ");
    }

    // try again with --change that could move the bookmark forward
    work_dir.write_file("file", "modified5");
    work_dir
        .run_jj([
            "bookmark",
            "set",
            "-r=@-",
            "--allow-backwards",
            "push-yostqsxwqrlt",
        ])
        .success();
    let output = work_dir.run_jj(["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy  (@) : yostqsxw 38cb417c bar
    Parent commit (@-): yqosqzyt a050abf4 push-yostqsxwqrlt* push-yqosqzytrlsw | foo
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "-c=@", "-b=push-yostqsxwqrlt"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: push-yostqsxwqrlt
    Hint: Use 'jj bookmark move' to move it, and 'jj git push -b push-yostqsxwqrlt [--allow-new]' to push it
    [EOF]
    [exit status: 1]
    ");
    }
    let output = work_dir.run_jj(["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy  (@) : yostqsxw 38cb417c bar
    Parent commit (@-): yqosqzyt a050abf4 push-yostqsxwqrlt* push-yqosqzytrlsw | foo
    [EOF]
    ");
    }

    // Test changing `git.push-bookmark-prefix`. It causes us to push again.
    let output = work_dir.run_jj([
        "git",
        "push",
        "--config=git.push-bookmark-prefix=test-",
        "--change=@",
    ]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_changes_with_name(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["new", "-m", "pushed"]).success();
    work_dir.write_file("file", "modified");

    // Normal behavior.
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark b1 to 3e677c129c1d
    [EOF]
    ");
    }
    // Spaces before the = sign are treated like part of the bookmark name and such
    // bookmarks cannot be pushed.
    let output = work_dir.run_jj(["git", "push", "--named", "b1 = @"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Could not parse 'b1 ' as a bookmark name
    Caused by:
    1: Failed to parse bookmark name: Syntax error
    2:  --> 1:3
      |
    1 | b1 
      |   ^---
      |
      = expected <EOI>
    Hint: For example, `--named myfeature=@` is valid syntax
    [EOF]
    [exit status: 2]
    ");
    }
    // test pushing a change with an empty name
    let output = work_dir.run_jj(["git", "push", "--named", "=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Argument '=@' must have the form NAME=REVISION, with both NAME and REVISION non-empty
    Hint: For example, `--named myfeature=@` is valid syntax
    [EOF]
    [exit status: 2]
    ");
    }
    // Unparsable name
    let output = work_dir.run_jj(["git", "push", "--named", ":!:=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Could not parse ':!:' as a bookmark name
    Caused by:
    1: Failed to parse bookmark name: Syntax error
    2:  --> 1:1
      |
    1 | :!:
      | ^---
      |
      = expected <identifier>, <string_literal>, or <raw_string_literal>
    Hint: For example, `--named myfeature=@` is valid syntax
    [EOF]
    [exit status: 2]
    ");
    }
    // test pushing a change with an empty revision
    let output = work_dir.run_jj(["git", "push", "--named", "b2="]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Argument 'b2=' must have the form NAME=REVISION, with both NAME and REVISION non-empty
    Hint: For example, `--named myfeature=@` is valid syntax
    [EOF]
    [exit status: 2]
    ");
    }
    // test pushing a change with no equals sign
    let output = work_dir.run_jj(["git", "push", "--named", "b2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Argument 'b2' must include '=' and have the form NAME=REVISION
    Hint: For example, `--named myfeature=@` is valid syntax
    [EOF]
    [exit status: 2]
    ");
    }

    // test pushing the same change with the same name again
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: b1
    Hint: Use 'jj bookmark move' to move it, and 'jj git push -b b1 [--allow-new]' to push it
    [EOF]
    [exit status: 1]
    ");
    }
    // test pushing two changes at once
    work_dir.write_file("file", "modified2");
    let output = work_dir.run_jj(["git", "push", "--named=b2=all:(@|@-)"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `all:(@|@-)` resolved to more than one revision
    Hint: The revset `all:(@|@-)` resolved to these revisions:
      yostqsxw 101e6730 b1* | pushed
      yqosqzyt a050abf4 foo
    [EOF]
    [exit status: 1]
    ");
    }

    // specifying the same bookmark with --named/--bookmark
    work_dir.write_file("file", "modified4");
    let output = work_dir.run_jj(["git", "push", "--named=b2=@", "-b=b2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark b2 to 477da21559d5
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_changes_with_name_deleted_tracked(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    // Unset immutable_heads so that untracking branches does not move the working
    // copy
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let work_dir = test_env.work_dir("local");
    // Create a second empty remote `another_remote`
    test_env
        .run_jj_in(".", ["git", "init", "another_remote"])
        .success();
    let another_remote_git_repo_path =
        git_repo_dir_for_jj_repo(&test_env.work_dir("another_remote"));
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "another_remote",
            another_remote_git_repo_path.to_str().unwrap(),
        ])
        .success();
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["new", "-m", "pushed"]).success();
    work_dir.write_file("file", "modified");
    // Normal push as part of the test setup
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark b1 to fd39fc9ddae4
    [EOF]
    ");
    }
    work_dir.run_jj(["bookmark", "delete", "b1"]).success();

    // Test the setup
    let output = work_dir
        .run_jj(["bookmark", "list", "--all", "b1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    b1 (deleted)
      @origin: kpqxywon fd39fc9d pushed
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");
    }

    // Can't push `b1` with --named to the same or another remote if it's deleted
    // locally and still tracked on `origin`
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@", "--remote=another_remote"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Tracked remote bookmarks exist for deleted bookmark: b1
    Hint: Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack 'glob:b1@*'` to disassociate them.
    [EOF]
    [exit status: 1]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@", "--remote=origin"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Tracked remote bookmarks exist for deleted bookmark: b1
    Hint: Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack 'glob:b1@*'` to disassociate them.
    [EOF]
    [exit status: 1]
    ");
    }

    // OK to push to a different remote once the bookmark is no longer tracked on
    // `origin`
    work_dir
        .run_jj(["bookmark", "untrack", "b1@origin"])
        .success();
    let output = work_dir
        .run_jj(["bookmark", "list", "--all", "b1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    b1@origin: kpqxywon fd39fc9d pushed
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@", "--remote=another_remote"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to another_remote:
      Add bookmark b1 to fd39fc9ddae4
    [EOF]
    ");
    }
    let output = work_dir
        .run_jj(["bookmark", "list", "--all", "b1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    b1: kpqxywon fd39fc9d pushed
      @another_remote: kpqxywon fd39fc9d pushed
    b1@origin: kpqxywon fd39fc9d pushed
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_changes_with_name_untracked_or_forgotten(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    // Unset immutable_heads so that untracking branches does not move the working
    // copy
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    work_dir.run_jj(["describe", "-m", "parent"]).success();
    work_dir.run_jj(["new", "-m", "pushed_to_remote"]).success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["new", "-m", "child", "--no-edit"])
        .success();
    work_dir.write_file("file", "modified");

    // Push a branch to a remote, but forget the local branch
    work_dir
        .run_jj(["git", "push", "--named", "b1=@"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "b1@origin"])
        .success();
    work_dir.run_jj(["bookmark", "delete", "b1"]).success();

    let output = work_dir
        .run_jj(&[
            "log",
            "-r=::@+",
            r#"-T=separate(" ", commit_id.shortest(3), bookmarks, description)"#,
        ])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ○  08f child
    @  ec9 b1@origin pushed_to_remote
    ○  57e parent
    ◆  000
    [EOF]
    ");
    }
    let output = work_dir
        .run_jj(["bookmark", "list", "--all", "b1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    b1@origin: yostqsxw ec992a1a pushed_to_remote
    [EOF]
    ");
    }

    let output = work_dir.run_jj(["git", "push", "--named", "b1=@"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Non-tracking remote bookmark b1@origin exists
    Hint: Run `jj bookmark track b1@origin` to import the remote bookmark.
    [EOF]
    [exit status: 1]
    ");
    }

    let output = work_dir.run_jj(["git", "push", "--named", "b1=@+"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Non-tracking remote bookmark b1@origin exists
    Hint: Run `jj bookmark track b1@origin` to import the remote bookmark.
    [EOF]
    [exit status: 1]
    ");
    }

    // The bookmarked is still pushed to the remote, but let's entirely forget
    // it. In other words, let's forget the remote-tracking bookmarks.
    work_dir
        .run_jj(&["bookmark", "forget", "b1", "--include-remotes"])
        .success();
    let output = work_dir
        .run_jj(["bookmark", "list", "--all", "b1"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @"");
    }

    // Make sure push still errors if we try to push a bookmark with the same name
    // to a different location.
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@-"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark b1 to 57ec90f54125
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/b1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    } else {
        // For libgit2, the error is the same but missing the "reason: stale info"
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark b1 to 57ec90f54125
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/b1
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
    }

    // The bookmark is still forgotten
    let output = work_dir.run_jj(["bookmark", "list", "--all", "b1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @"");
    }
    // This command behaves differently. In libgit2 flow, a conflict between a
    // revision and its descendant is resolved in favor of the latter, like in `jj
    // git fetch`. In subprocess flow, this logic is not implemented. As far as `jj
    // git push --named` goes, the stricter behavior of the subprocess flow is
    // probably slightly better.
    let output = work_dir.run_jj(["git", "push", "--named", "b1=@+"]);
    if subprocess {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark b1 to 08fcdf4055ae
        Error: Failed to push some bookmarks
        Hint: The following references unexpectedly moved on the remote:
          refs/heads/b1 (reason: stale info)
        Hint: Try fetching from the remote, then make the bookmark point to where you want it to be, and push again.
        [EOF]
        [exit status: 1]
        ");
        // In this case, pushing the bookmark to the same location where it already is
        // succeeds. TODO: This seems pretty safe, but perhaps it should still show
        // an error or some sort of warning?
        let output = work_dir.run_jj(["git", "push", "--named", "b1=@"]);
        insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark b1 to ec992a1a9381
        [EOF]
        ");
        }
    } else {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Add bookmark b1 to 08fcdf4055ae
        [EOF]
        ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_revisions(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["new", "-m", "bar"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-1"])
        .success();
    work_dir.write_file("file", "modified");
    work_dir.run_jj(["new", "-m", "baz"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-2a"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-2b"])
        .success();
    work_dir.write_file("file", "modified again");

    // Push an empty set
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=none()"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks point to the specified revisions: none()
    Nothing changed.
    [EOF]
    ");
    }
    // Push a revision with no bookmarks
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=@--"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No bookmarks point to the specified revisions: @--
    Nothing changed.
    [EOF]
    ");
    }
    // Push a revision with a single bookmark
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=@-", "--dry-run"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=@--", "-r=@-", "--dry-run"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=@", "--dry-run"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-r=@-", "-r=@-", "--dry-run"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_mixed(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["new", "-m", "bar"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-1"])
        .success();
    work_dir.write_file("file", "modified");
    work_dir.run_jj(["new", "-m", "baz"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-2a"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark-2b"])
        .success();
    work_dir.write_file("file", "modified again");

    // --allow-new is not implied for --bookmark=.. and -r=..
    let output = work_dir.run_jj([
        "git",
        "push",
        "--change=@--",
        "--bookmark=bookmark-1",
        "-r=@",
    ]);
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

    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--change=@--",
        "--bookmark=bookmark-1",
        "-r=@",
    ]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_unsnapshotted_change(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.run_jj(["describe", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["git", "push", "--change", "@"]).success();
    work_dir.write_file("file", "modified");
    work_dir.run_jj(["git", "push", "--change", "@"]).success();
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_conflict(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir.write_file("file", "first");
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.write_file("file", "second");
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.write_file("file", "third");
    work_dir
        .run_jj(["rebase", "-r", "@", "-d", "@--"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m", "third"]).success();
    let output = work_dir.run_jj(["git", "push", "--all"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_description(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m="]).success();
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark", "my-bookmark"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 5b36783cd11c since it has no description
    Hint: Rejected commit: yqosqzyt 5b36783c my-bookmark | (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
    }
    work_dir
        .run_jj([
            "git",
            "push",
            "--allow-new",
            "--bookmark",
            "my-bookmark",
            "--allow-empty-description",
        ])
        .success();
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_no_description_in_immutable(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "imm"])
        .success();
    work_dir.run_jj(["describe", "-m="]).success();
    work_dir.run_jj(["new", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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
    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_author(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let run_without_var = |var: &str, args: &[&str]| {
        work_dir
            .run_jj_with(|cmd| cmd.args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["new", "root()", "-m=initial"]);
    run_without_var("JJ_USER", &["bookmark", "create", "-r@", "missing-name"]);
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark", "missing-name"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark=missing-email"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_author_in_immutable(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let run_without_var = |var: &str, args: &[&str]| {
        work_dir
            .run_jj_with(|cmd| cmd.args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["new", "root()", "-m=no author name"]);
    run_without_var("JJ_EMAIL", &["new", "-m=no author email"]);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "imm"])
        .success();
    work_dir.run_jj(["new", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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
    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_committer(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let run_without_var = |var: &str, args: &[&str]| {
        work_dir
            .run_jj_with(|cmd| cmd.args(args).env_remove(var))
            .success();
    };
    work_dir
        .run_jj(["bookmark", "create", "-r@", "missing-name"])
        .success();
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark=missing-name"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Won't push commit 4fd190283d1a since it has no author and/or committer set
    Hint: Rejected commit: yqosqzyt 4fd19028 missing-name | (empty) no committer name
    [EOF]
    [exit status: 1]
    ");
    }
    work_dir.run_jj(["new", "root()"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "missing-email"])
        .success();
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark=missing-email"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark=missing-email"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_missing_committer_in_immutable(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let run_without_var = |var: &str, args: &[&str]| {
        work_dir
            .run_jj_with(|cmd| cmd.args(args).env_remove(var))
            .success();
    };
    run_without_var("JJ_USER", &["describe", "-m=no committer name"]);
    work_dir.run_jj(["new"]).success();
    run_without_var("JJ_EMAIL", &["describe", "-m=no committer email"]);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "imm"])
        .success();
    work_dir.run_jj(["new", "-m", "foo"]).success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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
    let output = work_dir.run_jj([
        "git",
        "push",
        "--allow-new",
        "--bookmark=my-bookmark",
        "--dry-run",
    ]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deleted(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    let output = work_dir.run_jj(["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Delete bookmark bookmark1 from d13ecdbda2a2
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["log", "-rall()"]);
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
    let output = work_dir.run_jj(["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_conflicting_bookmarks(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    test_env.add_config("git.auto-local-bookmark = true");
    let git_repo = {
        let mut git_repo_path = work_dir.root().to_owned();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git::open(&git_repo_path)
    };

    // Forget remote ref, move local ref, then fetch to create conflict.
    git_repo
        .find_reference("refs/remotes/origin/bookmark2")
        .unwrap()
        .delete()
        .unwrap();
    work_dir.run_jj(["git", "import"]).success();
    work_dir
        .run_jj(["new", "root()", "-m=description 3"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark2"])
        .success();
    work_dir.run_jj(["git", "fetch"]).success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
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
        work_dir.run_jj(["new", "bookmark1", "-m=bump"]).success();
        work_dir
            .run_jj(["bookmark", "set", "bookmark1", "-r@"])
            .success();
    };

    // Conflicting bookmark at @
    let output = work_dir.run_jj(["git", "push", "--allow-new"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "--bookmark", "bookmark2"]);
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
    let output = work_dir.run_jj(["git", "push", "--all"]);
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
    let output = work_dir.run_jj(["git", "push", "--allow-new", "-rall()"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_deleted_untracked(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    // Absent local bookmark shouldn't be considered "deleted" compared to
    // non-tracking remote bookmark.
    work_dir
        .run_jj(["bookmark", "delete", "bookmark1"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = work_dir.run_jj(["git", "push", "--deleted"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--bookmark=bookmark1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: bookmark1
    [EOF]
    [exit status: 1]
    ");
    }
}

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_tracked_vs_all(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    work_dir
        .run_jj(["new", "bookmark1", "-mmoved bookmark1"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();
    work_dir
        .run_jj(["new", "bookmark2", "-mmoved bookmark2"])
        .success();
    work_dir
        .run_jj(["bookmark", "delete", "bookmark2"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "bookmark1@origin"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark3"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: vruxwmqv db059e3f (empty) moved bookmark1
    bookmark1@origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2 (deleted)
      @origin: rlzusymt 8476341e (empty) description 2
    bookmark3: znkkpsqq 1aa4f1f2 (empty) moved bookmark2
    [EOF]
    ");
    }

    // At this point, only bookmark2 is still tracked.
    // `jj git push --tracked --deleted` would try to push it and no other
    // bookmarks.
    let output = work_dir.run_jj(["git", "push", "--tracked", "--dry-run"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refusing to push deleted bookmark bookmark2
    Hint: Push deleted bookmarks with --deleted or forget the bookmark to suppress this warning.
    Nothing changed.
    [EOF]
    ");
    }
    let output = work_dir.run_jj(["git", "push", "--tracked", "--deleted", "--dry-run"]);
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
    work_dir
        .run_jj(["bookmark", "untrack", "bookmark2@origin"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: vruxwmqv db059e3f (empty) moved bookmark1
    bookmark1@origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2@origin: rlzusymt 8476341e (empty) description 2
    bookmark3: znkkpsqq 1aa4f1f2 (empty) moved bookmark2
    [EOF]
    ");
    }

    // Now, no bookmarks are tracked. --tracked does not push anything
    let output = work_dir.run_jj(["git", "push", "--tracked"]);
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
    let output = work_dir.run_jj(["git", "push", "--all"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_moved_forward_untracked(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir
        .run_jj(["new", "bookmark1", "-mmoved bookmark1"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "bookmark1", "-r@"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = work_dir.run_jj(["git", "push", "--allow-new"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_moved_sideways_untracked(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");

    work_dir
        .run_jj(["new", "root()", "-mmoved bookmark1"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "--allow-backwards", "bookmark1", "-r@"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "bookmark1@origin"])
        .success();
    let output = work_dir.run_jj(["git", "push", "--allow-new"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_to_remote_named_git(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let git_repo_path = {
        let mut git_repo_path = work_dir.root().to_owned();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git_repo_path
    };
    git::rename_remote(&git_repo_path, "origin", "git");

    let output = work_dir.run_jj(["git", "push", "--all", "--remote=git"]);
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

#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_push_to_remote_with_slashes(subprocess: bool) {
    let test_env = TestEnvironment::default().with_git_subprocess(subprocess);
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    let git_repo_path = {
        let mut git_repo_path = work_dir.root().to_owned();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git_repo_path
    };
    git::rename_remote(&git_repo_path, "origin", "slash/origin");

    let output = work_dir.run_jj(["git", "push", "--all", "--remote=slash/origin"]);
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
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
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
    work_dir
        .run_jj(["new", "bookmark2", "-m", "commit to be signed 1"])
        .success();
    work_dir
        .run_jj(["new", "-m", "commit to be signed 2"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "bookmark2", "-r@"])
        .success();
    work_dir
        .run_jj(["new", "-m", "commit which should not be signed 1"])
        .success();
    work_dir
        .run_jj(["new", "-m", "commit which should not be signed 2"])
        .success();
    // There should be no signed commits initially
    let output = work_dir.run_jj(["log", "-T", template]);
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
    let output = work_dir.run_jj(["git", "push", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to 8710e91a14a1
    Dry-run requested, not pushing.
    [EOF]
    ");
    // There should be no signed commits after performing a dry run
    let output = work_dir.run_jj(["log", "-T", template]);
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
    let output = work_dir.run_jj(["git", "push"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Updated signatures of 2 commits
    Rebased 2 descendant commits
    Changes to push to origin:
      Move forward bookmark bookmark2 from 8476341eb395 to a6259c482040
    Working copy  (@) now at: kmkuslsw b5f47345 (empty) commit which should not be signed 2
    Parent commit (@-)      : kpqxywon 90df08d3 (empty) commit which should not be signed 1
    [EOF]
    ");
    // Only commits which are being pushed should be signed
    let output = work_dir.run_jj(["log", "-T", template]);
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
    let output = work_dir.run_jj([
        "bookmark",
        "create",
        "bookmark3",
        "-r",
        "description('commit which should not be signed 1')",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to kpqxywon 90df08d3 bookmark3 | (empty) commit which should not be signed 1
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "move", "bookmark2", "--to", "bookmark3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kpqxywon 90df08d3 bookmark2* bookmark3 | (empty) commit which should not be signed 1
    [EOF]
    ");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "bookmark3""#);
    let output = work_dir.run_jj(["git", "push"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refusing to create new remote bookmark bookmark3@origin
    Hint: Use --allow-new to push new bookmark. Use --remote to specify the remote to push to.
    Changes to push to origin:
      Move forward bookmark bookmark2 from a6259c482040 to 90df08d3d612
    [EOF]
    ");
    let output = work_dir.run_jj(["log", "-T", template, "-r", "::"]);
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

#[test]
fn test_git_push_rejected_by_remote() {
    let test_env = TestEnvironment::default();
    set_up(&test_env);
    let work_dir = test_env.work_dir("local");
    // show repo state
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bookmark1: xtvrqkyv d13ecdbd (empty) description 1
      @origin: xtvrqkyv d13ecdbd (empty) description 1
    bookmark2: rlzusymt 8476341e (empty) description 2
      @origin: rlzusymt 8476341e (empty) description 2
    [EOF]
    ");

    // create a hook on the remote that prevents pushing
    let hook_path = test_env
        .env_root()
        .join("origin")
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git")
        .join("hooks")
        .join("update");

    std::fs::write(&hook_path, "#!/bin/sh\nexit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    // create new commit on top of bookmark1
    work_dir.run_jj(["new", "bookmark1"]).success();
    work_dir.write_file("file", "file");
    work_dir.run_jj(["describe", "-m=update"]).success();

    // update bookmark
    work_dir.run_jj(["bookmark", "move", "bookmark1"]).success();

    // push bookmark
    let output = work_dir.run_jj(["git", "push"]);

    // The git remote sideband adds a dummy suffix of 8 spaces to attempt to clear
    // any leftover data. This is done to help with cases where the line is
    // rewritten.
    //
    // However, a common option in a lot of editors removes trailing whitespace.
    // This means that anyone with that option that opens this file would make the
    // following snapshot fail. Using the insta filter here normalizes the
    // output.
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(r"\s*\n", "\n");
    settings.bind(|| {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Changes to push to origin:
          Move forward bookmark bookmark1 from d13ecdbda2a2 to dd5c09b30f9f
        remote: error: hook declined to update refs/heads/bookmark1
        Error: Failed to push some bookmarks
        Hint: The remote rejected the following updates:
          refs/heads/bookmark1 (reason: hook declined)
        Hint: Try checking if you have permission to push to all the bookmarks.
        [EOF]
        [exit status: 1]
        ");
    });
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
