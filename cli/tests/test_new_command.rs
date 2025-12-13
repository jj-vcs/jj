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
use crate::common::create_commit_with_files;

#[test]
fn test_new() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "add a file"]).success();
    work_dir.run_jj(["new", "-m", "a new commit"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  22aec45f30a36a2d244c70e131e369d79e400962 a new commit
    ○  55eabcc47301440da7a71d5610d3db021d1925ca add a file
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Start a new change off of a specific commit (the root commit in this case).
    work_dir
        .run_jj(["new", "-m", "off of root", "root()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  8818c9ee28d00667cb3072a2114a67619ded7ceb off of root
    │ ○  22aec45f30a36a2d244c70e131e369d79e400962 a new commit
    │ ○  55eabcc47301440da7a71d5610d3db021d1925ca add a file
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // --edit is a no-op
    work_dir
        .run_jj(["new", "--edit", "-m", "yet another commit"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  9629f035563a7d9fa86becc783ae71557bd25269 yet another commit
    ○  8818c9ee28d00667cb3072a2114a67619ded7ceb off of root
    │ ○  22aec45f30a36a2d244c70e131e369d79e400962 a new commit
    │ ○  55eabcc47301440da7a71d5610d3db021d1925ca add a file
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // --edit cannot be used with --no-edit
    let output = work_dir.run_jj(["new", "--edit", "B", "--no-edit", "D"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--edit' cannot be used with '--no-edit'

    Usage: jj new <REVSETS>...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_new_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "add file1"]).success();
    work_dir.write_file("file1", "a");
    work_dir
        .run_jj(["new", "root()", "-m", "add file2"])
        .success();
    work_dir.write_file("file2", "b");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // Create a merge commit
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    94ce38ef81dc7912c1574cc5aa2f434b9057d58a
    ├─╮
    │ ○  5bf404a038660799fae348cc31b9891349c128c1 add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"a[EOF]");
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"b[EOF]");

    // Same test with `--no-edit`
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["new", "main", "@", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit kpqxywon 061f4210 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    061f42107e030034242b424264f22985429552c1
    ├─╮
    │ @  5bf404a038660799fae348cc31b9891349c128c1 add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Same test with `jj new`
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    cee60a55c085ff349af7fa1e7d6b7d4b7bdd4c3a
    ├─╮
    │ ○  5bf404a038660799fae348cc31b9891349c128c1 add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // merge with non-unique revisions
    let output = work_dir.run_jj(["new", "@", "3a44e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `3a44e` doesn't exist
    [EOF]
    [exit status: 1]
    ");
    // duplicates are allowed
    let output = work_dir.run_jj(["new", "@", "visible_heads()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: uyznsvlq 68a7f50c (empty) (no description set)
    Parent commit (@-)      : lylxulpl cee60a55 (empty) (no description set)
    [EOF]
    ");

    // merge with root
    let output = work_dir.run_jj(["new", "@", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_merge_conflicts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "1", &[], &[("file", "1a\n1b\n")]);
    create_commit_with_files(&work_dir, "2", &["1"], &[("file", "1a 2a\n1b\n2c\n")]);
    create_commit_with_files(&work_dir, "3", &["1"], &[("file", "3a 1a\n1b\n")]);

    // merge line by line by default
    let output = work_dir.run_jj(["new", "2|3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 0361ec6a (conflict) (empty) (no description set)
    Parent commit (@-)      : royxmykx 1b282e07 3 | 3
    Parent commit (@-)      : zsuskuln 7ac709e5 2 | 2
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -1a
    +3a 1a
    +++++++ Contents of side #2
    1a 2a
    >>>>>>> Conflict 1 of 1 ends
    1b
    2c
    ");

    // reset working copy
    work_dir.run_jj(["new", "root()"]).success();

    // merge word by word
    let output = work_dir.run_jj(["new", "2|3", "--config=merge.hunk-level=word"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: znkkpsqq 892ac90f (empty) (no description set)
    Parent commit (@-)      : royxmykx 1b282e07 3 | 3
    Parent commit (@-)      : zsuskuln 7ac709e5 2 | 2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r"
    3a 1a 2a
    1b
    2c
    ");
}

#[test]
fn test_new_merge_same_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "1", &[], &[("file", "a\n")]);
    create_commit_with_files(&work_dir, "2", &["1"], &[("file", "a\nb\n")]);
    create_commit_with_files(&work_dir, "3", &["1"], &[("file", "a\nb\n")]);

    // same-change conflict is resolved by default
    let output = work_dir.run_jj(["new", "2|3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 7bebf0fe (empty) (no description set)
    Parent commit (@-)      : royxmykx 1b9fe696 3 | 3
    Parent commit (@-)      : zsuskuln 829e1e90 2 | 2
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r"
    a
    b
    ");

    // reset working copy
    work_dir.run_jj(["new", "root()"]).success();

    // keep same-change conflict
    let output = work_dir.run_jj(["new", "2|3", "--config=merge.same-change=keep"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: znkkpsqq 453a144b (conflict) (empty) (no description set)
    Parent commit (@-)      : royxmykx 1b9fe696 3 | 3
    Parent commit (@-)      : zsuskuln 829e1e90 2 | 2
    Added 1 files, modified 0 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r"
    a
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    +b
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    ");
}

#[test]
fn test_new_insert_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    // --insert-after can be repeated; --after is an alias
    let output = work_dir.run_jj(["new", "-m", "G", "--insert-after", "B", "--after", "D"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kxryzmor 57acfedf (empty) G
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    Parent commit (@-)      : vruxwmqv 521674f5 D | (empty) D
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○  C
    │ ○  F
    ╭─┤
    @ │    G
    ├───╮
    │ │ ○  D
    ○ │ │  B
    ○ │ │  A
    ├───╯
    │ ○  E
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-m", "H", "--insert-after", "D"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits
    Working copy  (@) now at: uyznsvlq fd3f1413 (empty) H
    Parent commit (@-)      : vruxwmqv 521674f5 D | (empty) D
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○  C
    │ ○  F
    ╭─┤
    ○ │    G
    ├───╮
    │ │ @  H
    │ │ ○  D
    ○ │ │  B
    ○ │ │  A
    ├───╯
    │ ○  E
    ├─╯
    ◆  root
    [EOF]
    ");

    // --after cannot be used with revisions
    let output = work_dir.run_jj(["new", "--after", "B", "D"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--insert-after <REVSETS>' cannot be used with '[REVSETS]...'

    Usage: jj new --insert-after <REVSETS> [REVSETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_new_insert_after_children() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    // Attempting to insert G after A and C errors out due to the cycle created
    // as A is an ancestor of C.
    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-after",
        "A",
        "--insert-after",
        "C",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit d32ebe56a293 would be both an ancestor and a descendant of the new commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_insert_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-before",
        "C",
        "--insert-before",
        "F",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: kxryzmor 2f16c40d (empty) G
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    Parent commit (@-)      : vruxwmqv 521674f5 D | (empty) D
    Parent commit (@-)      : znkkpsqq 56a33cd0 E | (empty) E
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○  F
    │ ○  C
    ├─╯
    @      G
    ├─┬─╮
    │ │ ○  E
    │ ○ │  D
    │ ├─╯
    ○ │  B
    ○ │  A
    ├─╯
    ◆  root
    [EOF]
    ");

    // --before cannot be used with revisions
    let output = work_dir.run_jj(["new", "--before", "B", "D"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--insert-before <REVSETS>' cannot be used with '[REVSETS]...'

    Usage: jj new --insert-before <REVSETS> [REVSETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_new_insert_before_root_successors() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-before",
        "A",
        "--insert-before",
        "D",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 5 descendant commits
    Working copy  (@) now at: kxryzmor 8c026b06 (empty) G
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○    F
    ├─╮
    │ ○  E
    ○ │  D
    │ │ ○  C
    │ │ ○  B
    │ │ ○  A
    ├───╯
    @ │  G
    ├─╯
    ◆  root
    [EOF]
    ");
}

#[test]
fn test_new_insert_before_no_loop() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    let template = r#"commit_id.short() ++ " " ++ if(description, description, "root")"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    a8176a8a5348 F
    ├─╮
    │ ○  56a33cd09d90 E
    ○ │  521674f591a6 D
    ├─╯
    │ ○  d32ebe56a293 C
    │ ○  bb98b0102ef5 B
    │ ○  515354d01f1b A
    ├─╯
    ◆  000000000000 root
    [EOF]
    ");

    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-before",
        "A",
        "--insert-before",
        "C",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit bb98b0102ef5 would be both an ancestor and a descendant of the new commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_insert_before_no_root_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-before",
        "B",
        "--insert-before",
        "D",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_insert_before_root() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-m", "G", "--insert-before", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_insert_after_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    @    F
    ├─╮
    │ ○  E
    ○ │  D
    ├─╯
    │ ○  C
    │ ○  B
    │ ○  A
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-m", "G", "--after", "C", "--before", "F"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kxryzmor 55a63f47 (empty) G
    Parent commit (@-)      : mzvwutvl d32ebe56 C | (empty) C
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○      F
    ├─┬─╮
    │ │ @  G
    │ │ ○  C
    │ │ ○  B
    │ │ ○  A
    │ ○ │  E
    │ ├─╯
    ○ │  D
    ├─╯
    ◆  root
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-m", "H", "--after", "D", "--before", "B"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 descendant commits
    Working copy  (@) now at: uyznsvlq fd3f1413 (empty) H
    Parent commit (@-)      : vruxwmqv 521674f5 D | (empty) D
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @r"
    ○      F
    ├─┬─╮
    │ │ ○  G
    │ │ ○  C
    │ │ ○    B
    │ │ ├─╮
    │ │ │ @  H
    ├─────╯
    ○ │ │  D
    │ │ ○  A
    ├───╯
    │ ○  E
    ├─╯
    ◆  root
    [EOF]
    ");
}

#[test]
fn test_new_insert_after_before_no_loop() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    let template = r#"commit_id.short() ++ " " ++ if(description, description, "root")"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    a8176a8a5348 F
    ├─╮
    │ ○  56a33cd09d90 E
    ○ │  521674f591a6 D
    ├─╯
    │ ○  d32ebe56a293 C
    │ ○  bb98b0102ef5 B
    │ ○  515354d01f1b A
    ├─╯
    ◆  000000000000 root
    [EOF]
    ");

    let output = work_dir.run_jj([
        "new",
        "-m",
        "G",
        "--insert-before",
        "A",
        "--insert-after",
        "C",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit d32ebe56a293 would be both an ancestor and a descendant of the new commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_conflicting_bookmarks() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "one"]).success();
    work_dir.run_jj(["new", "-m", "two", "@-"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    work_dir
        .run_jj(["--at-op=@-", "bookmark", "create", "foo", "-rsubject(one)"])
        .success();

    // Trigger resolution of divergent operations
    work_dir.run_jj(["st"]).success();

    let output = work_dir.run_jj(["new", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Name `foo` is conflicted
    Hint: Use commit ID to select single revision from: 96948328bc42, 401ea16fc3fe
    Hint: Use `bookmarks(foo)` to select all revisions
    Hint: To set which revision the bookmark points to, run `jj bookmark set foo -r <REVISION>`
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_conflicting_change_ids() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "one"]).success();
    work_dir
        .run_jj(["--at-op=@-", "describe", "-m", "two"])
        .success();

    // Trigger resolution of divergent operations
    work_dir.run_jj(["st"]).success();

    let output = work_dir.run_jj(["new", "qpvuntsm"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Change ID `qpvuntsm` is divergent
    Hint: Use commit ID to select single revision from: 401ea16fc3fe, 2f175dfc5e0e
    Hint: Use `change_id(qpvuntsm)` to select all revisions
    Hint: To abandon unneeded revisions, run `jj abandon <commit_id>`
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_error_revision_does_not_exist() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "one"]).success();
    work_dir.run_jj(["new", "-m", "two"]).success();

    let output = work_dir.run_jj(["new", "this"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `this` doesn't exist
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_with_trailers() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "one"]).success();

    test_env.add_config(
        r#"[templates]
        commit_trailers = '"Signed-off-by: " ++ committer.email()'
        "#,
    );
    work_dir.run_jj(["new", "-m", "two"]).success();

    let output = work_dir.run_jj(["log", "--no-graph", "-r@", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    two

    Signed-off-by: test.user@example.com
    [EOF]
    ");

    // new without message has no trailer
    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj(["log", "--no-graph", "-r@", "-Tdescription"]);
    insta::assert_snapshot!(output, @"");
}

fn setup_before_insertion(work_dir: &TestWorkDir) {
    work_dir
        .run_jj(["bookmark", "create", "-r@", "A"])
        .success();
    work_dir.run_jj(["commit", "-m", "A"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "B"])
        .success();
    work_dir.run_jj(["commit", "-m", "B"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "C"])
        .success();
    work_dir.run_jj(["describe", "-m", "C"]).success();
    work_dir.run_jj(["new", "-m", "D", "root()"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "D"])
        .success();
    work_dir.run_jj(["new", "-m", "E", "root()"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "E"])
        .success();
    // Any number of -r's is ignored
    work_dir
        .run_jj(["new", "-m", "F", "-r", "D", "-r", "E"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "F"])
        .success();
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_short_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"if(description, description, "root")"#;
    work_dir.run_jj(["log", "-T", template])
}
