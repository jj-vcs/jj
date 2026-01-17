// Copyright 2025 The Jujutsu Authors
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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[must_use]
fn get_colocation_status(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj([
        "git",
        "colocation",
        "status",
        "--ignore-working-copy",
        "--quiet", // suppress hint
    ])
}

fn read_git_target(workspace_root: &std::path::Path) -> String {
    let mut path = workspace_root.to_path_buf();
    path.extend([".jj", "repo", "store", "git_target"]);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn test_git_colocation_enable_success() {
    let test_env = TestEnvironment::default();

    // Initialize a non-colocated Jujutsu/Git workspace
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();

    // Need at least one commit to be able to set git HEAD later
    work_dir.run_jj(["new"]).success();

    // Verify it's not colocated initially
    assert!(!workspace_root.join(".git").exists());
    assert_eq!(read_git_target(workspace_root), "git");

    // And that there is no Git HEAD yet
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently not colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Run colocate command
    let output = work_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace successfully converted into a colocated Jujutsu/Git workspace.
    [EOF]
    ");

    // Verify colocate succeeded
    assert!(workspace_root.join(".git").exists());
    assert!(
        !workspace_root
            .join(".jj")
            .join("repo")
            .join("store")
            .join("git")
            .exists()
    );
    assert_eq!(read_git_target(workspace_root), "../../../.git");

    // Verify .jj/.gitignore was created
    let gitignore_content =
        std::fs::read_to_string(workspace_root.join(".jj").join(".gitignore")).unwrap();
    assert_eq!(gitignore_content, "/*\n");

    // Verify that Git HEAD was set correctly
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");

    // Verify that the repo changed
    let output = work_dir.run_jj(["op", "show", "-T", "description ++ '\n'"]);
    insta::assert_snapshot!(output, @"
    set git head to working copy parent
    [EOF]
    ");
}

#[test]
fn test_git_colocation_enable_empty() {
    let test_env = TestEnvironment::default();

    // Initialize a non-colocated Jujutsu/Git workspace
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();
    let setup_op_id = work_dir.current_operation_id();

    // Verify initial state: no .git at workspace root
    assert!(
        !workspace_root.join(".git").exists(),
        ".git should not exist before enable"
    );

    // Run colocate command
    let output = work_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace successfully converted into a colocated Jujutsu/Git workspace.
    [EOF]
    ");

    // Verify filesystem changes: .git now exists at workspace root
    assert!(
        workspace_root.join(".git").exists(),
        ".git should exist after enable"
    );
    assert!(
        !workspace_root
            .join(".jj")
            .join("repo")
            .join("store")
            .join("git")
            .exists(),
        "Internal git store should be moved"
    );
    assert_eq!(read_git_target(workspace_root), "../../../.git");

    // Verify that Git HEAD was set correctly
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // No repo change required
    assert_eq!(setup_op_id, work_dir.current_operation_id());

    // Verify workspace is still functional
    let output = work_dir.run_jj(["status"]);
    assert!(
        output.status.success(),
        "jj status should work after enable"
    );
}

#[test]
fn test_git_colocation_enable_already_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a colocated Jujutsu/Git repo
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Try to colocate it again - should fail
    let output = work_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace is already colocated with Git.
    [EOF]
    ");
}

#[test]
fn test_git_colocation_enable_with_existing_git_dir() {
    let test_env = TestEnvironment::default();

    // Initialize a non-colocated Jujutsu/Git repo
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();

    // Create a .git directory manually
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    std::fs::write(workspace_root.join(".git").join("dummy"), "dummy").unwrap();

    // Try to colocate - should fail
    let output = work_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @"
    ------- stderr -------
    Warning: Workspace has a .git directory that is not managed by jj
    Hint: To remove this directory, run `rm -rf .git`
    Error: A .git directory already exists in the workspace root. Cannot colocate.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_colocation_disable_success() {
    let test_env = TestEnvironment::default();

    // Create a colocated Jujutsu/Git repo
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();

    // Need at least one commit to be able to set git HEAD later
    work_dir.run_jj(["new"]).success();

    // Verify that Git HEAD is set
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");

    // Verify it's colocated
    assert!(workspace_root.join(".git").exists());
    assert_eq!(read_git_target(workspace_root), "../../../.git");

    // Disable colocation
    let output = work_dir.run_jj(["git", "colocation", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace successfully converted into a non-colocated Jujutsu/Git workspace.
    [EOF]
    ");

    // Verify that disable colocation succeeded
    assert!(!workspace_root.join(".git").exists());
    assert!(
        workspace_root
            .join(".jj")
            .join("repo")
            .join("store")
            .join("git")
            .exists()
    );
    assert_eq!(read_git_target(workspace_root), "git");
    assert!(!workspace_root.join(".jj").join(".gitignore").exists());

    // Verify that Git HEAD was removed correctly
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently not colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Verify that the repo changed
    let output = work_dir.run_jj(["op", "show", "-T", "description ++ '\n'"]);
    insta::assert_snapshot!(output, @"
    remove git head reference
    [EOF]
    ");
}

#[test]
fn test_git_colocation_disable_empty() {
    let test_env = TestEnvironment::default();

    // Create a colocated Jujutsu/Git repo
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();
    let setup_op_id = work_dir.current_operation_id();

    // Verify initial state: .git exists at workspace root
    assert!(
        workspace_root.join(".git").exists(),
        ".git should exist before disable"
    );

    // Verify that Git HEAD is unset
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Disable colocation
    let output = work_dir.run_jj(["git", "colocation", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace successfully converted into a non-colocated Jujutsu/Git workspace.
    [EOF]
    ");

    // Verify filesystem changes: .git removed, internal git store exists
    assert!(
        !workspace_root.join(".git").exists(),
        ".git should be removed after disable"
    );
    assert!(
        workspace_root
            .join(".jj")
            .join("repo")
            .join("store")
            .join("git")
            .exists(),
        "Internal git store should be restored"
    );
    assert_eq!(read_git_target(workspace_root), "git");

    // No repo change required
    assert_eq!(setup_op_id, work_dir.current_operation_id());

    // Verify workspace is still functional
    let output = work_dir.run_jj(["status"]);
    assert!(
        output.status.success(),
        "jj status should work after disable"
    );
}

#[test]
fn test_git_colocation_disable_not_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a non-colocated Jujutsu/Git repo
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");

    // Try to disable colocation when not colocated - should fail
    let output = work_dir.run_jj(["git", "colocation", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Workspace is already not colocated with Git.
    [EOF]
    ");
}

#[test]
fn test_git_colocation_status_non_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a non-colocated Jujutsu/Git repo
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");

    // Check status - should show non-colocated
    let output = work_dir.run_jj(["git", "colocation", "status"]);
    insta::assert_snapshot!(output, @r"
    Workspace is currently not colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ------- stderr -------
    Hint: To enable colocation, run: `jj git colocation enable`
    [EOF]
    ");
}

#[test]
fn test_git_colocation_status_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a colocated jj repo
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Check status - should show colocated
    let output = work_dir.run_jj(["git", "colocation", "status"]);
    insta::assert_snapshot!(output, @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ------- stderr -------
    Hint: To disable colocation, run: `jj git colocation disable`
    [EOF]
    ");
}

#[test]
fn test_git_colocation_in_secondary_workspace() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--no-colocate", "main"])
        .success();
    let main_dir = test_env.work_dir("main");
    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();
    let secondary_dir = test_env.work_dir("secondary");

    let output = secondary_dir.run_jj(["git", "colocation", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command cannot be used in a non-main Jujutsu workspace
    [EOF]
    [exit status: 1]
    ");

    let output = secondary_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command cannot be used in a non-main Jujutsu workspace
    [EOF]
    [exit status: 1]
    ");

    let output = secondary_dir.run_jj(["git", "colocation", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command cannot be used in a non-main Jujutsu workspace
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_colocation_disable_with_secondary_workspaces_fails() {
    // This test verifies that colocation disable fails with a helpful error
    // when secondary colocated workspaces exist (since disabling would break them).
    let test_env = TestEnvironment::default();
    let primary_dir = test_env.work_dir("primary");

    // 1. Create colocated repo with a commit
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_dir.write_file("file", "contents");
    primary_dir.run_jj(["commit", "-m", "initial"]).success();

    // 2. Add colocated secondary workspace
    primary_dir
        .run_jj(["workspace", "add", "--colocate", "../secondary"])
        .success();

    // 3. Try to disable colocation - should fail
    let output = primary_dir.run_jj(["git", "colocation", "disable"]);
    assert!(!output.status.success());
    insta::assert_snapshot!(output.stderr.normalized(), @r"
    Error: Cannot disable colocation: secondary colocated workspaces exist.
    These workspaces would become broken Git worktrees.
    Either:
      - Run `jj workspace forget <name>` for each secondary workspace first
      - Use --force to disable anyway (secondary workspaces will be broken)
    ");
}

#[test]
fn test_git_colocation_disable_force_with_secondary_workspaces() {
    // This test verifies that colocation disable --force succeeds even with
    // secondary workspaces (though it leaves them broken).
    let test_env = TestEnvironment::default();
    let primary_dir = test_env.work_dir("primary");
    let secondary_dir = test_env.work_dir("secondary");

    // 1. Create colocated repo with a commit
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_dir.write_file("file", "contents");
    primary_dir.run_jj(["commit", "-m", "initial"]).success();

    // 2. Add colocated secondary workspace
    primary_dir
        .run_jj(["workspace", "add", "--colocate", "../secondary"])
        .success();

    // Verify secondary workspace has the file
    assert!(
        secondary_dir.root().join("file").exists(),
        "Secondary workspace should have file before disable"
    );

    // 3. Disable colocation with --force - should succeed
    let output = primary_dir.run_jj(["git", "colocation", "disable", "--force"]);
    assert!(output.status.success());

    // 4. Verify primary is now non-colocated
    let output = primary_dir.run_jj(["git", "colocation", "status"]);
    assert!(output.stdout.normalized().contains("not colocated"));

    // 5. Verify primary workspace data is preserved (critical: no data loss)
    assert!(
        primary_dir.root().join("file").exists(),
        "Primary workspace file should be preserved after force disable"
    );
    let contents = std::fs::read_to_string(primary_dir.root().join("file")).unwrap();
    assert_eq!(
        contents, "contents",
        "Primary workspace file contents should be unchanged"
    );

    // 6. Verify primary workspace is still functional
    let output = primary_dir.run_jj(["status"]);
    assert!(
        output.status.success(),
        "jj status should work in primary workspace after force disable"
    );

    // 7. Verify secondary workspace directory and file still exist (data preserved)
    assert!(
        secondary_dir.root().exists(),
        "Secondary workspace directory should still exist"
    );
    assert!(
        secondary_dir.root().join("file").exists(),
        "Secondary workspace file should be preserved (even if workspace is broken)"
    );

    // 8. Verify secondary workspace has a warning about broken git worktree
    let output = secondary_dir.run_jj(["status"]);
    let stderr = output.stderr.normalized();
    assert!(
        stderr.contains("broken Git worktree") || stderr.contains("Warning"),
        "Secondary workspace should show warning about broken state, got: {stderr}"
    );
}
