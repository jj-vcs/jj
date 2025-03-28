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
    @  34f3c770f1db22ac5c58df21d587aed1a030201f a new commit
    ○  bf8753cb48b860b68386c5c8cc997e8e37122485 add a file
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Start a new change off of a specific commit (the root commit in this case).
    work_dir
        .run_jj(["new", "-m", "off of root", "root()"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  026537ddb96b801b9cb909985d5443aab44616c1 off of root
    │ ○  34f3c770f1db22ac5c58df21d587aed1a030201f a new commit
    │ ○  bf8753cb48b860b68386c5c8cc997e8e37122485 add a file
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // --edit is a no-op
    work_dir
        .run_jj(["new", "--edit", "-m", "yet another commit"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  101cbec5cae8049cb9850a906ef3675631ed48fa yet another commit
    ○  026537ddb96b801b9cb909985d5443aab44616c1 off of root
    │ ○  34f3c770f1db22ac5c58df21d587aed1a030201f a new commit
    │ ○  bf8753cb48b860b68386c5c8cc997e8e37122485 add a file
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
    @    2f9a61ea1fef257eca52fcee2feec1cbd2e41660
    ├─╮
    │ ○  f399209d9dda06e8a25a0c8e9a0cde9f421ff35d add file2
    ○ │  8d996e001c23e298d0d353ab455665c81bf2080c add file1
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
    Created new commit znkkpsqq 496490a6 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    496490a66cebb31730c4103b7b22a1098d49af91
    ├─╮
    │ @  f399209d9dda06e8a25a0c8e9a0cde9f421ff35d add file2
    ○ │  8d996e001c23e298d0d353ab455665c81bf2080c add file1
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Same test with `jj new`
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new", "main", "@"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    114023233c454e2eca22b8b209f9e42f755eb28c
    ├─╮
    │ ○  f399209d9dda06e8a25a0c8e9a0cde9f421ff35d add file2
    ○ │  8d996e001c23e298d0d353ab455665c81bf2080c add file1
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
    Working copy now at: nkmrtpmo ed2dc1d9 (empty) (no description set)
    Parent commit      : wqnwkozp 11402323 (empty) (no description set)
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
    Working copy now at: kxryzmor 1fc93fd1 (empty) G
    Parent commit      : kkmpptxz bfd4157e B | (empty) B
    Parent commit      : vruxwmqv c9257eff D | (empty) D
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
    Working copy now at: uyznsvlq fcf8281b (empty) H
    Parent commit      : vruxwmqv c9257eff D | (empty) D
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
    Error: Refusing to create a loop: commit 83376b270925 would be both an ancestor and a descendant of the new commit
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
    Working copy now at: kxryzmor 7ed2d6ff (empty) G
    Parent commit      : kkmpptxz bfd4157e B | (empty) B
    Parent commit      : vruxwmqv c9257eff D | (empty) D
    Parent commit      : znkkpsqq 41a89ffc E | (empty) E
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
    Working copy now at: kxryzmor 36541977 (empty) G
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
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
    @    7705d353bf5d F
    ├─╮
    │ ○  41a89ffcbba2 E
    ○ │  c9257eff5bf9 D
    ├─╯
    │ ○  83376b270925 C
    │ ○  bfd4157e6ea4 B
    │ ○  5ef24e4bf2be A
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
    Error: Refusing to create a loop: commit bfd4157e6ea4 would be both an ancestor and a descendant of the new commit
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
    Working copy now at: kxryzmor 78a97058 (empty) G
    Parent commit      : mzvwutvl 83376b27 C | (empty) C
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
    Working copy now at: uyznsvlq fcf8281b (empty) H
    Parent commit      : vruxwmqv c9257eff D | (empty) D
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
    @    7705d353bf5d F
    ├─╮
    │ ○  41a89ffcbba2 E
    ○ │  c9257eff5bf9 D
    ├─╯
    │ ○  83376b270925 C
    │ ○  bfd4157e6ea4 B
    │ ○  5ef24e4bf2be A
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
    Error: Refusing to create a loop: commit 83376b270925 would be both an ancestor and a descendant of the new commit
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
      kkmpptxz 66c6502d foo?? | (empty) two
      qpvuntsm 876f4b7e foo?? | (empty) one
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
      qpvuntsm?? 66c6502d (empty) two
      qpvuntsm?? 876f4b7e (empty) one
    Hint: Some of these commits have the same change id. Abandon one of them with `jj abandon -r <REVISION>`.
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
