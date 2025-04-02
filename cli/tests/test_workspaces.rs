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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

/// Test adding a second workspace
#[test]
fn test_workspaces_add_second_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["commit", "-m", "initial"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 8183d0fc (empty) (no description set)
    [EOF]
    ");

    let output = main_dir.run_jj(["workspace", "add", "--name", "second", "../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Working copy  (@) now at: rzvqmyuk 5ed2222c (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  8183d0fcaa4c default@
    │ ○  5ed2222c28e2 second@
    ├─╯
    ○  751b12b7b981
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
    @  5ed2222c28e2 second@
    │ ○  8183d0fcaa4c default@
    ├─╯
    ○  751b12b7b981
    ◆  000000000000
    [EOF]
    ");

    // Both workspaces show up when we list them
    let output = main_dir.run_jj(["workspace", "list"]);
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
    test_env.run_jj_in(".", ["git", "init", "ws1"]).success();
    let ws1_dir = test_env.work_dir("ws1");
    let ws2_dir = test_env.work_dir("ws2");
    let ws3_dir = test_env.work_dir("ws3");
    let ws4_dir = test_env.work_dir("ws4");
    let ws5_dir = test_env.work_dir("ws5");
    let ws6_dir = test_env.work_dir("ws6");

    ws1_dir
        .run_jj(["sparse", "set", "--clear", "--add=foo"])
        .success();
    ws1_dir.run_jj(["workspace", "add", "../ws2"]).success();
    let output = ws2_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
    ws2_dir.run_jj(["sparse", "set", "--add=bar"]).success();
    ws2_dir.run_jj(["workspace", "add", "../ws3"]).success();
    let output = ws3_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    bar
    foo
    [EOF]
    ");
    // --sparse-patterns behavior
    ws3_dir
        .run_jj(["workspace", "add", "--sparse-patterns=copy", "../ws4"])
        .success();
    let output = ws4_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    bar
    foo
    [EOF]
    ");
    ws3_dir
        .run_jj(["workspace", "add", "--sparse-patterns=full", "../ws5"])
        .success();
    let output = ws5_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @r"
    .
    [EOF]
    ");
    ws3_dir
        .run_jj(["workspace", "add", "--sparse-patterns=empty", "../ws6"])
        .success();
    let output = ws6_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"");
}

/// Test adding a second workspace while the current workspace is editing a
/// merge
#[test]
fn test_workspaces_add_second_workspace_on_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.run_jj(["describe", "-m=left"]).success();
    main_dir.run_jj(["new", "@-", "-m=right"]).success();
    main_dir.run_jj(["new", "all:@-+", "-m=merge"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: zsuskuln 35e47bff (empty) merge
    [EOF]
    ");

    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();

    // The new workspace's working-copy commit shares all parents with the old one.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    // TODO: maybe better to error out early?
    let output = main_dir.run_jj(["workspace", "add", "--ignore-working-copy", "../secondary"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file1", "");
    let output = main_dir.run_jj(["commit", "-m1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 18d8b994 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 3364a7ed 1
    [EOF]
    ");

    main_dir.write_file("file2", "");
    let output = main_dir.run_jj(["commit", "-m2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 2e7dc5ab (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 0dbaa19a 2
    [EOF]
    ");

    // --at-op should disable snapshot in the main workspace, but the newly
    // created workspace should still be writable.
    main_dir.write_file("file3", "");
    let output = main_dir.run_jj(["workspace", "add", "--at-op=@-", "../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Working copy  (@) now at: rzvqmyuk a4d1cbc9 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 3364a7ed 1
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let secondary_dir = test_env.work_dir("secondary");

    // New snapshot can be taken in the secondary workspace.
    secondary_dir.write_file("file4", "");
    let output = secondary_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file4
    Working copy  (@) : rzvqmyuk 2ba74f85 (no description set)
    Parent commit (@-): qpvuntsm 3364a7ed 1
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    let output = secondary_dir.run_jj(["op", "log", "-Tdescription"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file-1", "contents");
    main_dir.run_jj(["commit", "-m", "first"]).success();

    main_dir.write_file("file-2", "contents");
    main_dir.run_jj(["commit", "-m", "second"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: kkmpptxz dadeedb4 (empty) (no description set)
    [EOF]
    ");

    let output = main_dir.run_jj([
        "workspace",
        "add",
        "--name",
        "second",
        "../secondary",
        "-r",
        "@--",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Working copy  (@) now at: zxsnswpr e374e74a (empty) (no description set)
    Parent commit (@-)      : qpvuntsm f6097c2f first
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  dadeedb493e8 default@
    ○  c420244c6398
    │ ○  e374e74aa0c8 second@
    ├─╯
    ○  f6097c2f7cac
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file-1", "contents");
    main_dir.run_jj(["commit", "-m", "first"]).success();
    main_dir.run_jj(["new", "-r", "root()"]).success();

    main_dir.write_file("file-2", "contents");
    main_dir.run_jj(["commit", "-m", "second"]).success();
    main_dir.run_jj(["new", "-r", "root()"]).success();

    main_dir.write_file("file-3", "contents");
    main_dir.run_jj(["commit", "-m", "third"]).success();
    main_dir.run_jj(["new", "-r", "root()"]).success();

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
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

    let output = main_dir.run_jj([
        "workspace",
        "add",
        "--name=merge",
        "../merged",
        "-r=description(third)",
        "-r=description(second)",
        "-r=description(first)",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../merged"
    Working copy  (@) now at: wmwvqwsz f4fa64f4 (empty) (no description set)
    Parent commit (@-)      : mzvwutvl 6c843d62 third
    Parent commit (@-)      : kkmpptxz 544cd61f second
    Parent commit (@-)      : qpvuntsm f6097c2f first
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    let subdir_dir = main_dir.create_dir("subdir");
    subdir_dir.write_file("file", "contents");
    main_dir.run_jj(["commit", "-m", "initial"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz e1038e77 (empty) (no description set)
    [EOF]
    ");

    // Create workspace while in sub-directory of current workspace
    let output = subdir_dir.run_jj(["workspace", "add", "../../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../../secondary"
    Working copy  (@) now at: rzvqmyuk 7ad84461 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm a3a43d9e initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces show up when we list them
    let output = secondary_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz e1038e77 (empty) (no description set)
    secondary: rzvqmyuk 7ad84461 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_workspaces_add_workspace_in_current_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["commit", "-m", "initial"]).success();

    // Try to create workspace using name instead of path
    let output = main_dir.run_jj(["workspace", "add", "secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "secondary"
    Warning: Workspace created inside current directory. If this was unintentional, delete the "secondary" directory and run `jj workspace forget secondary` to remove it.
    Working copy  (@) now at: pmmvwywv 0a77a39d (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Workspace created despite warning
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 46d9ba8b (no description set)
    secondary: pmmvwywv 0a77a39d (empty) (no description set)
    [EOF]
    ");

    // Use explicit path instead (no warning)
    let output = main_dir.run_jj(["workspace", "add", "./third"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "third"
    Working copy  (@) now at: zxsnswpr 64746d4b (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 751b12b7 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces created
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 477c647f (no description set)
    secondary: pmmvwywv 0a77a39d (empty) (no description set)
    third: zxsnswpr 64746d4b (empty) (no description set)
    [EOF]
    ");

    // Can see files from the other workspaces in main workspace, since they are
    // child directories and will therefore be snapshotted
    let output = main_dir.run_jj(["file", "list"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents\n");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Make changes in both working copies
    main_dir.write_file("file", "changed in main\n");
    secondary_dir.write_file("file", "changed in second\n");
    // Squash the changes from the main workspace into the initial commit (before
    // running any command in the secondary workspace
    let output = main_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
    let output = secondary_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation c81af45155a2).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // Same error on second run, and from another command
    let output = secondary_dir.run_jj(["log"]);
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
    let output = secondary_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Rebased 1 descendant commits onto commits rewritten by other operation
    Working copy  (@) now at: pmmvwywv?? e82cd4ee (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir),
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
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents\n");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    main_dir.write_file("file", "changed in main\n");
    let output = main_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");
    let output = secondary_dir.run_jj(["st"]);
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
    let output = secondary_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: pmmvwywv e82cd4ee (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir),
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

    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents\n");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  06b57f44a3ca default@
    │ ○  3224de8ae048 secondary@
    ├─╯
    ○  506f4ec3c2c6
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    main_dir.write_file("file", "changed in main\n");
    let output = main_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl a58c9a9b (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  a58c9a9b19ce default@
    │ ○  e82cd4ee8faa secondary@
    ├─╯
    ○  d41244767d45
    ◆  000000000000
    [EOF]
    ");

    // The first working copy gets automatically updated.
    let output = secondary_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : pmmvwywv e82cd4ee (empty) (no description set)
    Parent commit (@-): qpvuntsm d4124476 (no description set)
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: pmmvwywv e82cd4ee (empty) (no description set)
    Parent commit (@-)      : qpvuntsm d4124476 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit e82cd4ee8faa
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&secondary_dir),
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

    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("modified", "base\n");
    main_dir.write_file("deleted", "base\n");
    main_dir.write_file("sparse", "base\n");
    main_dir.run_jj(["new"]).success();
    main_dir.write_file("modified", "main\n");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();
    // Make unsnapshotted writes in the secondary working copy
    secondary_dir
        .run_jj([
            "sparse",
            "set",
            "--clear",
            "--add=modified",
            "--add=deleted",
            "--add=added",
        ])
        .success();
    secondary_dir.write_file("modified", "secondary\n");
    secondary_dir.remove_file("deleted");
    secondary_dir.write_file("added", "secondary\n");

    // Create an op by abandoning the parent commit. Importantly, that commit also
    // changes the target tree in the secondary workspace.
    main_dir.run_jj(["abandon", "@-"]).success();

    let output = main_dir.run_jj([
        "operation",
        "log",
        "--template",
        r#"id.short(10) ++ " " ++ description"#,
    ]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        @  64d9b429d9 abandon commit dc638a7f20571df2c846c84d1469b9fcd0edafc0
        ○  129f2dca87 create initial working-copy commit in workspace secondary
        ○  1516a7f851 add workspace 'secondary'
        ○  19bf99b2b1 new empty commit
        ○  38c9c18632 snapshot working copy
        ○  5e4f01399f new empty commit
        ○  299bc7a187 snapshot working copy
        ○  eac759b9ab add workspace 'default'
        ○  0000000000
        [EOF]
        ");
    }

    // Abandon ops, including the one the secondary workspace is currently on.
    main_dir.run_jj(["operation", "abandon", "..@-"]).success();
    main_dir.run_jj(["util", "gc", "--expire=now"]).success();

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&main_dir), @r"
        @  2d02e07ed190 default@
        │ ○  3df3bf89ddf1 secondary@
        ├─╯
        ○  e734830954d8
        ◆  000000000000
        [EOF]
        ");
    }

    if automatic {
        // Run a no-op command to set the randomness seed for commit hashes.
        secondary_dir.run_jj(["help"]).success();

        let output = secondary_dir.run_jj(["st"]);
        insta::assert_snapshot!(output, @r"
        Working copy changes:
        C {modified => added}
        D deleted
        M modified
        Working copy  (@) : kmkuslsw 0b518140 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit (@-): rzvqmyuk 3df3bf89 (empty) (no description set)
        [EOF]
        ------- stderr -------
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 129f2dca870b954e2966fba35893bb47a5bc6358db6e8c4065cee91d2d49073efc3e055b9b81269a13c443d964abb18e83d25de73db2376ff434c876c59976ac of type operation not found
        Created and checked out recovery commit 8ed0355c5d31
        [EOF]
        ");
    } else {
        let output = secondary_dir.run_jj(["st"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Error: Could not read working copy's operation.
        Hint: Run `jj workspace update-stale` to recover.
        See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
        [EOF]
        [exit status: 1]
        ");

        let output = secondary_dir.run_jj(["workspace", "update-stale"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 129f2dca870b954e2966fba35893bb47a5bc6358db6e8c4065cee91d2d49073efc3e055b9b81269a13c443d964abb18e83d25de73db2376ff434c876c59976ac of type operation not found
        Created and checked out recovery commit 8ed0355c5d31
        [EOF]
        ");
    }

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&main_dir), @r"
        @  2d02e07ed190 default@
        │ ○  0b5181407d03 secondary@
        │ ○  3df3bf89ddf1
        ├─╯
        ○  e734830954d8
        ◆  000000000000
        [EOF]
        ");
    }

    // The sparse patterns should remain
    let output = secondary_dir.run_jj(["sparse", "list"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        added
        deleted
        modified
        [EOF]
        ");
    }
    let output = secondary_dir.run_jj(["st"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        Working copy changes:
        C {modified => added}
        D deleted
        M modified
        Working copy  (@) : kmkuslsw 0b518140 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit (@-): rzvqmyuk 3df3bf89 (empty) (no description set)
        [EOF]
        ");
    }
    insta::allow_duplicates! {
        // The modified file should have the same contents it had before (not reset to
        // the base contents)
        insta::assert_snapshot!(secondary_dir.read_file("modified"), @"secondary");
    }

    let output = secondary_dir.run_jj(["evolog"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        @  kmkuslsw test.user@example.com 2001-02-03 08:05:18 secondary@ 0b518140
        │  RECOVERY COMMIT FROM `jj workspace update-stale`
        ○  kmkuslsw hidden test.user@example.com 2001-02-03 08:05:18 8ed0355c
           (empty) RECOVERY COMMIT FROM `jj workspace update-stale`
        [EOF]
        ");
    }
}

#[test]
fn test_workspaces_update_stale_noop() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    let output = main_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Attempted recovery, but the working copy is not stale
    [EOF]
    ");

    let output = main_dir.run_jj(["workspace", "update-stale", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command must be able to update the working copy.
    Hint: Don't use --ignore-working-copy.
    [EOF]
    [exit status: 1]
    ");

    let output = main_dir.run_jj(["op", "log", "-Tdescription"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "changed in main\n");
    main_dir.run_jj(["new"]).success();
    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    // Record new operation in one workspace.
    main_dir.run_jj(["new"]).success();

    // Snapshot the other working copy, which unfortunately results in concurrent
    // operations, but should be resolved cleanly.
    secondary_dir.write_file("file", "changed in second\n");
    let output = secondary_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Attempted recovery, but the working copy is not stale
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();
    let output = main_dir.run_jj(["workspace", "forget"]);
    insta::assert_snapshot!(output, @"");

    // When listing workspaces, only the secondary workspace shows up
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    secondary: pmmvwywv 18463f43 (empty) (no description set)
    [EOF]
    ");

    // `jj status` tells us that there's no working copy here
    let output = main_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    No working copy
    [EOF]
    ");

    // The old working copy doesn't get an "@" in the log output
    // TODO: It seems useful to still have the "secondary@" marker here even though
    // there's only one workspace. We should show it when the command is not run
    // from that workspace.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  18463f438cc9
    ○  4e8f9d2be039
    ◆  000000000000
    [EOF]
    ");

    // Revision "@" cannot be used
    let output = main_dir.run_jj(["log", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Workspace `default` doesn't have a working-copy commit
    [EOF]
    [exit status: 1]
    ");

    // Try to add back the workspace
    // TODO: We should make this just add it back instead of failing
    let output = main_dir.run_jj(["workspace", "add", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Workspace already exists
    [EOF]
    [exit status: 1]
    ");

    // Add a third workspace...
    main_dir.run_jj(["workspace", "add", "../third"]).success();
    // ... and then forget it, and the secondary workspace too
    let output = main_dir.run_jj(["workspace", "forget", "secondary", "third"]);
    insta::assert_snapshot!(output, @"");
    // No workspaces left
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_workspaces_forget_multi_transaction() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["new"]).success();

    main_dir.run_jj(["workspace", "add", "../second"]).success();
    main_dir.run_jj(["workspace", "add", "../third"]).success();

    // there should be three workspaces
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 909d51b1 (empty) (no description set)
    second: pmmvwywv 18463f43 (empty) (no description set)
    third: rzvqmyuk cc383fa2 (empty) (no description set)
    [EOF]
    ");

    // delete two at once, in a single tx
    main_dir
        .run_jj(["workspace", "forget", "second", "third"])
        .success();
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 909d51b1 (empty) (no description set)
    [EOF]
    ");

    // the op log should have multiple workspaces forgotten in a single tx
    let output = main_dir.run_jj(["op", "log", "--limit", "1"]);
    insta::assert_snapshot!(output, @r"
    @  60b2b5a71a84 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  forget workspaces second, third
    │  args: jj workspace forget second third
    [EOF]
    ");

    // now, undo, and that should restore both workspaces
    main_dir.run_jj(["op", "undo"]).success();

    // finally, there should be three workspaces at the end
    let output = main_dir.run_jj(["workspace", "list"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file", "contents");

    main_dir.run_jj(["workspace", "add", "../second"]).success();
    main_dir.run_jj(["workspace", "add", "../third"]).success();
    main_dir.run_jj(["workspace", "add", "../fourth"]).success();
    let third_dir = test_env.work_dir("third");
    third_dir.run_jj(["edit", "second@"]).success();
    let fourth_dir = test_env.work_dir("fourth");
    fourth_dir.run_jj(["edit", "second@"]).success();

    // there should be four workspaces, three of which are at the same empty commit
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 4e8f9d2b (no description set)
    fourth: uuqppmxq 57d63245 (empty) (no description set)
    second: uuqppmxq 57d63245 (empty) (no description set)
    third: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  4e8f9d2be039 default@
    │ ○  57d63245a308 fourth@ second@ third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the default workspace (should not abandon commit since not empty)
    main_dir
        .run_jj(["workspace", "forget", "default"])
        .success();
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  57d63245a308 fourth@ second@ third@
    │ ○  4e8f9d2be039
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the second workspace (should not abandon commit since other workspaces
    // still have commit checked out)
    main_dir.run_jj(["workspace", "forget", "second"]).success();
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  57d63245a308 fourth@ third@
    │ ○  4e8f9d2be039
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the last 2 workspaces (commit should be abandoned now even though
    // forgotten in same tx)
    main_dir
        .run_jj(["workspace", "forget", "third", "fourth"])
        .success();
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  4e8f9d2be039
    ◆  000000000000
    [EOF]
    ");
}

/// Test context of commit summary template
#[test]
fn test_list_workspaces_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    test_env.add_config(
        r#"
        templates.commit_summary = """commit_id.short() ++ " " ++ description.first_line() ++
                                      if(current_working_copy, " (current)")"""
        "#,
    );
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["commit", "-m", "initial"]).success();
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();

    // "current_working_copy" should point to the workspace we operate on
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: 8183d0fcaa4c  (current)
    second: 0a77a39d7d6f 
    [EOF]
    ");

    let output = secondary_dir.run_jj(["workspace", "list"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    let output = main_dir.run_jj(["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/main
    [EOF]
    ");
    let main_subdir_dir = main_dir.create_dir("subdir");
    let output = main_subdir_dir.run_jj(["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/main
    [EOF]
    ");

    main_dir
        .run_jj(["workspace", "add", "--name", "secondary", "../secondary"])
        .success();
    let output = secondary_dir.run_jj(["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/secondary
    [EOF]
    ");
    let secondary_subdir_dir = secondary_dir.create_dir("subdir");
    let output = secondary_subdir_dir.run_jj(["workspace", "root"]);
    insta::assert_snapshot!(output, @r"
    $TEST_ENV/secondary
    [EOF]
    ");
}

#[test]
fn test_debug_snapshot() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "contents");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  c55ebc67e3db test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  eac759b9ab75 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    let output = work_dir.run_jj(["op", "log"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let output = main_dir.run_jj(["workspace", "rename", "default"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_workspaces_rename_new_workspace_name_already_used() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    let output = main_dir.run_jj(["workspace", "rename", "second"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    main_dir.run_jj(["workspace", "forget", "second"]).success();
    let secondary_dir = test_env.work_dir("secondary");
    let output = secondary_dir.run_jj(["workspace", "rename", "third"]);
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
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    let secondary_dir = test_env.work_dir("secondary");

    // Both workspaces show up when we list them
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 230dd059 (empty) (no description set)
    second: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");

    let output = secondary_dir.run_jj(["workspace", "rename", "third"]);
    insta::assert_snapshot!(output, @"");

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm 230dd059 (empty) (no description set)
    third: uuqppmxq 57d63245 (empty) (no description set)
    [EOF]
    ");

    // Can see the working-copy commit in each workspace in the log output.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  230dd059e1b0 default@
    │ ○  57d63245a308 third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
    @  57d63245a308 third@
    │ ○  230dd059e1b0 default@
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
      commit_id.short(),
      working_copies,
      if(divergent, "(divergent)"),
    )
    "#;
    work_dir.run_jj(["log", "-T", template, "-r", "all()"])
}
