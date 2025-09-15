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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    Usage: jj rebase <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -r and -s
    let output = work_dir.run_jj(["rebase", "-r", "a", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--revisions <REVSETS>' cannot be used with '--source <REVSETS>'

    Usage: jj rebase --revisions <REVSETS> <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -b and -s
    let output = work_dir.run_jj(["rebase", "-b", "a", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--branch <REVSETS>' cannot be used with '--source <REVSETS>'

    Usage: jj rebase --branch <REVSETS> <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -d and --after
    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b", "--after", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--destination <REVSETS>' cannot be used with '--insert-after <REVSETS>'

    Usage: jj rebase --revisions <REVSETS> <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Both -d and --before
    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b", "--before", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--destination <REVSETS>' cannot be used with '--insert-before <REVSETS>'

    Usage: jj rebase --revisions <REVSETS> <--destination <REVSETS>|--insert-after <REVSETS>|--insert-before <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Rebase onto self with -r
    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot rebase 7d980be7a1d4 onto itself
    [EOF]
    [exit status: 1]
    ");

    // Rebase root with -r
    let output = work_dir.run_jj(["rebase", "-r", "root()", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root revision 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Rebase onto descendant with -s
    let output = work_dir.run_jj(["rebase", "-s", "a", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot rebase 7d980be7a1d4 onto descendant 123b4d91f6e5
    [EOF]
    [exit status: 1]
    ");

    // Rebase onto itself with -s
    let output = work_dir.run_jj(["rebase", "-s", "a", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot rebase 7d980be7a1d4 onto itself
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

    // TODO: Make all of these say "Nothing changed"?
    let output = work_dir.run_jj(["rebase", "-r=none()", "-d=b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["rebase", "-s=none()", "-d=b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Empty revision set
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["rebase", "-b=none()", "-d=b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Empty revision set
    [EOF]
    [exit status: 1]
    ");
    // Empty because "b..a" is empty
    let output = work_dir.run_jj(["rebase", "-b=a", "-d=b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-b", "c", "-d", "e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 commits to destination
    Working copy  (@) now at: znkkpsqq bbfb8557 e | e
    Parent revision (@-)    : zsuskuln 123b4d91 b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 commits to destination
    Working copy  (@) now at: znkkpsqq 1ffd7890 e | e
    Parent revision (@-)    : zsuskuln 123b4d91 b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-b", "d", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: znkkpsqq d5360d09 e | e
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Parent revision (@-)    : vruxwmqv 85a741d7 d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: znkkpsqq d3091c0f e | e
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Parent revision (@-)    : vruxwmqv 485905a3 d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "c", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: znkkpsqq 2baedee4 e | e
    Parent revision (@-)    : vruxwmqv 45142a83 d | d
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "d", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq b981a2bc e | e
    Parent revision (@-)    : zsuskuln 123b4d91 b | b
    Parent revision (@-)    : royxmykx 991a7501 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "c", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: vruxwmqv 0bb15a0f d | d
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Parent revision (@-)    : zsuskuln d18ca3e8 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "c", "-r", "e", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: xznxytkn 15078fab i | i
    Parent revision (@-)    : kmkuslsw d8579ed7 f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "b", "-r", "c", "-d", "e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: xznxytkn 4dec544d i | i
    Parent revision (@-)    : kmkuslsw b22816c9 f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "e::g", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn e73a0787 i | i
    Parent revision (@-)    : royxmykx dffaa0d4 c | c
    Parent revision (@-)    : vruxwmqv 6354123d d | d
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "d", "-r", "f", "-r", "h", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn f7c62b49 i | i
    Parent revision (@-)    : royxmykx dffaa0d4 c | c
    Parent revision (@-)    : znkkpsqq 1c3676c4 e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "d::e", "-d", "i"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: xznxytkn b4ece7ad i | i
    Parent revision (@-)    : kmkuslsw 1a05fe0d f | f
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "base", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: vruxwmqv 6a82c6c9 merge | merge
    Parent revision (@-)    : royxmykx 934eadd8 b | b
    Parent revision (@-)    : zsuskuln fd4e3113 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: cb005d7a588c (2001-02-03 08:05:15) create bookmark merge pointing to commit 08c0951bf69d0362708a5223a78446d664823b50
    Working copy  (@) now at: vruxwmqv 08c0951b merge | merge
    Parent revision (@-)    : royxmykx 6a7081ef b | b
    Parent revision (@-)    : zsuskuln 68fbc443 a | a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["rebase", "-r", "base", "-d", "merge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: vruxwmqv 6091a06e merge | merge
    Parent revision (@-)    : royxmykx 072bc1fa b | b
    Parent revision (@-)    : zsuskuln afb318cf a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c
    │ ○  b
    ├─╯
    │ ○  a
    ├─╯
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    a: b c
    ├─╮
    │ @  c
    ○ │  b
    ├─╯
    ◆
    [EOF]
    ");

    let setup_opid2 = work_dir.current_operation_id();
    let output = work_dir.run_jj([
        "rebase",
        "--config=ui.always-allow-large-revsets=false",
        "-r",
        "a",
        "-d",
        "b|c",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `b|c` resolved to more than one revision
    Hint: The revset `b|c` resolved to these revisions:
      royxmykx c12952d9 c | c
      zsuskuln d18ca3e8 b | b
    [EOF]
    [exit status: 1]
    ");

    // try with 'all:' and succeed
    let output = work_dir.run_jj([
        "rebase",
        "--config=ui.always-allow-large-revsets=false",
        "-r",
        "a",
        "-d",
        "all:b|c",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: In revset expression
     --> 1:1
      |
    1 | all:b|c
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    Rebased 1 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    a: c b
    ├─╮
    │ ○  b
    @ │  c
    ├─╯
    ◆
    [EOF]
    ");

    // undo and do it again, but without 'ui.always-allow-large-revsets=false'
    work_dir.run_jj(["op", "restore", &setup_opid2]).success();
    work_dir.run_jj(["rebase", "-r=a", "-d=b|c"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    a: c b
    ├─╮
    │ ○  b
    @ │  c
    ├─╯
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b|c", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "b", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-s", "b", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: vruxwmqv 7a9837e3 d | d
    Parent revision (@-)    : royxmykx ee1edcc0 c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Working copy  (@) now at: vruxwmqv e7720369 d | d
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: vruxwmqv 7186427a d | d
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: vruxwmqv f6c6224e d | d
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-b", "b-one", "-d", "this"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `this` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["rebase", "-b", "this", "-d", "b-one"]);
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-s", "base", "-d", "notroot"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-s", "a", "-d", "base"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 3 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-s", "a", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Working copy  (@) now at: znkkpsqq b65f55fb c | c
    Parent revision (@-)    : vruxwmqv c90d30a4 b | b
    [EOF]
    ");
    // Commit "a" should be rebased onto the root commit. Commit "b" should have
    // "base" and "a" as parents as before.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-b", "c", "-d", "base"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 3 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-b", "c", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Working copy  (@) now at: znkkpsqq 5b285fee c | c
    Parent revision (@-)    : vruxwmqv 988d520d b | b
    [EOF]
    ");
    // The commits in roots(a..c), i.e. commit "b" should be rebased onto "a",
    // which means "b" loses its "base" parent
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-b", "a", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 5 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-r", "base", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: znkkpsqq ad772a82 c | c
    Parent revision (@-)    : vruxwmqv 79030f30 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // The user would expect unsimplified ancestry here.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "base", "-d", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: znkkpsqq 3cdfeb24 c | c
    Parent revision (@-)    : vruxwmqv d77e32e7 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "base", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: znkkpsqq afc19fe0 c | c
    Parent revision (@-)    : vruxwmqv c319447d b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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

    let output = work_dir.run_jj(["rebase", "-r", "a", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: znkkpsqq 1cb17633 c | c
    Parent revision (@-)    : vruxwmqv 801b1b8c b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // In this case, it is unclear whether the user would always prefer unsimplified
    // ancestry (whether `b` should also be a direct child of the root commit).
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "b", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq f8853d0d c | c
    Parent revision (@-)    : zsuskuln 3a2d0837 base | base
    Parent revision (@-)    : royxmykx c7aebf99 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // The user would expect unsimplified ancestry here.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "b", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: znkkpsqq 6feb5c0f c | c
    Parent revision (@-)    : zsuskuln 3a2d0837 base | base
    Parent revision (@-)    : royxmykx c7aebf99 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    let output = work_dir.run_jj(["rebase", "-r", "c", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Working copy  (@) now at: znkkpsqq a0c5ea8f c | c
    Parent revision (@-)    : royxmykx c7aebf99 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn 2f59b944 f | f
    Parent revision (@-)    : kmkuslsw 88ddc78c c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn da215ecd f | f
    Parent revision (@-)    : kmkuslsw ed86d82a c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: xznxytkn 2c606e19 f | f
    Parent revision (@-)    : zsuskuln 62634b59 b1 | b1
    Added 0 files, modified 0 files, removed 5 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn e60a4b0c f | f
    Parent revision (@-)    : royxmykx 40646d19 b2 | b2
    Added 0 files, modified 0 files, removed 4 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn 84786555 f | f
    Parent revision (@-)    : kmkuslsw ed86d82a c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Working copy  (@) now at: xznxytkn d5324bb4 f | f
    Parent revision (@-)    : nkmrtpmo 50d9bd5d e | e
    Parent revision (@-)    : lylxulpl 610f541b d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: xznxytkn 17632090 f | f
    Parent revision (@-)    : kmkuslsw 726c937e c | c
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn 991bd3ac f | f
    Parent revision (@-)    : nkmrtpmo f9ecf426 e | e
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn 0123019e f | f
    Parent revision (@-)    : nkmrtpmo 903f8aa0 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: xznxytkn fdeb070b f | f
    Parent revision (@-)    : nkmrtpmo f1b259b2 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn c8ba9324 f | f
    Parent revision (@-)    : kmkuslsw 0bd3b3d1 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn 9ea28699 f | f
    Parent revision (@-)    : nkmrtpmo 989281ba e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 6 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn 63860138 f | f
    Parent revision (@-)    : nkmrtpmo cc7e9907 e | e
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 40646d195680 would be both an ancestor and a descendant of the rebased commits
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root revision 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Rebase a commit before another commit. "c" has parents "b2" and "b4", so its
    // children "d" and "e" should be rebased onto "b2" and "b4" respectively.
    let output = work_dir.run_jj(["rebase", "-r", "c", "--before", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 8 descendant commits
    Working copy  (@) now at: xznxytkn ff62b7d5 f | f
    Parent revision (@-)    : nkmrtpmo b007a305 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: xznxytkn d8eb20c6 f | f
    Parent revision (@-)    : kmkuslsw ed86d82a c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: xznxytkn 5fd4cd2f f | f
    Parent revision (@-)    : zsuskuln 62634b59 b1 | b1
    Added 0 files, modified 0 files, removed 5 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: xznxytkn 4bb8afb7 f | f
    Parent revision (@-)    : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 6 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn b1d5040b f | f
    Parent revision (@-)    : royxmykx 40646d19 b2 | b2
    Parent revision (@-)    : znkkpsqq 256ac307 b4 | b4
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: xznxytkn 9ce35fe1 f | f
    Parent revision (@-)    : nkmrtpmo 95558239 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 5 descendant commits
    Working copy  (@) now at: xznxytkn b984057d f | f
    Parent revision (@-)    : zsuskuln 62634b59 b1 | b1
    Parent revision (@-)    : vruxwmqv a1d9eeb3 b3 | b3
    Added 0 files, modified 0 files, removed 4 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 7 descendant commits
    Working copy  (@) now at: xznxytkn 13214a9a f | f
    Parent revision (@-)    : nkmrtpmo 257f541d e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: xznxytkn 903781e1 f | f
    Parent revision (@-)    : nkmrtpmo 2a0542d6 e | e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Rebased 7 descendant commits
    Working copy  (@) now at: xznxytkn 9d5abf25 f | f
    Parent revision (@-)    : kmkuslsw ecde78f5 c | c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: xznxytkn 912af1a2 f | f
    Parent revision (@-)    : nkmrtpmo 4e8eabcc e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 6 descendant commits
    Working copy  (@) now at: xznxytkn a2ed33ff f | f
    Parent revision (@-)    : nkmrtpmo b69d0e4b e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn 5fe09799 f | f
    Parent revision (@-)    : nkmrtpmo ee33f6f1 e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 2 commits that were already in place
    Rebased 4 commits to destination
    Rebased 2 descendant commits
    Working copy  (@) now at: xznxytkn d1c73b11 f | f
    Parent revision (@-)    : nkmrtpmo 53fc68f7 e | e
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 40646d195680 would be both an ancestor and a descendant of the rebased commits
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nmzmmopx 321e1d40 f | f
    Parent revision (@-)    : nkmrtpmo 7d7b0aec d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nmzmmopx a12b0ae1 f | f
    Parent revision (@-)    : xznxytkn d4334f29 e | e
    Parent revision (@-)    : nkmrtpmo a3204f2a d | d
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Rebased 3 descendant commits
    Working copy  (@) now at: nmzmmopx 97ebbe11 f | f
    Parent revision (@-)    : xznxytkn e53a8360 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nmzmmopx 5a27aa46 f | f
    Parent revision (@-)    : znkkpsqq 0780cdfa b1 | b1
    Parent revision (@-)    : kmkuslsw 0692c8ed b2 | b2
    Parent revision (@-)    : nkmrtpmo 07028fe4 d | d
    Parent revision (@-)    : xznxytkn 1aa724f0 e | e
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 commits to destination
    Rebased 1 descendant commits
    Working copy  (@) now at: nmzmmopx 5b68ace7 f | f
    Parent revision (@-)    : xznxytkn 8bfa4c49 e | e
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 commits to destination
    Rebased 4 descendant commits
    Working copy  (@) now at: nmzmmopx 68713d9d f | f
    Parent revision (@-)    : xznxytkn d4bcbcd4 e | e
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 0c9da0df7f7c would be both an ancestor and a descendant of the rebased commits
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
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 commits to destination
    Abandoned 1 newly emptied commits
    Working copy  (@) now at: yostqsxw 6b46781e (empty) also already empty
    Parent revision (@-)    : vruxwmqv 4861a0a8 (empty) already empty
    [EOF]
    ");

    // The parent commit became empty and was dropped, but the already empty commits
    // were kept
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
    @  also already empty
    ○  already empty
    ○  b
    ○  a
    ◆
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Test the setup
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
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
        "-r=description('will become empty')",
        "-d=b",
        "--skip-emptied",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Abandoned 1 newly emptied commits
    Working copy  (@) now at: yostqsxw bbfc2a27 (empty) also already empty
    Parent revision (@-)    : vruxwmqv 1b8c46b3 (empty) already empty
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // Rebasing a single commit which becomes empty abandons that commit, whilst its
    // already empty descendants were kept
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
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
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 3 descendant commits
    Working copy  (@) now at: znkkpsqq 6d024ab4 (empty) also already empty
    Parent revision (@-)    : yostqsxw bb87e185 (empty) already empty
    [EOF]
    ");

    // Commits not in the rebase target set should not be abandoned even if they
    // were emptied.
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "description"]), @r"
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
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  f  lylxulpl  cf8edc20:  e
    ○  e  kmkuslsw  65f1083b:  c
    │ ○  d  znkkpsqq  f91a8202:  c
    ├─╯
    ○    c  vruxwmqv  86997ac2:  b1 b2
    ├─╮
    │ ○  b2  royxmykx  1d9f22d8:  a
    ○ │  b1  zsuskuln  62634b59:  a
    ├─╯
    ○  a  rlvkpnrz  7d980be7
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -b
    let output = work_dir.run_jj(["rebase", "-b", "d", "-d", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 6 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  f  lylxulpl  cf8edc20:  e
    ○  e  kmkuslsw  65f1083b:  c
    │ ○  d  znkkpsqq  f91a8202:  c
    ├─╯
    ○    c  vruxwmqv  86997ac2:  b1 b2
    ├─╮
    │ ○  b2  royxmykx  1d9f22d8:  a
    ○ │  b1  zsuskuln  62634b59:  a
    ├─╯
    ○  a  rlvkpnrz  7d980be7
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -s
    let output = work_dir.run_jj(["rebase", "-s", "c", "-d", "b1", "-d", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 4 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  f  lylxulpl  cf8edc20:  e
    ○  e  kmkuslsw  65f1083b:  c
    │ ○  d  znkkpsqq  f91a8202:  c
    ├─╯
    ○    c  vruxwmqv  86997ac2:  b1 b2
    ├─╮
    │ ○  b2  royxmykx  1d9f22d8:  a
    ○ │  b1  zsuskuln  62634b59:  a
    ├─╯
    ○  a  rlvkpnrz  7d980be7
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase with -r since commit has no children
    let output = work_dir.run_jj(["rebase", "-r", "d", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  f  lylxulpl  cf8edc20:  e
    ○  e  kmkuslsw  65f1083b:  c
    │ ○  d  znkkpsqq  f91a8202:  c
    ├─╯
    ○    c  vruxwmqv  86997ac2:  b1 b2
    ├─╮
    │ ○  b2  royxmykx  1d9f22d8:  a
    ○ │  b1  zsuskuln  62634b59:  a
    ├─╯
    ○  a  rlvkpnrz  7d980be7
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Skip rebase of commit, but rebases children onto destination with -r
    let output = work_dir.run_jj(["rebase", "-r", "e", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipped rebase of 1 commits that were already in place
    Rebased 1 descendant commits
    Working copy  (@) now at: lylxulpl f2015644 f | f
    Parent revision (@-)    : vruxwmqv 86997ac2 c | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  f  lylxulpl  f2015644:  c
    │ ○  e  kmkuslsw  65f1083b:  c
    ├─╯
    │ ○  d  znkkpsqq  f91a8202:  c
    ├─╯
    ○    c  vruxwmqv  86997ac2:  b1 b2
    ├─╮
    │ ○  b2  royxmykx  1d9f22d8:  a
    ○ │  b1  zsuskuln  62634b59:  a
    ├─╯
    ○  a  rlvkpnrz  7d980be7
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
    work_dir.run_jj(["rebase", "-r", "b2", "-d", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  ecbe1d2f:  b1
    ○  b1  zsuskuln  48bf33ab:  a
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // By default, rebase should skip the duplicate of commit B
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-r", "c::", "-d", "d"]), @r"
    ------- stderr -------
    Abandoned 1 divergent commits that were already present in the destination:
      zsuskuln?? 3f194323 b2 | b2
    Rebased 1 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    ○  b2 c  royxmykx  56740329:  d
    @  d  znkkpsqq  ecbe1d2f:  b1
    ○  b1  zsuskuln  48bf33ab:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Rebasing should work even if the root of the target set is abandoned
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-s", "b1", "-d", "b2"]), @r"
    ------- stderr -------
    Abandoned 1 divergent commits that were already present in the destination:
      zsuskuln?? 48bf33ab b1 | b2
    Rebased 1 commits to destination
    Working copy  (@) now at: znkkpsqq 81e83d0f d | d
    Parent revision (@-)    : zsuskuln 3f194323 b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    // BUG: "d" should be on top of "b2", but it wasn't rebased
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  81e83d0f:  b1 b2
    ○  b1 b2  zsuskuln  3f194323:  c
    ○  c  royxmykx  0fdb9e5a:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Rebase with "--keep-divergent" shouldn't skip any duplicates
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    insta::assert_snapshot!(work_dir.run_jj(["rebase", "-s", "c", "-d", "d", "--keep-divergent"]), @r"
    ------- stderr -------
    Rebased 2 commits to destination
    [EOF]
    ");
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    ○  b2  zsuskuln  f8e418c5:  c
    ○  c  royxmykx  e232ead1:  d
    @  d  znkkpsqq  ecbe1d2f:  b1
    ○  b1  zsuskuln  48bf33ab:  a
    ○  a  rlvkpnrz  08789390
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
