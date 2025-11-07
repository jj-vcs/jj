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

use itertools::Itertools as _;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_concurrent_operation_divergence() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "message 1"]).success();
    work_dir
        .run_jj(["describe", "-m", "message 2", "--at-op", "@-"])
        .success();

    // "--at-op=@" disables op heads merging, and prints head operation ids.
    let output = work_dir.run_jj(["op", "log", "--at-op=@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: b2cffe4f3026, d8ced2ea64a8
    [EOF]
    [exit status: 1]
    "#);

    // "op log --at-op" should work without merging the head operations
    let output = work_dir.run_jj(["op", "log", "--at-op=d8ced2ea64a8"]);
    insta::assert_snapshot!(output, @r"
    @  d8ced2ea64a8 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 2' --at-op @-
    ○  8f47435a3990 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // We should be informed about the concurrent modification
    let output = get_log_output(&work_dir);
    insta::assert_snapshot!(output, @r"
    @  message 1
    │ ○  message 2
    ├─╯
    ◆
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_concurrent_operations_auto_rebase() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "contents");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["describe", "-m", "rewritten"]).success();
    work_dir
        .run_jj(["new", "--at-op=@-", "-m", "new child"])
        .success();

    // We should be informed about the concurrent modification
    let output = get_log_output(&work_dir);
    insta::assert_snapshot!(output, @r"
    ○  new child
    @  rewritten
    ◆
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Rebased 1 descendant commits onto commits rewritten by other operation
    [EOF]
    ");
}

#[test]
fn test_concurrent_operations_wc_modified() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "contents\n");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["new", "-m", "new child1"]).success();
    work_dir
        .run_jj(["new", "--at-op=@-", "-m", "new child2"])
        .success();
    work_dir.write_file("file", "modified\n");

    // We should be informed about the concurrent modification
    let output = get_log_output(&work_dir);
    insta::assert_snapshot!(output, @r"
    @  new child1
    │ ○  new child2
    ├─╯
    ○  initial
    ◆
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output, @r"
    diff --git a/file b/file
    index 12f00e90b6..2e0996000b 100644
    --- a/file
    +++ b/file
    @@ -1,1 +1,1 @@
    -contents
    +modified
    [EOF]
    ");

    // The working copy should be committed after merging the operations
    let output = work_dir.run_jj(["op", "log", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    @  snapshot working copy
    ○    reconcile divergent operations
    ├─╮
    ○ │  new empty commit
    │ ○  new empty commit
    ├─╯
    ○  describe commit 9a462e35578a347e6a3951bf7a58ad7146959a8b
    ○  snapshot working copy
    ○  add workspace 'default'
    ○
    [EOF]
    ");
}

#[test]
fn test_concurrent_snapshot_wc_reloadable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let op_heads_dir = work_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("op_heads")
        .join("heads");

    work_dir.write_file("base", "");
    work_dir.run_jj(["commit", "-m", "initial"]).success();

    // Create new commit and checkout it.
    work_dir.write_file("child1", "");
    work_dir.run_jj(["commit", "-m", "new child1"]).success();

    let template = r#"id.short() ++ "\n" ++ description ++ "\n" ++ tags"#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  a631dcf37fea
    │  commit c91a0909a9d3f3d8392ba9fab88f4b40fc0810ee
    │  args: jj commit -m 'new child1'
    ○  2b8e6f8683dc
    │  snapshot working copy
    │  args: jj commit -m 'new child1'
    ○  2e1c4ffb74ca
    │  commit 9af4c151edead0304de97ce3a0b414552921a425
    │  args: jj commit -m initial
    ○  cfe73d1664ae
    │  snapshot working copy
    │  args: jj commit -m initial
    ○  8f47435a3990
    │  add workspace 'default'
    ○  000000000000

    [EOF]
    ");
    let template = r#"id ++ "\n""#;
    let output = work_dir.run_jj(["op", "log", "--no-graph", "-T", template]);
    let [op_id_after_snapshot, _, op_id_before_snapshot] =
        output.stdout.raw().lines().next_array().unwrap();
    insta::assert_snapshot!(op_id_after_snapshot[..12], @"a631dcf37fea");
    insta::assert_snapshot!(op_id_before_snapshot[..12], @"2e1c4ffb74ca");

    // Simulate a concurrent operation that began from the "initial" operation
    // (before the "child1" snapshot) but finished after the "child1"
    // snapshot and commit.
    std::fs::rename(
        op_heads_dir.join(op_id_after_snapshot),
        op_heads_dir.join(op_id_before_snapshot),
    )
    .unwrap();
    work_dir.write_file("child2", "");
    let output = work_dir.run_jj(["describe", "-m", "new child2"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 493da83e new child2
    Parent commit (@-)      : rlvkpnrz 15bd889d new child1
    [EOF]
    "###);

    // Since the repo can be reloaded before snapshotting, "child2" should be
    // a child of "child1", not of "initial".
    let output = work_dir.run_jj(["log", "-T", "description", "-s"]);
    insta::assert_snapshot!(output, @r"
    @  new child2
    │  A child2
    ○  new child1
    │  A child1
    ○  initial
    │  A base
    ◆
    [EOF]
    ");
}

#[test]
#[should_panic(expected = "Race condition detected")]
fn test_git_head_race_condition() {
    // Test for race condition where concurrent jj processes create divergent
    // operations when importing/exporting Git HEAD. This test spawns two
    // processes in parallel: one running `jj debug snapshot` repeatedly
    // (which imports Git HEAD) and another running `jj next/prev` repeatedly
    // (which exports Git HEAD). Without the fix, this would create divergent
    // operations.

    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Create a large initial working copy to make snapshotting slower
    for j in 0..200 {
        let filename = format!("file_{j}");
        let content = format!("content for file {j}\n").repeat(100);
        work_dir.write_file(&filename, &content);
    }
    work_dir.run_jj(["commit", "-m", "initial"]).success();

    // Create additional commits to iterate through with jj next/prev
    for i in 0..10 {
        for j in 0..50 {
            let filename = format!("file_{i}_{j}");
            let content = format!("content for commit {i} file {j}\n").repeat(100);
            work_dir.write_file(&filename, &content);
        }
        work_dir
            .run_jj(["commit", "-m", &format!("commit {i}")])
            .success();
    }

    // Extract environment from TestEnvironment to use in spawned processes
    let base_cmd = test_env.new_jj_cmd();
    let jj_bin = base_cmd.get_program().to_owned();
    let base_env: Vec<_> = base_cmd
        .get_envs()
        .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v.to_owned())))
        // Filter out timestamp and randomness seed so each command gets different values
        .filter(|(k, _)| k != "JJ_TIMESTAMP" && k != "JJ_OP_TIMESTAMP" && k != "JJ_RANDOMNESS_SEED")
        .collect();

    let repo_path = work_dir.root();
    let duration = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();

    std::thread::scope(|s| {
        s.spawn(|| {
            while start.elapsed() < duration {
                // Snapshot repeatedly without delay
                let mut cmd = std::process::Command::new(&jj_bin);
                cmd.current_dir(repo_path);
                cmd.args(["debug", "snapshot"]);
                for (key, value) in &base_env {
                    cmd.env(key, value);
                }
                let _ = cmd.output();
            }
        });

        s.spawn(|| {
            let mut count = 0;
            while start.elapsed() < duration {
                // Go through commit history back and forth
                let direction = if count % 20 < 10 { "prev" } else { "next" };
                let mut cmd = std::process::Command::new(&jj_bin);
                cmd.current_dir(repo_path);
                cmd.arg(direction);
                for (key, value) in &base_env {
                    cmd.env(key, value);
                }
                let _ = cmd.output();
                count += 1;
            }
        });
    });

    // Check for concurrent operations
    let output = work_dir.run_jj(["op", "log", "-T", "description", "--ignore-working-copy"]);
    let concurrent_count = output
        .stdout
        .raw()
        .lines()
        .filter(|line| line.contains("import git head"))
        .count();

    if concurrent_count > 0 {
        eprintln!("Found {concurrent_count} concurrent operations:");
        eprintln!("{}", output.stdout.raw());
        panic!("Race condition detected: {concurrent_count} concurrent operations found");
    }
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["log", "-T", "description"])
}
