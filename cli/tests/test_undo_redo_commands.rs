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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_undo_root_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: f63ee16f9553 (2001-02-03 08:05:07) add workspace 'default'
    Restored to operation: 000000000000 root()
    Warning: The current workspace 'default' no longer exists after this operation. The working copy was left untouched.
    Hint: Restore to an operation that contains the workspace (e.g. `jj undo` or `jj redo`).
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot undo root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_merge_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m", "first"]).success();
    work_dir
        .run_jj(["new", "--at-op=@-", "-m", "second"])
        .success();

    // A merge operation can't be undone by restoring its parent, even with
    // `--allow-cross-workspace`, so the operations before it are reverted one
    // by one
    let output = work_dir.run_jj(["undo", "--allow-cross-workspace"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Undid operation: 6021eabf0af7 (2001-02-03 08:05:08) new empty commit
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 4ee0ac305aad (2001-02-03 08:05:09) new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  qpvuntsmwlqt
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No operation performed in workspace default is left to undo
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_other_workspace_operation_on_top() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    main_dir.run_jj(["new", "-m", "main 2"]).success();
    second_dir.run_jj(["describe", "-m", "second 1"]).success();

    // The latest operation of this workspace is reverted, keeping the newer
    // operation of the other workspace
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: b923a10a4e43 (2001-02-03 08:05:10) new empty commit
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: kkmpptxz 0db6b64b (empty) main 1
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Repeated undo continues with older operations of this workspace
    second_dir.run_jj(["new", "-m", "second 2"]).success();
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Skipped 2 operations from other workspaces.
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt default@
    │ ○  yqosqzytrlsw second@ "second 2"
    │ ○  uuqppmxqsxsn "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
    insta::assert_snapshot!(get_op_log_output(&main_dir), @"
    @  23fcf1714106 default@ undo: revert operation e850543be3877c274c3665dae045c43d1421e90b31d43062f16d75c616306f1c8887f9f000a7d61858a238da466a116e0933798a21be71ffa1824a57f3865ba4
    ○  e6ea6863ec1d second@ new empty commit
    ○  17d22b98c187 default@ undo: revert operation b923a10a4e434e63d7a30822ec8f63859e958bc18afb414f7348bc1f482598b47d81c750718cc1da8191ccf065b10509d3fd2528787cccdf33cf2993755c02c7
    ○  c08822825624 second@ describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    ○  b923a10a4e43 default@ new empty commit
    ○  e850543be387 default@ new empty commit
    ○  9b9b7064bf7b second@ create initial working-copy commit in workspace second
    ○  9f80ad1d8fd1 add workspace 'second'
    ○  f63ee16f9553 add workspace 'default'
    ○  000000000000
    [EOF]
    ");

    // No operation of this workspace is left to undo
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No operation performed in workspace default is left to undo
    [EOF]
    [exit status: 1]
    ");
    second_dir.run_jj(["new", "-m", "second 3"]).success();
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to undo operation 3718ee2b6874 because it was performed in workspace second
    Hint: Use `--allow-cross-workspace` to undo it anyway, or use `jj op revert` to revert a specific operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_rewrite_under_other_workspace_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["describe", "-m", "main 1"]).success();
    second_dir.run_jj(["describe", "-m", "second 1"]).success();

    // Reverting the rewrite puts the old commit back in place of the new one,
    // which is not divergence
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: e607499fd693 (2001-02-03 08:05:09) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt default@
    │ ○  uuqppmxqsxsn second@ "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
}

#[test]
fn test_undo_concurrent_operations_of_two_workspaces() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();
    let base_op_id = main_dir.current_operation_id();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    second_dir
        .run_jj(["describe", "-m", "second 1", "--at-op", &base_op_id])
        .success();

    // `jj undo` merges the concurrent operations, and then reverts the
    // operation of this workspace
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Undid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt default@
    │ ○  uuqppmxqsxsn second@ "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
    insta::assert_snapshot!(get_op_log_output(&main_dir), @"
    @  1d76236ba141 default@ undo: revert operation e850543be3877c274c3665dae045c43d1421e90b31d43062f16d75c616306f1c8887f9f000a7d61858a238da466a116e0933798a21be71ffa1824a57f3865ba4
    ○    6801256df9b9 default@ reconcile divergent operations
    ├─╮
    ○ │  e850543be387 default@ new empty commit
    │ ○  bdd96985e82b second@ describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    ├─╯
    ○  9b9b7064bf7b second@ create initial working-copy commit in workspace second
    ○  9f80ad1d8fd1 add workspace 'second'
    ○  f63ee16f9553 add workspace 'default'
    ○  000000000000
    [EOF]
    ");
}

#[test]
fn test_undo_restores_and_reverts_in_turn() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    second_dir.run_jj(["describe", "-m", "second 1"]).success();
    main_dir.run_jj(["new", "-m", "main 2"]).success();

    // The latest operation is this workspace's, so its parent is restored
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 75fb9d62f2eb (2001-02-03 08:05:11) new empty commit
    Restored to operation: fbe0fa4146ec (2001-02-03 08:05:10) describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    Working copy  (@) now at: kkmpptxz 0db6b64b (empty) main 1
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    // The next one is followed by the other workspace's operation, so it is
    // reverted
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // A new operation of this workspace is undone by restoring its parent, and
    // undoing further continues before the reverted operation
    main_dir.run_jj(["new", "-m", "main 3"]).success();
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 0751628f5e8b (2001-02-03 08:05:14) new empty commit
    Restored to operation: 840f5a567b5f (2001-02-03 08:05:13) undo: revert operation e850543be3877c274c3665dae045c43d1421e90b31d43062f16d75c616306f1c8887f9f000a7d61858a238da466a116e0933798a21be71ffa1824a57f3865ba4
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No operation performed in workspace default is left to undo
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt default@
    │ ○  uuqppmxqsxsn second@ "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
    insta::assert_snapshot!(get_op_log_output(&main_dir), @"
    @  7f705bfb5f5e default@ undo: restore to operation 840f5a567b5f1e98cea7161e51327ff6f3513ac027533edf5fdd611fb7d53ae7ca11e2a373d5f14edfe3028c084d2f5a3a887473468552fabb4f67b6d141d770
    ○  0751628f5e8b default@ new empty commit
    ○  840f5a567b5f default@ undo: revert operation e850543be3877c274c3665dae045c43d1421e90b31d43062f16d75c616306f1c8887f9f000a7d61858a238da466a116e0933798a21be71ffa1824a57f3865ba4
    ○  5844f3f81344 default@ undo: restore to operation fbe0fa4146ecbbcc2222d27ad557ed4a6f62a62e0f7182ffadd7da737b6a7fca6825eaa48f241b1cc152c2d0e9a27c2091697d103dc26a63e166468ffe3baeef
    ○  75fb9d62f2eb default@ new empty commit
    ○  fbe0fa4146ec second@ describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    ○  e850543be387 default@ new empty commit
    ○  9b9b7064bf7b second@ create initial working-copy commit in workspace second
    ○  9f80ad1d8fd1 add workspace 'second'
    ○  f63ee16f9553 add workspace 'default'
    ○  000000000000
    [EOF]
    ");
}

/// Sets up two workspaces, where the "main" workspace moves bookmark "foo" and
/// describes its working-copy commit, and then the "second" workspace moves
/// the bookmark and rewrites the commit again. Returns the ids of the main
/// workspace's operations.
fn set_up_conflicting_workspace_operations(test_env: &TestEnvironment) -> (String, String) {
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["commit", "-m", "one"]).success();
    main_dir
        .run_jj(["bookmark", "create", "foo", "-r", "@-"])
        .success();
    main_dir.run_jj(["commit", "-m", "two"]).success();
    main_dir
        .run_jj(["bookmark", "move", "foo", "--to", "@-"])
        .success();
    let move_op_id = main_dir.current_operation_id();
    main_dir.run_jj(["describe", "-m", "main"]).success();
    let describe_op_id = main_dir.current_operation_id();

    second_dir
        .run_jj(["bookmark", "move", "foo", "--to", "@", "--allow-backwards"])
        .success();
    second_dir
        .run_jj(["describe", "-r", "default@", "-m", "main, edited in second"])
        .success();
    (move_op_id, describe_op_id)
}

#[test]
fn test_undo_conflicts_with_other_workspace_operations() {
    let test_env = TestEnvironment::default();
    set_up_conflicting_workspace_operations(&test_env);
    let main_dir = test_env.work_dir("main");

    // Reverting the description brings back the old commit, which is divergent
    // with the other workspace's rewrite of it
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 94ae4ec38d5b (2001-02-03 08:05:13) describe commit b66c9a58232791585ca39a42e85523b850066a1d
    Skipped 2 operations from other workspaces.
    Warning: Undoing the operation left these changes divergent:
      mzvwutvlkqwt
    Hint: Use `jj log -r 'divergent()'` to see details. Use `jj converge` to resolve.
    [EOF]
    ");
    // Reverting the bookmark move conflicts with the other workspace's move
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 246ced1e7389 (2001-02-03 08:05:12) point bookmark foo to commit 0faaa5a8fa6f69817d210a437edce9e2504121a3
    Skipped 2 operations from other workspaces.
    Warning: Undoing the operation left these bookmarks conflicted:
      foo
    Hint: Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to resolve.
    [EOF]
    ");
    let undo_log = get_log_output(&main_dir);
    insta::assert_snapshot!(undo_log, @r#"
    @  mzvwutvlkqwt default@ (divergent) "main, edited in second"
    │ ○  mzvwutvlkqwt (divergent)
    ├─╯
    ○  kkmpptxzrspx "two"
    ○  qpvuntsmwlqt foo?? "one"
    │ ○  uuqppmxqsxsn foo?? second@
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);

    // The result is the same as reverting the operations with `jj op revert`
    let test_env = TestEnvironment::default();
    let (move_op_id, describe_op_id) = set_up_conflicting_workspace_operations(&test_env);
    let main_dir = test_env.work_dir("main");
    main_dir.run_jj(["op", "revert", &describe_op_id]).success();
    main_dir.run_jj(["op", "revert", &move_op_id]).success();
    assert_eq!(get_log_output(&main_dir), undo_log);
}

#[test]
fn test_undo_operation_already_reverted_by_other_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    main_dir
        .run_jj(["bookmark", "create", "foo", "-r", "@"])
        .success();
    second_dir.run_jj(["bookmark", "delete", "foo"]).success();

    // Reverting the bookmark creation changes nothing, but the undo is still
    // recorded, so that the next undo continues with the operation before it
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 06eed2a563de (2001-02-03 08:05:10) create bookmark foo pointing to commit 0db6b64bff3a5fdceea98bef16298e9b4d8ed2b2
    Skipped 1 operations from other workspaces.
    [EOF]
    ");
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @"
    @  qpvuntsmwlqt default@
    │ ○  uuqppmxqsxsn second@
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_undo_redo_in_single_workspace_restores() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["redo"]).success();
    insta::assert_snapshot!(get_op_log_output(&work_dir), @"
    @  413157fe5ba5 default@ redo: restore to operation 4cf5b484b74752dee31e6f4a304a36e837c39fc1e7d9876ac167ec45b5ee9edee07ecdd47472d13701fe4ce1d53f1ecef5420b1c0de257b3417986ef69fad373
    ○  cb522b7cdcb8 default@ undo: restore to operation f63ee16f95539ec5bda4a7178aa3ace2bf594719aa2cc3410cfc4d070cb31ae31225bef146eb2428baf760180976c936cd60891e7a9087e6836d3a315fe81030
    ○  4cf5b484b747 default@ undo: restore to operation 6021eabf0af70cdd07bffe730c14e6b123e353f3eda72a87782622fb577d18e2aed15941a0b657ddec394ab18db27c3ac2920796c465b145d50da1e5900a5776
    ○  4a3c686e5e0b default@ new empty commit
    ○  6021eabf0af7 default@ new empty commit
    ○  f63ee16f9553 add workspace 'default'
    ○  000000000000
    [EOF]
    ");
}

#[test]
fn test_undo_push_operation() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "origin"])
        .success();
    test_env
        .run_jj_in(".", ["git", "clone", "origin", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("foo", "foo");
    work_dir.run_jj(["commit", "-mfoo"]).success();
    work_dir.run_jj(["git", "push", "-c@-"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Undoing a push operation often leads to conflicted bookmarks.
    Hint: To avoid this, run `jj redo` now.
    Undid operation: 841f09db81a1 (2001-02-03 08:05:10) push bookmark push-rlvkpnrzqnoo to git remote origin
    Restored to operation: 27cf57bcf92a (2001-02-03 08:05:09) commit 3850397cf31988d0657948307ad5bbe873d76a38
    [EOF]
    ");
}

#[test]
fn test_jump_over_old_undo_stack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    for state in 'A'..='D' {
        work_dir.write_file("state", state.to_string());
        work_dir.run_jj(["debug", "snapshot"]).success();
    }
    assert_eq!(work_dir.read_file("state"), "D");

    // undo operations D and C, restoring the state of B
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");

    // create operations E and F
    work_dir.write_file("state", "E");
    work_dir.run_jj(["debug", "snapshot"]).success();
    work_dir.write_file("state", "F");
    work_dir.run_jj(["debug", "snapshot"]).success();
    assert_eq!(work_dir.read_file("state"), "F");

    // undo operations F, E and B, restoring the state of A while skipping the
    // undo-stack of C and D in the op log
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "E");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "A");
}

#[test]
fn test_undo_ignores_op_revert() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    work_dir.write_file("state", "A");
    work_dir.run_jj(["debug", "snapshot"]).success();
    work_dir.write_file("state", "B");
    work_dir.run_jj(["debug", "snapshot"]).success();
    assert_eq!(work_dir.read_file("state"), "B");

    // `op revert` works the same way as `undo` initially, but running `undo`
    // afterwards will result in a no-op. `undo` does not recognize operations
    // created by `op revert` as undo-operations on which an undo-stack can
    // be grown.
    work_dir.run_jj(["op", "revert"]).success();
    assert_eq!(work_dir.read_file("state"), "A");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
}

#[test]
fn test_undo_redo_cross_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    second_dir.run_jj(["describe", "-m", "second"]).success();

    // Undoing an operation from another workspace is refused by default
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to undo operation 33cbee28d986 because it was performed in workspace second
    Hint: Use `--allow-cross-workspace` to undo it anyway, or use `jj op revert` to revert a specific operation
    [EOF]
    [exit status: 1]
    ");

    // It can be allowed explicitly
    let output = main_dir.run_jj(["undo", "--allow-cross-workspace"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 33cbee28d986 (2001-02-03 08:05:09) describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    Restored to operation: 9b9b7064bf7b (2001-02-03 08:05:08) create initial working-copy commit in workspace second
    [EOF]
    ");

    // Redoing an undo-operation from another workspace is refused by default
    let output = second_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to redo operation 2879537879e9 because it was performed in workspace default
    Hint: Use `--allow-cross-workspace` to redo it anyway, or use `jj op revert` to revert a specific undo operation
    [EOF]
    [exit status: 1]
    ");

    // It can be allowed explicitly
    let output = second_dir.run_jj(["redo", "--allow-cross-workspace"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Restored to operation: 33cbee28d986 (2001-02-03 08:05:09) describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    Working copy  (@) now at: uuqppmxq a7e048f9 (empty) second
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_redo_non_undo_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m", "a"]).success();
    let output = work_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_jump_over_old_redo_stack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create a few normal operations
    for state in 'A'..='D' {
        work_dir.write_file("state", state.to_string());
        work_dir.run_jj(["debug", "snapshot"]).success();
    }
    assert_eq!(work_dir.read_file("state"), "D");

    insta::assert_snapshot!(work_dir.run_jj(["undo", "--quiet"]), @"");
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "A");

    // create two adjacent redo-stacks
    insta::assert_snapshot!(work_dir.run_jj(["redo", "--quiet"]), @"");
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");
    work_dir.run_jj(["undo"]).success();
    assert_eq!(work_dir.read_file("state"), "B");
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "C");

    // jump over two adjacent redo-stacks
    work_dir.run_jj(["redo"]).success();
    assert_eq!(work_dir.read_file("state"), "D");

    // nothing left to redo
    insta::assert_snapshot!(work_dir.run_jj(["redo"]), @"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_redo_other_workspace_operation_on_top() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    main_dir.run_jj(["new", "-m", "main 2"]).success();
    second_dir.run_jj(["describe", "-m", "second 1"]).success();
    main_dir.run_jj(["undo"]).success();
    main_dir.run_jj(["undo"]).success();
    second_dir.run_jj(["new", "-m", "second 2"]).success();

    // The latest undo-operation of this workspace is redone by reverting it,
    // keeping the newer operation of the other workspace
    let output = main_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Redid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: kkmpptxz 0db6b64b (empty) main 1
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Repeated redo continues with older undo-operations of this workspace
    second_dir.run_jj(["describe", "-m", "second 3"]).success();
    let output = main_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Redid operation: b923a10a4e43 (2001-02-03 08:05:10) new empty commit
    Skipped 2 operations from other workspaces.
    Working copy  (@) now at: zsuskuln a205be49 (empty) main 2
    Parent commit (@-)      : kkmpptxz 0db6b64b (empty) main 1
    [EOF]
    ");
    let output = main_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  zsuskulnrvyr default@ "main 2"
    ○  kkmpptxzrspx "main 1"
    ○  qpvuntsmwlqt
    │ ○  vruxwmqvtpmx second@ "second 3"
    │ ○  uuqppmxqsxsn "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
    insta::assert_snapshot!(get_op_log_output(&main_dir), @"
    @  d216b8887b94 default@ redo: revert operation 17d22b98c1877619eb0d7f5df2611d12cc914d0d9df9647f79d5be4ee3e1ee68a4dd27e52eaf33dd12837e5daf56f1c7ed9ec0817065673ab7a4d84656a9c4f9
    ○  55b5d4a244b8 second@ describe commit 45c078ebb600390c5fe31256fdb968898f496420
    ○  d689545403d5 default@ redo: revert operation b836a4ed51841fb4516b31767ae07b5d1e5fd3c6ab93930bdd8520f7d0fbbfa84d99ebb86066c8405d8bb4719db50b87194962b2c33e31b6677371be7dadaa23
    ○  5071a37eb75a second@ new empty commit
    ○  b836a4ed5184 default@ undo: revert operation e850543be3877c274c3665dae045c43d1421e90b31d43062f16d75c616306f1c8887f9f000a7d61858a238da466a116e0933798a21be71ffa1824a57f3865ba4
    ○  17d22b98c187 default@ undo: revert operation b923a10a4e434e63d7a30822ec8f63859e958bc18afb414f7348bc1f482598b47d81c750718cc1da8191ccf065b10509d3fd2528787cccdf33cf2993755c02c7
    ○  c08822825624 second@ describe commit 94f41578a9e101e2c85877f3b4eaa9f5e915783f
    ○  b923a10a4e43 default@ new empty commit
    ○  e850543be387 default@ new empty commit
    ○  9b9b7064bf7b second@ create initial working-copy commit in workspace second
    ○  9f80ad1d8fd1 add workspace 'second'
    ○  f63ee16f9553 add workspace 'default'
    ○  000000000000
    [EOF]
    ");

    // Undoing reverts the redo-operations again
    second_dir.run_jj(["new", "-m", "second 4"]).success();
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: d216b8887b94 (2001-02-03 08:05:17) redo: revert operation 17d22b98c1877619eb0d7f5df2611d12cc914d0d9df9647f79d5be4ee3e1ee68a4dd27e52eaf33dd12837e5daf56f1c7ed9ec0817065673ab7a4d84656a9c4f9
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: kkmpptxz 0db6b64b (empty) main 1
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: d689545403d5 (2001-02-03 08:05:15) redo: revert operation b836a4ed51841fb4516b31767ae07b5d1e5fd3c6ab93930bdd8520f7d0fbbfa84d99ebb86066c8405d8bb4719db50b87194962b2c33e31b6677371be7dadaa23
    Skipped 2 operations from other workspaces.
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt default@
    │ ○  kxryzmorwvtz second@ "second 4"
    │ ○  vruxwmqvtpmx "second 3"
    │ ○  uuqppmxqsxsn "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
}

#[test]
fn test_redo_restoring_undo_with_other_workspace_operation_on_top() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let second_dir = test_env.work_dir("second");
    main_dir.run_jj(["workspace", "add", "../second"]).success();

    main_dir.run_jj(["new", "-m", "main 1"]).success();
    // The latest operation is this workspace's, so its parent is restored
    let output = main_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Restored to operation: 9b9b7064bf7b (2001-02-03 08:05:08) create initial working-copy commit in workspace second
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    second_dir.run_jj(["describe", "-m", "second 1"]).success();

    // The undo-operation is redone by reverting it
    let output = main_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Redid operation: e850543be387 (2001-02-03 08:05:09) new empty commit
    Skipped 1 operations from other workspaces.
    Working copy  (@) now at: kkmpptxz 0db6b64b (empty) main 1
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  kkmpptxzrspx default@ "main 1"
    ○  qpvuntsmwlqt
    │ ○  uuqppmxqsxsn second@ "second 1"
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    "#);
    let output = main_dir.run_jj(["redo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Nothing to redo
    [EOF]
    [exit status: 1]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
      change_id.short(),
      bookmarks,
      working_copies,
      if(divergent, "(divergent)"),
      surround('"', '"', description.first_line()),
    )
    "#;
    work_dir.run_jj(["log", "-T", template, "-r", "all()"])
}

#[must_use]
fn get_op_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", id.short(), workspace_name, description) ++ "\n""#;
    work_dir.run_jj(["op", "log", "-T", template])
}
