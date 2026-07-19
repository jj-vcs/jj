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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_tag_set_delete() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-mcommit1"]).success();
    let output = work_dir.run_jj(["tag", "set", "-r@-", "foo", "bar"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created 2 tags pointing to qpvuntsm b876c5f4 (empty) commit1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  bbc749308d7f
    ◆  b876c5f49546 bar foo
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "set", "foo", "baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to move tag: foo
    Hint: Use --allow-move to update existing tags.
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  bbc749308d7f
    ◆  b876c5f49546 bar foo
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "set", "--allow-move", "foo", "baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 tags pointing to rlvkpnrz bbc74930 (empty) (no description set)
    Moved 1 tags to rlvkpnrz bbc74930 (empty) (no description set)
    Warning: The working-copy commit became immutable; a new commit has been created on top of it.
    Working copy  (@) now at: yqosqzyt 13cbd515 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz bbc74930 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  13cbd51558a6
    ◆  bbc749308d7f baz foo
    ◆  b876c5f49546 bar
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "delete", "foo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Deleted 1 tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  13cbd51558a6
    ◆  bbc749308d7f baz
    ◆  b876c5f49546 bar
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "set", "--allow-move", "-r@", "baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 tags to yqosqzyt 13cbd515 (empty) (no description set)
    Warning: The working-copy commit became immutable; a new commit has been created on top of it.
    Working copy  (@) now at: kpqxywon cca3d7af (empty) (no description set)
    Parent commit (@-)      : yqosqzyt 13cbd515 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  cca3d7af9d98
    ◆  13cbd51558a6 baz
    ◆  bbc749308d7f
    ◆  b876c5f49546 bar
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "delete", "b*"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Deleted 2 tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  cca3d7af9d98
    ○  13cbd51558a6
    ○  bbc749308d7f
    ○  b876c5f49546
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_tag_at_root() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["tag", "set", "-rroot()", "foo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 tags pointing to zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    Warning: Failed to export some tags:
      foo@git: Ref cannot point to the root commit in Git
    [EOF]
    ");
}

#[test]
fn test_tag_bad_name() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-mcommit1"]).success();

    let output = work_dir.run_jj(["tag", "set", ""]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: invalid value '' for '<NAMES>...': Failed to parse tag name: Syntax error

    For more information, try '--help'.
    Caused by:  --> 1:1
      |
    1 | 
      | ^---
      |
      = expected <identifier>, <string_literal>, or <raw_string_literal>
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let output = work_dir.run_jj(["tag", "set", "''"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: invalid value '''' for '<NAMES>...': Failed to parse tag name: Expected non-empty string

    For more information, try '--help'.
    Caused by:  --> 1:1
      |
    1 | ''
      | ^^
      |
      = Expected non-empty string
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let output = work_dir.run_jj(["tag", "set", "foo@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: invalid value 'foo@' for '<NAMES>...': Failed to parse tag name: Syntax error

    For more information, try '--help'.
    Caused by:  --> 1:4
      |
    1 | foo@
      |    ^---
      |
      = expected <EOI>
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    // quoted name works
    let output = work_dir.run_jj(["tag", "set", "-r@-", "'foo@'"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created 1 tags pointing to qpvuntsm b876c5f4 (empty) commit1
    [EOF]
    ");
}

#[test]
fn test_tag_unknown() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["tag", "delete", "unknown"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching tags for names: unknown
    No tags to delete.
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "delete", "unknown*"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No tags to delete.
    [EOF]
    ");
}

#[test]
fn test_tag_track_untrack() {
    let test_env = TestEnvironment::default();

    // Set up remote
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "origin"])
        .success();
    let origin_dir = test_env.work_dir("origin");
    origin_dir.run_jj(["commit", "-mcommit 1"]).success();
    origin_dir
        .run_jj(["tag", "set", "-r@-", "tag1", "tag2", "tag3", "tag4"])
        .success();

    // Remote tags are tracked by default
    let output = test_env.run_jj_in(".", ["git", "clone", "origin", "local"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/local"
    tag: tag1@origin [new] tracked
    tag: tag2@origin [new] tracked
    tag: tag3@origin [new] tracked
    tag: tag4@origin [new] tracked
    [EOF]
    "#);
    let local_dir = test_env.work_dir("local");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag3: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag4: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    [EOF]
    ");

    // Untrack existing and locally deleted tags: targets shouldn't be changed
    local_dir.run_jj(["tag", "delete", "tag3"]).success();
    let output = local_dir.run_jj(["tag", "untrack", "'tag[234]'"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Stopped tracking 3 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag3@origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag4: qpvuntsm 4de4efb4 (empty) commit 1
    tag4@origin: qpvuntsm 4de4efb4 (empty) commit 1
    [EOF]
    ");

    // Create and move tags
    local_dir.run_jj(["tag", "set", "tag5"]).success();
    local_dir.run_jj(["new", "root()"]).success();
    local_dir.run_jj(["commit", "-mcommit 2"]).success();
    local_dir
        .run_jj(["tag", "set", "--allow-move", "tag4"])
        .success();

    // Track tracked, untracked, conflicting, new, and unknown tags
    let output = local_dir.run_jj(["tag", "track", "'tag[1345]'", "unknown"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching tags for names: unknown
    Warning: Remote tag already tracked: tag1@origin
    Started tracking 3 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag3: qpvuntsm 4de4efb4 (empty) commit 1
      @origin: qpvuntsm 4de4efb4 (empty) commit 1
    tag4 (conflicted):
      + kpqxywon 3fea8afe (empty) (no description set)
      + qpvuntsm 4de4efb4 (empty) commit 1
      @origin (behind by 2 commits): qpvuntsm 4de4efb4 (empty) commit 1
    tag5: zsuskuln c2934cfb (empty) (no description set)
      @origin (not created yet)
    [EOF]
    ");

    // Fetch new commit: only tracking tags should be merged
    origin_dir.run_jj(["commit", "-mcommit 3"]).success();
    origin_dir
        .run_jj([
            "tag",
            "set",
            "-r@-",
            "--allow-move",
            "tag1",
            "tag2",
            "tag3",
            "tag4",
        ])
        .success();
    let output = local_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    tag: tag1@origin [updated] tracked
    tag: tag2@origin [updated] untracked
    tag: tag3@origin [updated] tracked
    tag: tag4@origin [updated] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: rlvkpnrz bdaad04f (empty) commit 3
      @origin: rlvkpnrz bdaad04f (empty) commit 3
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@origin: rlvkpnrz bdaad04f (empty) commit 3
    tag3: rlvkpnrz bdaad04f (empty) commit 3
      @origin: rlvkpnrz bdaad04f (empty) commit 3
    tag4 (conflicted):
      + kpqxywon 3fea8afe (empty) (no description set)
      + rlvkpnrz bdaad04f (empty) commit 3
      @origin (behind by 2 commits): rlvkpnrz bdaad04f (empty) commit 3
    tag5: zsuskuln c2934cfb (empty) (no description set)
      @origin (not created yet)
    [EOF]
    ");
}

#[test]
fn test_tag_track_untrack_multiple_remotes() {
    let test_env = TestEnvironment::default();

    // Set up remotes
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "remote1"])
        .success();
    let remote1_dir = test_env.work_dir("remote1");
    remote1_dir.run_jj(["commit", "-mcommit 1"]).success();
    remote1_dir
        .run_jj(["tag", "set", "-r@-", "tag1", "tag2", "tag3"])
        .success();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "remote2"])
        .success();
    let remote2_dir = test_env.work_dir("remote2");
    remote2_dir.run_jj(["commit", "-mcommit 2"]).success();
    remote2_dir
        .run_jj(["tag", "set", "-r@-", "tag2", "tag3", "tag4"])
        .success();

    // Set up colocated repo where pseudo @git remote exists
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "local"])
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["git", "remote", "add", "remote1", "../remote1"])
        .success();
    local_dir
        .run_jj(["git", "remote", "add", "remote2", "../remote2"])
        .success();

    // Remote tags are tracked by default
    let output = local_dir.run_jj(["git", "fetch", "--remote=*"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    tag: tag1@remote1 [new] tracked
    tag: tag2@remote1 [new] tracked
    tag: tag2@remote2 [new] tracked
    tag: tag3@remote1 [new] tracked
    tag: tag3@remote2 [new] tracked
    tag: tag4@remote2 [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2 (conflicted):
      + qpvuntsm 4de4efb4 (empty) commit 1
      + zsuskuln b322488a (empty) commit 2
      @remote1 (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (behind by 1 commits): zsuskuln b322488a (empty) commit 2
    tag3 (conflicted):
      + qpvuntsm 4de4efb4 (empty) commit 1
      + zsuskuln b322488a (empty) commit 2
      @remote1 (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (behind by 1 commits): zsuskuln b322488a (empty) commit 2
    tag4: zsuskuln b322488a (empty) commit 2
      @git: zsuskuln b322488a (empty) commit 2
      @remote2: zsuskuln b322488a (empty) commit 2
    [EOF]
    ");

    // Resolve conflict to reduce test complexity
    local_dir
        .run_jj([
            "tag",
            "set",
            "--allow-move",
            "-rsubject('commit 1')",
            "tag2",
            "tag3",
        ])
        .success();

    // Untrack by name@remote syntax
    let output = local_dir.run_jj(["tag", "untrack", "tag1@git", "tag2@remote1", "tag2@unknown"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching remote tags for names: tag2@unknown
    Warning: Git-tracking tag cannot be untracked: tag1@git
    Stopped tracking 1 remote tags.
    [EOF]
    ");
    // Untrack with --remote
    let output = local_dir.run_jj(["tag", "untrack", "tag2", "tag3", "--remote=remote2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Stopped tracking 2 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@remote2: zsuskuln b322488a (empty) commit 2
    tag3: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag3@remote2: zsuskuln b322488a (empty) commit 2
    tag4: zsuskuln b322488a (empty) commit 2
      @git: zsuskuln b322488a (empty) commit 2
      @remote2: zsuskuln b322488a (empty) commit 2
    [EOF]
    ");

    // Untrack all
    let output = local_dir.run_jj(["tag", "untrack", "*"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote tag not tracked yet: tag2@remote1
    Warning: Remote tag not tracked yet: tag2@remote2
    Warning: Remote tag not tracked yet: tag3@remote2
    Warning: Remote tag not tracked yet: tag4@remote1
    Warning: Remote tag not tracked yet: tag1@remote2
    Stopped tracking 3 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
    tag1@remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@remote2: zsuskuln b322488a (empty) commit 2
    tag3: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
    tag3@remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag3@remote2: zsuskuln b322488a (empty) commit 2
    tag4: zsuskuln b322488a (empty) commit 2
      @git: zsuskuln b322488a (empty) commit 2
    tag4@remote2: zsuskuln b322488a (empty) commit 2
    [EOF]
    ");

    // Noop untrack
    let output = local_dir.run_jj(["tag", "untrack", "tag1@remote1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote tag not tracked yet: tag1@remote1
    Nothing changed.
    [EOF]
    ");

    // Track by name@remote syntax
    let output = local_dir.run_jj(["tag", "track", "tag2@git", "tag3@remote2", "tag3@unknown"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching remote tags for names: tag3@unknown
    Warning: Remote tag already tracked: tag2@git
    Started tracking 1 remote tags.
    [EOF]
    ");
    // Track with --remote
    let output = local_dir.run_jj(["tag", "track", "tag1", "tag2", "--remote=remote1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Started tracking 2 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag2@remote2: zsuskuln b322488a (empty) commit 2
    tag3 (conflicted):
      + qpvuntsm 4de4efb4 (empty) commit 1
      + zsuskuln b322488a (empty) commit 2
      @git (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (behind by 1 commits): zsuskuln b322488a (empty) commit 2
    tag3@remote1: qpvuntsm 4de4efb4 (empty) commit 1
    tag4: zsuskuln b322488a (empty) commit 2
      @git: zsuskuln b322488a (empty) commit 2
    tag4@remote2: zsuskuln b322488a (empty) commit 2
    [EOF]
    ");

    // Track all
    let output = local_dir.run_jj(["tag", "track", "*"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote tag already tracked: tag1@remote1
    Warning: Remote tag already tracked: tag2@remote1
    Warning: Remote tag already tracked: tag3@remote2
    Started tracking 5 remote tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_tag_output(&local_dir), @"
    tag1: qpvuntsm 4de4efb4 (empty) commit 1
      @git: qpvuntsm 4de4efb4 (empty) commit 1
      @remote1: qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (not created yet)
    tag2 (conflicted):
      + qpvuntsm 4de4efb4 (empty) commit 1
      + zsuskuln b322488a (empty) commit 2
      @git (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote1 (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (behind by 1 commits): zsuskuln b322488a (empty) commit 2
    tag3 (conflicted):
      + qpvuntsm 4de4efb4 (empty) commit 1
      + zsuskuln b322488a (empty) commit 2
      @git (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote1 (behind by 1 commits): qpvuntsm 4de4efb4 (empty) commit 1
      @remote2 (behind by 1 commits): zsuskuln b322488a (empty) commit 2
    tag4: zsuskuln b322488a (empty) commit 2
      @git: zsuskuln b322488a (empty) commit 2
      @remote1 (not created yet)
      @remote2: zsuskuln b322488a (empty) commit 2
    [EOF]
    ");

    // Noop track
    let output = local_dir.run_jj(["tag", "track", "tag1@remote1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote tag already tracked: tag1@remote1
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_tag_track_untrack_bad_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["tag", "track", "--remote=foo", "bar@baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --remote cannot be used with <tag>@<remote> symbols
    [EOF]
    [exit status: 2]
    ");

    let output = work_dir.run_jj(["tag", "track", "foo", "bar@baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot specify both <tag> patterns and <tag>@<remote> symbols
    [EOF]
    [exit status: 2]
    ");

    let output = work_dir.run_jj(["tag", "track", "~foo@bar"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Failed to parse name pattern or remote symbol: Invalid string expression
    Caused by:  --> 1:2
      |
    1 | ~foo@bar
      |  ^-----^
      |
      = Invalid string expression
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["tag", "untrack", "--remote=foo", "bar@baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --remote cannot be used with <tag>@<remote> symbols
    [EOF]
    [exit status: 2]
    ");

    let output = work_dir.run_jj(["tag", "untrack", "foo", "bar@baz"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot specify both <tag> patterns and <tag>@<remote> symbols
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_tag_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "root()", "-mcommit1"]).success();
    work_dir.run_jj(["tag", "set", "-r@", "test_tag"]).success();
    work_dir.run_jj(["new", "root()", "-mcommit2"]).success();
    work_dir
        .run_jj(["tag", "set", "-r@", "test_tag2"])
        .success();
    work_dir.run_jj(["new", "root()", "-mcommit3"]).success();
    work_dir
        .run_jj(["tag", "set", "-rtest_tag", "conflicted_tag"])
        .success();
    work_dir
        .run_jj([
            "tag",
            "set",
            "--allow-move",
            "-rtest_tag2",
            "conflicted_tag",
        ])
        .success();
    work_dir
        .run_jj([
            "tag",
            "set",
            "--at-op=@-",
            "--allow-move",
            "-r@",
            "conflicted_tag",
        ])
        .success();

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list"]), @"
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "--color=always"]), @"
    [38;5;5mconflicted_tag[39m [38;5;1m(conflicted)[39m:
      - [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
      + [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
      + [1m[38;5;5mr[0m[38;5;8moyxmykx[39m [1m[38;5;4m1[0m[38;5;8m3c4e819[39m [38;5;2m(empty)[39m commit3
    [38;5;5mtest_tag[39m: [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
    [38;5;5mtest_tag2[39m: [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
    [EOF]
    ");

    // Test pattern matching.
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "test_tag2"]), @"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "'test_tag?'"]), @"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    // Filter by revset
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-rsubject(commit1)"]), @"
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-rsubject(commit2)"]), @"
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");
    // Filter by revset and name, which aren't intersected
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-rsubject(commit1)", "test_tag2"]), @"
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    // Unmatched exact name pattern should be warned. "test_tag2" exists, but
    // isn't included in the match.
    insta::assert_snapshot!(
        work_dir.run_jj(["tag", "list", "test* & ~*2", "unknown ~ test_tag2"]), @"
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    [EOF]
    ------- stderr -------
    Warning: No matching tags for names: unknown
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "--conflicted"]), @"
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    [EOF]
    ");

    let template = r#"
    concat(
      "[" ++ name ++ "]\n",
      separate(" ", "present:", present) ++ "\n",
      separate(" ", "conflict:", conflict) ++ "\n",
      separate(" ", "normal_target:", normal_target.description().first_line()) ++ "\n",
      separate(" ", "removed_targets:", removed_targets.map(|c| c.description().first_line())) ++ "\n",
      separate(" ", "added_targets:", added_targets.map(|c| c.description().first_line())) ++ "\n",
    )
    "#;
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-T", template]), @"
    [conflicted_tag]
    present: true
    conflict: true
    normal_target: <Error: No value set to Option<Commit>>
    removed_targets: commit1
    added_targets: commit2 commit3
    [test_tag]
    present: true
    conflict: false
    normal_target: commit1
    removed_targets:
    added_targets: commit1
    [test_tag2]
    present: true
    conflict: false
    normal_target: commit2
    removed_targets:
    added_targets: commit2
    [EOF]
    ");

    // Sort by command argument
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "--sort=committer-date-,name"]), @"
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    [EOF]
    ");

    // Default sort keys in config
    let config = "--config=ui.tag-list-sort-keys=['committer-date', 'name-']";
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", config]), @"
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    [EOF]
    ");
}

#[test]
fn test_tag_list_remotes() {
    let test_env = TestEnvironment::default();

    // TODO: set up remote tags in a similar way to test_bookmark_list_tracked()

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "local"])
        .success();
    let local_dir = test_env.work_dir("local");

    local_dir
        .run_jj(["new", "root()", "-m", "local-only"])
        .success();
    local_dir.run_jj(["tag", "set", "local-only"]).success();

    let output = local_dir.run_jj(["tag", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @"
    local-only: rlvkpnrz d3e8d245 (empty) local-only
      @git: rlvkpnrz d3e8d245 (empty) local-only
    [EOF]
    ");

    // Since there's no way to track/untrack tags manually, --tracked is useless
    // right now.
    let output = local_dir.run_jj(["tag", "list", "--tracked"]);
    insta::assert_snapshot!(output, @"");

    let output = local_dir.run_jj(["tag", "list", "--tracked", "--remote=git"]);
    insta::assert_snapshot!(output, @"
    local-only: rlvkpnrz d3e8d245 (empty) local-only
      @git: rlvkpnrz d3e8d245 (empty) local-only
    [EOF]
    ");

    let output = local_dir.run_jj(["tag", "list", "--remote=origin"]);
    insta::assert_snapshot!(output, @"");

    let output = local_dir.run_jj(["tag", "list", "--remote=git"]);
    insta::assert_snapshot!(output, @"
    local-only: rlvkpnrz d3e8d245 (empty) local-only
      @git: rlvkpnrz d3e8d245 (empty) local-only
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", commit_id.short(), tags) ++ "\n""#;
    work_dir.run_jj(["log", "-rall()", "-T", template])
}

#[must_use]
fn get_tag_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["tag", "list", "--all-remotes"])
}
