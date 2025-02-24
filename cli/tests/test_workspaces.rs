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

use test_case::test_case;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;

/// Test adding a second workspace
#[test]
fn test_workspaces_add_second_workspace() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "initial"]);

    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 8183d0fc (empty) (no description set)
    [EOF]
    ");

    let (stdout, stderr) = test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );
    insta::assert_snapshot!(stdout.normalize_backslash(), @"");
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "../secondary"
    Working copy now at: rzvqmyuk 5ed2222c (empty) (no description set)
    Parent commit      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  8183d0fcaa4c default@
    │ ○  5ed2222c28e2 second@
    ├─╯
    ○  751b12b7b981
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r"
    @  5ed2222c28e2 second@
    │ ○  8183d0fcaa4c default@
    ├─╯
    ○  751b12b7b981
    ◆  000000000000
    [EOF]
    ");

    // Both workspaces show up when we list them
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 8183d0fc (empty) (no description set)
    second: rzvqmyuk 5ed2222c (empty) (no description set)
    [EOF]
    ");
}

/// Test how sparse patterns are inherited
#[test]
fn test_workspaces_sparse_patterns() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "ws1"]);
    let ws1_path = test_env.env_root().join("ws1");
    let ws2_path = test_env.env_root().join("ws2");
    let ws3_path = test_env.env_root().join("ws3");
    let ws4_path = test_env.env_root().join("ws4");
    let ws5_path = test_env.env_root().join("ws5");
    let ws6_path = test_env.env_root().join("ws6");

    test_env.jj_cmd_ok(&ws1_path, &["sparse", "set", "--clear", "--add=foo"]);
    test_env.jj_cmd_ok(&ws1_path, &["workspace", "add", "../ws2"]);
    let output = test_env.run_jj_in(&ws2_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
    test_env.jj_cmd_ok(&ws2_path, &["sparse", "set", "--add=bar"]);
    test_env.jj_cmd_ok(&ws2_path, &["workspace", "add", "../ws3"]);
    let output = test_env.run_jj_in(&ws3_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    bar
    foo
    [EOF]
    ");
    // --sparse-patterns behavior
    test_env.jj_cmd_ok(
        &ws3_path,
        &["workspace", "add", "--sparse-patterns=copy", "../ws4"],
    );
    let output = test_env.run_jj_in(&ws4_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    bar
    foo
    [EOF]
    ");
    test_env.jj_cmd_ok(
        &ws3_path,
        &["workspace", "add", "--sparse-patterns=full", "../ws5"],
    );
    let output = test_env.run_jj_in(&ws5_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");
    test_env.jj_cmd_ok(
        &ws3_path,
        &["workspace", "add", "--sparse-patterns=empty", "../ws6"],
    );
    let output = test_env.run_jj_in(&ws6_path, ["sparse", "list"]);
    insta::assert_snapshot!(output, @"");
}

/// Test adding a second workspace while the current workspace is editing a
/// merge
#[test]
fn test_workspaces_add_second_workspace_on_merge() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    test_env.jj_cmd_ok(&main_path, &["describe", "-m=left"]);
    test_env.jj_cmd_ok(&main_path, &["new", "@-", "-m=right"]);
    test_env.jj_cmd_ok(&main_path, &["new", "all:@-+", "-m=merge"]);

    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: zsuskuln 35e47bff (empty) merge
    [EOF]
    ");

    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );

    // The new workspace's working-copy commit shares all parents with the old one.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @    35e47bff781e default@
    ├─╮
    │ │ ○  7013a493bd09 second@
    ╭─┬─╯
    │ ○  444b77e99d43
    ○ │  1694f2ddf8ec
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

/// Test that --ignore-working-copy is respected
#[test]
fn test_workspaces_add_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    // TODO: maybe better to error out early?
    let output = test_env.run_jj_in(
        &main_path,
        ["workspace", "add", "--ignore-working-copy", "../secondary"],
    );
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Error: This command must be able to update the working copy.
    Hint: Don't use --ignore-working-copy.
    [EOF]
    [exit status: 1]
    "#);
}

/// Test that --at-op is respected
#[test]
fn test_workspaces_add_at_operation() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file1"), "").unwrap();
    let (_stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["commit", "-m1"]);
    insta::assert_snapshot!(stderr, @r"
    Working copy now at: rlvkpnrz 18d8b994 (empty) (no description set)
    Parent commit      : qpvuntsm 3364a7ed 1
    [EOF]
    ");

    std::fs::write(main_path.join("file2"), "").unwrap();
    let (_stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["commit", "-m2"]);
    insta::assert_snapshot!(stderr, @r"
    Working copy now at: kkmpptxz 2e7dc5ab (empty) (no description set)
    Parent commit      : rlvkpnrz 0dbaa19a 2
    [EOF]
    ");

    // --at-op should disable snapshot in the main workspace, but the newly
    // created workspace should still be writable.
    std::fs::write(main_path.join("file3"), "").unwrap();
    let (_stdout, stderr) = test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--at-op=@-", "../secondary"],
    );
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "../secondary"
    Working copy now at: rzvqmyuk a4d1cbc9 (empty) (no description set)
    Parent commit      : qpvuntsm 3364a7ed 1
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let secondary_path = test_env.env_root().join("secondary");

    // New snapshot can be taken in the secondary workspace.
    std::fs::write(secondary_path.join("file4"), "").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["status"]);
    insta::assert_snapshot!(stdout, @r"
    Working copy changes:
    A file4
    Working copy : rzvqmyuk 2ba74f85 (no description set)
    Parent commit: qpvuntsm 3364a7ed 1
    [EOF]
    ");
    insta::assert_snapshot!(stderr, @r"
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    let output = test_env.run_jj_in(&secondary_path, ["op", "log", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    @  snapshot working copy
    ○    reconcile divergent operations
    ├─╮
    ○ │  commit cd06097124e3e5860867e35c2bb105902c28ea38
    │ ○  create initial working-copy commit in workspace secondary
    │ ○  add workspace 'secondary'
    ├─╯
    ○  snapshot working copy
    ○  commit 1c867a0762e30de4591890ea208849f793742c1b
    ○  snapshot working copy
    ○  add workspace 'default'
    ○
    [EOF]
    ");
}

/// Test adding a workspace, but at a specific revision using '-r'
#[test]
fn test_workspaces_add_workspace_at_revision() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file-1"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "first"]);

    std::fs::write(main_path.join("file-2"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "second"]);

    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: kkmpptxz dadeedb4 (empty) (no description set)
    [EOF]
    ");

    let (_, stderr) = test_env.jj_cmd_ok(
        &main_path,
        &[
            "workspace",
            "add",
            "--name",
            "second",
            "../secondary",
            "-r",
            "@--",
        ],
    );
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "../secondary"
    Working copy now at: zxsnswpr e374e74a (empty) (no description set)
    Parent commit      : qpvuntsm f6097c2f first
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  dadeedb493e8 default@
    ○  c420244c6398
    │ ○  e374e74aa0c8 second@
    ├─╯
    ○  f6097c2f7cac
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r"
    @  e374e74aa0c8 second@
    │ ○  dadeedb493e8 default@
    │ ○  c420244c6398
    ├─╯
    ○  f6097c2f7cac
    ◆  000000000000
    [EOF]
    ");
}

/// Test multiple `-r` flags to `workspace add` to create a workspace
/// working-copy commit with multiple parents.
#[test]
fn test_workspaces_add_workspace_multiple_revisions() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file-1"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "first"]);
    test_env.jj_cmd_ok(&main_path, &["new", "-r", "root()"]);

    std::fs::write(main_path.join("file-2"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "second"]);
    test_env.jj_cmd_ok(&main_path, &["new", "-r", "root()"]);

    std::fs::write(main_path.join("file-3"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "third"]);
    test_env.jj_cmd_ok(&main_path, &["new", "-r", "root()"]);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  5b36783cd11c
    │ ○  6c843d62ca29
    ├─╯
    │ ○  544cd61f2d26
    ├─╯
    │ ○  f6097c2f7cac
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let (_, stderr) = test_env.jj_cmd_ok(
        &main_path,
        &[
            "workspace",
            "add",
            "--name=merge",
            "../merged",
            "-r=description(third)",
            "-r=description(second)",
            "-r=description(first)",
        ],
    );
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "../merged"
    Working copy now at: wmwvqwsz f4fa64f4 (empty) (no description set)
    Parent commit      : mzvwutvl 6c843d62 third
    Parent commit      : kkmpptxz 544cd61f second
    Parent commit      : qpvuntsm f6097c2f first
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  5b36783cd11c default@
    │ ○      f4fa64f40944 merge@
    │ ├─┬─╮
    │ │ │ ○  f6097c2f7cac
    ├─────╯
    │ │ ○  544cd61f2d26
    ├───╯
    │ ○  6c843d62ca29
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_workspaces_add_workspace_from_subdir() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let subdir_path = main_path.join("subdir");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::create_dir(&subdir_path).unwrap();
    std::fs::write(subdir_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "initial"]);

    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz e1038e77 (empty) (no description set)
    [EOF]
    ");

    // Create workspace while in sub-directory of current workspace
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&subdir_path, &["workspace", "add", "../../secondary"]);
    insta::assert_snapshot!(stdout.normalize_backslash(), @"");
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "../../secondary"
    Working copy now at: rzvqmyuk 7ad84461 (empty) (no description set)
    Parent commit      : qpvuntsm a3a43d9e initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces show up when we list them
    let output = test_env.run_jj_in(&secondary_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz e1038e77 (empty) (no description set)
    secondary: rzvqmyuk 7ad84461 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_workspaces_add_workspace_in_current_workspace() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "initial"]);

    // Try to create workspace using name instead of path
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["workspace", "add", "secondary"]);
    insta::assert_snapshot!(stdout.normalize_backslash(), @"");
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "secondary"
    Warning: Workspace created inside current directory. If this was unintentional, delete the "secondary" directory and run `jj workspace forget secondary` to remove it.
    Working copy now at: pmmvwywv 0a77a39d (empty) (no description set)
    Parent commit      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Workspace created despite warning
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 46d9ba8b (no description set)
    secondary: pmmvwywv 0a77a39d (empty) (no description set)
    [EOF]
    ");

    // Use explicit path instead (no warning)
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["workspace", "add", "./third"]);
    insta::assert_snapshot!(stdout.normalize_backslash(), @"");
    insta::assert_snapshot!(stderr.normalize_backslash(), @r#"
    Created workspace in "third"
    Working copy now at: zxsnswpr 64746d4b (empty) (no description set)
    Parent commit      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces created
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 477c647f (no description set)
    secondary: pmmvwywv 0a77a39d (empty) (no description set)
    third: zxsnswpr 64746d4b (empty) (no description set)
    [EOF]
    ");

    // Can see files from the other workspaces in main workspace, since they are
    // child directories and will therefore be snapshotted
    let output = test_env.run_jj_in(&main_path, ["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file
    secondary/file
    third/file
    [EOF]
    ");
}

/// Test making changes to the working copy in a workspace as it gets rewritten
/// from another workspace
#[test]
fn test_workspaces_conflicting_edits() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Make changes in both working copies
    std::fs::write(main_path.join("file"), "changed in main\n").unwrap();
    std::fs::write(secondary_path.join("file"), "changed in second\n").unwrap();
    // Squash the changes from the main workspace into the initial commit (before
    // running any command in the secondary workspace
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["squash"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&secondary_path, ["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation c81af45155a2).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // Same error on second run, and from another command
    let output = test_env.run_jj_in(&secondary_path, ["log"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation c81af45155a2).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // It was detected that the working copy is now stale.
    // Since there was an uncommitted change in the working copy, it should
    // have been committed first (causing divergence)
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["workspace", "update-stale"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Concurrent modification detected, resolving automatically.
    Rebased 1 descendant commits onto commits rewritten by other operation
    Working copy now at: pmmvwywv?? e82cd4ee (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path),
    @r"
    @  e82cd4ee8faa secondary@ (divergent)
    │ ×  30816012e0da (divergent)
    ├─╯
    │ ○  a58c9a9b19ce default@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
    // The stale working copy should have been resolved by the previous command
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r"
    @  e82cd4ee8faa secondary@ (divergent)
    │ ×  30816012e0da (divergent)
    ├─╯
    │ ○  a58c9a9b19ce default@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
}

/// Test a clean working copy that gets rewritten from another workspace
#[test]
fn test_workspaces_updated_by_other() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    std::fs::write(main_path.join("file"), "changed in main\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["squash"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&secondary_path, ["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation c81af45155a2).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // It was detected that the working copy is now stale, but clean. So no
    // divergent commit should be created.
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["workspace", "update-stale"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Working copy now at: pmmvwywv e82cd4ee (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path),
    @r"
    @  e82cd4ee8faa secondary@
    │ ○  a58c9a9b19ce default@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
}

/// Test a clean working copy that gets rewritten from another workspace
#[test]
fn test_workspaces_updated_by_other_automatic() {
    let test_env = TestEnvironment::default();
    test_env.add_config("[snapshot]\nauto-update-stale = true\n");

    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);

    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    std::fs::write(main_path.join("file"), "changed in main\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["squash"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Rebased 1 descendant commits
    Working copy now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");

    // The first working copy gets automatically updated.
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["st"]);
    insta::assert_snapshot!(stdout, @r"
    The working copy has no changes.
    Working copy : pmmvwywv e82cd4ee (empty) (no description set)
    Parent commit: qpvuntsm d4124476 (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(stderr, @r"
    Working copy now at: pmmvwywv e82cd4ee (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path),
    @r"
    @  e82cd4ee8faa secondary@
    │ ○  a58c9a9b19ce default@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
}

#[test_case(false; "manual")]
#[test_case(true; "automatic")]
fn test_workspaces_current_op_discarded_by_other(automatic: bool) {
    let test_env = TestEnvironment::default();
    if automatic {
        test_env.add_config("[snapshot]\nauto-update-stale = true\n");
    }

    // Use the local backend because GitBackend::gc() depends on the git CLI.
    test_env.jj_cmd_ok(
        test_env.env_root(),
        &["init", "main", "--config=ui.allow-init-native=true"],
    );
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("modified"), "base\n").unwrap();
    std::fs::write(main_path.join("deleted"), "base\n").unwrap();
    std::fs::write(main_path.join("sparse"), "base\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);
    std::fs::write(main_path.join("modified"), "main\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);
    // Make unsnapshotted writes in the secondary working copy
    test_env.jj_cmd_ok(
        &secondary_path,
        &[
            "sparse",
            "set",
            "--clear",
            "--add=modified",
            "--add=deleted",
            "--add=added",
        ],
    );
    std::fs::write(secondary_path.join("modified"), "secondary\n").unwrap();
    std::fs::remove_file(secondary_path.join("deleted")).unwrap();
    std::fs::write(secondary_path.join("added"), "secondary\n").unwrap();

    // Create an op by abandoning the parent commit. Importantly, that commit also
    // changes the target tree in the secondary workspace.
    test_env.jj_cmd_ok(&main_path, &["abandon", "@-"]);

    let output = test_env.run_jj_in(
        &main_path,
        [
            "operation",
            "log",
            "--template",
            r#"id.short(10) ++ " " ++ description"#,
        ],
    );
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        @  757bc1140b abandon commit 20dd439c4bd12c6ad56c187ac490bd0141804618f638dc5c4dc92ff9aecba20f152b23160db9dcf61beb31a5cb14091d9def5a36d11c9599cc4d2e5689236af1
        ○  8d4abed655 create initial working-copy commit in workspace secondary
        ○  3de27432e5 add workspace 'secondary'
        ○  bcf69de808 new empty commit
        ○  a36b99a15c snapshot working copy
        ○  ddf023d319 new empty commit
        ○  829c93f6a3 snapshot working copy
        ○  2557266dd2 add workspace 'default'
        ○  0000000000
        [EOF]
        ");
    }

    // Abandon ops, including the one the secondary workspace is currently on.
    test_env.jj_cmd_ok(&main_path, &["operation", "abandon", "..@-"]);
    test_env.jj_cmd_ok(&main_path, &["util", "gc", "--expire=now"]);

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
        @  6c051bd1ccd5 default@
        │ ○  96b31dafdc41 secondary@
        ├─╯
        ○  7c5b25a4fc8f
        ◆  000000000000
        [EOF]
        ");
    }

    if automatic {
        // Run a no-op command to set the randomness seed for commit hashes.
        test_env.run_jj_in(&secondary_path, ["help"]).success();

        let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["st"]);
        insta::assert_snapshot!(stdout, @r"
        Working copy changes:
        A added
        D deleted
        M modified
        Working copy : kmkuslsw 15df8cb5 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit: rzvqmyuk 96b31daf (empty) (no description set)
        [EOF]
        ");
        insta::assert_snapshot!(stderr, @r"
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 8d4abed655badb70b1bab62aa87136619dbc3c8015a8ce8dfb7abfeca4e2f36c713d8f84e070a0613907a6cee7e1cc05323fe1205a319b93fe978f11a060c33c of type operation not found
        Created and checked out recovery commit 76d0126b3e5c
        [EOF]
        ");
    } else {
        let output = test_env.run_jj_in(&secondary_path, ["st"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Error: Could not read working copy's operation.
        Hint: Run `jj workspace update-stale` to recover.
        See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
        [EOF]
        [exit status: 1]
        ");

        let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["workspace", "update-stale"]);
        insta::assert_snapshot!(stdout, @"");
        insta::assert_snapshot!(stderr, @r"
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 8d4abed655badb70b1bab62aa87136619dbc3c8015a8ce8dfb7abfeca4e2f36c713d8f84e070a0613907a6cee7e1cc05323fe1205a319b93fe978f11a060c33c of type operation not found
        Created and checked out recovery commit 76d0126b3e5c
        [EOF]
        ");
    }

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
        @  6c051bd1ccd5 default@
        │ ○  15df8cb57d3f secondary@
        │ ○  96b31dafdc41
        ├─╯
        ○  7c5b25a4fc8f
        ◆  000000000000
        [EOF]
        ");
    }

    // The sparse patterns should remain
    let output = test_env.run_jj_in(&secondary_path, ["sparse", "list"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        added
        deleted
        modified
        [EOF]
        ");
    }
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["st"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(stdout, @r"
        Working copy changes:
        A added
        D deleted
        M modified
        Working copy : kmkuslsw 15df8cb5 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit: rzvqmyuk 96b31daf (empty) (no description set)
        [EOF]
        ");
    }
    insta::allow_duplicates! {
        insta::assert_snapshot!(stderr, @"");
    }
    insta::allow_duplicates! {
        // The modified file should have the same contents it had before (not reset to
        // the base contents)
        insta::assert_snapshot!(std::fs::read_to_string(secondary_path.join("modified")).unwrap(), @r###"
        secondary
        "###);
    }

    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["evolog"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(stdout, @r"
        @  kmkuslsw test.user@example.com 2001-02-03 08:05:18 secondary@ 15df8cb5
        │  RECOVERY COMMIT FROM `jj workspace update-stale`
        ○  kmkuslsw hidden test.user@example.com 2001-02-03 08:05:18 76d0126b
           (empty) RECOVERY COMMIT FROM `jj workspace update-stale`
        [EOF]
        ");
    }
    insta::allow_duplicates! {
        insta::assert_snapshot!(stderr, @"");
    }
}

#[test]
fn test_workspaces_update_stale_noop() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["workspace", "update-stale"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Attempted recovery, but the working copy is not stale
    [EOF]
    ");

    let output = test_env.run_jj_in(
        &main_path,
        ["workspace", "update-stale", "--ignore-working-copy"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command must be able to update the working copy.
    Hint: Don't use --ignore-working-copy.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&main_path, ["op", "log", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    @  add workspace 'default'
    ○
    [EOF]
    ");
}

/// Test "update-stale" in a dirty, but not stale working copy.
#[test]
fn test_workspaces_update_stale_snapshot() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "changed in main\n").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);
    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);

    // Record new operation in one workspace.
    test_env.jj_cmd_ok(&main_path, &["new"]);

    // Snapshot the other working copy, which unfortunately results in concurrent
    // operations, but should be resolved cleanly.
    std::fs::write(secondary_path.join("file"), "changed in second\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&secondary_path, &["workspace", "update-stale"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Concurrent modification detected, resolving automatically.
    Attempted recovery, but the working copy is not stale
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r"
    @  e672fd8fefac secondary@
    │ ○  ea37b073f5ab default@
    │ ○  b13c81dedc64
    ├─╯
    ○  e6e9989f1179
    ◆  000000000000
    [EOF]
    ");
}

/// Test forgetting workspaces
#[test]
fn test_workspaces_forget() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../secondary"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["workspace", "forget"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");

    // When listing workspaces, only the secondary workspace shows up
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    secondary: pmmvwywv 18463f43 (empty) (no description set)
    [EOF]
    ");

    // `jj status` tells us that there's no working copy here
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["st"]);
    insta::assert_snapshot!(stdout, @r"
    No working copy
    [EOF]
    ");
    insta::assert_snapshot!(stderr, @"");

    // The old working copy doesn't get an "@" in the log output
    // TODO: It seems useful to still have the "secondary@" marker here even though
    // there's only one workspace. We should show it when the command is not run
    // from that workspace.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    ○  18463f438cc9
    ○  4e8f9d2be039
    ◆  000000000000
    [EOF]
    ");

    // Revision "@" cannot be used
    let output = test_env.run_jj_in(&main_path, ["log", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Workspace `default` doesn't have a working-copy commit
    [EOF]
    [exit status: 1]
    ");

    // Try to add back the workspace
    // TODO: We should make this just add it back instead of failing
    let output = test_env.run_jj_in(&main_path, ["workspace", "add", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Workspace already exists
    [EOF]
    [exit status: 1]
    ");

    // Add a third workspace...
    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../third"]);
    // ... and then forget it, and the secondary workspace too
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&main_path, &["workspace", "forget", "secondary", "third"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    // No workspaces left
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_workspaces_forget_multi_transaction() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["new"]);

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../second"]);
    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../third"]);

    // there should be three workspaces
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 909d51b1 (empty) (no description set)
    second: pmmvwywv 18463f43 (empty) (no description set)
    third: rzvqmyuk cc383fa2 (empty) (no description set)
    [EOF]
    ");

    // delete two at once, in a single tx
    test_env.jj_cmd_ok(&main_path, &["workspace", "forget", "second", "third"]);
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 909d51b1 (empty) (no description set)
    [EOF]
    ");

    // the op log should have multiple workspaces forgotten in a single tx
    let output = test_env.run_jj_in(&main_path, ["op", "log", "--limit", "1"]);
    insta::assert_snapshot!(output, @r"
    @  60b2b5a71a84 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  forget workspaces second, third
    │  args: jj workspace forget second third
    [EOF]
    ");

    // now, undo, and that should restore both workspaces
    test_env.jj_cmd_ok(&main_path, &["op", "undo"]);

    // finally, there should be three workspaces at the end
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 909d51b1 (empty) (no description set)
    second: pmmvwywv 18463f43 (empty) (no description set)
    third: rzvqmyuk cc383fa2 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_workspaces_forget_abandon_commits() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");

    std::fs::write(main_path.join("file"), "contents").unwrap();

    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../second"]);
    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../third"]);
    test_env.jj_cmd_ok(&main_path, &["workspace", "add", "../fourth"]);
    let third_path = test_env.env_root().join("third");
    test_env.jj_cmd_ok(&third_path, &["edit", "second@"]);
    let fourth_path = test_env.env_root().join("fourth");
    test_env.jj_cmd_ok(&fourth_path, &["edit", "second@"]);

    // there should be four workspaces, three of which are at the same empty commit
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 4e8f9d2b (no description set)
    fourth: uuqppmxq 57d63245 (empty) (no description set)
    second: uuqppmxq 57d63245 (empty) (no description set)
    third: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  4e8f9d2be039 default@
    │ ○  57d63245a308 fourth@ second@ third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the default workspace (should not abandon commit since not empty)
    test_env
        .run_jj_in(&main_path, ["workspace", "forget", "default"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    ○  57d63245a308 fourth@ second@ third@
    │ ○  4e8f9d2be039
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the second workspace (should not abandon commit since other workspaces
    // still have commit checked out)
    test_env
        .run_jj_in(&main_path, ["workspace", "forget", "second"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    ○  57d63245a308 fourth@ third@
    │ ○  4e8f9d2be039
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the last 2 workspaces (commit should be abandoned now even though
    // forgotten in same tx)
    test_env
        .run_jj_in(&main_path, ["workspace", "forget", "third", "fourth"])
        .success();
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    ○  4e8f9d2be039
    ◆  000000000000
    [EOF]
    ");
}

/// Test context of commit summary template
#[test]
fn test_list_workspaces_template() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    test_env.add_config(
        r#"
        templates.commit_summary = """commit_id.short() ++ " " ++ description.first_line() ++
                                      if(current_working_copy, " (current)")"""
        "#,
    );
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    std::fs::write(main_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&main_path, &["commit", "-m", "initial"]);
    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );

    // "current_working_copy" should point to the workspace we operate on
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: 8183d0fcaa4c  (current)
    second: 0a77a39d7d6f 
    [EOF]
    ");

    let output = test_env.run_jj_in(&secondary_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: 8183d0fcaa4c 
    second: 0a77a39d7d6f  (current)
    [EOF]
    ");
}

/// Test getting the workspace root from primary and secondary workspaces
#[test]
fn test_workspaces_root() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let secondary_path = test_env.env_root().join("secondary");

    let output = test_env.run_jj_in(&main_path, ["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/main
    [EOF]
    ");
    let main_subdir_path = main_path.join("subdir");
    std::fs::create_dir(&main_subdir_path).unwrap();
    let output = test_env.run_jj_in(&main_subdir_path, ["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/main
    [EOF]
    ");

    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "secondary", "../secondary"],
    );
    let output = test_env.run_jj_in(&secondary_path, ["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/secondary
    [EOF]
    ");
    let secondary_subdir_path = secondary_path.join("subdir");
    std::fs::create_dir(&secondary_subdir_path).unwrap();
    let output = test_env.run_jj_in(&secondary_subdir_path, ["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/secondary
    [EOF]
    ");
}

#[test]
fn test_debug_snapshot() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "contents").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["debug", "snapshot"]);
    let output = test_env.run_jj_in(&repo_path, ["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  c55ebc67e3db test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  eac759b9ab75 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "initial"]);
    let output = test_env.run_jj_in(&repo_path, ["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  c9a40b951848 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  describe commit 4e8f9d2be039994f589b4e57ac5e9488703e604d
    │  args: jj describe -m initial
    ○  c55ebc67e3db test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  eac759b9ab75 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_workspaces_rename_nothing_changed() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    let (stdout, stderr) = test_env.jj_cmd_ok(&main_path, &["workspace", "rename", "default"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r"
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_workspaces_rename_new_workspace_name_already_used() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );
    let output = test_env.run_jj_in(&main_path, ["workspace", "rename", "second"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to rename a workspace
    Caused by: Workspace second already exists
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_workspaces_rename_forgotten_workspace() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );
    test_env.jj_cmd_ok(&main_path, &["workspace", "forget", "second"]);
    let secondary_path = test_env.env_root().join("secondary");
    let output = test_env.run_jj_in(&secondary_path, ["workspace", "rename", "third"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The current workspace 'second' is not tracked in the repo.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_workspaces_rename_workspace() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "main"]);
    let main_path = test_env.env_root().join("main");
    test_env.jj_cmd_ok(
        &main_path,
        &["workspace", "add", "--name", "second", "../secondary"],
    );
    let secondary_path = test_env.env_root().join("secondary");

    // Both workspaces show up when we list them
    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 230dd059 (empty) (no description set)
    second: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");

    let output = test_env.run_jj_in(&secondary_path, ["workspace", "rename", "third"]);
    insta::assert_snapshot!(output, @"");

    let output = test_env.run_jj_in(&main_path, ["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 230dd059 (empty) (no description set)
    third: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");

    // Can see the working-copy commit in each workspace in the log output.
    insta::assert_snapshot!(get_log_output(&test_env, &main_path), @r"
    @  230dd059e1b0 default@
    │ ○  57d63245a308 third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &secondary_path), @r"
    @  57d63245a308 third@
    │ ○  230dd059e1b0 default@
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> CommandOutput {
    let template = r#"
    separate(" ",
      commit_id.short(),
      working_copies,
      if(divergent, "(divergent)"),
    )
    "#;
    test_env.run_jj_in(cwd, ["log", "-T", template, "-r", "all()"])
}
