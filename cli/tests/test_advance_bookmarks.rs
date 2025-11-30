// Copyright 2024 The Jujutsu Authors
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

use test_case::test_case;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[must_use]
fn get_log_output_with_bookmarks(work_dir: &TestWorkDir) -> CommandOutput {
    // Don't include commit IDs since they will be different depending on
    // whether the test runs with `jj commit` or `jj describe` + `jj new`.
    let template = r#""bookmarks{" ++ local_bookmarks ++ "} desc: " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}

fn set_advance_bookmarks(test_env: &TestEnvironment, enabled: bool) {
    if enabled {
        test_env.add_config(
            r#"[experimental-advance-branches]
        enabled-branches = ["glob:*"]
        "#,
        );
    } else {
        test_env.add_config(
            r#"[experimental-advance-branches]
        enabled-branches = []
        "#,
        );
    }
}

// Runs a command in the specified test workspace that describes the current
// commit with `commit_message` and creates a new commit on top of it.
type CommitFn = fn(work_dir: &TestWorkDir, commit_message: &str);

// Implements CommitFn using the `jj commit` command.
fn commit_cmd(work_dir: &TestWorkDir, commit_message: &str) {
    work_dir.run_jj(["commit", "-m", commit_message]).success();
}

// Implements CommitFn using the `jj describe` and `jj new`.
fn describe_new_cmd(work_dir: &TestWorkDir, commit_message: &str) {
    work_dir
        .run_jj(["describe", "-m", commit_message])
        .success();
    work_dir.run_jj(["new"]).success();
}

// Check that enabling and disabling advance-bookmarks works as expected.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarks_enabled(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // First, test with advance-bookmarks enabled. Start by creating a bookmark on
    // the root commit.
    set_advance_bookmarks(&test_env, true);
    work_dir
        .run_jj(["bookmark", "create", "-r", "@-", "test_bookmark"])
        .success();

    // Check the initial state of the repo.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{test_bookmark} desc:
    [EOF]
    ");
    }

    // Run jj commit, which will advance the bookmark pointing to @-.
    make_commit(&work_dir, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }

    // Now disable advance bookmarks and commit again. The bookmark shouldn't move.
    set_advance_bookmarks(&test_env, false);
    make_commit(&work_dir, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: second
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
}

// Check that only a bookmark pointing to @- advances. Branches pointing to @
// are not advanced.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarks_at_minus(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);
    work_dir
        .run_jj(["bookmark", "create", "test_bookmark", "-r", "@"])
        .success();

    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{test_bookmark} desc:
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }

    make_commit(&work_dir, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }

    // Create a second bookmark pointing to @. On the next commit, only the first
    // bookmark, which points to @-, will advance.
    work_dir
        .run_jj(["bookmark", "create", "test_bookmark2", "-r", "@"])
        .success();
    make_commit(&work_dir, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{test_bookmark test_bookmark2} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
}

// Test that per-bookmark overrides invert the behavior of
// experimental-advance-branches.enabled.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarks_overrides(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // advance-branches is disabled by default.
    work_dir
        .run_jj(["bookmark", "create", "-r", "@-", "test_bookmark"])
        .success();

    // Check the initial state of the repo.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{test_bookmark} desc:
    [EOF]
    ");
    }

    // Commit will not advance the bookmark since advance-branches is disabled.
    make_commit(&work_dir, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: first
    ◆  bookmarks{test_bookmark} desc:
    [EOF]
    ");
    }

    // Now enable advance branches for "test_bookmark", move the bookmark, and
    // commit again.
    test_env.add_config(
        r#"[experimental-advance-branches]
    enabled-branches = ["test_bookmark"]
    "#,
    );
    work_dir
        .run_jj(["bookmark", "set", "test_bookmark", "-r", "@-"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
    make_commit(&work_dir, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{test_bookmark} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }

    // Now disable advance branches for "test_bookmark" and enable for
    // "second_bookmark", which we will use later. Disabling always takes
    // precedence over enabling.
    test_env.add_config(
        r#"[experimental-advance-branches]
    enabled-branches = ["test_bookmark", "second_bookmark"]
    disabled-branches = ["test_bookmark"]
    "#,
    );
    make_commit(&work_dir, "third");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: third
    ○  bookmarks{test_bookmark} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }

    // If we create a new bookmark at @- and move test_bookmark there as well. When
    // we commit, only "second_bookmark" will advance since "test_bookmark" is
    // disabled.
    work_dir
        .run_jj(["bookmark", "create", "second_bookmark", "-r", "@-"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "test_bookmark", "-r", "@-"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{second_bookmark test_bookmark} desc: third
    ○  bookmarks{} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
    make_commit(&work_dir, "fourth");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{second_bookmark} desc: fourth
    ○  bookmarks{test_bookmark} desc: third
    ○  bookmarks{} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
}

// If multiple eligible bookmarks point to @-, all of them will be advanced.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarks_multiple_bookmarks(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);
    work_dir
        .run_jj(["bookmark", "create", "-r", "@-", "first_bookmark"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r", "@-", "second_bookmark"])
        .success();

    insta::allow_duplicates! {
    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{first_bookmark second_bookmark} desc:
    [EOF]
    ");
    }

    // Both bookmarks are eligible and both will advance.
    make_commit(&work_dir, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{first_bookmark second_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
    }
}

// Call `jj new` on an interior commit and see that the bookmark pointing to its
// parent's parent is advanced.
#[test]
fn test_new_advance_bookmarks_interior() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{} desc:
    [EOF]
    ");

    // Create a gap in the commits for us to insert our new commit with --before.
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r", "@---", "test_bookmark"])
        .success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: third
    ○  bookmarks{} desc: second
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");

    work_dir.run_jj(["new", "-r", "@--"]).success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    │ ○  bookmarks{} desc: third
    ├─╯
    ○  bookmarks{test_bookmark} desc: second
    ○  bookmarks{} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
}

// If the `--before` flag is passed to `jj new`, bookmarks are not advanced.
#[test]
fn test_new_advance_bookmarks_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{} desc:
    [EOF]
    ");

    // Create a gap in the commits for us to insert our new commit with --before.
    work_dir.run_jj(["commit", "-m", "first"]).success();
    work_dir.run_jj(["commit", "-m", "second"]).success();
    work_dir.run_jj(["commit", "-m", "third"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r", "@---", "test_bookmark"])
        .success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: third
    ○  bookmarks{} desc: second
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");

    work_dir.run_jj(["new", "--before", "@-"]).success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    ○  bookmarks{} desc: third
    @  bookmarks{} desc:
    ○  bookmarks{} desc: second
    ○  bookmarks{test_bookmark} desc: first
    ◆  bookmarks{} desc:
    [EOF]
    ");
}

// If the `--after` flag is passed to `jj new`, bookmarks are not advanced.
#[test]
fn test_new_advance_bookmarks_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);
    work_dir
        .run_jj(["bookmark", "create", "-r", "@-", "test_bookmark"])
        .success();

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ◆  bookmarks{test_bookmark} desc:
    [EOF]
    ");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "--after", "@"]).success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc:
    ○  bookmarks{} desc: first
    ◆  bookmarks{test_bookmark} desc:
    [EOF]
    ");
}

#[test]
fn test_new_advance_bookmarks_merge_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    set_advance_bookmarks(&test_env, true);
    work_dir.run_jj(["desc", "-m", "0"]).success();
    work_dir.run_jj(["new", "-m", "1"]).success();
    work_dir
        .run_jj(["new", "subject(glob:0)", "-m", "2"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "test_bookmark", "-rsubject(glob:0)"])
        .success();

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @  bookmarks{} desc: 2
    │ ○  bookmarks{} desc: 1
    ├─╯
    ○  bookmarks{test_bookmark} desc: 0
    ◆  bookmarks{} desc:
    [EOF]
    ");

    // The bookmark won't advance because `jj  new` had multiple targets.
    work_dir
        .run_jj(["new", "subject(glob:1)", "subject(glob:2)"])
        .success();
    insta::assert_snapshot!(get_log_output_with_bookmarks(&work_dir), @r"
    @    bookmarks{} desc:
    ├─╮
    │ ○  bookmarks{} desc: 2
    ○ │  bookmarks{} desc: 1
    ├─╯
    ○  bookmarks{test_bookmark} desc: 0
    ◆  bookmarks{} desc:
    [EOF]
    ");
}
