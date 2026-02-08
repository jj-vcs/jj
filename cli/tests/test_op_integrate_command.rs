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
    @  92406f686752 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_integrate_sibling_operation() {
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
    )
    .unwrap();
    // We use --ignore-working-copy to prevent the automatic reloading of the repo
    // at the unintegrated operation that's mentioned in
    // `.jj/working_copy/checkout`.
    let output = work_dir.run_jj(["new", "-m=second", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation dee5e11ab6ee, which seems to be a sibling of the working copy's operation 64048c7b6840
    Hint: Run `jj op integrate 64048c7b6840` to add the working copy's operation to the operation log.
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
    @    529eaf22e97a test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate 64048c7b68400d7e092f6dba0ce10abe3f08dba46e9626b2345e2d366fbc9a2a09ec9a7ff52c498eb56202def7974bff406dfe485c6a653b56526d7eff5c5354
    ○ │  64048c7b6840 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=first'
    │ ○  dee5e11ab6ee test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  new empty commit
    │    args: jj new '-m=second' --ignore-working-copy
    ○  92406f686752 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_integrate_rebase_descendants() {
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
    )
    .unwrap();

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
    Internal error: The repo was loaded at operation c75ce541cdcb, which seems to be a sibling of the working copy's operation a0a02a3ba8eb
    Hint: Run `jj op integrate a0a02a3ba8eb` to add the working copy's operation to the operation log.
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
    @    d6b0370e1336 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate a0a02a3ba8eb2071be7996d0e8fb0ded50cab5f9e2883c20ce7cdfe9dad65866e783ab1e9f56d491e1b00bebe19acd9e46b75f13be0da61e00857207ab505c9d
    ○ │  a0a02a3ba8eb test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '-m=child 2'
    │ ○  c75ce541cdcb test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=parent' --ignore-working-copy
    ○  9fd1fe09079a test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new --no-edit '-m=child 1'
    ○  92406f686752 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
}

#[test]
fn test_integrate_concurrent_operations() {
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
    )
    .unwrap();

    // We use --ignore-working-copy to prevent the automatic reloading of the repo
    // at the unintegrated operation that's mentioned in
    // `.jj/working_copy/checkout`.
    let output = work_dir.run_jj(["describe", "-m=right", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"");

    // The working copy should now be at the old unintegrated sibling operation
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: The repo was loaded at operation 2d4e659af794, which seems to be a sibling of the working copy's operation aa0946a4becd
    Hint: Run `jj op integrate aa0946a4becd` to add the working copy's operation to the operation log.
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
    @    535bbc3dc3d5 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op integrate aa0946a4becdf50dbd3c69f37031adef1af3023f41c9ec9e569e60a97138c7e19e524882ed4170356a31d574a87141aa12168d83e46857864792202c245d3890
    ○ │  aa0946a4becd test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe '-m=left'
    │ ○  2d4e659af794 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe '-m=right' --ignore-working-copy
    ○  92406f686752 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
}
