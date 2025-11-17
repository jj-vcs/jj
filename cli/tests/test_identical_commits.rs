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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

fn create_test_environment() -> TestEnvironment {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("JJ_RANDOMNESS_SEED", "0");
    test_env.add_env_var("JJ_TIMESTAMP", "2001-01-01T00:00:00+00:00");
    test_env.add_config("experimental.record-predecessors-in-commit = false");
    test_env
}

#[test]
fn test_identical_commits() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test"]).success();
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["new", "root()", "-m=test"]), @r"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit e94ed463cbb0776612e308eba2ecaae74a7f8a73 already exists
    [EOF]
    [exit status: 255]
    ");
    // There should be a single "test" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e94ed463cbb0 test
    ◆  000000000000
    [EOF]
    ");
}

/// Create "test1" commit, then rewrite it in the same way "concurrently" (by
/// starting at the same operation)
#[test]
fn test_identical_commits_concurrently() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test1"]).success();
    work_dir.run_jj(["describe", "-m=test2"]).success();
    work_dir
        .run_jj(["describe", "-m=test2", "--at-op=@-"])
        .success();
    // There should be a single "test2" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c5abd2256ac0 test2
    ◆  000000000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

/// Create commit "test1", then rewrite it to "test2", then rewrite it back to
/// "test1"
#[test]
fn test_identical_commits_by_cycling_rewrite() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test1"]).success();
    work_dir.run_jj(["describe", "-m=test2"]).success();
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test1"]), @r"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit 053222c21fa06b9492e22346f8f70e732231ad4f already exists
    [EOF]
    [exit status: 255]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @r"
    @  oxmtprsl test.user@example.com 2001-01-01 11:00:00 c5abd225
    │  (empty) test2
    │  -- operation f184243937e9 describe commit 053222c21fa06b9492e22346f8f70e732231ad4f
    ○  oxmtprsl hidden test.user@example.com 2001-01-01 11:00:00 053222c2
       (empty) test1
       -- operation 509c18587028 new empty commit
    [EOF]
    ");
    // TODO: Test `jj op diff --from @--`
}

/// Create commits "test1" and "test2" and rewrite "test1". Then rewrite "test2"
/// to become identical to the rewritten "test1".
#[test]
fn test_identical_commits_by_convergent_rewrite() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test1"]).success();
    work_dir.run_jj(["new", "root()", "-m=test2"]).success();
    work_dir
        .run_jj(["describe", "-m=test3", "subject(glob:test1)"])
        .success();
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "subject(glob:test2)"]), @r"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit 460733f1f6f9283d5a810b231dd3f846fd3a6f04 already exists
    [EOF]
    [exit status: 255]
    ");
    // TODO: The "test3" commit should have either "test1" or "test2" as predecessor
    // (or both?)
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @r"
    @  oxmtprsl?? test.user@example.com 2001-01-01 11:00:00 c5abd225
       (empty) test2
       -- operation a1561db9359b new empty commit
    [EOF]
    ");
}

/// Create commits "test1" and "test2" and then rewrite both of them to be
/// identical in a single operation
#[test]
fn test_identical_commits_by_convergent_rewrite_one_operation() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test1"]).success();
    work_dir.run_jj(["new", "root()", "-m=test2"]).success();
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "root()+"]), @r"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit 460733f1f6f9283d5a810b231dd3f846fd3a6f04 already exists
    [EOF]
    [exit status: 255]
    ");
    // TODO: There should be a single "test3" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c5abd2256ac0 test2
    │ ○  053222c21fa0 test1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    // TODO: The "test3" commit should have either "test1" or "test2" as predecessor
    // (or both?)
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @r"
    @  oxmtprsl?? test.user@example.com 2001-01-01 11:00:00 c5abd225
       (empty) test2
       -- operation a1561db9359b new empty commit
    [EOF]
    ");
}

/// Create two stacked commits. Then reorder them so they become rewrites of
/// each other.
#[test]
fn test_identical_commits_swap_by_reordering() {
    let test_env = create_test_environment();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-m=test"]).success();
    work_dir.run_jj(["new", "-m=test"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5bae90c9b34d test
    ○  e94ed463cbb0 test
    ◆  000000000000
    [EOF]
    ");
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-r=@", "-B=@-"]), @r"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit e94ed463cbb0776612e308eba2ecaae74a7f8a73 already exists
    [EOF]
    [exit status: 255]
    ");
    // There same two commits should still be visible
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5bae90c9b34d test
    ○  e94ed463cbb0 test
    ◆  000000000000
    [EOF]
    ");
    // TODO: Each commit should be a predecessor of the other
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@"]), @r"
    @  oxmtprsl?? test.user@example.com 2001-01-01 11:00:00 5bae90c9
       (empty) test
       -- operation 380fbe20623e new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@-"]), @r"
    ○  oxmtprsl?? test.user@example.com 2001-01-01 11:00:00 e94ed463
       (empty) test
       -- operation 40e37b931010 new empty commit
    [EOF]
    ");
    // TODO: Test that `jj op show` displays something reasonable
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
