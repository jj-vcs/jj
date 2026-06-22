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
fn test_new() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "add a file"]).success();
    work_dir.run_jj(["new", "-m", "a new commit"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c7be36d5c1768e7731056dcce3c3ff2503b63b5a a new commit
    ○  55eabcc47301440da7a71d5610d3db021d1925ca add a file
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Start a new change off of a specific commit (the root commit in this case).
    work_dir
        .run_jj(["new", "-m", "off of root", "root()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  ade1c2296c9b3294b783f2ffc6208808a621c321 off of root
    │ ○  c7be36d5c1768e7731056dcce3c3ff2503b63b5a a new commit
    │ ○  55eabcc47301440da7a71d5610d3db021d1925ca add a file
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // --edit is a no-op
    work_dir
        .run_jj(["new", "--edit", "-m", "yet another commit"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  aec5b3a25e62407c315b8b7a2bf676c3fe31ba4d yet another commit
    ○  ade1c2296c9b3294b783f2ffc6208808a621c321 off of root
    │ ○  c7be36d5c1768e7731056dcce3c3ff2503b63b5a a new commit
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d4b1aaaf202e623d8fc50704d8d9593169344d21
    ├─╮
    │ ○  d0687397deaf2dae0df4ac9acc14a7e33308fa3c add file2
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created new commit lpqxywon 8dd19532 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    8dd195327ba0c74f720c5a854cad4e6b7cb6e9ec
    ├─╮
    │ @  d0687397deaf2dae0df4ac9acc14a7e33308fa3c add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Same test with `jj new`
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    1a2683ac02c9ed76b159cbc335ba8908c639fc86
    ├─╮
    │ ○  d0687397deaf2dae0df4ac9acc14a7e33308fa3c add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // merge with non-unique revisions
    let output = work_dir.run_jj(["new", "@", "3a44e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Revision `3a44e` doesn't exist
    [EOF]
    [exit status: 1]
    ");
    // duplicates are allowed
    let output = work_dir.run_jj(["new", "@", "visible_heads()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: wyznsvlq a8125eab (empty) (no description set)
    Parent commit (@-)      : mylxulpl 1a2683ac (empty) (no description set)
    [EOF]
    ");

    // merge with root
    let output = work_dir.run_jj(["new", "@", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_merge_parents_order() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "1", &[]);
    create_commit(&work_dir, "2", &[]);
    create_commit(&work_dir, "3", &[]);
    create_commit(&work_dir, "4", &[]);
    create_commit(&work_dir, "5", &[]);

    // The order of positional and -r/-o args should be preserved
    work_dir
        .run_jj([
            "new",
            "-osubject(2)",
            "subject(3)",
            "-rsubject(1)",
            "-dsubject(5)",
            "subject(4)",
        ])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @          cce3ff23115e7f554a9c77389b48c49b4f7962fb
    ├─┬─┬─┬─╮
    │ │ │ │ ○  a9a58f3dbfe40e8ea0901e381324498b68046bf2 4
    │ │ │ ○ │  abe836bad23ae391a542ee1c0eacaa392aec0249 5
    │ │ │ ├─╯
    │ │ ○ │  9a6abef0dba86ff0abb65f9899584be39a11114b 1
    │ │ ├─╯
    │ ○ │  f7edbc03066334b554f52800af76dd0aefab4b5c 3
    │ ├─╯
    ○ │  60bb72a1e8b37d6dbb279b0d088030c6f9fb9266 2
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: truxwmqv 2d1dc9f3 (conflict) (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 262ef62a 3 | 3
    Parent commit (@-)      : psuskuln 0f19fe92 2 | 2
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r#"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: ylvkpnrz b4ea5b9e "1"
    \\\\\\\        to: ooyxmykx 262ef62a "3"
    -1a
    +3a 1a
    +++++++ psuskuln 0f19fe92 "2"
    1a 2a
    >>>>>>> conflict 1 of 1 ends
    1b
    2c
    "#);

    // reset working copy
    work_dir.run_jj(["new", "root()"]).success();

    // merge word by word
    let output = work_dir.run_jj(["new", "2|3", "--config=merge.hunk-level=word"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: nnkkpsqq 26047b15 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 262ef62a 3 | 3
    Parent commit (@-)      : psuskuln 0f19fe92 2 | 2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: truxwmqv 751bd5bc (empty) (no description set)
    Parent commit (@-)      : ooyxmykx b7c11bbc 3 | 3
    Parent commit (@-)      : psuskuln d6387571 2 | 2
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @"
    a
    b
    ");

    // reset working copy
    work_dir.run_jj(["new", "root()"]).success();

    // keep same-change conflict
    let output = work_dir.run_jj(["new", "2|3", "--config=merge.same-change=keep"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: nnkkpsqq 74eddf2f (conflict) (empty) (no description set)
    Parent commit (@-)      : ooyxmykx b7c11bbc 3 | 3
    Parent commit (@-)      : psuskuln d6387571 2 | 2
    Added 1 files, modified 0 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @r#"
    a
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: ylvkpnrz 2883207c "1"
    \\\\\\\        to: ooyxmykx b7c11bbc "3"
    +b
    +++++++ psuskuln d6387571 "2"
    b
    >>>>>>> conflict 1 of 1 ends
    "#);
}

#[test]
fn test_new_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(r#"templates.new_description = '"custom default\n"'"#);

    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: ylvkpnrz a5fa58b3 (empty) custom default
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-m", "explicit message"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: nkmpptxz 9889dd1a (empty) explicit message
    Parent commit (@-)      : ylvkpnrz a5fa58b3 (empty) custom default
    [EOF]
    ");

    test_env.add_config(r#"templates.new_description = '""'"#);
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: psuskuln 810ff317 (empty) (no description set)
    Parent commit (@-)      : nkmpptxz 9889dd1a (empty) explicit message
    [EOF]
    ");

    // Test that template can access commit properties
    test_env.add_config(r#"templates.new_description = '"parents: " ++ parents.len() ++ "\n"'"#);
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rzvwutvl 6c4cf5e6 (empty) parents: 1
    Parent commit (@-)      : psuskuln 810ff317 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_new_insert_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: vxryzmor 3da64d04 (empty) G
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    Parent commit (@-)      : uruxwmqv 1c0d5121 D | (empty) D
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
    ○  C
    │ ○  F
    ╭─┤
    │ ○  E
    @ │    G
    ├───╮
    │ │ ○  D
    │ ├─╯
    ○ │  B
    ○ │  A
    ├─╯
    ◆  root
    [EOF]
    ");

    // Inserting a new commit should not change the order of its child commits'
    // parents (i.e. G should have the parents H and D).
    let output = work_dir.run_jj(["new", "-m", "H", "--insert-after", "B"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 3 descendant commits
    Working copy  (@) now at: wyznsvlq 4a5952c4 (empty) H
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
    ○  C
    │ ○  F
    ╭─┤
    │ ○  E
    ○ │    G
    ├───╮
    │ │ ○  D
    │ ├─╯
    @ │  H
    ○ │  B
    ○ │  A
    ├─╯
    ◆  root
    [EOF]
    ");

    // --after cannot be used with revisions
    let output = work_dir.run_jj(["new", "--after", "B", "D"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--insert-after <REVSETS>' cannot be used with:
      [REVSETS]...
      -o <REVSETS>

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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 2 descendant commits
    Working copy  (@) now at: vxryzmor 73470f9b (empty) G
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    Parent commit (@-)      : uruxwmqv 1c0d5121 D | (empty) D
    Parent commit (@-)      : pnkkpsqq 3ec50fe1 E | (empty) E
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--insert-before <REVSETS>' cannot be used with:
      [REVSETS]...
      -o <REVSETS>

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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 5 descendant commits
    Working copy  (@) now at: vxryzmor 22be0be4 (empty) G
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    @    6b02b566593b F
    ├─╮
    │ ○  3ec50fe121ee E
    ○ │  1c0d5121740c D
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: vxryzmor e98f75cd (empty) G
    Parent commit (@-)      : mzvwutvl d32ebe56 C | (empty) C
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 descendant commits
    Working copy  (@) now at: wyznsvlq 4b778f26 (empty) H
    Parent commit (@-)      : uruxwmqv 1c0d5121 D | (empty) D
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
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
    insta::assert_snapshot!(output, @"
    @    6b02b566593b F
    ├─╮
    │ ○  3ec50fe121ee E
    ○ │  1c0d5121740c D
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit d32ebe56a293 would be both an ancestor and a descendant of the new commit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_insert_after_empty_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    setup_before_insertion(&work_dir);
    let template = r#"commit_id.short() ++ " " ++ if(description, description, "root")"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @"
    @    6b02b566593b F
    ├─╮
    │ ○  3ec50fe121ee E
    ○ │  1c0d5121740c D
    ├─╯
    │ ○  d32ebe56a293 C
    │ ○  bb98b0102ef5 B
    │ ○  515354d01f1b A
    ├─╯
    ◆  000000000000 root
    [EOF]
    ");

    let output = work_dir.run_jj(["new", "-mG", "--insert-before=none()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No revisions found to use as parent
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["new", "-mG", "--insert-before=none()", "--insert-after=B"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: tkmrtpmo 7e66ad61 (empty) G
    Parent commit (@-)      : kkmpptxz bb98b010 B | (empty) B
    [EOF]
    ");
    insta::assert_snapshot!(get_short_log_output(&work_dir), @"
    @  G
    │ ○  C
    ├─╯
    ○  B
    ○  A
    │ ○    F
    │ ├─╮
    │ │ ○  E
    ├───╯
    │ ○  D
    ├─╯
    ◆  root
    [EOF]
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Name `foo` is conflicted
    Hint: Use commit ID to select single revision from: 6dec1091a14e, 401ea16fc3fe
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Change ID `qpvuntsm` is divergent
    Hint: Use change offset to select single revision: qpvuntsm/0, qpvuntsm/1
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    work_dir.run_jj(["new", "-m", "F", "D", "E"]).success();
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
