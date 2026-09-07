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
    @  f63ee16f9553 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    Internal error: The repo was loaded at operation 63a847e21093, which seems to be a sibling of the working copy's operation 4bffc36765bd
    Hint: Run `jj op integrate 4bffc36765bd` to add the working copy's operation to the operation log.
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
    @    f28c8b5750f5 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 4bffc36765bd9b796cc8bd631a09e92c60b686ef98c316a9baa7164c4f6f9d0071871d5678b014fec33b920d1177776b748af2a6395fc28031f92ab75a1d31ec
    ○ │  4bffc36765bd test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=first'
    │ ○  63a847e21093 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  new empty commit
    │    args: jj new '-m=second' --ignore-working-copy
    ○  f63ee16f9553 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    Rebased 1 descendant commits.
    [EOF]
    ");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation 02d52b605abd, which seems to be a sibling of the working copy's operation ec162faff901
    Hint: Run `jj op integrate ec162faff901` to add the working copy's operation to the operation log.
    [EOF]
    [exit status: 255]
    ");

    // Integrate the operation
    let output = work_dir.run_jj(["op", "integrate", &unintegrated_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits onto commits rewritten by other operation.
    The specified operation has been integrated with other existing operations.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    000180467209 test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate ec162faff90142f42a8fe4e514c61c06c10756a11b3ee0d645500b6f4280225e20c0b21b18759c23c37f144c6643492f170b8b809ae5f932f5d73dcf651941d1
    ○ │  ec162faff901 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=child 2'
    │ ○  02d52b605abd test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=parent' --ignore-working-copy
    ○  10ad95d988d6 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new --no-edit '-m=child 1'
    ○  f63ee16f9553 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Child 2 was successfully rebased
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:12 9780be6d
    │  (empty) child 2
    │ ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 ce1fb6c9
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
    Internal error: The repo was loaded at operation 383be968e923, which seems to be a sibling of the working copy's operation 49428a30d110
    Hint: Run `jj op integrate 49428a30d110` to add the working copy's operation to the operation log.
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
    @    ddbaedf9ee34 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 49428a30d110a365178cdca38a32661982fb260d18df5c19f2b76df283c9cd2fd367dce1b124a7a75cfe9487eeee3b5031bd1b2a851261a3fc3f1d9dba41d768
    ○ │  49428a30d110 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe '-m=left'
    │ ○  383be968e923 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=right' --ignore-working-copy
    ○  f63ee16f9553 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
