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

    // Initialize a non-colocated Jujutsu/Git repo
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

    // And that there is no git_head() yet
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 43444d88
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]");

    // Run colocate command
    let output = work_dir.run_jj(["git", "colocation", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Repository successfully converted into a colocated Jujutsu/Git repository.
    [EOF]");

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

    // Verify that git_head() was set correctly
    let output = work_dir.run_jj(["log", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 43444d88
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 git_head() e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]");
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
    Repository is already colocated with Git.
    [EOF]");
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
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: A .git directory already exists in the workspace root. Cannot colocate.
    [EOF]
    [exit status: 1]");
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

    // Verify that git_head() is set and visible in the log
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 43444d88
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 git_head() e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]");

    // Verify it's colocated
    assert!(workspace_root.join(".git").exists());
    assert_eq!(read_git_target(workspace_root), "../../../.git");

    // Disable colocation
    let output = work_dir.run_jj(["git", "colocation", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Repository successfully converted into a non-colocated Jujutsu/Git repository.
    [EOF]");

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

    // Verify that git_head() was removed correctly
    let output = work_dir.run_jj(["log", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 43444d88
    │  (empty) (no description set)
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]");
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
    Repository is already not colocated with Git.
    [EOF]");
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
    Repository is currently not colocated with Git.
    [EOF]
    ------- stderr -------
    Hint: To enable colocation, run: `jj git colocation enable`
    [EOF]");
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
    Repository is currently colocated with Git.
    [EOF]
    ------- stderr -------
    Hint: To disable colocation, run: `jj git colocation disable`
    [EOF]");
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
