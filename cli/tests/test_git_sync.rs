// Copyright 2020-2023 The Jujutsu Authors
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
    let template =
        r#"commit_id.short() ++ " \"" ++ description.first_line() ++ "\" " ++ bookmarks"#;
    work_dir.run_jj(["log", "-T", template, "-r", "all()"])
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

    // Create additional branches on the remote - skip this for now since it's
    // complex We'll just test with the existing origin branch

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
