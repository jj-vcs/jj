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

    // Create a merge commit
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    fd495246497571ee53aa327ac3d1e7846a1eeefd
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
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["new", "main", "@", "--no-edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit znkkpsqq bffdc06a (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    bffdc06aa66a747b995371bf39a4ac640c9c4386
    ├─╮
    │ @  5bf404a038660799fae348cc31b9891349c128c1 add file2
    ○ │  96ab002e5b86c39a661adc0524df211a3dac3f1b add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Same test with `jj new`
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    e6e472a9b9caff61ab319a8fb8664db62c6e65af
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
    // if prefixed with all:, duplicates are allowed
    let output = work_dir.run_jj(["new", "@", "all:visible_heads()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: nkmrtpmo 24484bf7 (empty) (no description set)
    Parent commit (@-)      : wqnwkozp e6e472a9 (empty) (no description set)
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
        .run_jj([
            "--at-op=@-",
            "bookmark",
            "create",
            "foo",
            "-r",
            r#"description("one")"#,
        ])
        .success();

    // Trigger resolution of divergent operations
    work_dir.run_jj(["st"]).success();

    let output = work_dir.run_jj(["new", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `foo` resolved to more than one revision
    Hint: Bookmark foo resolved to multiple revisions because it's conflicted.
    It resolved to these revisions:
      kkmpptxz 96948328 foo?? | (empty) two
      qpvuntsm 401ea16f foo?? | (empty) one
    Hint: Set which revision the bookmark points to with `jj bookmark set foo -r <REVISION>`.
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
    Error: Revset `qpvuntsm` resolved to more than one revision
    Hint: The revset `qpvuntsm` resolved to these revisions:
      qpvuntsm?? 2f175dfc (empty) two
      qpvuntsm?? 401ea16f (empty) one
    Hint: Some of these commits have the same change id. Abandon the unneeded commits with `jj abandon <commit_id>`.
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
