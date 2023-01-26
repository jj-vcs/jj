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

use crate::common::TestEnvironment;

pub mod common;

/// Test adding a second workspace
#[test]
fn test_workspaces_add_second_workspace() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&main_path, &["commit", "-m", "initial"]);

    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    default: e0e6d5672858 (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );
    insta::assert_snapshot!(stdout.replace('\\', "/"), @r###"
    Created workspace in "../secondary"
    Working copy now at: 397eac932ad3 (no description set)
    Added 1 files, modified 0 files, removed 0 files
    "###);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r###"
    o 397eac932ad3c349b2659fd2eb035a4dd3da4193 second@
    | @ e0e6d5672858dc9a57ec5b772b7c4f3270ed0223 default@
    |/  
    o 7d308bc9d934c53c6cc52935192e2d6ac5d78cfd 
    o 0000000000000000000000000000000000000000 
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r###"
    @ 397eac932ad3c349b2659fd2eb035a4dd3da4193 second@
    | o e0e6d5672858dc9a57ec5b772b7c4f3270ed0223 default@
    |/  
    o 7d308bc9d934c53c6cc52935192e2d6ac5d78cfd 
    o 0000000000000000000000000000000000000000 
    "###);

    // Both workspaces show up when we list them
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    default: e0e6d5672858 (no description set)
    second: 397eac932ad3 (no description set)
    "###);
}

/// Test making changes to the working copy in a workspace as it gets rewritten
/// from another workspace
#[test]
fn test_workspaces_conflicting_edits() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents\n").unwrap();
    test_env.jj_cmd_success(&main_path, &["new"]);

    test_env.jj_cmd_success(&main_path, &["workspace", "add", "../secondary"]);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r###"
    o 265af0cdbcc7bb33e3734ad72565c943ce3fb0d4 secondary@
    | @ 351099fa72cfbb1b34e410e89821efc623295974 default@
    |/  
    o cf911c223d3e24e001fc8264d6dbf0610804fc40 
    o 0000000000000000000000000000000000000000 
    "###);

    // Make changes in both working copies
    std::fs::write(main_path.join("file"), "changed in main\n").unwrap();
    std::fs::write(secondary_path.join("file"), "changed in second\n").unwrap();
    // Squash the changes from the main workspace into the initial commit (before
    // running any command in the secondary workspace
    let stdout = test_env.jj_cmd_success(&main_path, &["squash"]);
    insta::assert_snapshot!(stdout, @r###"
    Rebased 1 descendant commits
    Working copy now at: fe8f41ed01d6 (no description set)
    "###);

    // The secondary workspace's working-copy commit was updated
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r###"
    @ fe8f41ed01d693b2d4365cd89e42ad9c531a939b default@
    | o a1896a17282f19089a5cec44358d6609910e0513 secondary@
    |/  
    o c0d4a99ef98ada7da8dc73a778bbb747c4178385 
    o 0000000000000000000000000000000000000000 
    "###);
    let stderr = test_env.jj_cmd_failure(&secondary_path, &["st"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: The working copy is stale (not updated since operation 6a2e94fc65fb).
    Hint: Run `jj workspace update-stale` to update it.
    "###);
    // Same error on second run, and from another command
    let stderr = test_env.jj_cmd_failure(&secondary_path, &["log"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: The working copy is stale (not updated since operation 6a2e94fc65fb).
    Hint: Run `jj workspace update-stale` to update it.
    "###);
    let stdout = test_env.jj_cmd_success(&secondary_path, &["workspace", "update-stale"]);
    // It was detected that the working copy is now stale
    // TODO: Since there was an uncommitted change in the working copy, it should
    // have been committed first (causing divergence)
    insta::assert_snapshot!(stdout, @r###"
    Working copy now at: a1896a17282f (no description set)
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path),
    @r###"
    o fe8f41ed01d693b2d4365cd89e42ad9c531a939b default@
    | @ a1896a17282f19089a5cec44358d6609910e0513 secondary@
    |/  
    o c0d4a99ef98ada7da8dc73a778bbb747c4178385 
    o 0000000000000000000000000000000000000000 
    "###);
    // The stale working copy should have been resolved by the previous command
    let stdout = get_log_output(&test_env, &secondary_path);
    assert!(!stdout.starts_with("The working copy is stale"));
    insta::assert_snapshot!(stdout, @r###"
    o fe8f41ed01d693b2d4365cd89e42ad9c531a939b default@
    | @ a1896a17282f19089a5cec44358d6609910e0513 secondary@
    |/  
    o c0d4a99ef98ada7da8dc73a778bbb747c4178385 
    o 0000000000000000000000000000000000000000 
    "###);
}

/// Test forgetting workspaces
#[test]
fn test_workspaces_forget() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&main_path, &["new"]);

    test_env.jj_cmd_success(&main_path, &["workspace", "add", "../secondary"]);
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "forget"]);
    insta::assert_snapshot!(stdout, @"");

    // When listing workspaces, only the secondary workspace shows up
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    secondary: feda1c4e5ffe (no description set)
    "###);

    // `jj status` tells us that there's no working copy here
    let stdout = test_env.jj_cmd_success(&main_path, &["st"]);
    insta::assert_snapshot!(stdout, @r###"
    No working copy
    "###);

    // The old working copy doesn't get an "@" in the log output
    // TODO: We should abandon the empty working copy commit
    // TODO: It seems useful to still have the "secondary@" marker here even though
    // there's only one workspace. We should show it when the command is not run
    // from that workspace.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r###"
    o feda1c4e5ffe63fb16818ccdd8c21483537e31f2 
    | o e949be04e93e830fcce23fefac985c1deee52eea 
    |/  
    o 123ed18e4c4c0d77428df41112bc02ffc83fb935 
    o 0000000000000000000000000000000000000000 
    "###);

    // Revision "@" cannot be used
    let stderr = test_env.jj_cmd_failure(&main_path, &["log", "-r", "@"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Revision "@" doesn't exist
    "###);

    // Try to add back the workspace
    // TODO: We should make this just add it back instead of failing
    let stderr = test_env.jj_cmd_failure(&main_path, &["workspace", "add", "."]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Workspace already exists
    "###);

    // Forget the secondary workspace
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "forget", "secondary"]);
    insta::assert_snapshot!(stdout, @"");
    // No workspaces left
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @"");
}

/// Test context of commit summary template
#[test]
fn test_list_workspaces_template() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "main"]);
    test_env.add_config(
        r#"
        template.commit_summary = """commit_id.short() " " description.first_line()
                                     if(current_working_copy, " (current)")"""
        "#,
    );
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_success(&main_path, &["commit", "-m", "initial"]);
    test_env.jj_cmd_success(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );

    // "current_working_copy" should point to the workspace we operate on
    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    default: e0e6d5672858 (no description set) (current)
    second: f68da2d114f1 (no description set)
    "###);

    let stdout = test_env.jj_cmd_success(&secondary_path, &["workspace", "list"]);
    insta::assert_snapshot!(stdout, @r###"
    default: e0e6d5672858 (no description set)
    second: f68da2d114f1 (no description set) (current)
    "###);
}

/// Test getting the workspace root from primary and secondary workspaces
#[test]
fn test_workspaces_root() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "--git", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    let stdout = test_env.jj_cmd_success(&main_path, &["workspace", "root"]);
    insta::assert_snapshot!(stdout, @r###"
    $TEST_ENV/main
    "###);
    let main_subdir_path = main_path.join("subdir");
    std::fs::create_dir(&main_subdir_path).unwrap();
    let stdout = test_env.jj_cmd_success(&main_subdir_path, &["workspace", "root"]);
    insta::assert_snapshot!(stdout, @r###"
    $TEST_ENV/main
    "###);

    test_env.jj_cmd_success(
        &main_path,
        &["workspace", "add", "--name", "secondary", "../secondary"],
    );
    let stdout = test_env.jj_cmd_success(&secondary_path, &["workspace", "root"]);
    insta::assert_snapshot!(stdout, @r###"
    $TEST_ENV/secondary
    "###);
    let secondary_subdir_path = secondary_path.join("subdir");
    std::fs::create_dir(&secondary_subdir_path).unwrap();
    let stdout = test_env.jj_cmd_success(&secondary_subdir_path, &["workspace", "root"]);
    insta::assert_snapshot!(stdout, @r###"
    $TEST_ENV/secondary
    "###);
}

fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> String {
    test_env.jj_cmd_success(
        cwd,
        &[
            "log",
            "-T",
            r#"commit_id " " working_copies"#,
            "-r",
            "all()",
        ],
    )
}
