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
fn test_git_colocate_enable_success() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo backed by git
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Verify it's not co-located initially
    assert!(!workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "git");

    // Run colocate command
    let _output = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--enable"]);

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
fn test_git_colocate_enable_already_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a co-located jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Try to colocate again - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--enable"]);
    assert!(
        output
            .stderr
            .raw()
            .contains("Repository is already co-located with Git")
    );
}

#[test]
fn test_git_colocate_enable_with_existing_git_dir() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Create a .git directory manually
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    std::fs::write(workspace_root.join(".git").join("dummy"), "dummy").unwrap();

    // Try to colocate - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--enable"]);
    assert!(
        output
            .stderr
            .raw()
            .contains("A .git directory already exists")
    );
}

#[test]
fn test_git_colocate_disable_success() {
    let test_env = TestEnvironment::default();

    // Initialize and colocate a repo first
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--enable"]);

    // Verify it's co-located
    assert!(workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "../../../.git");

    // Disable co-location
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--disable"]);

    // Verify that disable co-location succeeded
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
fn test_git_colocate_disable_not_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo (non co-located)
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Try to disable co-location when not co-located - should fail
    let output = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--disable"]);
    assert!(
        output
            .stderr
            .raw()
            .contains("Repository is already not co-located with Git.")
    );
}

#[test]
fn test_git_colocate_round_trip() {
    let test_env = TestEnvironment::default();

    // Initialize repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Enable co-location
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--enable"]);
    assert!(workspace_root.join(".git").exists());

    // Disable co-location
    let _ = test_env.run_jj_in(&workspace_root, ["git", "colocate", "--disable"]);
    assert!(!workspace_root.join(".git").exists());
    assert_eq!(read_git_target(&workspace_root), "git");
}

#[test]
fn test_git_colocate_status_non_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo (non co-located)
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Check status - should show non co-located
    let output = test_env
        .run_jj_in(&workspace_root, ["git", "colocate"])
        .success();
    assert!(
        output
            .stderr
            .raw()
            .contains("Repository is currently not co-located with Git")
    );
    assert!(
        output
            .stderr
            .raw()
            .contains("To enable co-location, run: jj git colocate --enable")
    );
}

#[test]
fn test_git_colocate_status_colocated() {
    let test_env = TestEnvironment::default();

    // Initialize a co-located jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "--colocate", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Check status - should show co-located
    let output = test_env
        .run_jj_in(&workspace_root, ["git", "colocate"])
        .success();
    assert!(
        output
            .stderr
            .raw()
            .contains("Repository is currently co-located with Git")
    );
    assert!(
        output
            .stderr
            .raw()
            .contains("To disable co-location, run: jj git colocate --disable")
    );
}

#[test]
fn test_git_colocate_both_flags_error() {
    let test_env = TestEnvironment::default();

    // Initialize a regular jj repo
    let _ = test_env.run_jj_in(test_env.env_root(), ["git", "init", "repo"]);
    let workspace_root = test_env.env_root().join("repo");

    // Try to use both flags - should fail
    let output = test_env.run_jj_in(
        &workspace_root,
        ["git", "colocate", "--enable", "--disable"],
    );
    assert!(
        output
            .stderr
            .raw()
            .contains("Cannot specify both --enable and --disable flags")
    );
}
