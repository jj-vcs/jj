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
    default: rlvkpnrz 504e3d8c (empty) (no description set)
    [EOF]
    ");

    let output = main_dir.run_jj(["workspace", "add", "--name", "second", "../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Working copy  (@) now at: rzvqmyuk bcc858e1 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7b22a8cb initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  504e3d8c1bcd default@
    │ ○  bcc858e1d93f second@
    ├─╯
    ○  7b22a8cbe888 "initial"
    ◆  000000000000
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r#"
    @  bcc858e1d93f second@
    │ ○  504e3d8c1bcd default@
    ├─╯
    ○  7b22a8cbe888 "initial"
    ◆  000000000000
    [EOF]
    "#);

    // Both workspaces show up when we list them
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 504e3d8c (empty) (no description set)
    second: rzvqmyuk bcc858e1 (empty) (no description set)
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
    main_dir.run_jj(["new", "@-+", "-m=merge"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: zsuskuln 46ed31b6 (empty) merge
    [EOF]
    ");

    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();

    // The new workspace's working-copy commit shares all parents with the old one.
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @    46ed31b61ce9 default@ "merge"
    ├─╮
    │ │ ○  d23b2d4ff55c second@
    ╭─┬─╯
    │ ○  3c52528f5893 "left"
    ○ │  a3155ab1bf5a "right"
    ├─╯
    ◆  000000000000
    [EOF]
    "#);
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
    Working copy  (@) now at: rlvkpnrz 59e07459 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 9e4b0b91 1
    [EOF]
    ");

    main_dir.write_file("file2", "");
    let output = main_dir.run_jj(["commit", "-m2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 6e9610ac (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 8b7259b9 2
    [EOF]
    ");

    // --at-op should disable snapshot in the main workspace, but the newly
    // created workspace should still be writable.
    main_dir.write_file("file3", "");
    let output = main_dir.run_jj(["workspace", "add", "--at-op=@-", "../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../secondary"
    Working copy  (@) now at: rzvqmyuk b8772476 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 9e4b0b91 1
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
    Working copy  (@) : rzvqmyuk f2ff8257 (no description set)
    Parent commit (@-): qpvuntsm 9e4b0b91 1
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
    ○ │  commit 9152e822279787a168ddf4cede6440a21faa00d7
    │ ○  create initial working-copy commit in workspace secondary
    │ ○  add workspace 'secondary'
    ├─╯
    ○  snapshot working copy
    ○  commit 093c3c9624b6cfe22b310586f5638792aa80e6d7
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
    default: kkmpptxz 5ac9178d (empty) (no description set)
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
    Working copy  (@) now at: zxsnswpr ea5860fb (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 27473635 first
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Can see the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  5ac9178da8b2 default@
    ○  a47d8a593529 "second"
    │ ○  ea5860fbd622 second@
    ├─╯
    ○  27473635a942 "first"
    ◆  000000000000
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r#"
    @  ea5860fbd622 second@
    │ ○  5ac9178da8b2 default@
    │ ○  a47d8a593529 "second"
    ├─╯
    ○  27473635a942 "first"
    ◆  000000000000
    [EOF]
    "#);
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

    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  8d23abddc924
    │ ○  eba7f49e2358 "third"
    ├─╯
    │ ○  62444a45efcf "second"
    ├─╯
    │ ○  27473635a942 "first"
    ├─╯
    ◆  000000000000
    [EOF]
    "#);

    let output = main_dir.run_jj([
        "workspace",
        "add",
        "--name=merge",
        "../merged",
        "-r=subject(glob:third)",
        "-r=subject(glob:second)",
        "-r=subject(glob:first)",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../merged"
    Working copy  (@) now at: wmwvqwsz 2d7c9a2d (empty) (no description set)
    Parent commit (@-)      : mzvwutvl eba7f49e third
    Parent commit (@-)      : kkmpptxz 62444a45 second
    Parent commit (@-)      : qpvuntsm 27473635 first
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  8d23abddc924 default@
    │ ○      2d7c9a2d41dc merge@
    │ ├─┬─╮
    │ │ │ ○  27473635a942 "first"
    ├─────╯
    │ │ ○  62444a45efcf "second"
    ├───╯
    │ ○  eba7f49e2358 "third"
    ├─╯
    ◆  000000000000
    [EOF]
    "#);
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
    default: rlvkpnrz 0ba0ff35 (empty) (no description set)
    [EOF]
    ");

    // Create workspace while in sub-directory of current workspace
    let output = subdir_dir.run_jj(["workspace", "add", "../../secondary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "../../secondary"
    Working copy  (@) now at: rzvqmyuk dea1be10 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 80b67806 initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces show up when we list them
    let output = secondary_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz 0ba0ff35 (empty) (no description set)
    secondary: rzvqmyuk dea1be10 (empty) (no description set)
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
    Working copy  (@) now at: pmmvwywv 058f604d (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7b22a8cb initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Workspace created despite warning
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r###"
    default: rlvkpnrz 504e3d8c (empty) (no description set)
    secondary: pmmvwywv 058f604d (empty) (no description set)
    [EOF]
    "###);

    // Use explicit path instead (no warning)
    let output = main_dir.run_jj(["workspace", "add", "./third"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Created workspace in "third"
    Working copy  (@) now at: zxsnswpr 1c1effec (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7b22a8cb initial
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Both workspaces created
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r###"
    default: rlvkpnrz 504e3d8c (empty) (no description set)
    secondary: pmmvwywv 058f604d (empty) (no description set)
    third: zxsnswpr 1c1effec (empty) (no description set)
    [EOF]
    "###);

    let output = main_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r###"
    file
    [EOF]
    "###);
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
    @  393250c59e39 default@
    │ ○  547036666102 secondary@
    ├─╯
    ○  9a462e35578a
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
    Working copy  (@) now at: mzvwutvl 3a9b690d (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  3a9b690d6e67 default@
    │ ○  90f3d42e0bff secondary@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");
    let output = secondary_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation bd4f780d0422).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // Same error on second run, and from another command
    let output = secondary_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation bd4f780d0422).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
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
    Working copy  (@) now at: pmmvwywv?? 90f3d42e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit 90f3d42e0bff
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir),
    @r"
    @  90f3d42e0bff secondary@ (divergent)
    │ ×  3ef90f18334b (divergent)
    ├─╯
    │ ○  3a9b690d6e67 default@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");
    // The stale working copy should have been resolved by the previous command
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
    @  90f3d42e0bff secondary@ (divergent)
    │ ×  3ef90f18334b (divergent)
    ├─╯
    │ ○  3a9b690d6e67 default@
    ├─╯
    ○  b853f7c8b006
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
    @  393250c59e39 default@
    │ ○  547036666102 secondary@
    ├─╯
    ○  9a462e35578a
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    main_dir.write_file("file", "changed in main\n");
    let output = main_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 3a9b690d (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  3a9b690d6e67 default@
    │ ○  90f3d42e0bff secondary@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");
    let output = secondary_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation bd4f780d0422).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    // It was detected that the working copy is now stale, but clean. So no
    // divergent commit should be created.
    let output = secondary_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: pmmvwywv 90f3d42e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit 90f3d42e0bff
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir),
    @r"
    @  90f3d42e0bff secondary@
    │ ○  3a9b690d6e67 default@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");
}

/// Test a clean working copy that gets rewritten from another workspace
#[test]
fn test_workspaces_updated_by_other_automatic() {
    let test_env = TestEnvironment::default();
    test_env.add_config("snapshot.auto-update-stale = true\n");

    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.write_file("file", "contents\n");
    main_dir.run_jj(["new"]).success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  393250c59e39 default@
    │ ○  547036666102 secondary@
    ├─╯
    ○  9a462e35578a
    ◆  000000000000
    [EOF]
    ");

    // Rewrite the check-out commit in one workspace.
    main_dir.write_file("file", "changed in main\n");
    let output = main_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 3a9b690d (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    [EOF]
    ");

    // The secondary workspace's working-copy commit was updated.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  3a9b690d6e67 default@
    │ ○  90f3d42e0bff secondary@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");

    // The first working copy gets automatically updated.
    let output = secondary_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : pmmvwywv 90f3d42e (empty) (no description set)
    Parent commit (@-): qpvuntsm b853f7c8 (no description set)
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: pmmvwywv 90f3d42e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm b853f7c8 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit 90f3d42e0bff
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&secondary_dir),
    @r"
    @  90f3d42e0bff secondary@
    │ ○  3a9b690d6e67 default@
    ├─╯
    ○  b853f7c8b006
    ◆  000000000000
    [EOF]
    ");
}

#[test_case(false; "manual")]
#[test_case(true; "automatic")]
fn test_workspaces_current_op_discarded_by_other(automatic: bool) {
    let test_env = TestEnvironment::default();
    if automatic {
        test_env.add_config("snapshot.auto-update-stale = true\n");
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
        @  b0789def13 abandon commit de90575a14d8b9198dc0930f9de4a69f846ded36
        ○  778c9aae54 create initial working-copy commit in workspace secondary
        ○  219d4aca5c add workspace 'secondary'
        ○  31ad55e98c new empty commit
        ○  4ba7680cbe snapshot working copy
        ○  9739176f19 new empty commit
        ○  4b5baa44b7 snapshot working copy
        ○  8f47435a39 add workspace 'default'
        ○  0000000000
        [EOF]
        ");
    }

    // Abandon ops, including the one the secondary workspace is currently on.
    main_dir.run_jj(["operation", "abandon", "..@-"]).success();
    main_dir.run_jj(["util", "gc", "--expire=now"]).success();

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&main_dir), @r"
        @  320bc89effc9 default@
        │ ○  891f00062e10 secondary@
        ├─╯
        ○  367415be5b44
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
        Working copy  (@) : kmkuslsw 18851b39 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit (@-): rzvqmyuk 891f0006 (empty) (no description set)
        [EOF]
        ------- stderr -------
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 778c9aae54957e842bede2223fda227be33e08061732276a4cfb7b431a3e146e5c62187d640aa883095d3b2c6cf43d31ad5fde72076bb9a88b8594fb8b5e6606 of type operation not found
        Created and checked out recovery commit 866928d1e0fd
        [EOF]
        ");
    } else {
        let output = secondary_dir.run_jj(["st"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Error: Could not read working copy's operation.
        Hint: Run `jj workspace update-stale` to recover.
        See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
        [EOF]
        [exit status: 1]
        ");

        let output = secondary_dir.run_jj(["workspace", "update-stale"]);
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Failed to read working copy's current operation; attempting recovery. Error message from read attempt: Object 778c9aae54957e842bede2223fda227be33e08061732276a4cfb7b431a3e146e5c62187d640aa883095d3b2c6cf43d31ad5fde72076bb9a88b8594fb8b5e6606 of type operation not found
        Created and checked out recovery commit 866928d1e0fd
        [EOF]
        ");
    }

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&main_dir), @r#"
        @  320bc89effc9 default@
        │ ○  18851b397d09 secondary@ "RECOVERY COMMIT FROM `jj workspace update-stale`"
        │ ○  891f00062e10
        ├─╯
        ○  367415be5b44
        ◆  000000000000
        [EOF]
        "#);
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
        Working copy  (@) : kmkuslsw 18851b39 RECOVERY COMMIT FROM `jj workspace update-stale`
        Parent commit (@-): rzvqmyuk 891f0006 (empty) (no description set)
        [EOF]
        ");
    }
    insta::allow_duplicates! {
        // The modified file should have the same contents it had before (not reset to
        // the base contents)
        insta::assert_snapshot!(secondary_dir.read_file("modified"), @"secondary");
    }

    let output = secondary_dir.run_jj(["evolog"]);
    if automatic {
        insta::assert_snapshot!(output, @r"
        @  kmkuslsw test.user@example.com 2001-02-03 08:05:18 secondary@ 18851b39
        │  RECOVERY COMMIT FROM `jj workspace update-stale`
        │  -- operation 0a26da4b0149 snapshot working copy
        ○  kmkuslsw hidden test.user@example.com 2001-02-03 08:05:18 866928d1
           (empty) RECOVERY COMMIT FROM `jj workspace update-stale`
           -- operation 83f707034db1 recovery commit
        [EOF]
        ");
    } else {
        insta::assert_snapshot!(output, @r"
        @  kmkuslsw test.user@example.com 2001-02-03 08:05:18 secondary@ 18851b39
        │  RECOVERY COMMIT FROM `jj workspace update-stale`
        │  -- operation 0f876590219e snapshot working copy
        ○  kmkuslsw hidden test.user@example.com 2001-02-03 08:05:18 866928d1
           (empty) RECOVERY COMMIT FROM `jj workspace update-stale`
           -- operation 83f707034db1 recovery commit
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

/// If the working copy was last updated to an unpublished operation, it should
/// be reported, even if the latest published operation has the same tree.
#[test]
fn test_workspaces_unpublished_operation_same_tree() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.run_jj(["desc", "-m=A"]).success();
    let a_op_id = main_dir.current_operation_id();
    main_dir.run_jj(["new", "-m=B"]).success();
    let b_op_id = main_dir.current_operation_id();
    // Make the repo forget about the B operation
    main_dir.remove_file(format!(".jj/repo/op_heads/heads/{b_op_id}"));
    main_dir.write_file(format!(".jj/repo/op_heads/heads/{a_op_id}"), "");
    main_dir
        .run_jj(["new", "-m=C", "--ignore-working-copy"])
        .success();
    // The working copy should be stale and should require a `jj workspace
    // update-stale`
    let output = main_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: The repo was loaded at operation 502db81004ba, which seems to be a sibling of the working copy's operation 48631817a82e
    [EOF]
    [exit status: 255]
    ");
    let output = main_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: zsuskuln 36a15ac4 (empty) C
    Parent commit (@-)      : qpvuntsm 8777db25 (empty) A
    Updated working copy to fresh commit 36a15ac414e8
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
    @  35d779b3baea secondary@
    │ ○  c9516583d53b default@
    │ ○  f6ae7810ef56
    ├─╯
    ○  7d5738ba9943
    ◆  000000000000
    [EOF]
    ");
}

/// Test that "workspace update-stale" works in colocated repos.
///
/// This is a regression test for a bug introduced in commit 7a296ca1 where
/// the reload-to-HEAD logic (added to fix a race condition) would break
/// "workspace update-stale" by reloading the repo to HEAD before snapshotting,
/// even though recovery intentionally loads at an old operation.
#[test]
fn test_colocated_workspace_update_stale() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "main"])
        .success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");
    let git_repo = git::open(main_dir.root());

    main_dir.write_file("file", "contents\n");
    main_dir.run_jj(["new"]).success();

    // Create new bookmarked revision from the main workspace.
    main_dir
        .run_jj(["new", "--no-edit", "root()", "-mold book1"])
        .success();
    main_dir
        .run_jj(["bookmark", "set", "-rsubject(glob:'old book1')", "book1"])
        .success();

    main_dir
        .run_jj(["workspace", "add", "../secondary"])
        .success();

    // Rewrite the check-out commit from the secondary workspace.
    // This makes the main (colocated) workspace's working copy stale.
    secondary_dir.write_file("file", "changed in secondary\n");
    secondary_dir.run_jj(["squash"]).success();

    // Update and export the bookmark from the secondary workspace.
    secondary_dir
        .run_jj(["new", "--no-edit", "root()", "-mnew book1"])
        .success();
    secondary_dir
        .run_jj([
            "bookmark",
            "set",
            "-rsubject(glob:'new book1')",
            "--allow-backwards",
            "book1",
        ])
        .success();
    secondary_dir.run_jj(["git", "export"]).success();

    // Create new Git ref and commit which will be imported later by "jj
    // workspace update-stale".
    git::add_commit(&git_repo, "refs/heads/book2", "file", b"", "book2", &[]);

    insta::assert_snapshot!(get_log_output(&secondary_dir), @r#"
    @  9cb8253861b5 secondary@
    │ ○  f562bf82f2da default@
    ├─╯
    ○  30ed2f28b710
    │ ○  e97ad7861f78 book1 "new book1"
    ├─╯
    │ ○  f656b467890b "old book1"
    ├─╯
    ◆  000000000000
    [EOF]
    "#);

    // The main workspace's working copy is now stale.
    let output = main_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation a3fbf68cb3f8).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");

    // Before the fix, this would fail with the same "working copy is stale" error
    // because the colocated repo reload logic would reload to HEAD before
    // snapshotting, breaking the recovery.
    let output = main_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz f562bf82 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 30ed2f28 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Updated working copy to fresh commit f562bf82f2da
    Done importing changes from the underlying Git repo.
    [EOF]
    ");

    // Verify the workspace is now up-to-date. New bookmark "book2" should have
    // been imported by the previous command.
    let output = main_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : rlvkpnrz f562bf82 (empty) (no description set)
    Parent commit (@-): qpvuntsm 30ed2f28 (no description set)
    [EOF]
    ");

    // The updated bookmark "book1" shouldn't be re-imported as an external
    // change. If it were, the "old book1" revision would be abandoned.
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  f562bf82f2da default@
    │ ○  9cb8253861b5 secondary@
    ├─╯
    ○  30ed2f28b710
    │ ○  7fe3ff3b9a60 book2 "book2"
    ├─╯
    │ ○  e97ad7861f78 book1 "new book1"
    ├─╯
    │ ○  f656b467890b "old book1"
    ├─╯
    ◆  000000000000
    [EOF]
    "#);
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
    secondary: pmmvwywv 31da1455 (empty) (no description set)
    [EOF]
    ");

    // The old working copy doesn't get an "@" in the log output
    // TODO: It seems useful to still have the "secondary@" marker here even though
    // there's only one workspace. We should show it when the command is not run
    // from that workspace.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  31da14559558
    ○  006bd1130b84
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
    default: rlvkpnrz f6bf8819 (empty) (no description set)
    second: pmmvwywv 31da1455 (empty) (no description set)
    third: rzvqmyuk bf5b5b4d (empty) (no description set)
    [EOF]
    ");

    // delete two at once, in a single tx
    main_dir
        .run_jj(["workspace", "forget", "second", "third"])
        .success();
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz f6bf8819 (empty) (no description set)
    [EOF]
    ");

    // the op log should have multiple workspaces forgotten in a single tx
    let output = main_dir.run_jj(["op", "log", "--limit", "1"]);
    insta::assert_snapshot!(output, @r"
    @  d3aded9a10b6 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  forget workspaces second, third
    │  args: jj workspace forget second third
    [EOF]
    ");

    // now, undo, and that should restore both workspaces
    main_dir.run_jj(["undo"]).success();

    // finally, there should be three workspaces at the end
    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: rlvkpnrz f6bf8819 (empty) (no description set)
    second: pmmvwywv 31da1455 (empty) (no description set)
    third: rzvqmyuk bf5b5b4d (empty) (no description set)
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
    default: qpvuntsm 006bd113 (no description set)
    fourth: uuqppmxq 94f41578 (empty) (no description set)
    second: uuqppmxq 94f41578 (empty) (no description set)
    third: uuqppmxq 94f41578 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  006bd1130b84 default@
    │ ○  94f41578a9e1 fourth@ second@ third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the default workspace (should not abandon commit since not empty)
    main_dir
        .run_jj(["workspace", "forget", "default"])
        .success();
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  94f41578a9e1 fourth@ second@ third@
    │ ○  006bd1130b84
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // delete the second workspace (should not abandon commit since other workspaces
    // still have commit checked out)
    main_dir.run_jj(["workspace", "forget", "second"]).success();
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    ○  94f41578a9e1 fourth@ third@
    │ ○  006bd1130b84
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
    ○  006bd1130b84
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
        templates.workspace_list = """name ++ ": " ++ target.commit_id().short() ++ " " ++
                                      target.description().first_line() ++
                                      if(target.current_working_copy(), " (current)") ++ "\n""""
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
    default: 504e3d8c1bcd  (current)
    second: 058f604dffcd 
    [EOF]
    ");

    let output = secondary_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: 504e3d8c1bcd 
    second: 058f604dffcd  (current)
    [EOF]
    ");

    // Using template option
    let template = r#"name ++ ": " ++ target.commit_id().short() ++ "\n""#;
    let output = main_dir.run_jj(["workspace", "list", "-T", template]);
    insta::assert_snapshot!(output, @r"
    default: 504e3d8c1bcd
    second: 058f604dffcd
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
    @  594dfebf2565 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  8f47435a3990 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  81e4a0f2e793 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  describe commit 006bd1130b84e90ab082adeabd7409270d5a86da
    │  args: jj describe -m initial
    ○  594dfebf2565 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  8f47435a3990 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    default: qpvuntsm e8849ae1 (empty) (no description set)
    second: uuqppmxq 94f41578 (empty) (no description set)
    [EOF]
    ");

    let output = secondary_dir.run_jj(["workspace", "rename", "third"]);
    insta::assert_snapshot!(output, @"");

    let output = main_dir.run_jj(["workspace", "list"]);
    insta::assert_snapshot!(output, @r"
    default: qpvuntsm e8849ae1 (empty) (no description set)
    third: uuqppmxq 94f41578 (empty) (no description set)
    [EOF]
    ");

    // Can see the working-copy commit in each workspace in the log output.
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  e8849ae12c70 default@
    │ ○  94f41578a9e1 third@
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&secondary_dir), @r"
    @  94f41578a9e1 third@
    │ ○  e8849ae12c70 default@
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
      bookmarks,
      working_copies,
      if(divergent, "(divergent)"),
      surround('"', '"', description.first_line()),
    )
    "#;
    work_dir.run_jj(["log", "-T", template, "-r", "all()"])
}
