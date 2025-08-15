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

use crate::common::TestEnvironment;

#[test]
fn test_undo_root_operation() {
    // TODO: Adapt to future "undo" functionality: What happens if the user
    // progressively undoes everything all the way back to the root operation?

    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["undo", "000000000000"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Error: Cannot undo root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_merge_operation() {
    // TODO: What should future "undo" do with a merge operation? The
    // answer is probably not improbably not important, because users
    // who create merge operations will also be able to `op revert`
    // and `op restore` to achieve their goals on their own.
    // Possibilities:
    // - Fail and block any further attempt to undo anything.
    // - Restore directly to the fork point of the merge, ignoring any intermediate
    //   operations.
    // - Pick any path and walk only that backwards, ignoring the other paths.
    //   (Which path to pick?)
    // At first, it will be best to simply fail, before there is
    // agreement that doing anything else is not actively harmful.

    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new", "--at-op=@-"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Error: Cannot undo a merge operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_latest_undo_implicitly() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Double-undo creation of child
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");

    // Double-undo creation of sibling
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: e83fef84da56 (2001-02-03 08:05:12) revert operation c9a93954e94b008f7295813623c4c12aeffd2cb81728e8d7813a37c8b6252b8f9a15ddfc6e496b393de355bd07405e740a05178cf8845af0087fb02223e4c404
    Working copy  (@) now at: mzvwutvl 8afc18ff (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_latest_undo_explicitly() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", "-T=id.short()", "-n=1"])
        .success();
    let op_id_hex = output.stdout.raw();
    let output = work_dir.run_jj(["undo", op_id_hex]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Reverted operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: f344a2234512 (2001-02-03 08:05:13) revert operation a6365efd7a8958525e22cd7b4fb01f308260464facdc3f03c82a151892a073e0fbf6b4d9ad49991ab7ee4f05c55c147c74841714710c87ce7a990e112fe782b8
    Working copy  (@) now at: royxmykx ba0e5dca (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 43444d88 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_an_older_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", "-T=id.short()", "-n=1"])
        .success();
    let op_id_hex = output.stdout.raw();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", op_id_hex]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Reverted operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: `jj undo <operation>` is deprecated; use `jj op revert <operation>` instead
    Reverted operation: f2c360817b17 (2001-02-03 08:05:14) revert operation 20e354c3f097e96da28c0470b2d9e38c07370ebbae6c01b33c62a44bee913603086b66c97cb8a24a6b1df284b64a82edea14b7fa3cb124da55ebc4d743a92475
    [EOF]
    ");
}

#[test]
fn test_undo_an_undo_multiple_times() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: 91cc66ce7fb2 (2001-02-03 08:05:10) revert operation 5b31042c020bd6090d52b932c998a263655cd541b7922c2f56e372a0ee367aa4f7dfb0ccb89c55d2fa232ba8ff6fb22276607ccd12f03844841e6a3888f5972d
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: e26664a8b05f (2001-02-03 08:05:13) revert operation 3ac3dec981c7b97070849ee22a468ec4d16b2dbb1f3df26092ea2c11289b61bc3a38a1b5e3f5a91f576feb8769e171c1905f70f39e0284648c68c7a21f439817
    Working copy  (@) now at: royxmykx e7d0d5fd (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: cd5fdc0df497 (2001-02-03 08:05:14) revert operation e26664a8b05f2380b6857ea564f389363bc150ddbdc6cf087908787fc3831b3aa95575baed369581f8709c5cfc63f4787ff7b601e51639e8badc8251b8b2b9f9
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_bookmark_deletion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "foo", "-r=@"])
        .success();
    work_dir.run_jj(["bookmark", "delete", "foo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Reverted operation: e1bcf7cd8080 (2001-02-03 08:05:09) delete bookmark foo
    [EOF]
    ");
}
