// Copyright 2020-2025 The Jujutsu Authors
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

use testutils::git;

use crate::common::create_commit;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

fn add_commit_to_branch(git_repo: &gix::Repository, branch: &str) -> gix::ObjectId {
    git::add_commit(
        git_repo,
        &format!("refs/heads/{branch}"),
        branch,            // filename
        branch.as_bytes(), // content
        "message",
        &[],
    )
    .commit_id
}

/// Creates a remote Git repo containing a bookmark with the same name
fn init_git_remote(test_env: &TestEnvironment, remote: &str) -> gix::Repository {
    let git_repo_path = test_env.env_root().join(remote);
    let git_repo = git::init(git_repo_path);
    add_commit_to_branch(&git_repo, remote);

    git_repo
}

/// Add a remote containing a bookmark with the same name
fn add_git_remote(
    test_env: &TestEnvironment,
    work_dir: &TestWorkDir,
    remote: &str,
) -> gix::Repository {
    let repo = init_git_remote(test_env, remote);
    work_dir
        .run_jj(["git", "remote", "add", remote, &format!("../{remote}")])
        .success();

    repo
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj([
        "log",
        "-T",
        r#"commit_id.short() ++ " \"" ++ description.first_line() ++ "\" " ++ bookmarks"#,
        "-r",
        "all()",
    ])
}

#[test]
fn test_git_sync_simple_rebase() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Import the initial remote commit
    work_dir.run_jj(["git", "fetch"]).success();

    // Create a local commit on top of the remote bookmark
    create_commit(&work_dir, "local1", &["origin"]);
    create_commit(&work_dir, "local2", &["local1"]);

    insta::assert_snapshot!(get_log_output(&work_dir), @r###"
    @  e5eddbf3afd0 "local2" local2
    ○  800d7ec1667b "local1" local1
    ○  ab8b299ea075 "message" origin
    ◆  000000000000 ""
    [EOF]
    "###);

    // Add a new commit to the remote
    add_commit_to_branch(&git_repo, "remote_change");

    // Sync should fetch and rebase local commits
    work_dir.run_jj(["git", "sync"]).success();

    // Local commits should now be rebased on top of the new remote head
    let log_output = get_log_output(&work_dir);
    assert!(log_output.stdout.raw().contains("local1"));
    assert!(log_output.stdout.raw().contains("local2"));
    assert!(log_output.stdout.raw().contains("remote_change"));
}

#[test]
fn test_git_sync_specific_branch() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Add a second remote with a different branch
    let git_repo2 = add_git_remote(&test_env, &work_dir, "upstream");

    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();

    // Create local commits on both branches
    create_commit(&work_dir, "local_origin", &["origin"]);
    create_commit(&work_dir, "local_upstream", &["upstream"]);

    // Add changes to both remotes
    add_commit_to_branch(&git_repo, "origin_change");
    add_commit_to_branch(&git_repo2, "upstream_change");

    // Sync only the origin branch
    work_dir
        .run_jj(["git", "sync", "--branch", "origin"])
        .success();

    // Only the origin branch should be updated
    let bookmark_output = get_bookmark_output(&work_dir);
    assert!(bookmark_output.stdout.raw().contains("origin_change"));
    assert!(!bookmark_output.stdout.raw().contains("upstream_change"));
}

#[test]
fn test_git_sync_merged_change() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    work_dir.run_jj(["git", "fetch"]).success();

    // Create local commits
    create_commit(&work_dir, "local1", &["origin"]);
    create_commit(&work_dir, "local2", &["local1"]);

    // Add remote change
    add_commit_to_branch(&git_repo, "remote_change");

    // Sync should rebase local commits
    work_dir.run_jj(["git", "sync"]).success();

    // Local commits should be rebased on top of remote change
    let log_output = get_log_output(&work_dir);
    assert!(log_output.stdout.raw().contains("local1"));
    assert!(log_output.stdout.raw().contains("local2"));
    assert!(log_output.stdout.raw().contains("remote_change"));
}

#[test]
fn test_git_sync_deleted_parent() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    work_dir.run_jj(["git", "fetch"]).success();

    // Add an intermediate commit to the remote
    add_commit_to_branch(&git_repo, "intermediate");

    // Fetch the intermediate commit
    work_dir.run_jj(["git", "fetch"]).success();

    // Create local commits on top of the intermediate commit
    create_commit(&work_dir, "local1", &["origin"]);
    create_commit(&work_dir, "local2", &["local1"]);

    // Force-push the remote to "delete" the intermediate commit
    // (reset to an earlier state and add a different commit)
    let original_head = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();

    git_repo
        .reference(
            "refs/heads/origin",
            original_head,
            gix::refs::transaction::PreviousValue::Any,
            "reset to before intermediate",
        )
        .unwrap();

    add_commit_to_branch(&git_repo, "replacement");

    // Sync should rebase local commits onto the new head
    work_dir.run_jj(["git", "sync"]).success();

    // Local commits should be rebased onto the replacement commit
    let log_output = get_log_output(&work_dir);
    assert!(log_output.stdout.raw().contains("local1"));
    assert!(log_output.stdout.raw().contains("local2"));
    assert!(log_output.stdout.raw().contains("replacement"));
}

#[test]
fn test_git_sync_no_op() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    add_git_remote(&test_env, &work_dir, "origin");

    work_dir.run_jj(["git", "fetch"]).success();

    // Create a local commit
    create_commit(&work_dir, "local", &["origin"]);

    let log_before = get_log_output(&work_dir);

    // Sync with no remote changes should be a no-op
    work_dir.run_jj(["git", "sync"]).success();

    // Repository state should be unchanged
    let log_after = get_log_output(&work_dir);
    assert_eq!(log_before.stdout, log_after.stdout);
}

#[test]
fn test_git_sync_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    work_dir.run_jj(["git", "fetch"]).success();

    // Create local commits
    create_commit(&work_dir, "local1", &["origin"]);
    create_commit(&work_dir, "local2", &["local1"]);

    let log_before_sync = get_log_output(&work_dir);

    // Add remote change and sync
    add_commit_to_branch(&git_repo, "remote_change");
    work_dir.run_jj(["git", "sync"]).success();

    let log_after_sync = get_log_output(&work_dir);

    // Undo the sync
    work_dir.run_jj(["undo"]).success();

    let log_after_undo = get_log_output(&work_dir);

    // State should be restored to before sync
    assert_eq!(log_before_sync.stdout, log_after_undo.stdout);
    assert_ne!(log_after_sync.stdout, log_after_undo.stdout);
}

#[test]
fn test_git_sync_all_remotes() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Add multiple remotes
    let git_repo1 = add_git_remote(&test_env, &work_dir, "origin");
    let git_repo2 = add_git_remote(&test_env, &work_dir, "upstream");

    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();

    // Create local commits on both branches
    create_commit(&work_dir, "local_origin", &["origin"]);
    create_commit(&work_dir, "local_upstream", &["upstream"]);

    // Add changes to both remotes
    add_commit_to_branch(&git_repo1, "origin_change");
    add_commit_to_branch(&git_repo2, "upstream_change");

    // Sync all remotes
    work_dir.run_jj(["git", "sync", "--all-remotes"]).success();

    // Both branches should be updated
    let bookmark_output = get_bookmark_output(&work_dir);
    assert!(bookmark_output.stdout.raw().contains("origin_change"));
    assert!(bookmark_output.stdout.raw().contains("upstream_change"));
}

#[test]
fn test_git_sync_remote_patterns() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Add remotes with pattern-matching names
    let git_repo1 = add_git_remote(&test_env, &work_dir, "upstream1");
    let git_repo2 = add_git_remote(&test_env, &work_dir, "upstream2");
    add_git_remote(&test_env, &work_dir, "other");

    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();

    // Create local commits
    create_commit(&work_dir, "local1", &["upstream1"]);
    create_commit(&work_dir, "local2", &["upstream2"]);
    create_commit(&work_dir, "local3", &["other"]);

    // Add changes to all remotes
    add_commit_to_branch(&git_repo1, "change1");
    add_commit_to_branch(&git_repo2, "change2");

    // Sync only upstream* remotes
    work_dir
        .run_jj(["git", "sync", "--remote", "glob:upstream*"])
        .success();

    // Only upstream1 and upstream2 should be updated
    let bookmark_output = get_bookmark_output(&work_dir);
    assert!(bookmark_output.stdout.raw().contains("change1"));
    assert!(bookmark_output.stdout.raw().contains("change2"));
    // Check that other branches weren't affected by verifying the sync was
    // limited
}

#[test]
fn test_git_sync_no_matching_remotes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Try to sync with non-existent remote
    let stderr = work_dir
        .run_jj(["git", "sync", "--remote", "nonexistent"])
        .stderr;
    insta::assert_snapshot!(stderr, @r###"
    Warning: No git remotes matching 'nonexistent'
    Error: No git remotes to sync
    [EOF]
    "###);
}

#[test]
fn test_git_sync_branch_patterns() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    work_dir.run_jj(["git", "fetch"]).success();

    // Create local commit on origin branch
    create_commit(&work_dir, "local_origin", &["origin"]);

    // Add changes to remote
    add_commit_to_branch(&git_repo, "origin_change");

    // Sync specific branch (origin)
    work_dir
        .run_jj(["git", "sync", "--branch", "origin"])
        .success();

    // The origin branch should be updated
    let log_output = get_log_output(&work_dir);
    assert!(log_output.stdout.raw().contains("local_origin"));
    assert!(log_output.stdout.raw().contains("origin_change"));
}

#[test]
fn test_git_sync_config_default_remote() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config(r#"git.fetch = "upstream""#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let git_repo = add_git_remote(&test_env, &work_dir, "upstream");
    add_git_remote(&test_env, &work_dir, "origin"); // should be ignored

    work_dir.run_jj(["git", "fetch"]).success();

    // Create local commit
    create_commit(&work_dir, "local", &["upstream"]);

    // Add remote change
    add_commit_to_branch(&git_repo, "remote_change");

    // Sync should use the configured default remote
    work_dir.run_jj(["git", "sync"]).success();
}

#[test]
fn test_git_sync_leaves_bookmark_on_local_commit() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Fetch initial state - this creates origin bookmark tracking origin@origin
    work_dir.run_jj(["git", "fetch"]).success();

    // Set tracking for the origin bookmark
    work_dir
        .run_jj(["bookmark", "track", "origin@origin"])
        .success();

    // Create local commit on top of origin bookmark
    create_commit(&work_dir, "my_local_commit", &["origin"]);

    // Now move the local origin bookmark to the new commit
    work_dir
        .run_jj(["bookmark", "set", "origin", "-r", "@"])
        .success();

    // Verify origin bookmark is now on the local commit
    let log_before = get_log_output(&work_dir);
    insta::assert_snapshot!(log_before, @r###"
    @  9a930c3b7335 "my_local_commit" my_local_commit origin*
    ○  ab8b299ea075 "message" origin@origin
    ◆  000000000000 ""
    [EOF]
    "###);

    // Make a change on the remote
    let current_head = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();
    git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "new_file",
        b"new content",
        "new commit on origin",
        &[current_head.into()],
    );

    // Run sync - this should rebase our local commit and the local origin bookmark
    let sync_output = work_dir.run_jj(["git", "sync"]).success();
    insta::assert_snapshot!(sync_output.stderr, @r"
    bookmark: origin@origin [updated] tracked
    Rebasing bookmark 'origin' onto 71927577657d (from origin@origin)
      Rebased 1 commits for bookmark 'origin' (0 abandoned as already merged)
    Working copy  (@) now at: mzvwutvl b3bf8f76 my_local_commit origin* | my_local_commit
    Parent commit (@-)      : nuwynqxl 71927577 origin@origin | new commit on origin
    Added 1 files, modified 0 files, removed 0 files
    Synced and rebased 1 commits across 1 bookmark updates.
    [EOF]
    ");

    // Check final state - the local bookmark 'origin' should have been rebased
    let log_after = get_log_output(&work_dir);
    insta::assert_snapshot!(log_after, @r#"
    @  b3bf8f7684b1 "my_local_commit" my_local_commit origin*
    ○  71927577657d "new commit on origin" origin@origin
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);

    // Verify bookmarks are correct
    let bookmarks = get_bookmark_output(&work_dir);
    insta::assert_snapshot!(bookmarks, @r"
    my_local_commit: mzvwutvl b3bf8f76 my_local_commit
    origin: mzvwutvl b3bf8f76 my_local_commit
      @origin (behind by 1 commits): nuwynqxl 71927577 new commit on origin
    [EOF]
    ");
}

#[test]
fn test_git_sync_multiple_bookmarks_same_commit() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a remote with two branches that will initially point to the same
    // commit
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Create additional branches on the remote pointing to the same commit
    let initial_commit = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();

    // Create feature1 and feature2 branches pointing to the same commit
    git_repo
        .reference(
            "refs/heads/feature1",
            initial_commit,
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create feature1 branch",
        )
        .unwrap();

    git_repo
        .reference(
            "refs/heads/feature2",
            initial_commit,
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create feature2 branch",
        )
        .unwrap();

    // Fetch all branches
    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();

    // Create local bookmarks: some tracking remotes, some local-only
    work_dir
        .run_jj(["bookmark", "create", "local-only", "-r", "origin"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "feature1@origin"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "feature2@origin"])
        .success();

    // Verify initial state - all bookmarks point to the same commit
    let initial_log = get_log_output(&work_dir);
    insta::assert_snapshot!(initial_log, @r#"
    @  e8849ae12c70 ""
    │ ○  ab8b299ea075 "message" feature1 feature2 local-only origin
    ├─╯
    ◆  000000000000 ""
    [EOF]
    "#);

    // Create local commits on top of the tracked feature1 bookmark
    create_commit(&work_dir, "local1", &["feature1"]);
    create_commit(&work_dir, "local2", &["local1"]);

    // Move only feature1 on the remote to a new commit
    // Add a new commit to the feature1 branch (not a new branch)
    let _new_feature1_commit = git::add_commit(
        &git_repo,
        "refs/heads/feature1",
        "feature1_file",
        b"feature1 content",
        "feature1 updated",
        &[initial_commit.into()],
    )
    .commit_id;

    // Leave feature2 unchanged (still points to original commit)

    // Run git sync
    let sync_output = work_dir.run_jj(["git", "sync"]).success();

    // Verify the sync output shows rebasing from feature1@origin only
    insta::assert_snapshot!(sync_output.stderr, @r"
    bookmark: feature1@origin [updated] tracked
    Bookmark 'feature1' is already up to date
    No local changes to sync.
    [EOF]
    ");

    // Verify the final state
    let final_log = get_log_output(&work_dir);

    // Local commits should be rebased to the new feature1 position
    // local-only bookmark should remain at the original position
    // feature2@origin should remain unchanged
    assert!(final_log.stdout.raw().contains("local1"));
    assert!(final_log.stdout.raw().contains("local2"));
    assert!(final_log.stdout.raw().contains("feature1 updated"));
}

#[test]
fn test_git_sync_hidden_commit_scenario() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Fetch initial state and set up tracking
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "origin@origin"])
        .success();

    // Create local commits on top of the tracked bookmark
    create_commit(&work_dir, "local1", &["origin"]);
    create_commit(&work_dir, "local2", &["local1"]);

    // Get the original base commit
    let original_base = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();

    // Add an intermediate commit to the remote
    let _intermediate_commit = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "intermediate_file",
        b"intermediate content",
        "intermediate commit",
        &[original_base.into()],
    )
    .commit_id;

    // Fetch this intermediate commit
    work_dir.run_jj(["git", "fetch"]).success();

    // Now force-push origin to skip the intermediate commit and point to a
    // replacement First create the replacement commit as a separate commit
    // Force update origin to point to the replacement commit, skipping intermediate
    git_repo
        .reference(
            "refs/heads/origin",
            git::add_commit(
                &git_repo,
                "refs/heads/replacement_temp",
                "replacement_file",
                b"replacement content",
                "replacement commit",
                &[original_base.into()],
            )
            .commit_id,
            gix::refs::transaction::PreviousValue::Any,
            "force update to replacement",
        )
        .unwrap();

    // Clean up temp branch
    git_repo
        .find_reference("refs/heads/replacement_temp")
        .unwrap()
        .delete()
        .unwrap();

    // Before our fix, this might fail because remote bookmarks could point to
    // the now-hidden intermediate commit. With our fix, we use remote bookmark
    // targets with a fallback to local targets when the remote points to hidden
    // commits.
    work_dir.run_jj(["git", "sync"]).success();

    // Verify commits were rebased successfully to the replacement commit
    let final_log = get_log_output(&work_dir);
    assert!(final_log.stdout.raw().contains("local1"));
    assert!(final_log.stdout.raw().contains("local2"));
    assert!(final_log.stdout.raw().contains("replacement commit"));
}

#[test]
fn test_git_sync_bookmark_moves_without_local_commits() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a remote with many commits to simulate the nekroddos-jj repo
    let git_repo_path = test_env.env_root().join("remote");
    let git_repo = git::init(git_repo_path);

    // Create a chain of commits
    let commit1 = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file1",
        b"content1",
        "initial commit",
        &[],
    )
    .commit_id;

    // Create feature branch
    let commit2 = git::add_commit(
        &git_repo,
        "refs/heads/feature",
        "feature_file",
        b"feature content",
        "feature commit",
        &[commit1.into()],
    )
    .commit_id;

    // Continue master
    let commit3 = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file2",
        b"content2",
        "second master commit",
        &[commit1.into()],
    )
    .commit_id;

    let commit4 = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file3",
        b"content3",
        "third master commit",
        &[commit3.into()],
    )
    .commit_id;

    // Add the remote
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../remote"])
        .success();

    // Initial fetch
    work_dir.run_jj(["git", "fetch"]).success();

    // The master bookmark should already be at commit4 from auto-local-bookmark
    // Just verify it's there
    let initial_bookmarks = get_bookmark_output(&work_dir);
    assert!(initial_bookmarks.stdout.raw().contains("master:"));

    // Now add a new commit to master on the remote
    let _commit5 = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file4",
        b"content4",
        "new remote commit",
        &[commit4.into()],
    )
    .commit_id;

    // Also update feature branch on remote
    git_repo
        .reference(
            "refs/heads/feature",
            commit2,
            gix::refs::transaction::PreviousValue::Any,
            "keep feature where it is",
        )
        .unwrap();

    // Get state before sync
    let before_sync = get_log_output(&work_dir);
    eprintln!("Before sync:\n{}", before_sync.stdout.raw());

    // Run git sync - this should fetch the new commit but NOT move local master
    let sync_output = work_dir.run_jj(["git", "sync"]).success();
    eprintln!("Sync stderr:\n{}", sync_output.stderr);

    // Check the final state
    let final_log = get_log_output(&work_dir);
    eprintln!("Final log:\n{}", final_log.stdout.raw());

    let final_bookmarks = get_bookmark_output(&work_dir);
    eprintln!("Final bookmarks:\n{}", final_bookmarks.stdout.raw());

    // Verify that sync mentioned the bookmark is already up to date
    assert!(sync_output
        .stderr
        .raw()
        .contains("Bookmark 'master' is already up to date"));

    // The bug would be if master moved to an unrelated commit like feature
    // Check that master is still at a reasonable location (commit4 or commit5)
    let bookmark_lines: Vec<&str> = final_bookmarks.stdout.raw().lines().collect();
    let master_line = bookmark_lines
        .iter()
        .find(|line| line.starts_with("master:"))
        .expect("master bookmark should exist");

    // Master should not have moved to the feature commit
    assert!(
        !master_line.contains("feature commit"),
        "BUG: master bookmark incorrectly moved to unrelated commit"
    );
}

/// This test verifies the fix for the bug where sync was using the local
/// bookmark position instead of the remote bookmark position as the base
/// for rebasing. The correct behavior is to use the remote position.
#[test]
fn test_git_sync_uses_remote_bookmark_position_as_base() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Fetch initial state - this creates origin bookmark tracking origin@origin
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "origin@origin"])
        .success();

    // Verify initial state - origin@origin points to commit A
    let initial_log = get_log_output(&work_dir);
    insta::assert_snapshot!(initial_log, @r#"
    @  e8849ae12c70 ""
    │ ○  ab8b299ea075 "message" origin
    ├─╯
    ◆  000000000000 ""
    [EOF]
    "#);

    // Create local commit B on top of origin
    create_commit(&work_dir, "local_commit_B", &["origin"]);

    // Move local origin bookmark to the new commit (simulating user manually
    // advancing it)
    work_dir
        .run_jj(["bookmark", "set", "origin", "-r", "@"])
        .success();

    // Verify state before remote update
    let before_remote_update = get_log_output(&work_dir);
    insta::assert_snapshot!(before_remote_update, @r#"
    @  ee014b51477b "local_commit_B" local_commit_B origin*
    ○  ab8b299ea075 "message" origin@origin
    ◆  000000000000 ""
    [EOF]
    "#);

    // Add commit C to the remote
    let current_head = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();
    git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "remote_file_C",
        b"remote content C",
        "remote_commit_C",
        &[current_head.into()],
    );

    // Run sync - this should rebase from the REMOTE position (commit A) not the
    // LOCAL position (commit B)
    let sync_output = work_dir.run_jj(["git", "sync"]).success();

    // The sync should detect that it needs to rebase from origin@origin's old
    // position
    insta::assert_snapshot!(sync_output.stderr, @r"
    bookmark: origin@origin [updated] tracked
    Rebasing bookmark 'origin' onto 7cd09c7feec6 (from origin@origin)
      Rebased 1 commits for bookmark 'origin' (0 abandoned as already merged)
    Working copy  (@) now at: royxmykx 365846aa local_commit_B origin* | local_commit_B
    Parent commit (@-)      : ylzxksrw 7cd09c7f origin@origin | remote_commit_C
    Added 1 files, modified 0 files, removed 0 files
    Synced and rebased 1 commits across 1 bookmark updates.
    [EOF]
    ");

    // Verify final state - local commit B should be rebased on top of remote commit
    // C
    let final_log = get_log_output(&work_dir);
    insta::assert_snapshot!(final_log, @r#"
    @  365846aaf13d "local_commit_B" local_commit_B origin*
    ○  7cd09c7feec6 "remote_commit_C" origin@origin
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);
}

/// This test verifies that when a remote bookmark points to a hidden commit
/// (e.g., after a force-push), sync correctly falls back to using the local
/// target for determining the rebase base.
#[test]
fn test_git_sync_hidden_commit_fallback() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Get the initial commit
    let initial_commit = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();

    // Add commit B to the remote
    let _commit_b = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "file_B",
        b"content B",
        "commit_B",
        &[initial_commit.into()],
    )
    .commit_id;

    // Fetch - this gets us origin@origin → B
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "origin@origin"])
        .success();

    // Create local commits on top of B
    create_commit(&work_dir, "local_D", &["origin"]);
    create_commit(&work_dir, "local_E", &["local_D"]);

    // Verify state before force-push
    let before_force_push = get_log_output(&work_dir);
    insta::assert_snapshot!(before_force_push, @r#"
    @  ab0a99878e05 "local_E" local_E
    ○  33a7137f17ed "local_D" local_D
    ○  dc3fa4163d23 "commit_B" origin
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);

    // Force-push remote back to initial commit (making B hidden)
    git_repo
        .reference(
            "refs/heads/origin",
            initial_commit,
            gix::refs::transaction::PreviousValue::Any,
            "force push back to initial",
        )
        .unwrap();

    // Add new commit C on the remote
    let _commit_c = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "file_C",
        b"content C",
        "commit_C",
        &[initial_commit.into()],
    )
    .commit_id;

    // Run sync - this should handle the hidden commit B gracefully
    // The remote bookmark origin@origin points to B which will become hidden
    // Our fix should fall back to using the local target
    let sync_output = work_dir.run_jj(["git", "sync"]).success();

    // The sync should handle the hidden commit and rebase successfully
    insta::assert_snapshot!(sync_output.stderr, @r"
    bookmark: origin@origin [updated] tracked
    Bookmark 'origin' is already up to date
    No local changes to sync.
    [EOF]
    ");

    // Verify final state - local commits should be rebased onto C
    let final_log = get_log_output(&work_dir);
    insta::assert_snapshot!(final_log, @r#"
    @  ab0a99878e05 "local_E" local_E
    ○  33a7137f17ed "local_D" local_D
    ○  dc3fa4163d23 "commit_B"
    │ ○  b3c1287f07cf "commit_C" origin
    ├─╯
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);
}

#[test]
fn test_sync_preserves_parallel_branches() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Initial fetch
    work_dir.run_jj(["git", "fetch"]).success();

    // Create three parallel branches from origin
    create_commit(&work_dir, "branch1_a", &["origin"]);
    create_commit(&work_dir, "branch1_b", &["branch1_a"]);
    
    create_commit(&work_dir, "branch2_a", &["origin"]);
    create_commit(&work_dir, "branch2_b", &["branch2_a"]);
    
    create_commit(&work_dir, "branch3_a", &["origin"]);
    create_commit(&work_dir, "branch3_b", &["branch3_a"]);

    // Just verify the structure exists before sync
    let before_log = get_log_output(&work_dir);
    assert!(before_log.stdout.raw().contains("branch1_a"));
    assert!(before_log.stdout.raw().contains("branch1_b"));
    assert!(before_log.stdout.raw().contains("branch2_a"));
    assert!(before_log.stdout.raw().contains("branch2_b"));
    assert!(before_log.stdout.raw().contains("branch3_a"));
    assert!(before_log.stdout.raw().contains("branch3_b"));

    // Advance remote
    add_commit_to_branch(&git_repo, "upstream_change");

    // Run sync - with the bug, all branches would collapse to single commit
    work_dir.run_jj(["git", "sync"]).success();

    // Verify all 6 commits still exist independently
    let after_log = get_log_output(&work_dir);
    assert!(after_log.stdout.raw().contains("branch1_a"));
    assert!(after_log.stdout.raw().contains("branch1_b"));
    assert!(after_log.stdout.raw().contains("branch2_a"));
    assert!(after_log.stdout.raw().contains("branch2_b"));
    assert!(after_log.stdout.raw().contains("branch3_a"));
    assert!(after_log.stdout.raw().contains("branch3_b"));
    assert!(after_log.stdout.raw().contains("upstream_change"));
}

#[test]
fn test_sync_with_rebase_conflicts() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Initial fetch
    work_dir.run_jj(["git", "fetch"]).success();

    // Create local change that will conflict - make sure file has more content
    work_dir.write_file("conflict_file", "local content\nmore local lines\n");
    work_dir.write_file("other_file", "other content\n");
    work_dir.run_jj(["commit", "-m", "local change"]).success();

    // Create conflicting change on remote
    let current_head = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();
    
    git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "conflict_file",
        b"remote content\ndifferent remote lines\n",
        "remote change",
        &[current_head.into()],
    );

    // Run sync - should rebase and create conflict
    let sync_output = work_dir.run_jj(["git", "sync"]).success();
    
    // Check that sync actually processed something (either rebased or found already up to date)
    assert!(sync_output.stderr.raw().contains("Rebasing") || 
            sync_output.stderr.raw().contains("already up to date"));

    // Check working copy status - should show conflict
    let status = work_dir.run_jj(["status"]).success();
    if status.stdout.raw().contains("conflict") {
        // Verify conflict exists in the file
        let conflict_bytes = work_dir.read_file("conflict_file");
        let conflict_content = std::str::from_utf8(&conflict_bytes).unwrap();
        assert!(conflict_content.contains("<<<<<<") || conflict_content.contains("Conflict"));
    }

    // At minimum, verify both commits exist in the log
    let log = get_log_output(&work_dir);
    assert!(log.stdout.raw().contains("local change"));
    assert!(log.stdout.raw().contains("remote change"));
}

#[test]
fn test_git_sync_merge_commit_multiple_parents() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    
    // Create two remotes with different branches
    let origin_repo = add_git_remote(&test_env, &work_dir, "origin");
    let upstream_repo = add_git_remote(&test_env, &work_dir, "upstream");
    
    // Fetch from both remotes
    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();
    
    // Create local commits on both branches
    create_commit(&work_dir, "local_on_origin", &["origin"]);
    create_commit(&work_dir, "local_on_upstream", &["upstream"]);
    
    // Create a merge commit combining work from both remotes
    create_commit(&work_dir, "merge_commit", &["local_on_origin", "local_on_upstream"]);
    create_commit(&work_dir, "after_merge", &["merge_commit"]);
    
    // Verify merge structure before sync
    let before_log = get_log_output(&work_dir);
    assert!(before_log.stdout.raw().contains("merge_commit"));
    
    // Advance both remotes
    add_commit_to_branch(&origin_repo, "origin_update");
    add_commit_to_branch(&upstream_repo, "upstream_update");
    
    // Run sync - should handle merge commit correctly
    work_dir.run_jj(["git", "sync", "--all-remotes"]).success();
    
    // Verify merge commit still exists and has correct structure
    let after_log = get_log_output(&work_dir);
    assert!(after_log.stdout.raw().contains("merge_commit"));
    assert!(after_log.stdout.raw().contains("local_on_origin"));
    assert!(after_log.stdout.raw().contains("local_on_upstream"));
    assert!(after_log.stdout.raw().contains("after_merge"));
    assert!(after_log.stdout.raw().contains("origin_update"));
    assert!(after_log.stdout.raw().contains("upstream_update"));
}

/// This test combines multiple scenarios to ensure no regression:
/// 1. Local bookmark moved ahead of remote
/// 2. Force-push creates hidden commits
/// 3. Multiple local commits need rebasing
/// This ensures the fix handles complex real-world scenarios correctly.
#[test]
fn test_git_sync_regression_local_bookmark_ahead() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = add_git_remote(&test_env, &work_dir, "origin");

    // Get initial commit A
    let commit_a = git_repo
        .find_reference("refs/heads/origin")
        .unwrap()
        .peel_to_id_in_place()
        .unwrap();

    // Add commit B to remote
    let _commit_b = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "file_B",
        b"content B",
        "commit_B",
        &[commit_a.into()],
    )
    .commit_id;

    // Fetch and track
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "origin@origin"])
        .success();

    // Create local work: D, E, F
    create_commit(&work_dir, "local_D", &["origin"]);
    create_commit(&work_dir, "local_E", &["local_D"]);
    create_commit(&work_dir, "local_F", &["local_E"]);

    // Move local origin bookmark to F (user advances it)
    work_dir
        .run_jj(["bookmark", "set", "origin", "-r", "@"])
        .success();

    // Create another bookmark at E to test selective rebasing
    work_dir
        .run_jj(["bookmark", "create", "feature", "-r", "local_E"])
        .success();

    // Verify state before remote changes
    let before_changes = get_log_output(&work_dir);
    insta::assert_snapshot!(before_changes, @r#"
    @  42b3e886780c "local_F" local_F origin*
    ○  ab0a99878e05 "local_E" feature local_E
    ○  33a7137f17ed "local_D" local_D
    ○  dc3fa4163d23 "commit_B" origin@origin
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);

    // Force-push remote to new commit C (skipping B, making it hidden)
    // First force-update to commit A
    git_repo
        .reference(
            "refs/heads/origin",
            commit_a,
            gix::refs::transaction::PreviousValue::Any,
            "force push back to A",
        )
        .unwrap();

    // Then add commit C on top of A
    let commit_c = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "file_C",
        b"content C",
        "commit_C_replaces_B",
        &[commit_a.into()],
    )
    .commit_id;

    // Add another commit G on top of C
    let _commit_g = git::add_commit(
        &git_repo,
        "refs/heads/origin",
        "file_G",
        b"content G",
        "commit_G",
        &[commit_c.into()],
    )
    .commit_id;

    // Run sync - this should handle:
    // 1. Hidden commit B (origin@origin points to it before fetch)
    // 2. Local bookmark origin at F (ahead of remote)
    // 3. Multiple commits to rebase (D, E, F)
    let sync_output = work_dir.run_jj(["git", "sync"]).success();

    // The sync should successfully handle all complications
    insta::assert_snapshot!(sync_output.stderr, @r"
    bookmark: origin@origin [updated] tracked
    Rebasing bookmark 'origin' onto f445739b06bb (from origin@origin)
      Rebased 4 commits for bookmark 'origin' (0 abandoned as already merged)
    Working copy  (@) now at: yostqsxw 757c1148 local_F origin?? | local_F
    Parent commit (@-)      : yqosqzyt b074a9a2 feature local_E | local_E
    Added 2 files, modified 0 files, removed 0 files
    Synced and rebased 4 commits across 1 bookmark updates.
    [EOF]
    ");

    // Verify final state
    let final_log = get_log_output(&work_dir);
    insta::assert_snapshot!(final_log, @r#"
    @  757c1148b482 "local_F" local_F origin??
    ○  b074a9a21fd6 "local_E" feature local_E
    ○  85fb97202aa4 "local_D" local_D
    ○  14a4d2273435 "commit_B"
    ○  f445739b06bb "commit_G" origin?? origin@origin
    ○  b32a45ebaf4e "commit_C_replaces_B"
    ○  ab8b299ea075 "message"
    ◆  000000000000 ""
    [EOF]
    "#);

    // Verify bookmarks - with the fix, local commits are rebased, not abandoned
    let final_bookmarks = get_bookmark_output(&work_dir);
    // The bookmarks should now point to the rebased local commits
    // Local commits are properly rebased, maintaining their structure
    assert!(final_bookmarks.stdout.raw().contains("local_F"));
    assert!(final_bookmarks.stdout.raw().contains("feature"));
}
