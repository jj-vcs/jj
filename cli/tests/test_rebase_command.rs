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
use crate::common::create_commit;
use crate::common::create_commit_with_files;

#[test]
fn test_rebase_invalid() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);

    // Missing destination
    let output = work_dir.run_jj(["rebase"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    Usage: jj rebase <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -r and -s
    let output = work_dir.run_jj(["rebase", "-r", "a", "-s", "a", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--revision <REVSETS>' cannot be used with '--source <REVSETS>'

    Usage: jj rebase --revision <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -b and -s
    let output = work_dir.run_jj(["rebase", "-b", "a", "-s", "a", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--branch <REVSETS>' cannot be used with '--source <REVSETS>'

    Usage: jj rebase --branch <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -o and --after
    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b", "--after", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--onto <REVSETS>' cannot be used with '--insert-after <REVSETS>'

    Usage: jj rebase --revision <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -o and --before
    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b", "--before", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--onto <REVSETS>' cannot be used with '--insert-before <REVSETS>'

    Usage: jj rebase --revision <REVSETS> <--onto <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Rebase onto self with -r
    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot rebase a1afb5834d8e onto itself
    [EOF]
    [exit status: 1]
    ");

    // Rebase root with -r
    let output = work_dir.run_jj(["rebase", "-r", "root()", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Rebase onto descendant with -s
    let output = work_dir.run_jj(["rebase", "-s", "a", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot rebase a1afb5834d8e onto descendant dd148a1be8f0
    [EOF]
    [exit status: 1]
    ");

    // Rebase onto itself with -s
    let output = work_dir.run_jj(["rebase", "-s", "a", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot rebase a1afb5834d8e onto itself
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_rebase_empty_sets() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);

    let output = work_dir.run_jj(["rebase", "-r=none()", "-o=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to rebase.
    [EOF]
    ");
    let output = work_dir.run_jj(["rebase", "-s=none()", "-o=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to rebase.
    [EOF]
    ");
    let output = work_dir.run_jj(["rebase", "-b=none()", "-o=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to rebase.
    [EOF]
    ");
    // Empty because "b..a" is empty
    let output = work_dir.run_jj(["rebase", "-b=a", "-o=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to rebase.
    [EOF]
    ");
}

#[test]
fn test_rebase_bookmark() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["b"]);
    create_commit(&work_dir, "e", &["a"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e: a
    │ ○  d: b
    │ │ ○  c: b
    │ ├─╯
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["rebase", "-b", "c", "-o", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  d: b
    │ ○  c: b
    ├─╯
    ○  b: e
    @  e: a
    ○  a
    ◆
    [EOF]
    ");

    // Test rebasing multiple bookmarks at once
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-b=e", "-b=d", "-d=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 commits to destination
    Working copy  (@) now at: nnkkpsqq 43605c50 e | e
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e: b
    │ ○  d: b
    ├─╯
    │ ○  c: b
    ├─╯
    ○  b: a
    ○  a
    ◆
    [EOF]
    ");

    // Same test but with more than one revision per argument
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-b=e|d", "-d=b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 commits to destination
    Working copy  (@) now at: nnkkpsqq c5d0887d e | e
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e: b
    │ ○  d: b
    ├─╯
    │ ○  c: b
    ├─╯
    ○  b: a
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_bookmark_with_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &[]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["a", "d"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e: a d
    ├─╮
    │ ○  d: c
    │ ○  c
    │ │ ○  b: a
    ├───╯
    ○ │  a
    ├─╯
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["rebase", "-b", "d", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: nnkkpsqq 96eb9563 e | e
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : truxwmqv 45bfb346 d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e: a d
    ├─╮
    │ ○  d: c
    │ ○  c: b
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: nnkkpsqq 73206d9a e | e
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : truxwmqv 0f80b7df d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e: a d
    ├─╮
    │ ○  d: c
    │ ○  c: b
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_single_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    create_commit(&work_dir, "d", &["b", "c"]);
    create_commit(&work_dir, "e", &["d"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e: d
    ○    d: b c
    ├─╮
    │ ○  c: a
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Descendants of the rebased commit "c" should be rebased onto parents. First
    // we test with a non-merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "c", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: nnkkpsqq f1a93137 e | e
    Parent commit (@-)      : truxwmqv c12379e7 d | d
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e: d
    ○    d: b a
    ├─╮
    │ │ ○  c: b
    ├───╯
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Now, let's try moving the merge commit. After, both parents of "d" ("b" and
    // "c") should become parents of "e".
    let output = work_dir.run_jj(["rebase", "-r", "d", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nnkkpsqq 3fee6b6e e | e
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Parent commit (@-)      : ooyxmykx 45ee1acd c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e: b c
    ├─╮
    │ ○  c: a
    ○ │  b: a
    ├─╯
    │ ○  d: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_single_revision_merge_parent() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["a", "c"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d: a c
    ├─╮
    │ ○  c: b
    │ ○  b
    ○ │  a
    ├─╯
    ◆
    [EOF]
    ");

    // Descendants of the rebased commit should be rebased onto parents, and if
    // the descendant is a merge commit, it shouldn't forget its other parents.
    let output = work_dir.run_jj(["rebase", "-r", "c", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: truxwmqv c645d2f2 d | d
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : psuskuln 68197203 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d: a b
    ├─╮
    │ ○  b
    │ │ ○  c: a
    ├───╯
    ○ │  a
    ├─╯
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_multiple_revisions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["a"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["c", "e"]);
    create_commit(&work_dir, "g", &["f"]);
    create_commit(&work_dir, "h", &["g"]);
    create_commit(&work_dir, "i", &["f"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  i: f
    │ ○  h: g
    │ ○  g: f
    ├─╯
    ○    f: c e
    ├─╮
    │ ○  e: d
    │ ○  d: a
    ○ │  c: b
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Test with two non-related non-merge commits.
    let output = work_dir.run_jj(["rebase", "-r", "c", "-r", "e", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: mznxytkn 66da4867 i | i
    Parent commit (@-)      : wmkuslsw 9e5af42d f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  i: f
    │ ○  h: g
    │ ○  g: f
    ├─╯
    ○    f: b d
    ├─╮
    │ ○  d: a
    ○ │  b: a
    ├─╯
    │ ○  e: a
    ├─╯
    │ ○  c: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Test with two related non-merge commits. Since "b" is a parent of "c", when
    // rebasing commits "b" and "c", their ancestry relationship should be
    // preserved.
    let output = work_dir.run_jj(["rebase", "-r", "b", "-r", "c", "-o", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: mznxytkn 418a1685 i | i
    Parent commit (@-)      : wmkuslsw 1bed2ac3 f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  i: f
    │ ○  h: g
    │ ○  g: f
    ├─╯
    ○    f: a e
    ├─╮
    │ │ ○  c: b
    │ │ ○  b: e
    │ ├─╯
    │ ○  e: d
    │ ○  d: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Test with a subgraph containing a merge commit. Since the merge commit "f"
    // was extracted, its descendants which are not part of the subgraph will
    // inherit its descendants which are not in the subtree ("c" and "d").
    // "f" will retain its parent "c" since "c" is outside the target set, and not
    // a descendant of any new children.
    let output = work_dir.run_jj(["rebase", "-r", "e::g", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn c9744725 i | i
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Parent commit (@-)      : truxwmqv 4128ea23 d | d
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    i: c d
    ├─╮
    │ │ ○  h: c d
    ╭─┬─╯
    │ ○  d: a
    │ │ ○  g: f
    │ │ ○  f: c e
    ╭───┤
    │ │ ○  e: a
    │ ├─╯
    ○ │  c: b
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Test with commits in a disconnected subgraph. The subgraph has the
    // relationship d->e->f->g->h, but only "d", "f" and "h" are in the set of
    // rebased commits. "d" should be a new parent of "f", and "f" should be a
    // new parent of "h". "f" will retain its parent "c" since "c" is outside the
    // target set, and not a descendant of any new children.
    let output = work_dir.run_jj(["rebase", "-r", "d", "-r", "f", "-r", "h", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn dc967ff0 i | i
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Parent commit (@-)      : nnkkpsqq b1c30a09 e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    i: c e
    ├─╮
    │ │ ○  g: c e
    ╭─┬─╯
    │ ○  e: a
    │ │ ○  h: f
    │ │ ○  f: c d
    ╭───┤
    │ │ ○  d: b
    ○ │ │  c: b
    ├───╯
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Test rebasing a subgraph onto its descendants.
    let output = work_dir.run_jj(["rebase", "-r", "d::e", "-o", "i"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: mznxytkn fb7c5fda i | i
    Parent commit (@-)      : wmkuslsw f2cd3f2f f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: d
    ○  d: i
    @  i: f
    │ ○  h: g
    │ ○  g: f
    ├─╯
    ○    f: c a
    ├─╮
    ○ │  c: b
    ○ │  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_revision_onto_descendant() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "base", &[]);
    create_commit(&work_dir, "a", &["base"]);
    create_commit(&work_dir, "b", &["base"]);
    create_commit(&work_dir, "merge", &["b", "a"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    merge: b a
    ├─╮
    │ ○  a: base
    ○ │  b: base
    ├─╯
    ○  base
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Simpler example
    let output = work_dir.run_jj(["rebase", "-r", "base", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: truxwmqv 19e391d7 merge | merge
    Parent commit (@-)      : ooyxmykx d9d2f54e b | b
    Parent commit (@-)      : psuskuln 6dd92f91 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    merge: b a
    ├─╮
    ○ │  b
    │ │ ○  base: a
    │ ├─╯
    │ ○  a
    ├─╯
    ◆
    [EOF]
    ");

    // Now, let's rebase onto the descendant merge
    let output = work_dir.run_jj(["op", "restore", &setup_opid]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Restored to operation: e09af3b83472 (2001-02-03 08:05:15) create bookmark merge pointing to commit dd829e0392b85222f36fbe9ae7849485f21a42f8
    Working copy  (@) now at: truxwmqv dd829e03 merge | merge
    Parent commit (@-)      : ooyxmykx 0a53dfba b | b
    Parent commit (@-)      : psuskuln b84f2eaa a | a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["rebase", "-r", "base", "-o", "merge"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: truxwmqv 1f81f03c merge | merge
    Parent commit (@-)      : ooyxmykx 38f11f69 b | b
    Parent commit (@-)      : psuskuln 74823f78 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  base: merge
    @    merge: b a
    ├─╮
    │ ○  a
    ○ │  b
    ├─╯
    ◆
    [EOF]
    ");

    // TODO(ilyagr): These will be good tests for `jj rebase --insert-after` and
    // `--insert-before`, once those are implemented.
}

#[test]
fn test_rebase_multiple_destinations() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c
    │ ○  b
    ├─╯
    │ ○  a
    ├─╯
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    a: b c
    ├─╮
    │ @  c
    ○ │  b
    ├─╯
    ◆
    [EOF]
    ");

    work_dir.run_jj(["rebase", "-r=a", "-d=b|c"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    a: c b
    ├─╮
    │ ○  b
    @ │  c
    ├─╯
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b|c", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_rebase_with_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &["a", "b"]);
    create_commit(&work_dir, "d", &["c"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: c
    ○    c: a b
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["rebase", "-s", "b", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: truxwmqv 3f23f84f d | d
    Parent commit (@-)      : ooyxmykx 44d8d318 c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: c
    ○    c: a b
    ├─╮
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Rebase several subtrees at once.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-s=c", "-s=d", "-d=a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Working copy  (@) now at: truxwmqv 69451db8 d | d
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: a
    │ ○  c: a
    ├─╯
    ○  a
    │ ○  b
    ├─╯
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Reminder of the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: c
    ○    c: a b
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ◆
    [EOF]
    ");

    // `d` was a descendant of `b`, and both are moved to be direct descendants of
    // `a`. `c` remains a descendant of `b`.
    let output = work_dir.run_jj(["rebase", "-s=b", "-s=d", "-d=a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: truxwmqv f7da4541 d | d
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: a
    │ ○  c: a b
    ╭─┤
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Same test as above, but with multiple commits per argument
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-s=b|d", "-d=a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: truxwmqv fa0a8327 d | d
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d: a
    │ ○  c: a b
    ╭─┤
    │ ○  b: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_error_revision_does_not_exist() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "one"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b-one"])
        .success();
    work_dir.run_jj(["new", "-r", "@-", "-m", "two"]).success();

    let output = work_dir.run_jj(["rebase", "-b", "b-one", "-o", "this"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Revision `this` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["rebase", "-b", "this", "-o", "b-one"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Revision `this` doesn't exist
    [EOF]
    [exit status: 1]
    ");
}

// This behavior illustrates https://github.com/jj-vcs/jj/issues/2600
#[test]
fn test_rebase_with_child_and_descendant_bug_2600() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "notroot", &[]);
    create_commit(&work_dir, "base", &["notroot"]);
    create_commit(&work_dir, "a", &["base"]);
    create_commit(&work_dir, "b", &["base", "a"]);
    create_commit(&work_dir, "c", &["b"]);
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    // ===================== rebase -s tests =================
    // This should be a no-op
    let output = work_dir.run_jj(["rebase", "-s", "base", "-o", "notroot"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // This should be a no-op
    let output = work_dir.run_jj(["rebase", "-s", "a", "-o", "base"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 3 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-s", "a", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: nnkkpsqq 28c8257b c | c
    Parent commit (@-)      : truxwmqv bf656505 b | b
    [EOF]
    ");
    // Commit "a" should be rebased onto the root commit. Commit "b" should have
    // "base" and "a" as parents as before.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a
    ○ │  base: notroot
    ○ │  notroot
    ├─╯
    ◆
    [EOF]
    ");

    // ===================== rebase -b tests =================
    // ====== Reminder of the setup =========
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    // The commits in roots(base..c), i.e. commit "a" should be rebased onto "base",
    // which is a no-op
    let output = work_dir.run_jj(["rebase", "-b", "c", "-o", "base"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 3 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-b", "c", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Working copy  (@) now at: nnkkpsqq 1ce813dd c | c
    Parent commit (@-)      : truxwmqv 24bc9f16 b | b
    [EOF]
    ");
    // The commits in roots(a..c), i.e. commit "b" should be rebased onto "a",
    // which means "b" loses its "base" parent
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○  b: a
    ○  a: base
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // This should be a no-op
    let output = work_dir.run_jj(["rebase", "-b", "a", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 5 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    // ===================== rebase -r tests =================
    // ====== Reminder of the setup =========
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "base", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: nnkkpsqq 24fecdce c | c
    Parent commit (@-)      : truxwmqv 4ca66b87 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // The user would expect unsimplified ancestry here.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: notroot a
    ├─╮
    │ ○  a: notroot
    ├─╯
    ○  notroot
    │ ○  base
    ├─╯
    ◆
    [EOF]
    ");

    // This tests the algorithm for rebasing onto descendants. The result should
    // have unsimplified ancestry.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-r", "base", "-o", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: nnkkpsqq 92dfe4c3 c | c
    Parent commit (@-)      : truxwmqv dd6dfb0f b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    │ ○  base: b
    ├─╯
    ○    b: notroot a
    ├─╮
    │ ○  a: notroot
    ├─╯
    ○  notroot
    ◆
    [EOF]
    ");

    // This tests the algorithm for rebasing onto descendants. The result should
    // have unsimplified ancestry.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-r", "base", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: nnkkpsqq 007f3567 c | c
    Parent commit (@-)      : truxwmqv b11b9da4 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: notroot a
    ├─╮
    │ │ ○  base: a
    │ ├─╯
    │ ○  a: notroot
    ├─╯
    ○  notroot
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // ====== Reminder of the setup =========
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○    b: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: nnkkpsqq 8d6c0157 c | c
    Parent commit (@-)      : truxwmqv 65268dcb b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // In this case, it is unclear whether the user would always prefer unsimplified
    // ancestry (whether `b` should also be a direct child of the root commit).
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: b
    ○  b: base
    ○  base: notroot
    ○  notroot
    │ ○  a
    ├─╯
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-r", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nnkkpsqq 894aeb1a c | c
    Parent commit (@-)      : psuskuln 6196dfbc base | base
    Parent commit (@-)      : ooyxmykx 047e2289 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // The user would expect unsimplified ancestry here.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    c: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    │ ○  b
    ├─╯
    ◆
    [EOF]
    ");

    // This tests the algorithm for rebasing onto descendants. The result should
    // have unsimplified ancestry.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-r", "b", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nnkkpsqq 351a5a09 c | c
    Parent commit (@-)      : psuskuln 6196dfbc base | base
    Parent commit (@-)      : ooyxmykx 047e2289 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  b: c
    @    c: base a
    ├─╮
    │ ○  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");

    // In this test, the commit with weird ancestry is not rebased (neither directly
    // nor indirectly).
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["rebase", "-r", "c", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Working copy  (@) now at: nnkkpsqq 91590152 c | c
    Parent commit (@-)      : ooyxmykx 047e2289 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c: a
    │ ○  b: base a
    ╭─┤
    ○ │  a: base
    ├─╯
    ○  base: notroot
    ○  notroot
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b1", &["a"]);
    create_commit(&work_dir, "b2", &["b1"]);
    create_commit(&work_dir, "b3", &["a"]);
    create_commit(&work_dir, "b4", &["b3"]);
    create_commit(&work_dir, "c", &["b2", "b4"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["c"]);
    create_commit(&work_dir, "f", &["e"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Rebasing a commit after its parents should be a no-op.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--after", "b2", "--after", "b4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Rebasing a commit after itself should be a no-op.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--after", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Rebase a commit after another commit. "c" has parents "b2" and "b4", so its
    // children "d" and "e" should be rebased onto "b2" and "b4" respectively.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--after", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn bce7abff f | f
    Parent commit (@-)      : wmkuslsw 50ffbdf1 c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: c
    ○  c: e
    ○    e: b2 b4
    ├─╮
    │ │ ○  d: b2 b4
    ╭─┬─╯
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after a leaf commit.
    let output = work_dir.run_jj(["rebase", "-r", "e", "--after", "f"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: mznxytkn 6c2cc0e2 f | f
    Parent commit (@-)      : wmkuslsw 4ced35a9 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: f
    @  f: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after a commit in a bookmark of a merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--after", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: mznxytkn 53eb6e61 f | f
    Parent commit (@-)      : psuskuln f25fb4a7 b1 | b1
    Added 0 files, modified 0 files, removed 5 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: f
    @ │  f: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after the last commit in a bookmark of a merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn 97018816 f | f
    Parent commit (@-)      : ooyxmykx 45d26d47 b2 | b2
    Added 0 files, modified 0 files, removed 4 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: f b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    @ │  f: b2
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after a commit with multiple children.
    // "c" has two children "d" and "e", so the rebased commit "f" will inherit the
    // two children.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--after", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn b562ffed f | f
    Parent commit (@-)      : wmkuslsw 4ced35a9 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: f
    │ ○  d: f
    ├─╯
    @  f: c
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after multiple commits.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--after", "e", "--after", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Working copy  (@) now at: mznxytkn 8d811d78 f | f
    Parent commit (@-)      : ukmrtpmo d2187801 e | e
    Parent commit (@-)      : lylxulpl 8cdda097 d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    f: e d
    ├─╮
    │ ○  d: c
    ○ │  e: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase two unrelated commits.
    let output = work_dir.run_jj(["rebase", "-r", "d", "-r", "e", "--after", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: mznxytkn facccbdf f | f
    Parent commit (@-)      : wmkuslsw 7ddff975 c | c
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: c
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○    b3: d e
    │ ├─╮
    ○ │ │  b2: b1
    ○ │ │  b1: d e
    ╰─┬─╮
      │ ○  e: a
      ○ │  d: a
      ├─╯
      ○  a
      ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph with merge commit and two parents, which should preserve
    // the merge.
    let output = work_dir.run_jj(["rebase", "-r", "b2", "-r", "b4", "-r", "c", "--after", "f"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn 866e79ac f | f
    Parent commit (@-)      : ukmrtpmo 8def8f6d e | e
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    c: b2 b4
    ├─╮
    │ ○  b4: f
    ○ │  b2: f
    ├─╯
    @  f: e
    ○    e: b1 b3
    ├─╮
    │ │ ○  d: b1 b3
    ╭─┬─╯
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph with four commits after one of the commits itself.
    let output = work_dir.run_jj(["rebase", "-r", "b1::d", "--after", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn 75476d72 f | f
    Parent commit (@-)      : ukmrtpmo ec42d80d e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: d
    ○  d: c
    ○    c: b2 b4
    ├─╮
    ○ │  b2: b1
    ○ │  b1: a b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph before the parents of one of the commits in the subgraph.
    // "c" had parents "b2" and "b4", but no longer has "b4" as a parent since
    // "b4" would be a descendant of "c" after the rebase.
    let output = work_dir.run_jj(["rebase", "-r", "b2::d", "--after", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: mznxytkn 3a0f2a4c f | f
    Parent commit (@-)      : ukmrtpmo 3d196a08 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○    e: b1 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a: d
    ○  d: c
    ○  c: b2
    ○  b2
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph with disconnected commits. Since "b2" is an ancestor of
    // "e", "b2" should be a parent of "e" after the rebase.
    let output = work_dir.run_jj(["rebase", "-r", "e", "-r", "b2", "--after", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn 4640c66e f | f
    Parent commit (@-)      : wmkuslsw c06f67e9 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: c
    │ ○  e: b2
    │ ○  b2: d
    │ ○  d: c
    ├─╯
    ○    c: b1 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -s` of commit "c" and its descendants after itself should be a no-op.
    let output = work_dir.run_jj(["rebase", "-s", "c", "--after", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -s` of a commit and its descendants after multiple commits.
    let output = work_dir.run_jj(["rebase", "-s", "c", "--after", "b1", "--after", "b3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn 68a7e117 f | f
    Parent commit (@-)      : ukmrtpmo cd418fb4 e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    b4: d f
    ├─╮
    │ │ ○  b2: d f
    ╭─┬─╯
    │ @  f: e
    │ ○  e: c
    ○ │  d: c
    ├─╯
    ○    c: b1 b3
    ├─╮
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -b` of commit "b3" after "b1" moves its descendants which are not
    // already descendants of "b1" (just "b3" and "b4") in between "b1" and its
    // child "b2".
    let output = work_dir.run_jj(["rebase", "-b", "b3", "--after", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 6 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: mznxytkn cbb45c01 f | f
    Parent commit (@-)      : ukmrtpmo e33f8015 e | e
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    b2: d f
    ├─╮
    │ @  f: e
    │ ○  e: c
    ○ │  d: c
    ├─╯
    ○  c: b4
    ○  b4: b3
    ○  b3: b1
    ○  b1: a
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["rebase", "-r", "e", "--after", "a", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit 45d26d47b68e would be both an ancestor and a descendant of the rebased commits
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_rebase_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b1", &["a"]);
    create_commit(&work_dir, "b2", &["b1"]);
    create_commit(&work_dir, "b3", &["a"]);
    create_commit(&work_dir, "b4", &["b3"]);
    create_commit(&work_dir, "c", &["b2", "b4"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["c"]);
    create_commit(&work_dir, "f", &["e"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Rebasing a commit before its children should be a no-op.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--before", "d", "--before", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Rebasing a commit before itself should be a no-op.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--before", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    // Rebasing a commit before the root commit should error.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--before", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Rebase a commit before another commit. "c" has parents "b2" and "b4", so its
    // children "d" and "e" should be rebased onto "b2" and "b4" respectively.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 8 descendant commits
    Working copy  (@) now at: mznxytkn 9ad72921 f | f
    Parent commit (@-)      : ukmrtpmo 8a9188c6 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○    e: b2 b4
    ├─╮
    │ │ ○  d: b2 b4
    ╭─┬─╯
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a: c
    ○  c
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before its parent.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--before", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: mznxytkn 2efa7fc1 f | f
    Parent commit (@-)      : wmkuslsw 4ced35a9 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: f
    @  f: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before a commit in a bookmark of a merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: mznxytkn a26aa5fc f | f
    Parent commit (@-)      : psuskuln f25fb4a7 b1 | b1
    Added 0 files, modified 0 files, removed 5 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: f
    @ │  f: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before the first commit in a bookmark of a merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--before", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: mznxytkn f61575c2 f | f
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 6 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: f
    @ │  f: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before a merge commit. "c" has two parents "b2" and "b4", so
    // the rebased commit "f" will have the two commits "b2" and "b4" as its
    // parents.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--before", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn 47568833 f | f
    Parent commit (@-)      : ooyxmykx 45d26d47 b2 | b2
    Parent commit (@-)      : nnkkpsqq dbdc90ad b4 | b4
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○  c: f
    @    f: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before multiple commits.
    let output = work_dir.run_jj(["rebase", "-r", "b1", "--before", "d", "--before", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: mznxytkn 12850b5a f | f
    Parent commit (@-)      : ukmrtpmo 77d27d96 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: b1
    │ ○  d: b1
    ├─╯
    ○  b1: c
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit before two commits in separate bookmarks to create a merge
    // commit.
    let output = work_dir.run_jj(["rebase", "-r", "f", "--before", "b2", "--before", "b4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: mznxytkn 7f7076b2 f | f
    Parent commit (@-)      : psuskuln f25fb4a7 b1 | b1
    Parent commit (@-)      : truxwmqv f4e418ee b3 | b3
    Added 0 files, modified 0 files, removed 4 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: f
    ○ │  b2: f
    ├─╯
    @    f: b1 b3
    ├─╮
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase two unrelated commits "b2" and "b4" before a single commit "a". This
    // creates a merge commit "a" with the two parents "b2" and "b4".
    let output = work_dir.run_jj(["rebase", "-r", "b2", "-r", "b4", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 7 descendant commits
    Working copy  (@) now at: mznxytkn d91de079 f | f
    Parent commit (@-)      : ukmrtpmo f4496ded e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b1 b3
    ├─╮
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○    a: b2 b4
    ├─╮
    │ ○  b4
    ○ │  b2
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph with a merge commit and two parents.
    let output = work_dir.run_jj(["rebase", "-r", "b2", "-r", "b4", "-r", "c", "--before", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: mznxytkn 9b9d3e65 f | f
    Parent commit (@-)      : ukmrtpmo 5df1a0f3 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    ○    c: b2 b4
    ├─╮
    │ ○    b4: b1 b3
    │ ├─╮
    ○ │ │  b2: b1 b3
    ╰─┬─╮
    ○ │ │  d: b1 b3
    ╰─┬─╮
      │ ○  b3: a
      ○ │  b1: a
      ├─╯
      ○  a
      ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph with disconnected commits. Since "b1" is an ancestor of
    // "e", "b1" should be a parent of "e" after the rebase.
    let output = work_dir.run_jj(["rebase", "-r", "b1", "-r", "e", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 7 descendant commits
    Working copy  (@) now at: mznxytkn 7324154c f | f
    Parent commit (@-)      : wmkuslsw b16d99f4 c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: a
    ├─╯
    ○  a: e
    ○  e: b1
    ○  b1
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph before the parents of one of the commits in the subgraph.
    // "c" had parents "b2" and "b4", but no longer has "b4" as a parent since
    // "b4" would be a descendant of "c" after the rebase.
    let output = work_dir.run_jj(["rebase", "-r", "b2::d", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: mznxytkn 10241544 f | f
    Parent commit (@-)      : ukmrtpmo 6ad210f0 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○    e: b1 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a: d
    ○  d: c
    ○  c: b2
    ○  b2
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a subgraph before the parents of one of the commits in the subgraph.
    // "c" had parents "b2" and "b4", but no longer has "b4" as a parent since
    // "b4" would be a descendant of "c" after the rebase.
    let output = work_dir.run_jj(["rebase", "-r", "b2::d", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: mznxytkn 0e72fc2a f | f
    Parent commit (@-)      : ukmrtpmo 1cd0849d e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○    e: b1 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a: d
    ○  d: c
    ○  c: b2
    ○  b2
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -s` of commit "c" and its descendants before itself should be a
    // no-op.
    let output = work_dir.run_jj(["rebase", "-s", "c", "--before", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b2 b4
    ├─╮
    │ ○  b4: b3
    │ ○  b3: a
    ○ │  b2: b1
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -s` of a commit and its descendants before multiple commits.
    let output = work_dir.run_jj(["rebase", "-s", "c", "--before", "b2", "--before", "b4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn e66b4e24 f | f
    Parent commit (@-)      : ukmrtpmo b53bae5c e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    b4: d f
    ├─╮
    │ │ ○  b2: d f
    ╭─┬─╯
    │ @  f: e
    │ ○  e: c
    ○ │  d: c
    ├─╯
    ○    c: b1 b3
    ├─╮
    │ ○  b3: a
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -b` of commit "b3" before "b2" moves its descendants which are not
    // already descendants of its parent "b1" (just "b3" and "b4") in between "b1"
    // and its child "b2".
    let output = work_dir.run_jj(["rebase", "-b", "b3", "--before", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 2 commits that were already in place
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: mznxytkn 47cd1279 f | f
    Parent commit (@-)      : ukmrtpmo 30126eda e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  b2: b1
    ○    b1: d f
    ├─╮
    │ @  f: e
    │ ○  e: c
    ○ │  d: c
    ├─╯
    ○  c: b4
    ○  b4: b3
    ○  b3: a
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["rebase", "-r", "e", "--before", "b2", "--before", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit 45d26d47b68e would be both an ancestor and a descendant of the rebased commits
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_rebase_after_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "x", &[]);
    create_commit(&work_dir, "y", &["x"]);
    create_commit(&work_dir, "z", &["y"]);
    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b1", &["a"]);
    create_commit(&work_dir, "b2", &["a"]);
    create_commit(&work_dir, "c", &["b1", "b2"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["c"]);
    create_commit(&work_dir, "f", &["e"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○    c: b1 b2
    ├─╮
    │ ○  b2: a
    ○ │  b1: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Rebase a commit after another commit and before that commit's child to
    // insert directly between the two commits.
    let output = work_dir.run_jj(["rebase", "-r", "d", "--after", "e", "--before", "f"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: rmzmmopx 75afe005 f | f
    Parent commit (@-)      : ukmrtpmo 2a7788db d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: d
    ○  d: e
    ○  e: c
    ○    c: b1 b2
    ├─╮
    │ ○  b2: a
    ○ │  b1: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase a commit after another commit and before that commit's descendant to
    // create a new merge commit.
    let output = work_dir.run_jj(["rebase", "-r", "d", "--after", "a", "--before", "f"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: rmzmmopx 1d306167 f | f
    Parent commit (@-)      : mznxytkn 7cf7536e e | e
    Parent commit (@-)      : ukmrtpmo cbc96295 d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    f: e d
    ├─╮
    │ ○  d: a
    ○ │  e: c
    ○ │    c: b1 b2
    ├───╮
    │ │ ○  b2: a
    │ ├─╯
    ○ │  b1: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // "c" has parents "b1" and "b2", so when it is rebased, its children "d" and
    // "e" should have "b1" and "b2" as parents as well. "c" is then inserted in
    // between "d" and "e", making "e" a merge commit with 3 parents "b1", "b2",
    // and "c".
    let output = work_dir.run_jj(["rebase", "-r", "c", "--after", "d", "--before", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: rmzmmopx 5aefadf1 f | f
    Parent commit (@-)      : mznxytkn 044c18c0 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○      e: b1 b2 c
    ├─┬─╮
    │ │ ○  c: d
    │ │ ○  d: b1 b2
    ╭─┬─╯
    │ ○  b2: a
    ○ │  b1: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Rebase multiple commits and preserve their ancestry. Apart from the heads of
    // the target commits ("d" and "e"), "f" also has commits "b1" and "b2" as
    // parents since its parents "d" and "e" were in the target set and were
    // replaced by their closest ancestors outside the target set.
    let output = work_dir.run_jj([
        "rebase", "-r", "c", "-r", "d", "-r", "e", "--after", "a", "--before", "f",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: rmzmmopx ca5eea1b f | f
    Parent commit (@-)      : nnkkpsqq 8d926ed2 b1 | b1
    Parent commit (@-)      : wmkuslsw 5ad2ce01 b2 | b2
    Parent commit (@-)      : ukmrtpmo 77dc7a23 d | d
    Parent commit (@-)      : mznxytkn 191822b8 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @        f: b1 b2 d e
    ├─┬─┬─╮
    │ │ │ ○  e: c
    │ │ ○ │  d: c
    │ │ ├─╯
    │ │ ○  c: a
    │ ○ │  b2: a
    │ ├─╯
    ○ │  b1: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -s` of a commit and its descendants.
    let output = work_dir.run_jj(["rebase", "-s", "c", "--before", "b1", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: rmzmmopx 48416ff2 f | f
    Parent commit (@-)      : mznxytkn bb9a5da7 e | e
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○      b1: a d f
    ├─┬─╮
    │ │ @  f: e
    │ │ ○  e: c
    │ ○ │  d: c
    │ ├─╯
    │ ○  c: b2
    │ ○  b2: a
    ├─╯
    ○  a
    │ ○  z: y
    │ ○  y: x
    │ ○  x
    ├─╯
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // `rebase -b` of a commit "y" to a destination after "a" will rebase all
    // commits in "roots(a..y)" and their descendants, corresponding to "x", "y"
    // and "z". They will be inserted in a new branch after "a" and before "c".
    let output = work_dir.run_jj(["rebase", "-b", "y", "--after", "a", "--before", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: rmzmmopx 01ca8d88 f | f
    Parent commit (@-)      : mznxytkn f0d2ea8a e | e
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f: e
    ○  e: c
    │ ○  d: c
    ├─╯
    ○      c: b1 b2 z
    ├─┬─╮
    │ │ ○  z: y
    │ │ ○  y: x
    │ │ ○  x: a
    │ ○ │  b2: a
    │ ├─╯
    ○ │  b1: a
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["rebase", "-r", "e", "--after", "c", "--before", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit d7c690483f8f would be both an ancestor and a descendant of the rebased commits
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_rebase_skip_emptied() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    work_dir
        .run_jj(["new", "a", "-m", "will become empty"])
        .success();
    work_dir.run_jj(["restore", "--from=b"]).success();
    work_dir.run_jj(["new", "-m", "already empty"]).success();
    work_dir
        .run_jj(["new", "-m", "also already empty"])
        .success();
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    ○  will become empty
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-d=b", "--skip-emptied"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 commits to destination
    Abandoned 1 newly emptied commits
    Working copy  (@) now at: rostqsxw 473f1afe (empty) also already empty
    Parent commit (@-)      : truxwmqv 5c9b67d6 (empty) already empty
    [EOF]
    ");

    // The parent commit became empty and was dropped, but the already empty commits
    // were kept
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    ○  b
    ○  a
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Test the setup
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    ○  will become empty
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj([
        "rebase",
        "-r=subject('will become empty')",
        "-d=b",
        "--skip-emptied",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Abandoned 1 newly emptied commits
    Working copy  (@) now at: rostqsxw 9699c50e (empty) also already empty
    Parent commit (@-)      : truxwmqv 900cf9c9 (empty) already empty
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // Rebasing a single commit which becomes empty abandons that commit, whilst its
    // already empty descendants were kept
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_skip_emptied_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    work_dir
        .run_jj(["new", "a", "-m", "c (will become empty)"])
        .success();
    work_dir.run_jj(["restore", "--from=b"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.run_jj(["new", "-m", "already empty"]).success();
    work_dir
        .run_jj(["new", "-m", "also already empty"])
        .success();

    // Test the setup
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    ○  c (will become empty)
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "b", "--before", "c", "--skip-emptied"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 3 descendant commits
    Working copy  (@) now at: nnkkpsqq 5e95fd69 (empty) also already empty
    Parent commit (@-)      : rostqsxw bdfdcd74 (empty) already empty
    [EOF]
    ");

    // Commits not in the rebase target set should not be abandoned even if they
    // were emptied.
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @"
    @  also already empty
    ○  already empty
    ○  c (will become empty)
    ○  b
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_rebase_skip_if_on_destination() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b1", &["a"]);
    create_commit(&work_dir, "b2", &["a"]);
    create_commit(&work_dir, "c", &["b1", "b2"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["c"]);
    create_commit(&work_dir, "f", &["e"]);
    // Test the setup
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  f  lylxulpl  e3d4541d:  e
    ○  e  wmkuslsw  b5d5eea1:  c
    │ ○  d  nnkkpsqq  ecd2023a:  c
    ├─╯
    ○    c  truxwmqv  e6f915d4:  b1 b2
    ├─╮
    │ ○  b2  ooyxmykx  92cf5426:  a
    ○ │  b1  psuskuln  f25fb4a7:  a
    ├─╯
    ○  a  ylvkpnrz  a1afb583
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -b
    let output = work_dir.run_jj(["rebase", "-b", "d", "-o", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 6 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  f  lylxulpl  e3d4541d:  e
    ○  e  wmkuslsw  b5d5eea1:  c
    │ ○  d  nnkkpsqq  ecd2023a:  c
    ├─╯
    ○    c  truxwmqv  e6f915d4:  b1 b2
    ├─╮
    │ ○  b2  ooyxmykx  92cf5426:  a
    ○ │  b1  psuskuln  f25fb4a7:  a
    ├─╯
    ○  a  ylvkpnrz  a1afb583
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -s
    let output = work_dir.run_jj(["rebase", "-s", "c", "-o", "b1", "-o", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  f  lylxulpl  e3d4541d:  e
    ○  e  wmkuslsw  b5d5eea1:  c
    │ ○  d  nnkkpsqq  ecd2023a:  c
    ├─╯
    ○    c  truxwmqv  e6f915d4:  b1 b2
    ├─╮
    │ ○  b2  ooyxmykx  92cf5426:  a
    ○ │  b1  psuskuln  f25fb4a7:  a
    ├─╯
    ○  a  ylvkpnrz  a1afb583
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -r since commit has no children
    let output = work_dir.run_jj(["rebase", "-r", "d", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  f  lylxulpl  e3d4541d:  e
    ○  e  wmkuslsw  b5d5eea1:  c
    │ ○  d  nnkkpsqq  ecd2023a:  c
    ├─╯
    ○    c  truxwmqv  e6f915d4:  b1 b2
    ├─╮
    │ ○  b2  ooyxmykx  92cf5426:  a
    ○ │  b1  psuskuln  f25fb4a7:  a
    ├─╯
    ○  a  ylvkpnrz  a1afb583
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase of commit, but rebases children onto destination with -r
    let output = work_dir.run_jj(["rebase", "-r", "e", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 descendant commits
    Working copy  (@) now at: lylxulpl f85207d4 f | f
    Parent commit (@-)      : truxwmqv e6f915d4 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  f  lylxulpl  f85207d4:  c
    │ ○  e  wmkuslsw  b5d5eea1:  c
    ├─╯
    │ ○  d  nnkkpsqq  ecd2023a:  c
    ├─╯
    ○    c  truxwmqv  e6f915d4:  b1 b2
    ├─╮
    │ ○  b2  ooyxmykx  92cf5426:  a
    ○ │  b1  psuskuln  f25fb4a7:  a
    ├─╯
    ○  a  ylvkpnrz  a1afb583
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
}

#[test]
fn test_rebase_skip_duplicate_divergent() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  d  qnkkpsqq  176a675a:  b1
    ○  b1  psuskuln  62c0c393:  a
    │ ○  b2  psuskuln  9f995fe1:  c
    │ ○  c  ooyxmykx  01c753d5:  a
    ├─╯
    ○  a  ylvkpnrz  b09f5d37
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // By default, rebase should skip the duplicate of commit B
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-r", "c::", "-o", "d"]), @"
    ------- stderr -------
    Abandoned 1 divergent commits that were already present in the destination:
      psuskuln/0 9f995fe1 b2 | (divergent) b2
    Rebased 1 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    ○  b2 c  ooyxmykx  9addd745:  d
    @  d  qnkkpsqq  176a675a:  b1
    ○  b1  psuskuln  62c0c393:  a
    ○  a  ylvkpnrz  b09f5d37
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Rebasing should work even if the root of the target set is abandoned
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-s", "b1", "-o", "b2"]), @"
    ------- stderr -------
    Abandoned 1 divergent commits that were already present in the destination:
      psuskuln/1 62c0c393 b1 | (divergent) b2
    Rebased 1 commits to destination
    Working copy  (@) now at: qnkkpsqq fcb2c657 d | d
    Parent commit (@-)      : psuskuln 9f995fe1 b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    // BUG: "d" should be on top of "b2", but it wasn't rebased
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  d  qnkkpsqq  fcb2c657:  b1 b2
    ○  b1 b2  psuskuln  9f995fe1:  c
    ○  c  ooyxmykx  01c753d5:  a
    ○  a  ylvkpnrz  b09f5d37
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Rebase with "--keep-divergent" shouldn't skip any duplicates
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-s", "c", "-o", "d", "--keep-divergent"]), @"
    ------- stderr -------
    Rebased 2 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    ○  b2  psuskuln  6938ed92:  c
    ○  c  ooyxmykx  9a1d551e:  d
    @  d  qnkkpsqq  176a675a:  b1
    ○  b1  psuskuln  62c0c393:  a
    ○  a  ylvkpnrz  b09f5d37
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
}

#[test]
fn test_rebase_simplify_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "a1", &["a"]);
    create_commit(&work_dir, "a2", &["a1"]);
    create_commit(&work_dir, "a3", &["a2", "a1"]);
    create_commit(&work_dir, "b", &[]);
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  b  nnkkpsqq  f7b4d1b6
    │ ○    a3  truxwmqv  c64a94f8:  a2 a1
    │ ├─╮
    │ ○ │  a2  ooyxmykx  e09d3a5d:  a1
    │ ├─╯
    │ ○  a1  psuskuln  ea8ceb1c:  a
    │ ○  a  ylvkpnrz  a1afb583
    ├─╯
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // without --simplify-parents, should transplant the whole tree structure
    work_dir.run_jj(["rebase", "-s", "a", "-o", "b"]).success();
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    ○    a3  truxwmqv  79dc04ed:  a2 a1
    ├─╮
    ○ │  a2  ooyxmykx  8e6523c9:  a1
    ├─╯
    ○  a1  psuskuln  f1ee7969:  a
    ○  a  ylvkpnrz  fcca0365:  b
    @  b  nnkkpsqq  f7b4d1b6
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // with --simplify-parents, should drop the redundant a2 parent on a3
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj(["rebase", "-s", "a", "-o", "b", "--simplify-parents"])
        .success();
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    ○  a3  truxwmqv  60c499ca:  a2
    ○  a2  ooyxmykx  1a830140:  a1
    ○  a1  psuskuln  cd265e09:  a
    ○  a  ylvkpnrz  38a98cae:  b
    @  b  nnkkpsqq  f7b4d1b6
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // but, does not apply to autorebased commits
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj(["rebase", "-r", "a", "-o", "b", "--simplify-parents"])
        .success();
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    ○  a  ylvkpnrz  7407d5f2:  b
    @  b  nnkkpsqq  f7b4d1b6
    │ ○    a3  truxwmqv  eba1aca8:  a2 a1
    │ ├─╮
    │ ○ │  a2  ooyxmykx  586aa140:  a1
    │ ├─╯
    │ ○  a1  psuskuln  5e779699
    ├─╯
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = "bookmarks ++ surround(': ', '', parents.map(|c| c.bookmarks()))";
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_long_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = "bookmarks ++ '  ' ++ change_id.shortest(8) ++ '  ' ++ commit_id.shortest(8) \
                    ++ surround(':  ', '', parents.map(|c| c.bookmarks()))";
    work_dir.run_jj(["log", "-T", template])
}
