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
    insta::assert_snapshot!(work_dir.run_jj(["new", "root()", "-m=test"]), @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    // There should be a single "test" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
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
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test1"]), @"
    ------- stderr -------
    Working copy  (@) now at: oxmtprsl 053222c2 (empty) test1
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  oxmtprsl test.user@example.com 2001-01-01 11:00:00 053222c2
       (empty) test1
       -- operation 72d9ec4ca389 new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "diff", "--from", "@--"]), @"
    From operation: 72d9ec4ca389 (2001-02-03 08:05:08) new empty commit
      To operation: b767cff44a9a (2001-02-03 08:05:10) describe commit c5abd2256ac0decc240b8f7a99f4804029b19c70
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @"
    @  b767cff44a9a test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  describe commit c5abd2256ac0decc240b8f7a99f4804029b19c70
    │  args: jj describe '-m=test1'
    ○  f0e7f7b1629b test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit 053222c21fa06b9492e22346f8f70e732231ad4f
    │  args: jj describe '-m=test2'
    ○  72d9ec4ca389 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new 'root()' '-m=test1'
    ○  08cd29df8293 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
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
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "subject(test2)"]), @"
    ------- stderr -------
    Working copy  (@) now at: oxmtprsl 460733f1 (empty) test3
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // The "test3" commit should have "test1" as predecessor
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  oxmtprsl test.user@example.com 2001-01-01 11:00:00 460733f1
    │  (empty) test3
    │  -- operation 453be26fdc5d describe commit 053222c21fa06b9492e22346f8f70e732231ad4f
    ○  oxmtprsl/2 test.user@example.com 2001-01-01 11:00:00 053222c2 (hidden)
       (empty) test1
       -- operation 72d9ec4ca389 new empty commit
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
    insta::assert_snapshot!(work_dir.run_jj(["describe", "-m=test3", "root()+"]), @"
    ------- stderr -------
    Updated 2 commits
    Working copy  (@) now at: oxmtprsl 460733f1 (empty) test3
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    // There should be a single "test3" commit
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  460733f1f6f9 test3
    ◆  000000000000
    [EOF]
    ");
    // The "test3" commit should have "test1" as predecessor
    insta::assert_snapshot!(work_dir.run_jj(["evolog"]), @"
    @  oxmtprsl test.user@example.com 2001-01-01 11:00:00 460733f1
    │  (empty) test3
    │  -- operation 97a25b6be841 describe commit c5abd2256ac0decc240b8f7a99f4804029b19c70 and 1 more
    ○  oxmtprsl/2 test.user@example.com 2001-01-01 11:00:00 053222c2 (hidden)
       (empty) test1
       -- operation 72d9ec4ca389 new empty commit
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
    @  5bae90c9b34d test
    ○  e94ed463cbb0 test
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-r=@", "-B=@-"]), @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Nothing changed.
    [EOF]
    ");
    // The same two commits should still be visible
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  5bae90c9b34d test
    ○  e94ed463cbb0 test
    ◆  000000000000
    [EOF]
    ");
    // Each commit should be a predecessor of the other
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@"]), @"
    @  oxmtprsl/0 test.user@example.com 2001-01-01 11:00:00 5bae90c9 (divergent)
       (empty) test
       -- operation 51fb079a7e7c new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r=@-"]), @"
    ○  oxmtprsl/1 test.user@example.com 2001-01-01 11:00:00 e94ed463 (divergent)
       (empty) test
       -- operation 03fecb732164 new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    51fb079a7e7c test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    new empty commit
    args: jj new '-m=test'

    Changed commits:
    ○  + oxmtprsl/0 5bae90c9 (divergent) (empty) test

    Changed working copy default@:
    + oxmtprsl/0 5bae90c9 (divergent) (empty) test
    - oxmtprsl/1 e94ed463 (divergent) (empty) test
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
