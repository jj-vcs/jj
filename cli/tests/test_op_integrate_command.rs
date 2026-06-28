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

use std::path::PathBuf;

use testutils::TestResult;

use crate::common::TestEnvironment;

/// Integrating an already integrated operation is a no-op
#[test]
fn test_integrate_integrated_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "integrate", "@"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @  90267f31f904 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_integrate_sibling_operation() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let base_op_id = work_dir.current_operation_id();
    work_dir.run_jj(["new", "-m=first"]).success();
    let unintegrated_id = work_dir.current_operation_id();
    assert_ne!(unintegrated_id, base_op_id);
    // Manually remove the last operation from the operation log
    let heads_dir = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "op_heads", "heads"]));
    std::fs::rename(
        heads_dir.join(&unintegrated_id),
        heads_dir.join(&base_op_id),
    )?;
    // We use --ignore-working-copy to prevent the automatic reloading of the repo
    // at the unintegrated operation that's mentioned in
    // `.jj/working_copy/checkout`.
    let output = work_dir.run_jj(["new", "-m=second", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation f18ccd3bfa74, which seems to be a sibling of the working copy's operation 7a2b59297e0e
    Hint: Run `jj op integrate 7a2b59297e0e` to add the working copy's operation to the operation log.
    [EOF]
    [exit status: 255]
    ");

    // Integrate the operation
    let output = work_dir.run_jj(["op", "integrate", &unintegrated_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    The specified operation has been integrated with other existing operations.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    8e55362edb3b test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 7a2b59297e0e60fd21595c2f349842a15d04f9ba0a432c982a562a1473fa9fd4f69a04a6907493e62b45bc2a7d8a412d8502b4e0e9ed1a13646cfc6bd5437b86
    ○ │  7a2b59297e0e test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=first'
    │ ○  f18ccd3bfa74 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  new empty commit
    │    args: jj new '-m=second' --ignore-working-copy
    ○  90267f31f904 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_integrate_rebase_descendants() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["new", "--no-edit", "-m=child 1"])
        .success();

    let base_op_id = work_dir.current_operation_id();
    work_dir.run_jj(["new", "-m=child 2"]).success();
    let unintegrated_id = work_dir.current_operation_id();
    assert_ne!(unintegrated_id, base_op_id);
    // Manually remove the last operation from the operation log
    let heads_dir = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "op_heads", "heads"]));
    std::fs::rename(
        heads_dir.join(&unintegrated_id),
        heads_dir.join(&base_op_id),
    )?;

    // We use --ignore-working-copy to prevent the automatic reloading of the repo
    // at the unintegrated operation that's mentioned in
    // `.jj/working_copy/checkout`.
    let output = work_dir.run_jj(["describe", "-m=parent", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    [EOF]
    ");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation de930e1713ca, which seems to be a sibling of the working copy's operation 1d87b9209f7f
    Hint: Run `jj op integrate 1d87b9209f7f` to add the working copy's operation to the operation log.
    [EOF]
    [exit status: 255]
    ");

    // Integrate the operation
    let output = work_dir.run_jj(["op", "integrate", &unintegrated_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits onto commits rewritten by other operation
    The specified operation has been integrated with other existing operations.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    c589859f0d79 test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 1d87b9209f7f9cf04ffaaa849deac095ac2d942b97471fb1968c668b9120ce5a805fddb3d3834fe19c9afa1e11e780b887968885de9979642b2e64a5cd7dd6b4
    ○ │  1d87b9209f7f test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=child 2'
    │ ○  de930e1713ca test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=parent' --ignore-working-copy
    ○  5d5fd3f3a2fe test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new --no-edit '-m=child 1'
    ○  90267f31f904 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Child 2 was successfully rebased
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  nkmpptxz test.user@example.com 2001-02-03 08:05:12 4a1fe649
    │  (empty) child 2
    │ ○  ylvkpnrz test.user@example.com 2001-02-03 08:05:10 afe37616
    ├─╯  (empty) child 1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:10 5f8729eb
    │  (empty) parent
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_integrate_concurrent_operations() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let base_op_id = work_dir.current_operation_id();
    work_dir.run_jj(["describe", "-m=left"]).success();
    let unintegrated_id = work_dir.current_operation_id();
    assert_ne!(unintegrated_id, base_op_id);
    // Manually remove the last operation from the operation log
    let heads_dir = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "op_heads", "heads"]));
    std::fs::rename(
        heads_dir.join(&unintegrated_id),
        heads_dir.join(&base_op_id),
    )?;

    // We use --ignore-working-copy to prevent the automatic reloading of the repo
    // at the unintegrated operation that's mentioned in
    // `.jj/working_copy/checkout`.
    let output = work_dir.run_jj(["describe", "-m=right", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation e18a1a944247, which seems to be a sibling of the working copy's operation 1bc121d24e06
    Hint: Run `jj op integrate 1bc121d24e06` to add the working copy's operation to the operation log.
    [EOF]
    [exit status: 255]
    ");

    // Integrate the operation
    let output = work_dir.run_jj(["op", "integrate", &unintegrated_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    The specified operation has been integrated with other existing operations.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    da91ff6e5aec test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 1bc121d24e06835c4c22e5f02b623c1deffec8b1bbf68a41e1f0b720d116c6b931e5588fcd262ea41962a4d5e746de19d23323ca08baa13cd0b5bc25402242ff
    ○ │  1bc121d24e06 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe '-m=left'
    │ ○  e18a1a944247 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=right' --ignore-working-copy
    ○  90267f31f904 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Produces divergence equivalent to concurrent `jj describe`
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  qpvuntsm/1 test.user@example.com 2001-02-03 08:05:08 3c52528f (divergent)
    │  (empty) left
    │ ○  qpvuntsm/0 test.user@example.com 2001-02-03 08:05:09 fc350e9c (divergent)
    ├─╯  (empty) right
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    Ok(())
}
