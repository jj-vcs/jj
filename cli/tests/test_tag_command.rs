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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 2 tags pointing to qpvuntsm b876c5f4 (empty) commit1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bbc749308d7f
    ◆  b876c5f49546 bar foo
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "set", "foo", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move tag: foo
    Hint: Use --allow-move to update existing tags.
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bbc749308d7f
    ◆  b876c5f49546 bar foo
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "set", "--allow-move", "foo", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 tags pointing to rlvkpnrz bbc74930 (empty) (no description set)
    Moved 1 tags to rlvkpnrz bbc74930 (empty) (no description set)
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy  (@) now at: yqosqzyt 13cbd515 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz bbc74930 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  13cbd51558a6
    ◆  bbc749308d7f baz foo
    ◆  b876c5f49546 bar
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "delete", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 1 tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  13cbd51558a6
    ◆  bbc749308d7f baz
    ◆  b876c5f49546 bar
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["tag", "delete", "glob:b*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 tags.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  13cbd51558a6
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 tags pointing to zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such tag: unknown
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["tag", "delete", "glob:unknown*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching tags for patterns: unknown*
    [EOF]
    [exit status: 1]
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

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list"]), @r"
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

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "--color=always"]), @r"
    [38;5;5mconflicted_tag[39m [38;5;1m(conflicted)[39m:
      - [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
      + [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
      + [1m[38;5;5mr[0m[38;5;8moyxmykx[39m [1m[38;5;4m1[0m[38;5;8m3c4e819[39m [38;5;2m(empty)[39m commit3
    [38;5;5mtest_tag[39m: [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
    [38;5;5mtest_tag2[39m: [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
    [EOF]
    ");

    // Test pattern matching.
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "test_tag2"]), @r"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "glob:'test_tag?'"]), @r"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "glob:test* & ~glob:*2"]), @r"
    test_tag: rlvkpnrz 893e67dc (empty) commit1
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
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-T", template]), @r"
    [conflicted_tag]
    present: true
    conflict: true
    normal_target: <Error: No Commit available>
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
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", commit_id.short(), tags) ++ "\n""#;
    work_dir.run_jj(["log", "-rall()", "-T", template])
}
