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
    Hint: Try specifying one of the operations by ID: c4991b0d765d, 9087f4bfa0de
    [EOF]
    [exit status: 1]
    "#);

    // "op log --at-op" should work without merging the head operations
    let output = work_dir.run_jj(["op", "log", "--at-op=9087f4bfa0de"]);
    insta::assert_snapshot!(output, @r"
    @  9087f4bfa0de test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 2' --at-op @-
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // We should be informed about the concurrent modification
    let output = work_dir.run_jj(["log", "-T", "description"]);
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
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  aea22f517416 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 006bd1130b84e90ab082adeabd7409270d5a86da
    │  args: jj describe -m initial
    ○  b5433cef81fc test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj describe -m initial
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    let op_id_hex = output.stdout.raw()[3..15].to_string();

    work_dir.run_jj(["describe", "-m", "rewritten"]).success();
    work_dir
        .run_jj(["new", "--at-op", &op_id_hex, "-m", "new child"])
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
    let output = work_dir.run_jj(["op", "log"]).success();
    let op_id_hex = output.stdout.raw()[3..15].to_string();

    work_dir
        .run_jj(["new", "--at-op", &op_id_hex, "-m", "new child1"])
        .success();
    work_dir
        .run_jj(["new", "--at-op", &op_id_hex, "-m", "new child2"])
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

    let template = r#"id ++ "\n" ++ description ++ "\n" ++ tags"#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  9009349b5198b481138bc77a268145474c016cd218f1f038317f2fa6f25e4e896e0c4c4e3271b188cb2726938b92ba8135ee2fb62ddf82b2bd41a9c839337b04
    │  commit c91a0909a9d3f3d8392ba9fab88f4b40fc0810ee
    │  args: jj commit -m 'new child1'
    ○  0b8f20a1bd79d95aa49a517fbeb0b58caa024ba887c4b8da5b0feee6e2376757fa78fec2a07ee593f61d43eb3487c1ff389df5fb2c9489c313819193ccc0e401
    │  snapshot working copy
    │  args: jj commit -m 'new child1'
    ○  b544b8f44a8b084f965cdb3e5f32b4f3423899c1ac004036567125cb596f3eded7f4141561078463e31e7b3dd5832912348d970c78ed2468d118efe584f6e9f0
    │  commit 9af4c151edead0304de97ce3a0b414552921a425
    │  args: jj commit -m initial
    ○  8b49a5a258dd19ac6ea757de66f57933978e6e7af948da48247ac98d5933ea6d0f78ee2a3c08757ded39ad87805a2e528dd55fd2e1da71da28e501c0c3454d9a
    │  snapshot working copy
    │  args: jj commit -m initial
    ○  2affa702525487ca490c4bc8a9a365adf75f972efb5888dd58716de7603e822ba1ed1ed0a50132ee44572bb9d819f37589d0ceb790b397ddcc88c976fde2bf02
    │  add workspace 'default'
    ○  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000

    [EOF]
    ");
    let op_log_lines = output.stdout.raw().lines().collect_vec();
    let current_op_id = op_log_lines[0].split_once("  ").unwrap().1;
    let previous_op_id = op_log_lines[6].split_once("  ").unwrap().1;

    // Another process started from the "initial" operation, but snapshots after
    // the "child1" checkout have been completed.
    std::fs::rename(
        op_heads_dir.join(current_op_id),
        op_heads_dir.join(previous_op_id),
    )
    .unwrap();
    work_dir.write_file("child2", "");
    let output = work_dir.run_jj(["describe", "-m", "new child2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 5a2a6177 new child2
    Parent commit (@-)      : rlvkpnrz 15bd889d new child1
    [EOF]
    ");

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

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["log", "-T", "description"])
}
