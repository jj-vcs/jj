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

use crate::common::TestEnvironment;

fn read_git_target(workspace_root: &std::path::Path) -> String {
    let mut path = workspace_root.to_path_buf();
    path.extend([".jj", "repo", "store", "git_target"]);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn test_git_colocation_enable_success() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo backed by git
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Verify it's not colocated initially
    assert!(!workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "git");

    // Run colocate command
    let _output = test_env.run_jj_in(&workspace_root, ["git", "colocation", "enable"]);

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
    assert_eq!(read_git_target(&workspace_root), "../../../.git");

    // Verify .jj/.gitignore was created
    let gitignore_content =
        std::fs::read_to_string(workspace_root.join(".jj").join(".gitignore")).unwrap();
    assert_eq!(gitignore_content, "/*\n");
}

#[test]
fn test_git_colocation_enable_already_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a colocated jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Try to colocate again - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocation", "enable"]);
    insta::assert_snapshot!(output.stderr,@"
        Repository is already colocated with Git.
        [EOF]");
}

#[test]
fn test_git_colocation_enable_with_existing_git_dir() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Create a .git directory manually
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    std::fs::write(workspace_root.join(".git").join("dummy"), "dummy").unwrap();

    // Try to colocate - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocation", "enable"]);
    assert!(
        output
            .stderr
            .raw()
            .contains("A .git directory already exists")
    );
}

#[test]
fn test_git_colocation_disable_success() {
    let test_env = TestEnvironment::default();

    // Initialize and colocate a repo first
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let work_dir = test_env.work_dir("repo");
    let workspace_root = test_env.env_root().join("repo");
    let _ = work_dir.run_jj(["git", "colocation", "enable"]);

    // Verify it's colocated
    assert!(workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "../../../.git");

    // Disable colocation
    let _ = work_dir.run_jj(["git", "colocation", "disable"]);

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
    assert_eq!(read_git_target(&workspace_root), "git");
    assert!(!workspace_root.join(".jj").join(".gitignore").exists());
}

#[test]
fn test_git_colocation_disable_not_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo (non colocated)
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Try to disable colocation when not colocated - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocation", "disable"]);
    assert!(
        output
            .stderr
            .raw()
            .contains("Repository is already not colocated with Git.")
    );
}

#[test]
fn test_git_colocation_round_trip() {
    let test_env = TestEnvironment::default();

    // Initialize repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Enable colocation
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocation", "enable"]);
    assert!(workspace_root.join(".git").exists());

    // Disable colocation
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocation", "disable"]);
    assert!(!workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "git");
}

#[test]
fn test_git_colocation_status_non_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo (non colocated)
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Check status - should show non colocated
    let output = test_env
        .run_jj_in(&workspace_root, ["git", "colocation", "status"])
        .success();
    insta::assert_snapshot!(output.stdout, @r"
    Repository is currently not colocated with Git.
    [EOF]
    ");
    insta::assert_snapshot!(output.stderr, @"
        Hint: To enable colocation, run: `jj git colocation enable`
        [EOF]");
}

#[test]
fn test_git_colocation_status_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a colocated jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Check status - should show colocated
    let output = test_env
        .run_jj_in(&workspace_root, ["git", "colocation", "status"])
        .success();
    insta::assert_snapshot!(output.stdout,@r"
    Repository is currently colocated with Git.
    [EOF]
    ");
    insta::assert_snapshot!(output.stderr,@"
        Hint: To disable colocation, run: `jj git colocation disable`
        [EOF]");
}
