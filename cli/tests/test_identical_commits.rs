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
    insta::assert_snapshot!(work_dir.run_jj(["new", "root()", "-m=test"]), @"
    ------- stderr -------
    Working copy  (@) now at: xxmtprsl dfb66bee (empty) test
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // There should be a single "test" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  dfb66bee7ef3 test
    │ ○  271040159836 test
    ├─╯
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  36c77e6a69f6 test2
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
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test1"]), @"
    ------- stderr -------
    Internal error: Unexpected error from backend
    Caused by: Newly-created commit da532833b539c03f7bc6043cdbbe7e74e17f4031 already exists
    [EOF]
    [exit status: 255]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  yxmtprsl test.user@example.com 2001-01-01 11:00:00 36c77e6a
    │  (empty) test2
    │  -- operation 4b462ee5d71e describe commit da532833b539c03f7bc6043cdbbe7e74e17f4031
    ○  yxmtprsl/1 test.user@example.com 2001-01-01 11:00:00 da532833 (hidden)
       (empty) test1
       -- operation 161e6444330b new empty commit
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
        .run_jj(["describe", "-m=test3", "subject(test1)"])
        .success();
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "subject(test2)"]), @"
    ------- stderr -------
    Working copy  (@) now at: xxmtprsl 50c2fa20 (empty) test3
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // TODO: The "test3" commit should have either "test1" or "test2" as predecessor
    // (or both?)
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  xxmtprsl test.user@example.com 2001-01-01 11:00:00 50c2fa20
    │  (empty) test3
    │  -- operation d9f8db473dd0 describe commit 51ca84bd62397818d82ae3c9906094e5800c78bd
    ○  xxmtprsl/1 test.user@example.com 2001-01-01 11:00:00 51ca84bd (hidden)
       (empty) test2
       -- operation 1937456f2199 new empty commit
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
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "root()+"]), @"
    ------- stderr -------
    Updated 2 commits
    Working copy  (@) now at: xxmtprsl 50c2fa20 (empty) test3
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // TODO: There should be a single "test3" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  50c2fa209245 test3
    │ ○  68dcc29a6927 test3
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    // TODO: The "test3" commit should have either "test1" or "test2" as predecessor
    // (or both?)
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  xxmtprsl test.user@example.com 2001-01-01 11:00:00 50c2fa20
    │  (empty) test3
    │  -- operation be5088ecda76 describe commit 51ca84bd62397818d82ae3c9906094e5800c78bd and 1 more
    ○  xxmtprsl/1 test.user@example.com 2001-01-01 11:00:00 51ca84bd (hidden)
       (empty) test2
       -- operation 1937456f2199 new empty commit
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  10877c9de412 test
    ○  271040159836 test
    ◆  000000000000
    [EOF]
    ");
    // TODO: Should not fail
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-r=@", "-B=@-"]), @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: xxmtprsl dfb66bee (empty) test
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // There same two commits should still be visible
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  852ad760b5c4 test
    @  dfb66bee7ef3 test
    ◆  000000000000
    [EOF]
    ");
    // TODO: Each commit should be a predecessor of the other
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@"]), @"
    @  xxmtprsl test.user@example.com 2001-01-01 11:00:00 dfb66bee
    │  (empty) test
    │  -- operation aa3cbe1cfd92 rebase commit 10877c9de412e62ba46641a702034170210162a0
    ○  xxmtprsl/1 test.user@example.com 2001-01-01 11:00:00 10877c9d (hidden)
       (empty) test
       -- operation 5b46434cebfd new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@-"]), @"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    // TODO: Test that `jj op show` displays something reasonable
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
