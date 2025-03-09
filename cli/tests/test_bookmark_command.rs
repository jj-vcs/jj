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

use std::path::Path;

use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;

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
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 2 bookmarks pointing to qpvuntsm 230dd059 bar foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar foo 230dd059e1b0
    ◆   000000000000
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "foo", "bar", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 2 bookmarks to zsuskuln 8bb159bc bar foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar foo 8bb159bc30a9
    ○   230dd059e1b0
    ◆   000000000000
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "delete", "foo", "bar", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   8bb159bc30a9
    ○   230dd059e1b0
    ◆   000000000000
    [EOF]
    ");

    // Hint should be omitted if -r is specified
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@-", "foo", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 2 bookmarks pointing to qpvuntsm 230dd059 bar foo | (empty) (no description set)
    [EOF]
    ");

    // Create and move with explicit -r
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-r@", "bar", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to zsuskuln 8bb159bc bar baz | (empty) (no description set)
    Moved 1 bookmarks to zsuskuln 8bb159bc bar baz | (empty) (no description set)
    [EOF]
    ");

    // Noop changes should not be included in the stats
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-r@", "foo", "bar", "baz"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to zsuskuln 8bb159bc bar baz foo | (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_at_root() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "fred", "-r=root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to zzzzzzzz 00000000 fred | (empty) (no description set)
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["git", "export"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    Warning: Failed to export some bookmarks:
      fred: Ref cannot point to the root commit in Git
    [EOF]
    ");
}

#[test]
fn test_bookmark_bad_name() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", ""]);
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
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "''"]);
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
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "x", ""]);
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
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    // common errors
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "@-", "foo"]);
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
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for how to quote symbols.
    [EOF]
    [exit status: 2]
    ");

    let stderr = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-r@-", "foo@bar"]);
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
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", "'foo@bar'"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to qpvuntsm 230dd059 foo@bar | (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_move() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: foo
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: foo
    Hint: Use `jj bookmark set` to update it.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "foo", "--revision", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to mzvwutvl 167f90e7 foo | (empty) (no description set)
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-r@-", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "set", "-r@-", "--allow-backwards", "foo"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to mzvwutvl 167f90e7 foo | (empty) (no description set)
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "--to=@-", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "move", "--to=@-", "--allow-backwards", "foo"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");

    // Delete bookmark locally, but is still tracking remote
    test_env
        .run_jj_in(&repo_path, ["describe", "@-", "-mcommit"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["git", "push", "--allow-new", "-r@-"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "delete", "foo"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    foo (deleted)
      @origin: qpvuntsm 1eb845f3 (empty) commit
    [EOF]
    ");

    // Deleted tracking bookmark name should still be allocated
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Tracked remote bookmarks exist for deleted bookmark: foo
    Hint: Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack 'glob:foo@*'` to disassociate them.
    [EOF]
    [exit status: 1]
    ");

    // Restoring local target shouldn't invalidate tracking state
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to mzvwutvl 66d48752 foo* | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    foo: mzvwutvl 66d48752 (empty) (no description set)
      @origin (behind by 1 commits): qpvuntsm 1eb845f3 (empty) commit
    [EOF]
    ");

    // Untracked remote bookmark shouldn't block creation of local bookmark
    test_env
        .run_jj_in(&repo_path, ["bookmark", "untrack", "foo@origin"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "delete", "foo"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to mzvwutvl 66d48752 foo | (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    foo: mzvwutvl 66d48752 (empty) (no description set)
    foo@origin: qpvuntsm 1eb845f3 (empty) commit
    [EOF]
    ");
}

#[test]
fn test_bookmark_move_matching() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a1", "a2"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-mhead1"]).success();
    test_env.run_jj_in(&repo_path, ["new", "root()"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b1"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "c1"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-mhead2"]).success();
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   a2781dd9ee37
    ○  c1 f4f38657a3dd
    ○  b1 f652c32197cf
    │ ○   6b5e840ea72b
    │ ○  a1 a2 230dd059e1b0
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // The default could be considered "--from=all() glob:*", but is disabled
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--from <REVSETS>|NAMES>

    Usage: jj bookmark move --to <REVSET> <--from <REVSETS>|NAMES>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // No bookmarks pointing to the source revisions
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "--from=none()", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to update.
    [EOF]
    ");

    // No matching bookmarks within the source revisions
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "move", "--from=::@", "glob:a?", "--to=@"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching bookmarks for patterns: a?
    [EOF]
    [exit status: 1]
    ");

    // Noop move
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "--to=a1", "a2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No bookmarks to update.
    [EOF]
    ");

    // Move from multiple revisions
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "--from=::@", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 2 bookmarks to vruxwmqv a2781dd9 b1 c1 | (empty) head2
    Hint: Specify bookmark by name to update just one of the bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  b1 c1 a2781dd9ee37
    ○   f4f38657a3dd
    ○   f652c32197cf
    │ ○   6b5e840ea72b
    │ ○  a1 a2 230dd059e1b0
    ├─╯
    ◆   000000000000
    [EOF]
    ");
    test_env.run_jj_in(&repo_path, ["undo"]).success();

    // Try to move multiple bookmarks, but one of them isn't fast-forward
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "glob:?1", "--to=@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: a1
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   a2781dd9ee37
    ○  c1 f4f38657a3dd
    ○  b1 f652c32197cf
    │ ○   6b5e840ea72b
    │ ○  a1 a2 230dd059e1b0
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Select by revision and name
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "move", "--from=::a1+", "--to=a1+", "glob:?1"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kkmpptxz 6b5e840e a1 | (empty) head1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   a2781dd9ee37
    ○  c1 f4f38657a3dd
    ○  b1 f652c32197cf
    │ ○  a1 6b5e840ea72b
    │ ○  a2 230dd059e1b0
    ├─╯
    ◆   000000000000
    [EOF]
    ");
}

#[test]
fn test_bookmark_move_conflicting() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let get_log = || {
        let template = r#"separate(" ", description.first_line(), bookmarks)"#;
        test_env.run_jj_in(&repo_path, ["log", "-T", template])
    };

    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-mA0"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-mB0"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-mC0"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "description(A0)", "-mA1"])
        .success();

    // Set up conflicting bookmark.
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "create", "-rdescription(A0)", "foo"],
        )
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            [
                "bookmark",
                "create",
                "--at-op=@-",
                "-rdescription(B0)",
                "foo",
            ],
        )
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
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-rdescription(C0)", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to move bookmark backwards or sideways: foo
    Hint: Use --allow-backwards to allow it.
    [EOF]
    [exit status: 1]
    ");

    // Can move the bookmark to A1 since it's descendant of A0. It's not
    // descendant of B0, though.
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "-rdescription(A1)", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to mzvwutvl 9328d344 foo | (empty) A1
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
    let repo_path = test_env.env_root().join("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "bnoexist", "blocal"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: bnoexist
    [EOF]
    [exit status: 1]
    ");

    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit-0"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "blocal"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "blocal", "blocal1"]);
    insta::assert_snapshot!(output, @"");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit-1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bexist"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "blocal1", "bexist"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Bookmark already exists: bexist
    [EOF]
    [exit status: 1]
    ");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit-2"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bremote"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["git", "push", "--allow-new", "-b=bremote"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "bremote", "bremote2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Tracked remote bookmarks for bookmark bremote were not renamed.
    Hint: To rename the bookmark on the remote, you can `jj git push --bookmark bremote` first (to delete it on the remote), and then `jj git push --bookmark bremote2`. `jj git push --all` would also be sufficient.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "bremote2", "bremote"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Tracked remote bookmarks for bookmark bremote exist.
    Hint: Run `jj bookmark untrack 'glob:bremote@*'` to disassociate them.
    [EOF]
    ");
}

#[test]
fn test_bookmark_rename_colocated() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "repo", "--colocate"])
        .success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit-0"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "blocal"])
        .success();

    // Make sure that git tracking bookmarks don't cause a warning
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "rename", "blocal", "blocal1"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_bookmark_forget_glob() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar-2"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-3"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-4"])
        .success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 foo-1 foo-3 foo-4 230dd059e1b0
    ◆   000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "forget", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 2 local bookmarks.
    [EOF]
    ");
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "forget", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 2 local bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 foo-4 230dd059e1b0
    ◆   000000000000
    [EOF]
    ");

    // Forgetting a bookmark via both explicit name and glob pattern, or with
    // multiple glob patterns, shouldn't produce an error.
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "forget", "foo-4", "glob:foo-*", "glob:foo-*"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 230dd059e1b0
    ◆   000000000000
    [EOF]
    ");

    // Malformed glob
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "forget", "glob:foo-[1-3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'glob:foo-[1-3' for '<NAMES>...': Pattern syntax error near position 4: invalid range pattern

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // We get an error if none of the globs match anything
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "forget", "glob:bar*", "glob:baz*", "glob:boom*"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching bookmarks for patterns: baz*, boom*
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bookmark_delete_glob() {
    // Set up a git repo with a bookmark and a jj repo that has it as a remote.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
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
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar-2"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-3"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo-4"])
        .success();
    // Push to create remote-tracking bookmarks
    test_env
        .run_jj_in(&repo_path, ["git", "push", "--all"])
        .success();

    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 foo-1 foo-3 foo-4 312a98d6f27b
    ◆   000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 2 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 foo-1@origin foo-3@origin foo-4 312a98d6f27b
    ◆   000000000000
    [EOF]
    ");

    // We get an error if none of the globs match live bookmarks. Unlike `jj
    // bookmark forget`, it's not allowed to delete already deleted bookmarks.
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "delete", "glob:foo-[1-3]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No matching bookmarks for patterns: foo-[1-3]
    [EOF]
    [exit status: 1]
    ");

    // Deleting a bookmark via both explicit name and glob pattern, or with
    // multiple glob patterns, shouldn't produce an error.
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "delete", "foo-4", "glob:foo-*", "glob:foo-*"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 1 bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @  bar-2 foo-1@origin foo-3@origin foo-4@origin 312a98d6f27b
    ◆   000000000000
    [EOF]
    ");

    // The deleted bookmarks are still there
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    bar-2: qpvuntsm 312a98d6 (empty) commit
      @origin: qpvuntsm 312a98d6 (empty) commit
    foo-1 (deleted)
      @origin: qpvuntsm 312a98d6 (empty) commit
    foo-3 (deleted)
      @origin: qpvuntsm 312a98d6 (empty) commit
    foo-4 (deleted)
      @origin: qpvuntsm 312a98d6 (empty) commit
    [EOF]
    ");

    // Malformed glob
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "delete", "glob:foo-[1-3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'glob:foo-[1-3' for '<NAMES>...': Pattern syntax error near position 4: invalid range pattern

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Unknown pattern kind
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "forget", "whatever:bookmark"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'whatever:bookmark' for '<NAMES>...': Invalid string pattern kind `whatever:`

    For more information, try '--help'.
    Hint: Try prefixing with one of `exact:`, `glob:`, `regex:`, `sub:`, or one of these with `-i` added (e.g. `glob-i:`) for case-insensitive matching
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_bookmark_delete_export() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"])
        .success();
    test_env.run_jj_in(&repo_path, ["git", "export"]).success();

    test_env
        .run_jj_in(&repo_path, ["bookmark", "delete", "foo"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    foo (deleted)
      @git: rlvkpnrz 65b6b74e (empty) (no description set)
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be deleted from the underlying Git repo on the next `jj git export`.
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["git", "export"]).success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
}

#[test]
fn test_bookmark_forget_export() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    foo: rlvkpnrz 65b6b74e (empty) (no description set)
    [EOF]
    ");

    // Exporting the bookmark to git creates a local-git tracking bookmark
    let output = test_env.run_jj_in(&repo_path, ["git", "export"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "forget", "--include-remotes", "foo"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");
    // Forgetting a bookmark with --include-remotes deletes local and
    // remote-tracking bookmarks including the corresponding git-tracking bookmark.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    let output = test_env.run_jj_in(&repo_path, ["log", "-r=foo", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `foo` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    // `jj git export` will delete the bookmark from git. In a colocated repo,
    // this will happen automatically immediately after a `jj bookmark forget`.
    // This is demonstrated in `test_git_colocated_bookmark_forget` in
    // test_git_colocated.rs
    let output = test_env.run_jj_in(&repo_path, ["git", "export"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
}

#[test]
fn test_bookmark_forget_fetched_bookmark() {
    // Much of this test is borrowed from `test_git_fetch_remote_only_bookmark` in
    // test_git_fetch.rs

    // Set up a git repo with a bookmark and a jj repo that has it as a remote.
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init_bare(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
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
    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // TEST 1: with export-import
    // Forget the bookmark with --include-remotes
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "forget", "--include-remotes", "feature1"],
        )
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");

    // At this point `jj git export && jj git import` does *not* recreate the
    // bookmark. This behavior is important in colocated repos, as otherwise a
    // forgotten bookmark would be immediately resurrected.
    //
    // Technically, this is because `jj bookmark forget` preserved
    // the ref in jj view's `git_refs` tracking the local git repo's remote-tracking
    // bookmark.
    // TODO: Show that jj git push is also a no-op
    let output = test_env.run_jj_in(&repo_path, ["git", "export"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");

    // We can fetch feature1 again.
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // TEST 2: No export/import (otherwise the same as test 1)
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "forget", "--include-remotes", "feature1"],
        )
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    // Fetch works even without the export-import
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
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
    let output = test_env.run_jj_in(
        &repo_path,
        ["bookmark", "forget", "--include-remotes", "feature1"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");

    // Fetching a moved bookmark does not create a conflict
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: tyvxnvqr 9175cb32 (empty) another message
      @origin: tyvxnvqr 9175cb32 (empty) another message
    [EOF]
    ");

    // TEST 4: If `--include-remotes` isn't used, remote bookmarks are untracked
    test_env
        .run_jj_in(&repo_path, ["bookmark", "forget", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1@origin: tyvxnvqr 9175cb32 (empty) another message
    [EOF]
    ");
    // There should be no output here since the remote bookmark wasn't forgotten
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
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
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
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
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    // Fetch and then delete the bookmark
    test_env
        .run_jj_in(&repo_path, ["git", "fetch", "--remote=origin"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "delete", "feature1"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1 (deleted)
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // ============ End of test setup ============

    // We can forget a deleted bookmark
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "forget", "--include-remotes", "feature1"],
        )
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");

    // Can't forget a non-existent bookmark
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "forget", "i_do_not_exist"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No such bookmark: i_do_not_exist
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bookmark_track_untrack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
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
    test_env.add_config("git.auto-local-bookmark = false");
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] untracked
    bookmark: feature2@origin [new] untracked
    bookmark: main@origin     [new] untracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1@origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main@origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   230dd059e1b0
    │ ◆  feature1@origin feature2@origin main@origin bd843888ee66
    ├─╯
    ◆   000000000000
    [EOF]
    ");

    // Track new bookmark. Local bookmark should be created.
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "track", "feature1@origin", "main@origin"],
        )
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");

    // Track existing bookmark. Local bookmark should result in conflict.
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "feature2"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "track", "feature2@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    feature2 (conflicted):
      + qpvuntsm 230dd059 (empty) (no description set)
      + qxxqrkql bd843888 commit 1
      @origin (behind by 1 commits): qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");

    // Untrack existing and locally-deleted bookmarks. Bookmark targets should be
    // unchanged
    test_env
        .run_jj_in(&repo_path, ["bookmark", "delete", "feature2"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "untrack", "feature1@origin", "feature2@origin"],
        )
        .success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: qxxqrkql bd843888 commit 1
    feature2@origin: qxxqrkql bd843888 commit 1
    main: qxxqrkql bd843888 commit 1
      @origin: qxxqrkql bd843888 commit 1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   230dd059e1b0
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
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [updated] untracked
    bookmark: feature2@origin [updated] untracked
    bookmark: main@origin     [updated] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: psynomvr 48ec79a4 commit 2
    feature2@origin: psynomvr 48ec79a4 commit 2
    main: psynomvr 48ec79a4 commit 2
      @origin: psynomvr 48ec79a4 commit 2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   230dd059e1b0
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
    test_env.add_config("git.auto-local-bookmark = true");
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [updated] untracked
    bookmark: feature2@origin [updated] untracked
    bookmark: feature3@origin [new] tracked
    bookmark: main@origin     [updated] tracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: qxxqrkql bd843888 commit 1
    feature1@origin: yumopmsr d8cd3e02 commit 3
    feature2@origin: yumopmsr d8cd3e02 commit 3
    feature3: yumopmsr d8cd3e02 commit 3
      @origin: yumopmsr d8cd3e02 commit 3
    main: yumopmsr d8cd3e02 commit 3
      @origin: yumopmsr d8cd3e02 commit 3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r"
    @   230dd059e1b0
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
    let repo_path = test_env.env_root().join("repo");

    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m", "a"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["git", "push", "--allow-new", "-b", "main"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "untrack", "main@origin"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["describe", "-m", "b", "-r", "main", "--ignore-immutable"],
        )
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "track", "main@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    main (conflicted):
      + qpvuntsm?? e802c4f8 (empty) b
      + qpvuntsm?? 427890ea (empty) a
      @origin (behind by 1 commits): qpvuntsm?? 427890ea (empty) a
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack_patterns() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = git::init(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    // Create remote commit
    create_commit_with_refs(
        &git_repo,
        "commit",
        b"content",
        &["refs/heads/feature1", "refs/heads/feature2"],
    );

    // Fetch new commit without auto tracking
    test_env.add_config("git.auto-local-bookmark = false");
    let output = test_env.run_jj_in(&repo_path, ["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: feature1@origin [new] untracked
    bookmark: feature2@origin [new] untracked
    [EOF]
    ");

    // Track local bookmark
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();
    insta::assert_snapshot!(
        test_env.run_jj_in(&repo_path, ["bookmark", "track", "main"]), @r"
    ------- stderr -------
    error: invalid value 'main' for '<BOOKMARK@REMOTE>...': remote bookmark must be specified in bookmark@remote form

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Track/untrack unknown bookmark
    insta::assert_snapshot!(
        test_env.run_jj_in(&repo_path, ["bookmark", "track", "main@origin"]), @r"
    ------- stderr -------
    Error: No such remote bookmark: main@origin
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(
        test_env.run_jj_in(&repo_path, ["bookmark", "untrack", "main@origin"]), @r"
    ------- stderr -------
    Error: No such remote bookmark: main@origin
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(
        test_env.run_jj_in(&repo_path, ["bookmark", "track", "glob:maine@*"]), @r"
    ------- stderr -------
    Error: No matching remote bookmarks for patterns: maine@*
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(
        test_env.run_jj_in(
            &repo_path,
            ["bookmark", "untrack", "main@origin", "glob:main@o*"],
        ), @r"
    ------- stderr -------
    Error: No matching remote bookmarks for patterns: main@origin, main@o*
    [EOF]
    [exit status: 1]
    ");

    // Track already tracked bookmark
    test_env
        .run_jj_in(&repo_path, ["bookmark", "track", "feature1@origin"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "track", "feature1@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark already tracked: feature1@origin
    Nothing changed.
    [EOF]
    ");

    // Untrack non-tracking bookmark
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "untrack", "feature2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark not tracked yet: feature2@origin
    Nothing changed.
    [EOF]
    ");

    // Untrack Git-tracking bookmark
    test_env.run_jj_in(&repo_path, ["git", "export"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "untrack", "main@git"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Git-tracking bookmark cannot be untracked: main@git
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    feature2@origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");

    // Untrack by pattern
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "untrack", "glob:*@*"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Git-tracking bookmark cannot be untracked: feature1@git
    Warning: Remote bookmark not tracked yet: feature2@origin
    Warning: Git-tracking bookmark cannot be untracked: main@git
    Stopped tracking 1 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
    feature1@origin: yrnqsqlx 41e7a49d commit
    feature2@origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");

    // Track by pattern
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "track", "glob:feature?@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 2 remote bookmarks.
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    feature1: yrnqsqlx 41e7a49d commit
      @git: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    feature2: yrnqsqlx 41e7a49d commit
      @origin: yrnqsqlx 41e7a49d commit
    main: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_list() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_path = test_env.env_root().join("remote");
    for bookmark in [
        "remote-sync",
        "remote-unsync",
        "remote-untrack",
        "remote-delete",
    ] {
        test_env
            .run_jj_in(&remote_path, ["new", "root()", "-m", bookmark])
            .success();
        test_env
            .run_jj_in(&remote_path, ["bookmark", "create", "-r@", bookmark])
            .success();
    }
    test_env.run_jj_in(&remote_path, ["new"]).success();
    test_env
        .run_jj_in(&remote_path, ["git", "export"])
        .success();

    // Initialize local refs
    let mut remote_git_path = remote_path;
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_path = test_env.env_root().join("local");
    test_env
        .run_jj_in(&local_path, ["new", "root()", "-m", "local-only"])
        .success();
    test_env
        .run_jj_in(&local_path, ["bookmark", "create", "-r@", "local-only"])
        .success();

    // Mutate refs in local repository
    test_env
        .run_jj_in(&local_path, ["bookmark", "delete", "remote-delete"])
        .success();
    test_env
        .run_jj_in(&local_path, ["bookmark", "delete", "remote-untrack"])
        .success();
    test_env
        .run_jj_in(
            &local_path,
            ["bookmark", "untrack", "remote-untrack@origin"],
        )
        .success();
    test_env
        .run_jj_in(
            &local_path,
            [
                "bookmark",
                "set",
                "--allow-backwards",
                "--to=@",
                "remote-unsync",
            ],
        )
        .success();

    // Synchronized tracking remotes and non-tracking remotes aren't listed by
    // default
    let output = test_env.run_jj_in(&local_path, ["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    local-only: wqnwkozp 4e887f78 (empty) local-only
    remote-delete (deleted)
      @origin: mnmymoky 203e60eb (empty) remote-delete
    remote-sync: zwtyzrop c761c7ea (empty) remote-sync
    remote-unsync: wqnwkozp 4e887f78 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    let output = test_env.run_jj_in(&local_path, ["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    local-only: wqnwkozp 4e887f78 (empty) local-only
    remote-delete (deleted)
      @origin: mnmymoky 203e60eb (empty) remote-delete
    remote-sync: zwtyzrop c761c7ea (empty) remote-sync
      @origin: zwtyzrop c761c7ea (empty) remote-sync
    remote-unsync: wqnwkozp 4e887f78 (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    remote-untrack@origin: vmortlor 71a16b05 (empty) remote-untrack
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
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
    let output = test_env.run_jj_in(
        &local_path,
        ["bookmark", "list", "--all-remotes", "-T", template],
    );
    insta::assert_snapshot!(output, @r"
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
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_filtered() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_path = test_env.env_root().join("remote");
    for bookmark in ["remote-keep", "remote-delete", "remote-rewrite"] {
        test_env
            .run_jj_in(&remote_path, ["new", "root()", "-m", bookmark])
            .success();
        test_env
            .run_jj_in(&remote_path, ["bookmark", "create", "-r@", bookmark])
            .success();
    }
    test_env.run_jj_in(&remote_path, ["new"]).success();
    test_env
        .run_jj_in(&remote_path, ["git", "export"])
        .success();

    // Initialize local refs
    let mut remote_git_path = remote_path;
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_path = test_env.env_root().join("local");
    test_env
        .run_jj_in(&local_path, ["new", "root()", "-m", "local-keep"])
        .success();
    test_env
        .run_jj_in(&local_path, ["bookmark", "create", "-r@", "local-keep"])
        .success();

    // Mutate refs in local repository
    test_env
        .run_jj_in(&local_path, ["bookmark", "delete", "remote-delete"])
        .success();
    test_env
        .run_jj_in(&local_path, ["describe", "-mrewritten", "remote-rewrite"])
        .success();

    let template = r#"separate(" ", commit_id.short(), bookmarks, if(hidden, "(hidden)"))"#;
    insta::assert_snapshot!(
        test_env.run_jj_in(
            &local_path,
            ["log", "-r::(bookmarks() | remote_bookmarks())", "-T", template],
        ),
        @r"
    @  c7b4c09cd77c local-keep
    │ ○  e31634b64294 remote-rewrite*
    ├─╯
    │ ○  3e9a5af6ef15 remote-rewrite@origin (hidden)
    ├─╯
    │ ○  dad5f298ca57 remote-delete@origin
    ├─╯
    │ ○  911e912015fb remote-keep
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // All bookmarks are listed by default.
    let output = test_env.run_jj_in(&local_path, ["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    remote-delete (deleted)
      @origin: yxusvupt dad5f298 (empty) remote-delete
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    let query = |args: &[&str]| {
        test_env.run_jj_with(|cmd| {
            cmd.current_dir(&local_path)
                .args(["bookmark", "list"])
                .args(args)
        })
    };

    // "all()" doesn't include deleted bookmarks since they have no local targets.
    // So "all()" is identical to "bookmarks()".
    insta::assert_snapshot!(query(&["-rall()"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");

    // Exclude remote-only bookmarks. "remote-rewrite@origin" is included since
    // local "remote-rewrite" target matches.
    insta::assert_snapshot!(query(&["-rbookmarks()"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks by name.
    insta::assert_snapshot!(query(&["remote-rewrite"]), @r"
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");
    insta::assert_snapshot!(query(&["-rbookmarks(remote-rewrite)"]), @r"
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks by name, combined with --all-remotes
    test_env.run_jj_in(&local_path, ["git", "export"]).success();
    insta::assert_snapshot!(query(&["--all-remotes", "remote-rewrite"]), @r"
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @git: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--all-remotes", "-rbookmarks(remote-rewrite)"]), @r"
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @git: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");

    // Select bookmarks with --remote
    insta::assert_snapshot!(query(&["--remote", "origin"]), @r"
    remote-delete (deleted)
      @origin: yxusvupt dad5f298 (empty) remote-delete
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
      @origin: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--remote", "glob:gi?"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
      @git: kpqxywon c7b4c09c (empty) local-keep
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
      @git: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @git: xyxluytn e31634b6 (empty) rewritten
    [EOF]
    ");
    insta::assert_snapshot!(query(&["--remote", "origin", "--remote", "git"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
      @git: kpqxywon c7b4c09c (empty) local-keep
    remote-delete (deleted)
      @origin: yxusvupt dad5f298 (empty) remote-delete
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
      @git: nlwprzpn 911e9120 (empty) remote-keep
      @origin: nlwprzpn 911e9120 (empty) remote-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @git: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    // Can select deleted bookmark by name pattern, but not by revset.
    insta::assert_snapshot!(query(&["remote-delete"]), @r"
    remote-delete (deleted)
      @origin: yxusvupt dad5f298 (empty) remote-delete
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");
    insta::assert_snapshot!(query(&["-rbookmarks(remote-delete)"]), @"");
    insta::assert_snapshot!(query(&["-rremote-delete"]), @r"
    ------- stderr -------
    Error: Revision `remote-delete` doesn't exist
    Hint: Did you mean `remote-delete@origin`, `remote-keep`, `remote-rewrite`, `remote-rewrite@origin`?
    [EOF]
    [exit status: 1]
    ");

    // Name patterns are OR-ed.
    insta::assert_snapshot!(query(&["glob:*-keep", "remote-delete"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    remote-delete (deleted)
      @origin: yxusvupt dad5f298 (empty) remote-delete
    remote-keep: nlwprzpn 911e9120 (empty) remote-keep
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    // Unmatched name pattern shouldn't be an error. A warning can be added later.
    insta::assert_snapshot!(query(&["local-keep", "glob:push-*"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    [EOF]
    ");

    // Name pattern and revset are OR-ed.
    insta::assert_snapshot!(query(&["local-keep", "-rbookmarks(remote-rewrite)"]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @origin (ahead by 1 commits, behind by 1 commits): xyxluytn hidden 3e9a5af6 (empty) remote-rewrite
    [EOF]
    ");

    // … but still filtered by --remote
    insta::assert_snapshot!(query(&[
        "local-keep",
        "-rbookmarks(remote-rewrite)",
        "--remote",
        "git",
    ]), @r"
    local-keep: kpqxywon c7b4c09c (empty) local-keep
      @git: kpqxywon c7b4c09c (empty) local-keep
    remote-rewrite: xyxluytn e31634b6 (empty) rewritten
      @git: xyxluytn e31634b6 (empty) rewritten
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_much_remote_divergence() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_path = test_env.env_root().join("remote");
    test_env
        .run_jj_in(&remote_path, ["new", "root()", "-m", "remote-unsync"])
        .success();
    for _ in 0..15 {
        test_env
            .run_jj_in(&remote_path, ["new", "-m", "remote-unsync"])
            .success();
    }
    test_env
        .run_jj_in(&remote_path, ["bookmark", "create", "-r@", "remote-unsync"])
        .success();
    test_env.run_jj_in(&remote_path, ["new"]).success();
    test_env
        .run_jj_in(&remote_path, ["git", "export"])
        .success();

    // Initialize local refs
    let mut remote_git_path = remote_path;
    remote_git_path.extend([".jj", "repo", "store", "git"]);
    test_env
        .run_jj_in(
            ".",
            ["git", "clone", remote_git_path.to_str().unwrap(), "local"],
        )
        .success();
    let local_path = test_env.env_root().join("local");
    test_env
        .run_jj_in(&local_path, ["new", "root()", "-m", "local-only"])
        .success();
    for _ in 0..15 {
        test_env
            .run_jj_in(&local_path, ["new", "-m", "local-only"])
            .success();
    }
    test_env
        .run_jj_in(&local_path, ["bookmark", "create", "-r@", "local-only"])
        .success();

    // Mutate refs in local repository
    test_env
        .run_jj_in(
            &local_path,
            [
                "bookmark",
                "set",
                "--allow-backwards",
                "--to=@",
                "remote-unsync",
            ],
        )
        .success();

    let output = test_env.run_jj_in(&local_path, ["bookmark", "list"]);
    insta::assert_snapshot!(output, @r"
    local-only: zkyosouw 4ab3f751 (empty) local-only
    remote-unsync: zkyosouw 4ab3f751 (empty) local-only
      @origin (ahead by at least 10 commits, behind by at least 10 commits): lxyktnks 19582022 (empty) remote-unsync
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_tracked() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");

    // Initialize remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_path = test_env.env_root().join("remote");
    for bookmark in [
        "remote-sync",
        "remote-unsync",
        "remote-untrack",
        "remote-delete",
    ] {
        test_env
            .run_jj_in(&remote_path, ["new", "root()", "-m", bookmark])
            .success();
        test_env
            .run_jj_in(&remote_path, ["bookmark", "create", "-r@", bookmark])
            .success();
    }
    test_env.run_jj_in(&remote_path, ["new"]).success();
    test_env
        .run_jj_in(&remote_path, ["git", "export"])
        .success();

    // Initialize local refs
    let mut remote_git_path = test_env.env_root().join("remote");
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
    let mut upstream_git_path = test_env.env_root().join("upstream");
    test_env
        .run_jj_in(&upstream_git_path, ["new", "root()", "-m", "upstream-sync"])
        .success();
    test_env
        .run_jj_in(
            &upstream_git_path,
            ["bookmark", "create", "-r@", "upstream-sync"],
        )
        .success();
    test_env.run_jj_in(&upstream_git_path, ["new"]).success();
    test_env
        .run_jj_in(&upstream_git_path, ["git", "export"])
        .success();

    upstream_git_path.extend([".jj", "repo", "store", "git"]);

    let local_path = test_env.env_root().join("local");

    test_env
        .run_jj_in(
            &local_path,
            [
                "git",
                "remote",
                "add",
                "upstream",
                upstream_git_path.to_str().unwrap(),
            ],
        )
        .success();
    test_env
        .run_jj_in(&local_path, ["git", "fetch", "--all-remotes"])
        .success();

    test_env
        .run_jj_in(&local_path, ["new", "root()", "-m", "local-only"])
        .success();
    test_env
        .run_jj_in(&local_path, ["bookmark", "create", "-r@", "local-only"])
        .success();

    // Mutate refs in local repository
    test_env
        .run_jj_in(&local_path, ["bookmark", "delete", "remote-delete"])
        .success();
    test_env
        .run_jj_in(&local_path, ["bookmark", "delete", "remote-untrack"])
        .success();
    test_env
        .run_jj_in(
            &local_path,
            ["bookmark", "untrack", "remote-untrack@origin"],
        )
        .success();
    test_env
        .run_jj_in(
            &local_path,
            [
                "git",
                "push",
                "--allow-new",
                "--remote",
                "upstream",
                "--bookmark",
                "remote-unsync",
            ],
        )
        .success();
    test_env
        .run_jj_in(
            &local_path,
            [
                "bookmark",
                "set",
                "--to=@",
                "--allow-backwards",
                "remote-unsync",
            ],
        )
        .success();

    let output = test_env.run_jj_in(&local_path, ["bookmark", "list", "--all-remotes"]);
    insta::assert_snapshot!(output, @r"
    local-only: nmzmmopx e1da745b (empty) local-only
      @git: nmzmmopx e1da745b (empty) local-only
    remote-delete (deleted)
      @origin: mnmymoky 203e60eb (empty) remote-delete
    remote-sync: zwtyzrop c761c7ea (empty) remote-sync
      @git: zwtyzrop c761c7ea (empty) remote-sync
      @origin: zwtyzrop c761c7ea (empty) remote-sync
    remote-unsync: nmzmmopx e1da745b (empty) local-only
      @git: nmzmmopx e1da745b (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    remote-untrack@origin: vmortlor 71a16b05 (empty) remote-untrack
    upstream-sync: lolpmnqw 32fa6da0 (empty) upstream-sync
      @git: lolpmnqw 32fa6da0 (empty) upstream-sync
      @upstream: lolpmnqw 32fa6da0 (empty) upstream-sync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    let output = test_env.run_jj_in(&local_path, ["bookmark", "list", "--tracked"]);
    insta::assert_snapshot!(output, @r"
    remote-delete (deleted)
      @origin: mnmymoky 203e60eb (empty) remote-delete
    remote-sync: zwtyzrop c761c7ea (empty) remote-sync
      @origin: zwtyzrop c761c7ea (empty) remote-sync
    remote-unsync: nmzmmopx e1da745b (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    upstream-sync: lolpmnqw 32fa6da0 (empty) upstream-sync
      @upstream: lolpmnqw 32fa6da0 (empty) upstream-sync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    let output = test_env.run_jj_in(
        &local_path,
        ["bookmark", "list", "--tracked", "--remote", "origin"],
    );
    insta::assert_snapshot!(output, @r"
    remote-delete (deleted)
      @origin: mnmymoky 203e60eb (empty) remote-delete
    remote-sync: zwtyzrop c761c7ea (empty) remote-sync
      @origin: zwtyzrop c761c7ea (empty) remote-sync
    remote-unsync: nmzmmopx e1da745b (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    [EOF]
    ------- stderr -------
    Hint: Bookmarks marked as deleted will be *deleted permanently* on the remote on the next `jj git push`. Use `jj bookmark forget` to prevent this.
    [EOF]
    ");

    let output = test_env.run_jj_in(
        &local_path,
        ["bookmark", "list", "--tracked", "remote-unsync"],
    );
    insta::assert_snapshot!(output, @r"
    remote-unsync: nmzmmopx e1da745b (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
      @upstream (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    [EOF]
    ");

    let output = test_env.run_jj_in(
        &local_path,
        ["bookmark", "list", "--tracked", "remote-untrack"],
    );
    insta::assert_snapshot!(output, @"");

    test_env
        .run_jj_in(
            &local_path,
            ["bookmark", "untrack", "remote-unsync@upstream"],
        )
        .success();

    let output = test_env.run_jj_in(
        &local_path,
        ["bookmark", "list", "--tracked", "remote-unsync"],
    );
    insta::assert_snapshot!(output, @r"
    remote-unsync: nmzmmopx e1da745b (empty) local-only
      @origin (ahead by 1 commits, behind by 1 commits): qpsqxpyq 38ef8af7 (empty) remote-unsync
    [EOF]
    ");
}

#[test]
fn test_bookmark_list_conflicted() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    // Track existing bookmark. Local bookmark should result in conflict.
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "a"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "root()", "-m", "b"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar"])
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            ["bookmark", "create", "foo", "-r", "description(a)"],
        )
        .success();
    test_env
        .run_jj_in(
            &repo_path,
            [
                "bookmark",
                "create",
                "foo",
                "-r",
                "description(b)",
                "--at-op=@-",
            ],
        )
        .success();
    test_env.run_jj_in(&repo_path, ["status"]).success();
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r"
    bar: kkmpptxz 06a973bc (empty) b
    foo (conflicted):
      + rlvkpnrz d8d5f980 (empty) a
      + kkmpptxz 06a973bc (empty) b
    [EOF]
    ");
    insta::assert_snapshot!(test_env.run_jj_in(&repo_path, ["bookmark", "list", "--conflicted"]), @r"
    foo (conflicted):
      + rlvkpnrz d8d5f980 (empty) a
      + kkmpptxz 06a973bc (empty) b
    [EOF]
    ");
}

#[test]
fn test_bookmark_create_with_default_target_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision was not specified, defaulting to the working copy (-r@). In the near future it will be required to explicitly specify target revision.
    Created 1 bookmarks pointing to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_set_with_default_target_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision was not specified, defaulting to the working copy (--revision=@). In the near future it will be required to explicitly specify target revision.
    Created 1 bookmarks pointing to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_bookmark_move_with_default_target_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    // Set up remote
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(
            &repo_path,
            ["git", "remote", "add", "origin", "../git-repo"],
        )
        .success();

    let output = test_env.run_jj_in(&repo_path, ["bookmark", "create", "foo", "-r@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to qpvuntsm 230dd059 foo | (empty) (no description set)
    [EOF]
    ");

    test_env.run_jj_in(&repo_path, ["new"]).success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "move", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision was not specified, defaulting to the working copy (--to=@). In the near future it will be required to explicitly specify it.
    Moved 1 bookmarks to zsuskuln 8bb159bc foo | (empty) (no description set)
    [EOF]
    ");
}

#[must_use]
fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> CommandOutput {
    let template = r#"bookmarks ++ " " ++ commit_id.short()"#;
    test_env.run_jj_in(cwd, ["log", "-T", template])
}

#[must_use]
fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    test_env.run_jj_in(repo_path, ["bookmark", "list", "--all-remotes", "--quiet"])
}
