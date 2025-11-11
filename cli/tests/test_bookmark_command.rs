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

use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

fn create_commit_with_refs(
    repo: &gix::Repository,
    message: &str,
    content: &[u8],
    ref_names: &[&str],
) {
    let git::CommitResult {
        tree_id: _,
        commit_id,
    } = git::add_commit(repo, "refs/heads/dummy", "file", content, message, &[]);
    repo.find_reference("dummy").unwrap().delete().unwrap();

    for name in ref_names {
        repo.reference(
            *name,
            commit_id,
            gix::refs::transaction::PreviousValue::Any,
            "log message",
        )
        .unwrap();
    }
}

#[test]
fn test_bookmark_multiple_names() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["bookmark", "create", "-r@", "foo", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 2 bookmarks pointing to qpvuntsm e8849ae1 bar foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar foo e8849ae12c70
    ◆   000000000000
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["bookmark", "set", "foo", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 2 bookmarks to zsuskuln 0e555a27 bar foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar foo 0e555a27ac99
    ○   e8849ae12c70
    ◆   000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "delete", "foo", "bar", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   0e555a27ac99
    ○   e8849ae12c70
    ◆   000000000000
    [EOF]
    ");

    // Hint should be omitted if -r is specified
    let output = work_dir.run_jj(["bookmark", "create", "-r@-", "foo", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 2 bookmarks pointing to qpvuntsm e8849ae1 bar foo | (empty) (no description set)
    [EOF]
    ");

    // Create and move with explicit -r
    let output = work_dir.run_jj(["bookmark", "set", "-r@", "bar", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to zsuskuln 0e555a27 bar baz | (empty) (no description set)
    Moved 1 bookmarks to zsuskuln 0e555a27 bar baz | (empty) (no description set)
    [EOF]
    ");

    // Noop changes should not be included in the stats
    let output = work_dir.run_jj(["bookmark", "set", "-r@", "foo", "bar", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to zsuskuln 0e555a27 bar baz foo | (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_at_root() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["bookmark", "create", "fred", "-r=root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to zzzzzzzz 00000000 fred | (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    Warning: Failed to export some bookmarks:
      fred@git: Ref cannot point to the root commit in Git
    [EOF]
    ");
}

#[test]
fn test_bookmark_bad_name() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["bookmark", "create", ""]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '' for '<NAMES>...': Failed to parse bookmark name: Syntax error

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

    let output = work_dir.run_jj(["bookmark", "set", "''"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '''' for '<NAMES>...': Failed to parse bookmark name: Expected non-empty string

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

    let output = work_dir.run_jj(["bookmark", "rename", "x", ""]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '' for '<NEW>': Failed to parse bookmark name: Syntax error

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

    // common errors
    let output = work_dir.run_jj(["bookmark", "set", "@-", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '@-' for '<NAMES>...': Failed to parse bookmark name: Syntax error

    For more information, try '--help'.
    Caused by:  --> 1:1
      |
    1 | @-
      | ^---
      |
      = expected <identifier>, <string_literal>, or <raw_string_literal>
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let stderr = work_dir.run_jj(["bookmark", "set", "-r@-", "foo@bar"]);
    insta::assert_snapshot!(stderr, @r"
    ------- stderr -------
    error: invalid value 'foo@bar' for '<NAMES>...': Failed to parse bookmark name: Syntax error

    For more information, try '--help'.
    Caused by:  --> 1:4
      |
    1 | foo@bar
      |    ^---
      |
      = expected <EOI>
    Hint: Looks like remote bookmark. Run `jj bookmark track foo@bar` to track it.
    [EOF]
    [exit status: 2]
    ");

    // quoted name works
    let output = work_dir.run_jj(["bookmark", "create", "'foo@bar'"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to qpvuntsm e8849ae1 "foo@bar" | (empty) (no description set)
    [EOF]
    "#);
}

#[test]
fn test_bookmark_move() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    let output = work_dir.run_jj(["bookmark", "move", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching bookmarks for names: foo
    No bookmarks to update.
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "set", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to qpvuntsm e8849ae1 foo | (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["bookmark", "create", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: foo
    Hint: Use `jj bookmark set` to update it.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["bookmark", "set", "foo", "--revision", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to mzvwutvl 8afc18ff foo | (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "set", "-r@-", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["bookmark", "set", "-r@-", "--allow-backwards", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to qpvuntsm e8849ae1 foo | (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "move", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to mzvwutvl 8afc18ff foo | (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "move", "--to=@-", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["bookmark", "move", "--to=@-", "--allow-backwards", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to qpvuntsm e8849ae1 foo | (empty) (no description set)
    [EOF]
    ");

    // Delete bookmark locally, but is still tracking remote
    work_dir.run_jj(["describe", "@-", "-mcommit"]).success();
    work_dir
        .run_jj(["git", "push", "--allow-new", "-r@-"])
        .success();
    work_dir.run_jj(["bookmark", "delete", "foo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    foo (deleted)
      @origin: qpvuntsm 5f3ceb1e (empty) commit
    [EOF]
    ");

    // Deleted tracking bookmark name should still be allocated
    let output = work_dir.run_jj(["bookmark", "create", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Tracked remote bookmarks exist for deleted bookmark: foo
    Hint: Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack 'glob:foo@*'` to disassociate them.
    [EOF]
    [exit status: 1]
    ");

    // Restoring local target shouldn't invalidate tracking state
    let output = work_dir.run_jj(["bookmark", "set", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to mzvwutvl 91b59745 foo* | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    foo: mzvwutvl 91b59745 (empty) (no description set)
      @origin (behind by 1 commits): qpvuntsm 5f3ceb1e (empty) commit
    [EOF]
    ");

    // Untracked remote bookmark shouldn't block creation of local bookmark
    work_dir
        .run_jj(["bookmark", "untrack", "foo@origin"])
        .success();
    work_dir.run_jj(["bookmark", "delete", "foo"]).success();
    let output = work_dir.run_jj(["bookmark", "create", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to mzvwutvl 91b59745 foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    foo: mzvwutvl 91b59745 (empty) (no description set)
    foo@origin: qpvuntsm 5f3ceb1e (empty) commit
    [EOF]
    ");
}

#[test]
fn test_bookmark_move_matching() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "a1", "a2"])
        .success();
    work_dir.run_jj(["new", "-mhead1"]).success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.run_jj(["bookmark", "create", "b1"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["bookmark", "create", "c1"]).success();
    work_dir.run_jj(["new", "-mhead2"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   0dd9a4b12283
    ○  c1 2cbf65662e56
    ○  b1 c2934cfbfb19
    │ ○   9328ecc52471
    │ ○  a1 a2 e8849ae12c70
    ├─╯
    ◆   000000000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // The default could be considered "--from=all() glob:*", but is disabled
    let output = work_dir.run_jj(["bookmark", "move"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <NAMES|--from <REVSETS>>

    Usage: jj bookmark move <NAMES|--from <REVSETS>>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // No bookmarks pointing to the source revisions
    let output = work_dir.run_jj(["bookmark", "move", "--from=none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to update.
    [EOF]
    ");

    // No matching bookmarks within the source revisions
    let output = work_dir.run_jj(["bookmark", "move", "--from=::@", "glob:a?"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to update.
    [EOF]
    ");

    // Noop move
    let output = work_dir.run_jj(["bookmark", "move", "--to=a1", "a2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to update.
    [EOF]
    ");

    // Move from multiple revisions
    let output = work_dir.run_jj(["bookmark", "move", "--from=::@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 2 bookmarks to vruxwmqv 0dd9a4b1 b1 c1 | (empty) head2
    Hint: Specify bookmark by name to update just one of the bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b1 c1 0dd9a4b12283
    ○   2cbf65662e56
    ○   c2934cfbfb19
    │ ○   9328ecc52471
    │ ○  a1 a2 e8849ae12c70
    ├─╯
    ◆   000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Move multiple bookmarks by name
    let output = work_dir.run_jj(["bookmark", "move", "b1", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 2 bookmarks to vruxwmqv 0dd9a4b1 b1 c1 | (empty) head2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b1 c1 0dd9a4b12283
    ○   2cbf65662e56
    ○   c2934cfbfb19
    │ ○   9328ecc52471
    │ ○  a1 a2 e8849ae12c70
    ├─╯
    ◆   000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Try to move multiple bookmarks, but one of them isn't fast-forward
    let output = work_dir.run_jj(["bookmark", "move", "glob:?1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: a1
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   0dd9a4b12283
    ○  c1 2cbf65662e56
    ○  b1 c2934cfbfb19
    │ ○   9328ecc52471
    │ ○  a1 a2 e8849ae12c70
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Select by revision and name
    let output = work_dir.run_jj(["bookmark", "move", "--from=::a1+", "--to=a1+", "glob:?1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kkmpptxz 9328ecc5 a1 | (empty) head1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   0dd9a4b12283
    ○  c1 2cbf65662e56
    ○  b1 c2934cfbfb19
    │ ○  a1 9328ecc52471
    │ ○  a2 e8849ae12c70
    ├─╯
    ◆   000000000000
    [EOF]
    ");
}

#[test]
fn test_bookmark_move_conflicting() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let get_log = || {
        let template = r#"separate(" ", description.first_line(), bookmarks)"#;
        work_dir.run_jj(["log", "-T", template])
    };

    work_dir.run_jj(["new", "root()", "-mA0"]).success();
    work_dir.run_jj(["new", "root()", "-mB0"]).success();
    work_dir.run_jj(["new", "root()", "-mC0"]).success();
    work_dir
        .run_jj(["new", "subject(glob:A0)", "-mA1"])
        .success();

    // Set up conflicting bookmark.
    work_dir
        .run_jj(["bookmark", "create", "-rsubject(glob:A0)", "foo"])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "--at-op=@-",
            "-rsubject(glob:B0)",
            "foo",
        ])
        .success();
    insta::assert_snapshot!(get_log(), @r"
    @  A1
    ○  A0 foo??
    │ ○  C0
    ├─╯
    │ ○  B0 foo??
    ├─╯
    ◆
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    // Can't move the bookmark to C0 since it's sibling.
    let output = work_dir.run_jj(["bookmark", "set", "-rsubject(glob:C0)", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    // Can move the bookmark to A1 since it's descendant of A0. It's not
    // descendant of B0, though.
    let output = work_dir.run_jj(["bookmark", "set", "-rsubject(glob:A1)", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to mzvwutvl 0f5f3e2c foo | (empty) A1
    [EOF]
    ");
    insta::assert_snapshot!(get_log(), @r"
    @  A1 foo
    ○  A0
    │ ○  C0
    ├─╯
    │ ○  B0
    ├─╯
    ◆
    [EOF]
    ");
}

#[test]
fn test_bookmark_rename() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    let output = work_dir.run_jj(["bookmark", "rename", "bnoexist", "blocal"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: bnoexist
    [EOF]
    [exit status: 1]
    ");

    work_dir.run_jj(["describe", "-m=commit-0"]).success();
    work_dir.run_jj(["bookmark", "create", "blocal"]).success();
    let output = work_dir.run_jj(["bookmark", "rename", "blocal", "blocal1"]);
    insta::assert_snapshot!(output, @"");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["describe", "-m=commit-1"]).success();
    work_dir.run_jj(["bookmark", "create", "bexist"]).success();
    let output = work_dir.run_jj(["bookmark", "rename", "blocal1", "bexist"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: bexist
    [EOF]
    [exit status: 1]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["describe", "-m=commit-2"]).success();
    work_dir
        .run_jj(["bookmark", "create", "bremote", "buntracked"])
        .success();
    work_dir
        .run_jj(["git", "push", "--allow-new", "-b=bremote", "-b=buntracked"])
        .success();

    let output = work_dir.run_jj(["bookmark", "rename", "bremote", "bremote2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Tracked remote bookmarks for bookmark bremote were not renamed.
    Hint: To rename the bookmark on the remote, you can `jj git push --bookmark bremote` first (to delete it on the remote), and then `jj git push --bookmark bremote2`. `jj git push --all --deleted` would also be sufficient.
    [EOF]
    ");
    let op_id_after_rename = work_dir.current_operation_id();
    let output = work_dir.run_jj(["bookmark", "rename", "bremote2", "bremote"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Tracked remote bookmarks for bookmark bremote exist.
    Hint: Run `jj bookmark untrack 'glob:bremote@*'` to disassociate them.
    [EOF]
    ");
    work_dir
        .run_jj(["op", "restore", &op_id_after_rename])
        .success();
    let output = work_dir.run_jj(["git", "push", "--bookmark", "bremote2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Add bookmark bremote2 to a9d7418c1c3f
    [EOF]
    ");
    work_dir
        .run_jj(["git", "push", "--named", "bremote-untracked=@"])
        .success();
    work_dir
        .run_jj(["bookmark", "forget", "bremote-untracked"])
        .success();
    let output = work_dir.run_jj(["bookmark", "rename", "bremote2", "bremote-untracked"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: The renamed bookmark already exists on the remote 'origin', tracking state was dropped.
    Hint: To track the existing remote bookmark, run `jj bookmark track bremote-untracked@origin`
    Warning: Tracked remote bookmarks for bookmark bremote2 were not renamed.
    Hint: To rename the bookmark on the remote, you can `jj git push --bookmark bremote2` first (to delete it on the remote), and then `jj git push --bookmark bremote-untracked`. `jj git push --all --deleted` would also be sufficient.
    [EOF]
    ");

    // rename an untracked bookmark
    work_dir
        .run_jj(["bookmark", "untrack", "buntracked@origin"])
        .success();
    let output = work_dir.run_jj(["bookmark", "rename", "buntracked", "buntracked2"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_bookmark_rename_colocated() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "repo", "--colocate"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m=commit-0"]).success();
    work_dir.run_jj(["bookmark", "create", "blocal"]).success();

    // Make sure that git tracking bookmarks don't cause a warning
    let output = work_dir.run_jj(["bookmark", "rename", "blocal", "blocal1"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_bookmark_forget_glob() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["bookmark", "create", "foo-1"]).success();
    work_dir.run_jj(["bookmark", "create", "bar-2"]).success();
    work_dir.run_jj(["bookmark", "create", "foo-3"]).success();
    work_dir.run_jj(["bookmark", "create", "foo-4"]).success();
    let setup_opid = work_dir.current_operation_id();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 foo-1 foo-3 foo-4 e8849ae12c70
    ◆   000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "forget", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 2 local bookmarks.
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["bookmark", "forget", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 2 local bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 foo-4 e8849ae12c70
    ◆   000000000000
    [EOF]
    ");

    // Forgetting a bookmark via both explicit name and glob pattern, or with
    // multiple glob patterns, shouldn't produce an error.
    let output = work_dir.run_jj(["bookmark", "forget", "foo-4", "glob:foo-*", "glob:foo-*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 e8849ae12c70
    ◆   000000000000
    [EOF]
    ");

    // Malformed glob
    let output = work_dir.run_jj(["bookmark", "forget", "glob:foo-[1-3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'glob:foo-[1-3' for '<NAMES>...': error parsing glob 'foo-[1-3': unclosed character class; missing ']'

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // None of the globs match anything
    let output = work_dir.run_jj(["bookmark", "forget", "glob:baz*", "glob:boom*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to forget.
    [EOF]
    ");
}

#[test]
fn test_bookmark_delete_glob() {
    // Set up a git repo with a bookmark and a jj repo that has it as a remote.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init_bare(git_repo_path);
    let blob_oid = git_repo.write_blob(b"content").unwrap();
    let mut tree_editor = git_repo
        .edit_tree(gix::ObjectId::empty_tree(gix::hash::Kind::default()))
        .unwrap();
    tree_editor
        .upsert("file", gix::object::tree::EntryKind::Blob, blob_oid)
        .unwrap();
    let _tree_id = tree_editor.write().unwrap();
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    work_dir.run_jj(["describe", "-m=commit"]).success();
    work_dir.run_jj(["bookmark", "create", "foo-1"]).success();
    work_dir.run_jj(["bookmark", "create", "bar-2"]).success();
    work_dir.run_jj(["bookmark", "create", "foo-3"]).success();
    work_dir.run_jj(["bookmark", "create", "foo-4"]).success();
    // Push to create remote-tracking bookmarks
    work_dir.run_jj(["git", "push", "--all"]).success();
    // Add absent-tracked bookmark
    work_dir.run_jj(["bookmark", "create", "foo-5"]).success();
    work_dir
        .run_jj(["bookmark", "track", "foo-5@origin"])
        .success();
    let setup_opid = work_dir.current_operation_id();

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 foo-1 foo-3 foo-4 foo-5* 8e056f6b8c37
    ◆   000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 foo-1@origin foo-3@origin foo-4 foo-5* 8e056f6b8c37
    ◆   000000000000
    [EOF]
    ");

    // We get an error if none of the globs match live bookmarks. Unlike `jj
    // bookmark forget`, it's not allowed to delete already deleted bookmarks.
    let output = work_dir.run_jj(["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to delete.
    [EOF]
    ");

    // Deleting a bookmark via both explicit name and glob pattern, or with
    // multiple glob patterns, shouldn't produce an error.
    let output = work_dir.run_jj(["bookmark", "delete", "foo-4", "glob:foo-*", "glob:foo-*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bar-2 foo-1@origin foo-3@origin foo-4@origin 8e056f6b8c37
    ◆   000000000000
    [EOF]
    ");

    // The deleted bookmarks are still there, whereas absent-tracked bookmarks
    // aren't.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bar-2: qpvuntsm 8e056f6b (empty) commit
      @origin: qpvuntsm 8e056f6b (empty) commit
    foo-1 (deleted)
      @origin: qpvuntsm 8e056f6b (empty) commit
    foo-3 (deleted)
      @origin: qpvuntsm 8e056f6b (empty) commit
    foo-4 (deleted)
      @origin: qpvuntsm 8e056f6b (empty) commit
    [EOF]
    ");

    // Malformed glob
    let output = work_dir.run_jj(["bookmark", "delete", "glob:foo-[1-3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'glob:foo-[1-3' for '<NAMES>...': error parsing glob 'foo-[1-3': unclosed character class; missing ']'

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Unknown pattern kind
    let output = work_dir.run_jj(["bookmark", "forget", "whatever:bookmark"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'whatever:bookmark' for '<NAMES>...': Invalid string pattern kind `whatever:`

    For more information, try '--help'.
    Hint: Try prefixing with one of `exact:`, `glob:`, `regex:`, `substring:`, or one of these with `-i` suffix added (e.g. `glob-i:`) for case-insensitive matching
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_bookmark_delete_export() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["bookmark", "create", "foo"]).success();
    work_dir.run_jj(["git", "export"]).success();

    work_dir.run_jj(["bookmark", "delete", "foo"]).success();
    let output = work_dir.run_jj(["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    foo (deleted)
      @git: rlvkpnrz 43444d88 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be deleted from the underlying Git repo on the next `jj git export`.
    [EOF]
    ");

    work_dir.run_jj(["git", "export"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
}

#[test]
fn test_bookmark_forget_export() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["bookmark", "create", "foo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    foo: rlvkpnrz 43444d88 (empty) (no description set)
    [EOF]
    ");

    // Exporting the bookmark to git creates a local-git tracking bookmark
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["bookmark", "forget", "--include-remotes", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");
    // Forgetting a bookmark with --include-remotes deletes local and
    // remote-tracking bookmarks including the corresponding git-tracking bookmark.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    let output = work_dir.run_jj(["log", "-r=foo", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `foo` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    // `jj git export` will delete the bookmark from git. In a colocated
    // workspace, this will happen automatically immediately after a `jj bookmark
    // forget`. This is demonstrated in `test_git_colocated_bookmark_forget` in
    // test_git_colocated.rs
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
}

#[test]
fn test_bookmark_forget_fetched_bookmark() {
    // Much of this test is borrowed from `test_git_fetch_remote_only_bookmark` in
    // test_git_fetch.rs

    // Set up a git repo with a bookmark and a jj repo that has it as a remote.
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();
    // Create a commit and a bookmark in the git repo
    let git::CommitResult {
        tree_id,
        commit_id: first_git_repo_commit,
    } = git::add_commit(
        &git_repo,
        "refs/heads/feature1",
        "file",
        b"content",
        "message",
        &[],
    );

    // Fetch normally
    work_dir
        .run_jj(["git", "fetch", "--remote=origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // TEST 1: with export-import
    // Forget the bookmark with --include-remotes
    work_dir
        .run_jj(["bookmark", "forget", "--include-remotes", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");

    // At this point `jj git export && jj git import` does *not* recreate the
    // bookmark. This behavior is important in colocated workspaces, as otherwise a
    // forgotten bookmark would be immediately resurrected.
    //
    // Technically, this is because `jj bookmark forget` preserved
    // the ref in jj view's `git_refs` tracking the local git repo's remote-tracking
    // bookmark.
    // TODO: Show that jj git push is also a no-op
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");

    // We can fetch feature1 again.
    let output = work_dir.run_jj(["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // TEST 2: No export/import (otherwise the same as test 1)
    work_dir
        .run_jj(["bookmark", "forget", "--include-remotes", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    // Fetch works even without the export-import
    let output = work_dir.run_jj(["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // TEST 3: fetch bookmark that was moved & forgotten with --include-remotes

    // Move the bookmark in the git repo.
    git::write_commit(
        &git_repo,
        "refs/heads/feature1",
        tree_id,
        "another message",
        &[first_git_repo_commit],
    );
    let output = work_dir.run_jj(["bookmark", "forget", "--include-remotes", "feature1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");

    // Fetching a moved bookmark does not create a conflict
    let output = work_dir.run_jj(["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: tyvxnvqr 9175cb32 (empty) another message
      @origin: tyvxnvqr 9175cb32 (empty) another message
    [EOF]
    ");

    // TEST 4: If `--include-remotes` isn't used, remote bookmarks are untracked
    work_dir
        .run_jj(["bookmark", "forget", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1@origin: tyvxnvqr 9175cb32 (empty) another message
    [EOF]
    ");
    // There should be no output here since the remote bookmark wasn't forgotten
    let output = work_dir.run_jj(["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1@origin: tyvxnvqr 9175cb32 (empty) another message
    [EOF]
    ");
}

#[test]
fn test_bookmark_forget_deleted_or_nonexistent_bookmark() {
    // Much of this test is borrowed from `test_git_fetch_remote_only_bookmark` in
    // test_git_fetch.rs

    // ======== Beginning of test setup ========
    // Set up a git repo with a bookmark and a jj repo that has it as a remote.
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init_bare(git_repo_path);
    // Create a commit and a bookmark in the git repo
    git::add_commit(
        &git_repo,
        "refs/heads/feature1",
        "file",
        b"content",
        "message",
        &[],
    );
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    // Fetch and then delete the bookmark
    work_dir
        .run_jj(["git", "fetch", "--remote=origin"])
        .success();
    work_dir
        .run_jj(["bookmark", "delete", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1 (deleted)
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // ============ End of test setup ============

    // We can forget a deleted bookmark
    work_dir
        .run_jj(["bookmark", "forget", "--include-remotes", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");

    // Can't forget a non-existent bookmark
    let output = work_dir.run_jj(["bookmark", "forget", "i_do_not_exist"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching bookmarks for names: i_do_not_exist
    No bookmarks to forget.
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    // Fetch new commit without auto tracking. No local bookmarks should be
    // created.
    create_commit_with_refs(
        &git_repo,
        "commit 1",
        b"content 1",
        &[
            "refs/heads/main",
            "refs/heads/feature1",
            "refs/heads/feature2",
        ],
    );
    test_env.add_config("remotes.origin.auto-track-bookmarks = ''");
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] untracked
    bookmark: feature2@origin [new] untracked
    bookmark: main@origin     [new] untracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1@origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main@origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   e8849ae12c70
    │ ◆  feature1@origin feature2@origin main@origin bd843888ee66
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Track new bookmark. Local bookmark should be created.
    work_dir
        .run_jj(["bookmark", "track", "feature1@origin", "main@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");

    // Track non-existent remote bookmark
    let output = work_dir.run_jj(["bookmark", "track", "feature3@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching remote bookmarks for names: feature3@origin
    Nothing changed.
    [EOF]
    ");

    // Track existing bookmark. Local bookmark should result in conflict.
    work_dir
        .run_jj(["bookmark", "create", "-r@", "feature2"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "feature2@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    feature2 (conflicted):
      + qpvuntsm e8849ae1 (empty) (no description set)
      + qxxqrkql bd843888 commit 1
      @origin (behind by 1 commits): qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");

    // Untrack existing and locally-deleted bookmarks. Bookmark targets should be
    // unchanged
    work_dir
        .run_jj(["bookmark", "delete", "feature2"])
        .success();
    work_dir
        .run_jj(["bookmark", "untrack", "feature1@origin", "feature2@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   e8849ae12c70
    │ ◆  feature1 feature1@origin feature2@origin main bd843888ee66
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Fetch new commit. Only tracking bookmark "main" should be merged.
    create_commit_with_refs(
        &git_repo,
        "commit 2",
        b"content 2",
        &[
            "refs/heads/main",
            "refs/heads/feature1",
            "refs/heads/feature2",
        ],
    );
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [updated] untracked
    bookmark: feature2@origin [updated] untracked
    bookmark: main@origin     [updated] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: psynomvr 48ec79a4 commit 2
    feature2@origin: psynomvr 48ec79a4 commit 2
    main: psynomvr 48ec79a4 commit 2
      @origin: psynomvr 48ec79a4 commit 2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   e8849ae12c70
    │ ◆  feature1@origin feature2@origin main 48ec79a430e9
    ├─╯
    │ ○  feature1 bd843888ee66
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Fetch new commit with auto tracking. Tracking bookmark "main" and new
    // bookmark "feature3" should be merged.
    create_commit_with_refs(
        &git_repo,
        "commit 3",
        b"content 3",
        &[
            "refs/heads/main",
            "refs/heads/feature1",
            "refs/heads/feature2",
            "refs/heads/feature3",
        ],
    );
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [updated] untracked
    bookmark: feature2@origin [updated] untracked
    bookmark: feature3@origin [new] tracked
    bookmark: main@origin     [updated] tracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: yumopmsr d8cd3e02 commit 3
    feature2@origin: yumopmsr d8cd3e02 commit 3
    feature3: yumopmsr d8cd3e02 commit 3
      @origin: yumopmsr d8cd3e02 commit 3
    main: yumopmsr d8cd3e02 commit 3
      @origin: yumopmsr d8cd3e02 commit 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @   e8849ae12c70
    │ ◆  feature1@origin feature2@origin feature3 main d8cd3e020382
    ├─╯
    │ ○  feature1 bd843888ee66
    ├─╯
    ◆   000000000000
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // add three remotes
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();
    let git_repo_path = test_env.env_root().join("git-repo2");
    git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin2", "../git-repo2"])
        .success();
    let git_repo_path = test_env.env_root().join("git-repo3");
    git::init_bare(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin3", "../git-repo3"])
        .success();

    // create bookmark and push to origin
    work_dir.run_jj(["bookmark", "create", "main"]).success();
    work_dir.run_jj(["describe", "-m", "a"]).success();
    work_dir
        .run_jj(["git", "push", "--allow-new", "-b", "main"])
        .success();

    // adjust main and push to origin2, again for origin3
    work_dir
        .run_jj(["describe", "-m", "b", "-r", "main", "--ignore-immutable"])
        .success();
    work_dir
        .run_jj(["git", "push", "-N", "-b", "main", "--remote", "origin2"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "c", "-r", "main", "--ignore-immutable"])
        .success();
    work_dir
        .run_jj(["git", "push", "-N", "-b", "main", "--remote", "origin3"])
        .success();

    // stop and retrack origin; creates conflict
    // origin2 and origin3 are not shown
    work_dir
        .run_jj(["bookmark", "untrack", "main@origin"])
        .success();
    let output = work_dir.run_jj(["bookmark", "track", "main@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    main (conflicted):
      + qpvuntsm?? 467e027c (empty) c
      + qpvuntsm?? 48ded843 (empty) a
      @origin (behind by 1 commits): qpvuntsm?? 48ded843 (empty) a
    [EOF]
    ");

    // origin2 differs but is not in conflict
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      + qpvuntsm?? 467e027c (empty) c
      + qpvuntsm?? 48ded843 (empty) a
      @origin (behind by 1 commits): qpvuntsm?? 48ded843 (empty) a
      @origin2 (ahead by 1 commits, behind by 2 commits): qpvuntsm hidden 579e0acd (empty) b
      @origin3 (behind by 1 commits): qpvuntsm?? 467e027c (empty) c
    [EOF]
    ");

    // retracking origin2 adds to the conflict
    work_dir
        .run_jj(["bookmark", "untrack", "main@origin2"])
        .success();
    let output = work_dir.run_jj(["bookmark", "track", "main@origin2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    main (conflicted):
      + qpvuntsm?? 467e027c (empty) c
      + qpvuntsm?? 48ded843 (empty) a
      + qpvuntsm?? 579e0acd (empty) b
      @origin2 (behind by 2 commits): qpvuntsm?? 579e0acd (empty) b
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack_patterns() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init(git_repo_path);
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();

    // Create remote commit
    create_commit_with_refs(
        &git_repo,
        "commit",
        b"content",
        &["refs/heads/feature1", "refs/heads/feature2"],
    );

    // Fetch new commit without auto tracking
    test_env.add_config("remotes.origin.auto-track-bookmarks = ''");
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] untracked
    bookmark: feature2@origin [new] untracked
    [EOF]
    ");

    // Track local bookmark
    work_dir.run_jj(["bookmark", "create", "main"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "track", "main"]), @r"
    ------- stderr -------
    error: invalid value 'main' for '<BOOKMARK@REMOTE>...': remote bookmark must be specified in bookmark@remote form

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Track/untrack new bookmark that doesn't exist at remote
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "track", "main@origin"]), @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "untrack", "main@origin"]), @r"
    ------- stderr -------
    Stopped tracking 1 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(
        work_dir.run_jj(["bookmark", "untrack", "main@origin", "glob:main@o*"]), @r"
    ------- stderr -------
    Warning: Remote bookmark not tracked yet: main@origin
    Nothing changed.
    [EOF]
    ");

    // Track/untrack unknown bookmark
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "track", "glob:maine@*"]), @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(
        work_dir.run_jj(["bookmark", "untrack", "maine@origin", "glob:maine@o*"]), @r"
    ------- stderr -------
    Warning: No matching remote bookmarks for names: maine@origin
    Nothing changed.
    [EOF]
    ");

    // Track already tracked bookmark
    work_dir
        .run_jj(["bookmark", "track", "feature1@origin"])
        .success();
    let output = work_dir.run_jj(["bookmark", "track", "feature1@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark already tracked: feature1@origin
    Nothing changed.
    [EOF]
    ");

    // Untrack non-tracking bookmark
    let output = work_dir.run_jj(["bookmark", "untrack", "feature2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark not tracked yet: feature2@origin
    Nothing changed.
    [EOF]
    ");

    // Untrack Git-tracking bookmark
    work_dir.run_jj(["git", "export"]).success();
    let output = work_dir.run_jj(["bookmark", "untrack", "main@git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Git-tracking bookmark cannot be untracked: main@git
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    feature2@origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Untrack by pattern
    let output = work_dir.run_jj(["bookmark", "untrack", "glob:*@*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Git-tracking bookmark cannot be untracked: feature1@git
    Warning: Remote bookmark not tracked yet: feature2@origin
    Warning: Git-tracking bookmark cannot be untracked: main@git
    Warning: Remote bookmark not tracked yet: main@origin
    Stopped tracking 1 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
    feature1@origin: yrnqsqlx 41e7a49d commit
    feature2@origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Track by pattern
    let output = work_dir.run_jj(["bookmark", "track", "glob:feature?@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 2 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    feature2: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_list() {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    for bookmark in [
        "remote-sync",
        "remote-unsync",
        "remote-untrack",
        "remote-delete",
    ] {
        remote_dir
            .run_jj(["new", "root()", "-m", bookmark])
            .success();
        remote_dir
            .run_jj(["bookmark", "create", bookmark])
            .success();
    }
    remote_dir.run_jj(["new"]).success();
    remote_dir.run_jj(["git", "export"]).success();

    // Initialize local refs
    let mut remote_git_path = remote_dir.root().to_owned();
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["new", "root()", "-m", "local-only"])
        .success();
    local_dir
        .run_jj([
            "--config=remotes.origin.auto-track-bookmarks=''",
            "bookmark",
            "create",
            "local-only",
            "absent-tracked",
        ])
        .success();

    // Mutate refs in local repository
    local_dir
        .run_jj(["bookmark", "delete", "remote-delete"])
        .success();
    local_dir
        .run_jj(["bookmark", "delete", "remote-untrack"])
        .success();
    local_dir
        .run_jj(["bookmark", "track", "absent-tracked@origin"])
        .success();
    local_dir
        .run_jj(["bookmark", "untrack", "remote-untrack@origin"])
        .success();
    local_dir
        .run_jj(["bookmark", "set", "--allow-backwards", "remote-unsync"])
        .success();

    // Synchronized tracking remotes and non-tracking remotes aren't listed by
    // default
    let output = local_dir.run_jj(["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    absent-tracked: wqnwkozp 0353dd35 (empty) local-only
      @origin (not created yet)
    local-only: wqnwkozp 0353dd35 (empty) local-only
    remote-delete (deleted)
      @origin: vruxwmqv b32031cf (empty) remote-delete
    remote-sync: rlvkpnrz 7a07dbee (empty) remote-sync
    remote-unsync: wqnwkozp 0353dd35 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    absent-tracked: wqnwkozp 0353dd35 (empty) local-only
      @origin (not created yet)
    local-only: wqnwkozp 0353dd35 (empty) local-only
    remote-delete (deleted)
      @origin: vruxwmqv b32031cf (empty) remote-delete
    remote-sync: rlvkpnrz 7a07dbee (empty) remote-sync
      @origin: rlvkpnrz 7a07dbee (empty) remote-sync
    remote-unsync: wqnwkozp 0353dd35 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    remote-untrack@origin: royxmykx 149bc756 (empty) remote-untrack
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--all-remotes", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [38;5;5mabsent-tracked[39m: [1m[38;5;13mw[38;5;8mqnwkozp[39m [38;5;12m03[38;5;8m53dd35[39m [38;5;10m(empty)[39m local-only[0m
      [38;5;5m@origin[39m (not created yet)
    [38;5;5mlocal-only[39m: [1m[38;5;13mw[38;5;8mqnwkozp[39m [38;5;12m03[38;5;8m53dd35[39m [38;5;10m(empty)[39m local-only[0m
    [38;5;5mremote-delete[39m (deleted)
      [38;5;5m@origin[39m: [1m[38;5;5mv[0m[38;5;8mruxwmqv[39m [1m[38;5;4mb[0m[38;5;8m32031cf[39m [38;5;2m(empty)[39m remote-delete
    [38;5;5mremote-sync[39m: [1m[38;5;5mr[0m[38;5;8mlvkpnrz[39m [1m[38;5;4m7[0m[38;5;8ma07dbee[39m [38;5;2m(empty)[39m remote-sync
      [38;5;5m@origin[39m: [1m[38;5;5mr[0m[38;5;8mlvkpnrz[39m [1m[38;5;4m7[0m[38;5;8ma07dbee[39m [38;5;2m(empty)[39m remote-sync
    [38;5;5mremote-unsync[39m: [1m[38;5;13mw[38;5;8mqnwkozp[39m [38;5;12m03[38;5;8m53dd35[39m [38;5;10m(empty)[39m local-only[0m
      [38;5;5m@origin[39m (ahead by 1 commits, behind by 1 commits): [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m5[0m[38;5;8m53203ba[39m [38;5;2m(empty)[39m remote-unsync
    [38;5;5mremote-untrack@origin[39m: [1m[38;5;5mro[0m[38;5;8myxmykx[39m [1m[38;5;4m1[0m[38;5;8m49bc756[39m [38;5;2m(empty)[39m remote-untrack
    [EOF]
    ------- stderr -------
    [1m[38;5;6mHint: [0m[39mBookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.[39m
    [EOF]
    ");

    let template = r#"
    concat(
      "[" ++ name ++ if(remote, "@" ++ remote) ++ "]\n",
      separate(" ", "present:", present) ++ "\n",
      separate(" ", "conflict:", conflict) ++ "\n",
      separate(" ", "normal_target:", normal_target.description().first_line()) ++ "\n",
      separate(" ", "removed_targets:", removed_targets.map(|c| c.description().first_line())) ++ "\n",
      separate(" ", "added_targets:", added_targets.map(|c| c.description().first_line())) ++ "\n",
      separate(" ", "tracked:", tracked) ++ "\n",
      separate(" ", "tracking_present:", tracking_present) ++ "\n",
      separate(" ", "tracking_ahead_count:", tracking_ahead_count.lower()) ++ "\n",
      separate(" ", "tracking_behind_count:", tracking_behind_count.lower()) ++ "\n",
    )
    "#;
    let output = local_dir.run_jj(["bookmark", "list", "--all-remotes", "-T", template]);
    insta::assert_snapshot!(output, @r"
    [absent-tracked]
    present: true
    conflict: false
    normal_target: local-only
    removed_targets:
    added_targets: local-only
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [absent-tracked@origin]
    present: false
    conflict: false
    normal_target: <Error: No Commit available>
    removed_targets:
    added_targets:
    tracked: true
    tracking_present: true
    tracking_ahead_count: 0
    tracking_behind_count: 2
    [local-only]
    present: true
    conflict: false
    normal_target: local-only
    removed_targets:
    added_targets: local-only
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [remote-delete]
    present: false
    conflict: false
    normal_target: <Error: No Commit available>
    removed_targets:
    added_targets:
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [remote-delete@origin]
    present: true
    conflict: false
    normal_target: remote-delete
    removed_targets:
    added_targets: remote-delete
    tracked: true
    tracking_present: false
    tracking_ahead_count: 2
    tracking_behind_count: 0
    [remote-sync]
    present: true
    conflict: false
    normal_target: remote-sync
    removed_targets:
    added_targets: remote-sync
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [remote-sync@origin]
    present: true
    conflict: false
    normal_target: remote-sync
    removed_targets:
    added_targets: remote-sync
    tracked: true
    tracking_present: true
    tracking_ahead_count: 0
    tracking_behind_count: 0
    [remote-unsync]
    present: true
    conflict: false
    normal_target: local-only
    removed_targets:
    added_targets: local-only
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [remote-unsync@origin]
    present: true
    conflict: false
    normal_target: remote-unsync
    removed_targets:
    added_targets: remote-unsync
    tracked: true
    tracking_present: true
    tracking_ahead_count: 1
    tracking_behind_count: 1
    [remote-untrack@origin]
    present: true
    conflict: false
    normal_target: remote-untrack
    removed_targets:
    added_targets: remote-untrack
    tracked: false
    tracking_present: false
    tracking_ahead_count: <Error: Not a tracked remote ref>
    tracking_behind_count: <Error: Not a tracked remote ref>
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", r#"-Tjson(self) ++ "\n""#]);
    insta::assert_snapshot!(output, @r#"
    {"name":"absent-tracked","target":["0353dd35c56156971ce5f023a1db7a6196160a8a"]}
    {"name":"absent-tracked","remote":"origin","target":[null],"tracking_target":["0353dd35c56156971ce5f023a1db7a6196160a8a"]}
    {"name":"local-only","target":["0353dd35c56156971ce5f023a1db7a6196160a8a"]}
    {"name":"remote-delete","target":[null]}
    {"name":"remote-delete","remote":"origin","target":["b32031cf329fbb90d042635c295b4e3fa2ca2651"],"tracking_target":[null]}
    {"name":"remote-sync","target":["7a07dbeef135886b7ba7adb27d05190c39cd92ab"]}
    {"name":"remote-unsync","target":["0353dd35c56156971ce5f023a1db7a6196160a8a"]}
    {"name":"remote-unsync","remote":"origin","target":["553203baa52803406124962dbc0bcdc0227b20b2"],"tracking_target":["0353dd35c56156971ce5f023a1db7a6196160a8a"]}
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    "#);
}

#[test]
fn test_bookmark_list_filtered() {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    for bookmark in ["remote-keep", "remote-delete", "remote-rewrite"] {
        remote_dir
            .run_jj(["new", "root()", "-m", bookmark])
            .success();
        remote_dir
            .run_jj(["bookmark", "create", bookmark])
            .success();
    }
    remote_dir.run_jj(["new"]).success();
    remote_dir.run_jj(["git", "export"]).success();

    // Initialize local refs
    let mut remote_git_path = remote_dir.root().to_owned();
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["new", "root()", "-m", "local-keep"])
        .success();
    local_dir
        .run_jj([
            "--config=remotes.origin.auto-track-bookmarks=''",
            "bookmark",
            "create",
            "local-keep",
        ])
        .success();

    // Mutate refs in local repository
    local_dir
        .run_jj(["bookmark", "delete", "remote-delete"])
        .success();
    local_dir
        .run_jj(["describe", "-mrewritten", "remote-rewrite"])
        .success();

    let template = r#"separate(" ", commit_id.short(), bookmarks, if(hidden, "(hidden)"))"#;
    insta::assert_snapshot!(
        local_dir.run_jj(["log", "-r::(bookmarks() | remote_bookmarks())", "-T", template]), @r"
    @  4b2bc95cbda6 local-keep
    │ ○  e6970e0e1f55 remote-rewrite*
    ├─╯
    │ ○  331d500d2fda remote-rewrite@origin (hidden)
    ├─╯
    │ ○  0e6b796871e6 remote-delete@origin
    ├─╯
    │ ○  c2f2ee40f03a remote-keep
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // All bookmarks are listed by default.
    let output = local_dir.run_jj(["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let query =
        |args: &[&str]| local_dir.run_jj_with(|cmd| cmd.args(["bookmark", "list"]).args(args));

    // "all()" doesn't include deleted bookmarks since they have no local targets.
    // So "all()" is identical to "bookmarks()".
    insta::assert_snapshot!(query(&["-rall()"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");

    // Exclude remote-only bookmarks. "remote-rewrite@origin" is included since
    // local "remote-rewrite" target matches.
    insta::assert_snapshot!(query(&["-rbookmarks()"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks by name.
    insta::assert_snapshot!(query(&["remote-rewrite"]), @r"
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");
    insta::assert_snapshot!(query(&["-rbookmarks(glob:remote-rewrite)"]), @r"
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks by name, combined with --all-remotes
    local_dir.run_jj(["git", "export"]).success();
    insta::assert_snapshot!(query(&["--all-remotes", "remote-rewrite"]), @r"
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @git: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--all-remotes", "-rbookmarks(glob:remote-rewrite)"]), @r"
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @git: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks with --remote
    insta::assert_snapshot!(query(&["--remote", "origin"]), @r"
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
      @origin: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--remote", "glob:'gi?'"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
      @git: kpqxywon 4b2bc95c (empty) local-keep
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
      @git: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @git: royxmykx e6970e0e (empty) rewritten
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--remote", "origin", "--remote", "git"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
      @git: kpqxywon 4b2bc95c (empty) local-keep
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
      @git: rlvkpnrz c2f2ee40 (empty) remote-keep
      @origin: rlvkpnrz c2f2ee40 (empty) remote-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @git: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    // Can select deleted bookmark by name pattern, but not by revset.
    insta::assert_snapshot!(query(&["remote-delete"]), @r"
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");
    insta::assert_snapshot!(query(&["-rbookmarks(glob:remote-delete)"]), @"");
    insta::assert_snapshot!(query(&["-rremote-delete"]), @r"
    ------- stderr -------
    Error: Revision `remote-delete` doesn't exist
    Hint: Did you mean `remote-delete@origin`, `remote-keep`, `remote-rewrite`, `remote-rewrite@origin`?
    [EOF]
    [exit status: 1]
    ");

    // Name patterns are OR-ed.
    insta::assert_snapshot!(query(&["glob:*-keep", "glob:remote-* & glob:*-delete"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    remote-keep: rlvkpnrz c2f2ee40 (empty) remote-keep
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    // Unmatched exact name pattern should be warned. "remote-delete" exists in
    // remote. "remote-rewrite" exists, but isn't included in the match.
    insta::assert_snapshot!(
        query(&["local-keep", "glob:push-*", "unknown | remote-delete ~ remote-rewrite"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-delete (deleted)
      @origin: zsuskuln 0e6b7968 (empty) remote-delete
    [EOF]
    ------- stderr -------
    Warning: No matching bookmarks for names: unknown
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    // Name pattern and revset are OR-ed.
    insta::assert_snapshot!(query(&["local-keep", "-rbookmarks(glob:remote-rewrite)"]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): royxmykx hidden 331d500d (empty) remote-rewrite
    [EOF]
    ");

    // … but still filtered by --remote
    insta::assert_snapshot!(query(&[
        "local-keep",
        "-rbookmarks(glob:remote-rewrite)",
        "--remote",
        "git",
    ]), @r"
    local-keep: kpqxywon 4b2bc95c (empty) local-keep
      @git: kpqxywon 4b2bc95c (empty) local-keep
    remote-rewrite: royxmykx e6970e0e (empty) rewritten
      @git: royxmykx e6970e0e (empty) rewritten
    [EOF]
    ");

    // Syntax error in name pattern
    insta::assert_snapshot!(query(&["foo &"]), @r"
    ------- stderr -------
    Error: Failed to parse name pattern: Syntax error
    Caused by:  --> 1:6
      |
    1 | foo &
      |      ^---
      |
      = expected `::`, `..`, `~`, or <primary>
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bookmark_list_quoted_name() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "'with space'"])
        .success();

    // quoted by default
    let output = work_dir.run_jj(["bookmark", "list"]);
    insta::assert_snapshot!(output, @r#"
    "with space": qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    "#);

    // string method should apply to the original (unquoted) name
    let template = r#"
    separate(' ',
      self,
      name.contains('"'),
      name.len(),
    ) ++ "\n"
    "#;
    let output = work_dir.run_jj(["bookmark", "list", "-T", template]);
    insta::assert_snapshot!(output, @r#"
    "with space" false 10
    [EOF]
    "#);
}

#[test]
fn test_bookmark_list_much_remote_divergence() {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    remote_dir
        .run_jj(["new", "root()", "-m", "remote-unsync"])
        .success();
    for _ in 0..15 {
        remote_dir.run_jj(["new", "-m", "remote-unsync"]).success();
    }
    remote_dir
        .run_jj(["bookmark", "create", "-r@", "remote-unsync"])
        .success();
    remote_dir.run_jj(["new"]).success();
    remote_dir.run_jj(["git", "export"]).success();

    // Initialize local refs
    let mut remote_git_path = remote_dir.root().to_owned();
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["new", "root()", "-m", "local-only"])
        .success();
    for _ in 0..15 {
        local_dir.run_jj(["new", "-m", "local-only"]).success();
    }
    local_dir
        .run_jj([
            "--config=remotes.origin.auto-track-bookmarks=''",
            "bookmark",
            "create",
            "local-only",
        ])
        .success();

    // Mutate refs in local repository
    local_dir
        .run_jj(["bookmark", "set", "--allow-backwards", "remote-unsync"])
        .success();

    let output = local_dir.run_jj(["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    local-only: zkyosouw a30800ad (empty) local-only
    remote-unsync: zkyosouw a30800ad (empty) local-only
      @origin (ahead by at least 10 commits, behind by at least 10 commits): uyznsvlq a52367f8 (empty) remote-unsync
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_tracked() {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    test_env.add_config("remotes.upstream.auto-track-bookmarks = 'glob:*'");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    for bookmark in [
        "remote-sync",
        "remote-unsync",
        "remote-untrack",
        "remote-delete",
    ] {
        remote_dir
            .run_jj(["new", "root()", "-m", bookmark])
            .success();
        remote_dir
            .run_jj(["bookmark", "create", bookmark])
            .success();
    }
    remote_dir.run_jj(["new"]).success();
    remote_dir.run_jj(["git", "export"]).success();

    // Initialize local refs
    let mut remote_git_path = remote_dir.root().to_owned();
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "clone",
                "--colocate",
                remote_git_path.to_str().unwrap(),
                "local",
            ],
        )
        .success();

    test_env
        .run_jj_in(".", ["git", "init", "upstream"])
        .success();

    // Initialize a second remote
    let upstream_dir = test_env.work_dir("upstream");
    upstream_dir
        .run_jj(["new", "root()", "-m", "upstream-sync"])
        .success();
    upstream_dir
        .run_jj(["bookmark", "create", "upstream-sync"])
        .success();
    upstream_dir.run_jj(["new"]).success();
    upstream_dir.run_jj(["git", "export"]).success();

    let mut upstream_git_path = upstream_dir.root().to_owned();
    upstream_git_path.extend([".jj", "repo", "store", "git"]);

    let local_dir = test_env.work_dir("local");

    local_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "upstream",
            upstream_git_path.to_str().unwrap(),
        ])
        .success();
    local_dir
        .run_jj(["git", "fetch", "--all-remotes"])
        .success();

    local_dir
        .run_jj(["new", "root()", "-m", "local-only"])
        .success();
    local_dir
        .run_jj([
            "--config=remotes.origin.auto-track-bookmarks=''",
            "--config=remotes.upstream.auto-track-bookmarks=''",
            "bookmark",
            "create",
            "local-only",
        ])
        .success();

    // Mutate refs in local repository
    local_dir
        .run_jj(["bookmark", "delete", "remote-delete"])
        .success();
    local_dir
        .run_jj(["bookmark", "delete", "remote-untrack"])
        .success();
    local_dir
        .run_jj(["bookmark", "untrack", "remote-untrack@origin"])
        .success();
    local_dir
        .run_jj([
            "git",
            "push",
            "--allow-new",
            "--remote",
            "upstream",
            "--bookmark",
            "remote-unsync",
        ])
        .success();
    local_dir
        .run_jj(["bookmark", "set", "--allow-backwards", "remote-unsync"])
        .success();

    let output = local_dir.run_jj(["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    local-only: nmzmmopx 2a685e16 (empty) local-only
      @git: nmzmmopx 2a685e16 (empty) local-only
    remote-delete (deleted)
      @origin: vruxwmqv b32031cf (empty) remote-delete
    remote-sync: rlvkpnrz 7a07dbee (empty) remote-sync
      @git: rlvkpnrz 7a07dbee (empty) remote-sync
      @origin: rlvkpnrz 7a07dbee (empty) remote-sync
    remote-unsync: nmzmmopx 2a685e16 (empty) local-only
      @git: nmzmmopx 2a685e16 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    remote-untrack@origin: royxmykx 149bc756 (empty) remote-untrack
    upstream-sync: lylxulpl 169ba7d9 (empty) upstream-sync
      @git: lylxulpl 169ba7d9 (empty) upstream-sync
      @upstream: lylxulpl 169ba7d9 (empty) upstream-sync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--tracked"]);
    insta::assert_snapshot!(output, @r"
    remote-delete (deleted)
      @origin: vruxwmqv b32031cf (empty) remote-delete
    remote-sync: rlvkpnrz 7a07dbee (empty) remote-sync
      @origin: rlvkpnrz 7a07dbee (empty) remote-sync
    remote-unsync: nmzmmopx 2a685e16 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    upstream-sync: lylxulpl 169ba7d9 (empty) upstream-sync
      @upstream: lylxulpl 169ba7d9 (empty) upstream-sync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--tracked", "--remote", "origin"]);
    insta::assert_snapshot!(output, @r"
    remote-delete (deleted)
      @origin: vruxwmqv b32031cf (empty) remote-delete
    remote-sync: rlvkpnrz 7a07dbee (empty) remote-sync
      @origin: rlvkpnrz 7a07dbee (empty) remote-sync
    remote-unsync: nmzmmopx 2a685e16 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted can be *deleted permanently* on the remote by running `jj git push --deleted`. Use `jj bookmark forget` if you don't want that.
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--tracked", "remote-unsync"]);
    insta::assert_snapshot!(output, @r"
    remote-unsync: nmzmmopx 2a685e16 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    [EOF]
    ");

    let output = local_dir.run_jj(["bookmark", "list", "--tracked", "remote-untrack"]);
    insta::assert_snapshot!(output, @"");

    local_dir
        .run_jj(["bookmark", "untrack", "remote-unsync@upstream"])
        .success();

    let output = local_dir.run_jj(["bookmark", "list", "--tracked", "remote-unsync"]);
    insta::assert_snapshot!(output, @r"
    remote-unsync: nmzmmopx 2a685e16 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): zsuskuln 553203ba (empty) remote-unsync
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_conflicted() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Track existing bookmark. Local bookmark should result in conflict.
    work_dir.run_jj(["new", "root()", "-m", "a"]).success();
    work_dir.run_jj(["new", "root()", "-m", "b"]).success();
    work_dir.run_jj(["bookmark", "create", "bar"]).success();
    work_dir
        .run_jj(["bookmark", "create", "foo", "-rsubject(glob:a)"])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "foo",
            "-rsubject(glob:b)",
            "--at-op=@-",
        ])
        .success();
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    bar: kkmpptxz a82129fb (empty) b
    foo (conflicted):
      + rlvkpnrz 4e1b2d80 (empty) a
      + kkmpptxz a82129fb (empty) b
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "list", "--conflicted"]), @r"
    foo (conflicted):
      + rlvkpnrz 4e1b2d80 (empty) a
      + kkmpptxz a82129fb (empty) b
    [EOF]
    ------- stderr -------
    Hint: Some bookmarks have conflicts. Use `jj bookmark set <name> -r <rev>` to resolve.
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_sort_unknown_key_error() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "list", "--sort", "date"]), @r"
    ------- stderr -------
    error: invalid value 'date' for '--sort <SORT_KEY>'
      [possible values: name, name-, author-name, author-name-, author-email, author-email-, author-date, author-date-, committer-name, committer-name-, committer-email, committer-email-, committer-date, committer-date-]

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_bookmark_list_sort_multiple_keys() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    for (bookmark, email) in [("c", "bob@g.c"), ("b", "alice@g.c"), ("a", "bob@g.c")] {
        work_dir
            .run_jj([
                &format!("--config=user.email={email}"),
                "new",
                "root()",
                "-m",
                "fix",
            ])
            .success();
        work_dir.run_jj(["bookmark", "create", bookmark]).success();
    }

    let template =
        r#"name ++ ": " ++ if(normal_target, normal_target.committer().email()) ++ "\n""#;
    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "list", "-T", template, "--sort", "committer-email,committer-date-"]), @r"
    b: alice@g.c
    a: bob@g.c
    c: bob@g.c
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_sort_using_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    for (bookmark, email) in [("c", "bob@g.c"), ("b", "alice@g.c"), ("a", "bob@g.c")] {
        work_dir
            .run_jj([
                &format!("--config=user.email={email}"),
                "new",
                "root()",
                "-m",
                "fix",
            ])
            .success();
        work_dir.run_jj(["bookmark", "create", bookmark]).success();
    }

    let template = r#"name ++ ": " ++ if(normal_target, normal_target.author().email()) ++ "\n""#;
    insta::assert_snapshot!(work_dir.run_jj([
        "--config=ui.bookmark-list-sort-keys=['author-email', 'author-date-']",
        "bookmark",
        "list",
        "-T",
        template
    ]), @r"
    b: alice@g.c
    a: bob@g.c
    c: bob@g.c
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_sort_overriding_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    for (bookmark, email) in [("c", "bob@g.c"), ("b", "alice@g.c"), ("a", "bob@g.c")] {
        work_dir
            .run_jj([
                &format!("--config=user.email={email}"),
                "new",
                "root()",
                "-m",
                "fix",
            ])
            .success();
        work_dir.run_jj(["bookmark", "create", bookmark]).success();
    }

    let template = r#"name ++ ": " ++ if(normal_target, normal_target.author().email()) ++ "\n""#;
    insta::assert_snapshot!(work_dir.run_jj([
        "--config=ui.bookmark-list-sort-keys=['author-email', 'author-date-']",
        "bookmark",
        "list",
        "--sort=name-", // overriding config.
        "-T",
        template
    ]), @r"
    c: bob@g.c
    b: alice@g.c
    a: bob@g.c
    [EOF]
    ");
}

#[test]
fn test_create_and_set_auto_track_bookmarks() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    root_dir
        .run_jj(["git", "init", "--colocate", "origin"])
        .success();
    test_env.add_config(
        "
        [remotes.origin]
        auto-track-bookmarks = 'glob:mine/*'
        [remotes.fork]
        auto-track-bookmarks = 'glob:*'
        ",
    );

    root_dir.run_jj(["git", "init", "repo"]).success();
    let repo_dir = test_env.work_dir("repo");
    repo_dir
        .run_jj(["git", "remote", "add", "origin", "../origin/.git"])
        .success();
    repo_dir
        .run_jj(["git", "remote", "add", "fork", "dummy"])
        .success();

    // jj bookmark create obeys remotes.<name>.auto-track-bookmarks
    repo_dir
        .run_jj(["bookmark", "create", "mine/create", "not-mine/create"])
        .success();
    let output = repo_dir.run_jj([
        "bookmark",
        "list",
        "--all",
        "mine/create",
        "not-mine/create",
    ]);
    insta::assert_snapshot!(output, @r"
    mine/create: rlvkpnrz 7eb1c95e (empty) (no description set)
      @fork (not created yet)
      @origin (not created yet)
    not-mine/create: rlvkpnrz 7eb1c95e (empty) (no description set)
      @fork (not created yet)
    [EOF]
    ");
    repo_dir.run_jj(["commit", "--message", "create"]).success();

    // jj bookmark set obeys remotes.<name>.auto-track-bookmarks
    repo_dir
        .run_jj(["bookmark", "set", "mine/set", "not-mine/set"])
        .success();
    let output = repo_dir.run_jj(["bookmark", "list", "--all", "mine/set", "not-mine/set"]);
    insta::assert_snapshot!(output, @r"
    mine/set: yqosqzyt 5fbe2b20 (empty) (no description set)
      @fork (not created yet)
      @origin (not created yet)
    not-mine/set: yqosqzyt 5fbe2b20 (empty) (no description set)
      @fork (not created yet)
    [EOF]
    ");
    repo_dir.run_jj(["commit", "--message", "set"]).success();

    // jj bookmark create warns when auto-tracking existing bookmark
    repo_dir.run_jj(["git", "push"]).success();
    repo_dir
        .run_jj(["bookmark", "forget", "mine/create"])
        .success();
    let output = repo_dir.run_jj(["bookmark", "create", "mine/create"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Warning: Auto-tracking bookmark that exists on the remote: mine/create@origin
    Created 1 bookmarks pointing to znkkpsqq 2e899fb8 mine/create* | (empty) (no description set)
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"bookmarks ++ " " ++ commit_id.short()"#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
