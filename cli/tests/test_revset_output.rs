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

use crate::common::TestEnvironment;

#[test]
fn test_syntax_error() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-r", ":x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: `:` is not a prefix operator
    Caused by:  --> 1:1
      |
    1 | :x
      | ^
      |
      = `:` is not a prefix operator
    Hint: Did you mean `::` for ancestors?
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "x &"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Syntax error
    Caused by:  --> 1:4
      |
    1 | x &
      |    ^---
      |
      = expected `::`, `..`, `~`, or <primary>
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "x - y"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: `-` is not an infix operator
    Caused by:  --> 1:3
      |
    1 | x - y
      |   ^
      |
      = `-` is not an infix operator
    Hint: Did you mean `~` for difference?
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "HEAD^"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: `^` is not a postfix operator
    Caused by:  --> 1:5
      |
    1 | HEAD^
      |     ^
      |
      = `^` is not a postfix operator
    Hint: Did you mean `-` for parents?
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bad_function_call() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-r", "all(or::nothing)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `all`: Expected 0 arguments
    Caused by:  --> 1:5
      |
    1 | all(or::nothing)
      |     ^---------^
      |
      = Function `all`: Expected 0 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "parents()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `parents`: Expected 1 to 2 arguments
    Caused by:  --> 1:9
      |
    1 | parents()
      |         ^
      |
      = Function `parents`: Expected 1 to 2 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "parents(foo, bar, baz)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `parents`: Expected 1 to 2 arguments
    Caused by:  --> 1:9
      |
    1 | parents(foo, bar, baz)
      |         ^-----------^
      |
      = Function `parents`: Expected 1 to 2 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "heads(foo, bar)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `heads`: Expected 1 arguments
    Caused by:  --> 1:7
      |
    1 | heads(foo, bar)
      |       ^------^
      |
      = Function `heads`: Expected 1 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "latest(a, not_an_integer)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Expected integer
    Caused by:  --> 1:11
      |
    1 | latest(a, not_an_integer)
      |           ^------------^
      |
      = Expected integer
    [EOF]
    [exit status: 1]
    ");

    // "N to M arguments"
    let output = work_dir.run_jj(["log", "-r", "ancestors()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `ancestors`: Expected 1 to 2 arguments
    Caused by:  --> 1:11
      |
    1 | ancestors()
      |           ^
      |
      = Function `ancestors`: Expected 1 to 2 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "change_id(glob:a)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Expected change ID prefix
    Caused by:  --> 1:11
      |
    1 | change_id(glob:a)
      |           ^----^
      |
      = Expected change ID prefix
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "commit_id(xyzzy)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Invalid commit ID prefix
    Caused by:  --> 1:11
      |
    1 | commit_id(xyzzy)
      |           ^---^
      |
      = Invalid commit ID prefix
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "files(not::a-fileset)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In fileset expression
    Caused by:
    1:  --> 1:7
      |
    1 | files(not::a-fileset)
      |       ^------------^
      |
      = In fileset expression
    2:  --> 1:5
      |
    1 | not::a-fileset
      |     ^---
      |
      = expected <identifier>, <string_literal>, or <raw_string_literal>
    Hint: See https://jj-vcs.github.io/jj/latest/filesets/ or use `jj help -k filesets` for filesets syntax and how to match file paths.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", r#"files(foo:"bar")"#]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse revset: In fileset expression
    Caused by:
    1:  --> 1:7
      |
    1 | files(foo:"bar")
      |       ^-------^
      |
      = In fileset expression
    2:  --> 1:1
      |
    1 | foo:"bar"
      | ^-------^
      |
      = Invalid file pattern
    3: Invalid file pattern kind `foo:`
    Hint: See https://jj-vcs.github.io/jj/latest/filesets/#file-patterns or `jj help -k filesets` for valid prefixes.
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["log", "-r", r#"files("../out")"#]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse revset: In fileset expression
    Caused by:
    1:  --> 1:7
      |
    1 | files("../out")
      |       ^------^
      |
      = In fileset expression
    2:  --> 1:1
      |
    1 | "../out"
      | ^------^
      |
      = Invalid file pattern
    3: Path "../out" is not in the repo "."
    4: Invalid component ".." in repo-relative path "../out"
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["log", "-r", "bookmarks(bad:pattern)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Invalid string pattern
    Caused by:
    1:  --> 1:11
      |
    1 | bookmarks(bad:pattern)
      |           ^---------^
      |
      = Invalid string pattern
    2: Invalid string pattern kind `bad:`
    Hint: Try prefixing with one of `exact:`, `glob:`, `regex:`, `substring:`, or one of these with `-i` suffix added (e.g. `glob-i:`) for case-insensitive matching
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "bookmarks(regex:'(')"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Invalid string pattern
    Caused by:
    1:  --> 1:11
      |
    1 | bookmarks(regex:'(')
      |           ^-------^
      |
      = Invalid string pattern
    2: regex parse error:
        (
        ^
    error: unclosed group
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "root()::whatever()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `whatever` doesn't exist
    Caused by:  --> 1:9
      |
    1 | root()::whatever()
      |         ^------^
      |
      = Function `whatever` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "remote_bookmarks(a, b, remote=c)"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse revset: Function `remote_bookmarks`: Got multiple values for keyword "remote"
    Caused by:  --> 1:24
      |
    1 | remote_bookmarks(a, b, remote=c)
      |                        ^------^
      |
      = Function `remote_bookmarks`: Got multiple values for keyword "remote"
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["log", "-r", "remote_bookmarks(remote=a, b)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `remote_bookmarks`: Positional argument follows keyword argument
    Caused by:  --> 1:28
      |
    1 | remote_bookmarks(remote=a, b)
      |                            ^
      |
      = Function `remote_bookmarks`: Positional argument follows keyword argument
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "remote_bookmarks(=foo)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Syntax error
    Caused by:  --> 1:18
      |
    1 | remote_bookmarks(=foo)
      |                  ^---
      |
      = expected <strict_identifier> or <expression>
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "remote_bookmarks(remote=)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Syntax error
    Caused by:  --> 1:25
      |
    1 | remote_bookmarks(remote=)
      |                         ^---
      |
      = expected <expression>
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_function_name_hint() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let evaluate = |expr| work_dir.run_jj(["log", "-r", expr]);

    test_env.add_config(
        r###"
    [revset-aliases]
    'bookmarks(x)' = 'x' # override builtin function
    'my_author(x)' = 'author(x)' # similar name to builtin function
    'author_sym' = 'x' # not a function alias
    'my_bookmarks' = 'bookmark()' # typo in alias
    "###,
    );

    // The suggestion "bookmarks" shouldn't be duplicated
    insta::assert_snapshot!(evaluate("bookmark()"), @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `bookmark` doesn't exist
    Caused by:  --> 1:1
      |
    1 | bookmark()
      | ^------^
      |
      = Function `bookmark` doesn't exist
    Hint: Did you mean `bookmarks`, `remote_bookmarks`?
    [EOF]
    [exit status: 1]
    ");

    // Both builtin function and function alias should be suggested
    insta::assert_snapshot!(evaluate("author_()"), @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `author_` doesn't exist
    Caused by:  --> 1:1
      |
    1 | author_()
      | ^-----^
      |
      = Function `author_` doesn't exist
    Hint: Did you mean `author`, `author_date`, `author_email`, `author_name`, `my_author`?
    [EOF]
    [exit status: 1]
    ");

    insta::assert_snapshot!(evaluate("my_bookmarks"), @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `my_bookmarks`
    Caused by:
    1:  --> 1:1
      |
    1 | my_bookmarks
      | ^----------^
      |
      = In alias `my_bookmarks`
    2:  --> 1:1
      |
    1 | bookmark()
      | ^------^
      |
      = Function `bookmark` doesn't exist
    Hint: Did you mean `bookmarks`, `remote_bookmarks`?
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bad_symbol_or_argument_should_not_be_optimized_out() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-r", "unknown & none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `unknown` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "all() | unknown"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `unknown` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "description(regex:'(') & none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Invalid string pattern
    Caused by:
    1:  --> 1:13
      |
    1 | description(regex:'(') & none()
      |             ^-------^
      |
      = Invalid string pattern
    2: regex parse error:
        (
        ^
    error: unclosed group
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "files('..') & none()"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse revset: In fileset expression
    Caused by:
    1:  --> 1:7
      |
    1 | files('..') & none()
      |       ^--^
      |
      = In fileset expression
    2:  --> 1:1
      |
    1 | '..'
      | ^--^
      |
      = Invalid file pattern
    3: Path ".." is not in the repo "."
    4: Invalid component ".." in repo-relative path "../"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_alias() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(
        r###"
    [revset-aliases]
    'my-root' = 'root()'
    'syntax-error' = 'whatever &'
    'recurse' = 'recurse1'
    'recurse1' = 'recurse2()'
    'recurse2()' = 'recurse'
    'identity(x)' = 'x'
    'my_author(x)' = 'author(x)'
    "###,
    );

    let output = work_dir.run_jj(["log", "-r", "my-root"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "identity(my-root)"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "root() & syntax-error"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `syntax-error`
    Caused by:
    1:  --> 1:10
      |
    1 | root() & syntax-error
      |          ^----------^
      |
      = In alias `syntax-error`
    2:  --> 1:11
      |
    1 | whatever &
      |           ^---
      |
      = expected `::`, `..`, `~`, or <primary>
    Hint: See https://jj-vcs.github.io/jj/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "identity()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Function `identity`: Expected 1 arguments
    Caused by:  --> 1:10
      |
    1 | identity()
      |          ^
      |
      = Function `identity`: Expected 1 arguments
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "my_author(none())"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `my_author(x)`
    Caused by:
    1:  --> 1:1
      |
    1 | my_author(none())
      | ^---------------^
      |
      = In alias `my_author(x)`
    2:  --> 1:8
      |
    1 | author(x)
      |        ^
      |
      = In function parameter `x`
    3:  --> 1:11
      |
    1 | my_author(none())
      |           ^----^
      |
      = Invalid string expression
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "my_author(unknown:pat)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `my_author(x)`
    Caused by:
    1:  --> 1:1
      |
    1 | my_author(unknown:pat)
      | ^--------------------^
      |
      = In alias `my_author(x)`
    2:  --> 1:8
      |
    1 | author(x)
      |        ^
      |
      = In function parameter `x`
    3:  --> 1:11
      |
    1 | my_author(unknown:pat)
      |           ^---------^
      |
      = Invalid string pattern
    4: Invalid string pattern kind `unknown:`
    Hint: Try prefixing with one of `exact:`, `glob:`, `regex:`, `substring:`, or one of these with `-i` suffix added (e.g. `glob-i:`) for case-insensitive matching
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["log", "-r", "root() & recurse"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `recurse`
    Caused by:
    1:  --> 1:10
      |
    1 | root() & recurse
      |          ^-----^
      |
      = In alias `recurse`
    2:  --> 1:1
      |
    1 | recurse1
      | ^------^
      |
      = In alias `recurse1`
    3:  --> 1:1
      |
    1 | recurse2()
      | ^--------^
      |
      = In alias `recurse2()`
    4:  --> 1:1
      |
    1 | recurse
      | ^-----^
      |
      = Alias `recurse` expanded recursively
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_alias_override() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(
        r###"
    [revset-aliases]
    'f(x)' = 'user'
    "###,
    );

    // 'f(x)' should be overridden by --config 'f(a)'. If aliases were sorted
    // purely by name, 'f(a)' would come first.
    let output = work_dir.run_jj(["log", "-r", "f(_)", "--config=revset-aliases.'f(a)'=arg"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `arg` doesn't exist
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bad_alias_decl() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(
        r#"
    [revset-aliases]
    'my-root' = 'root()'
    '"bad"' = 'root()'
    'badfn(a, a)' = 'root()'
    "#,
    );

    // Invalid declaration should be warned and ignored.
    let output = work_dir.run_jj(["log", "-r", "my-root"]);
    insta::assert_snapshot!(output, @r#"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Warning: Failed to load `revset-aliases."bad"`:  --> 1:1
      |
    1 | "bad"
      | ^---
      |
      = expected <strict_identifier> or <function_name>
    Warning: Failed to load `revset-aliases.badfn(a, a)`:  --> 1:7
      |
    1 | badfn(a, a)
      |       ^--^
      |
      = Redefinition of function parameter
    [EOF]
    "#);
}

#[test]
fn test_all_modifier() {
    let test_env = TestEnvironment::default();
    test_env.add_config("ui.always-allow-large-revsets=false");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Command that accepts single revision by default
    let output = work_dir.run_jj(["new", "all()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `all()` resolved to more than one revision
    Hint: The revset `all()` resolved to these revisions:
      qpvuntsm e8849ae1 (empty) (no description set)
      zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["new", "all:all()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: In revset expression
     --> 1:1
      |
    1 | all:all()
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");

    // Command that accepts multiple revisions by default
    let output = work_dir.run_jj(["log", "-rall:all()"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Warning: In revset expression
     --> 1:1
      |
    1 | all:all()
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    [EOF]
    ");

    // Command that accepts only single revision
    let output = work_dir.run_jj(["bookmark", "create", "-rall:@", "x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: In revset expression
     --> 1:1
      |
    1 | all:@
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to qpvuntsm e8849ae1 x | (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "set", "-rall:all()", "x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: In revset expression
     --> 1:1
      |
    1 | all:all()
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    Error: Revset `all:all()` resolved to more than one revision
    Hint: The revset `all:all()` resolved to these revisions:
      qpvuntsm e8849ae1 x | (empty) (no description set)
      zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    [exit status: 1]
    ");

    // Template expression that accepts multiple revisions by default
    let output = work_dir.run_jj(["log", "-Tself.contained_in('all:all()')"]);
    insta::assert_snapshot!(output, @r"
    @  true
    ◆  true
    [EOF]
    ------- stderr -------
    Warning: In template expression
     --> 1:19
      |
    1 | self.contained_in('all:all()')
      |                   ^---------^
      |
      = In revset expression
     --> 1:1
      |
    1 | all:all()
      | ^-^
      |
      = Multiple revisions are allowed by default; `all:` is planned for removal
    [EOF]
    ");

    // Typo
    let output = work_dir.run_jj(["new", "ale:x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Modifier `ale` doesn't exist
    Caused by:  --> 1:1
      |
    1 | ale:x
      | ^-^
      |
      = Modifier `ale` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    // Modifier shouldn't be allowed in sub expression
    let output = work_dir.run_jj(["new", "x..", "--config=revset-aliases.x='all:@'"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: In alias `x`
    Caused by:
    1:  --> 1:1
      |
    1 | x..
      | ^
      |
      = In alias `x`
    2:  --> 1:1
      |
    1 | all:@
      | ^-^
      |
      = Modifier `all:` is not allowed in sub expression
    [EOF]
    [exit status: 1]
    ");

    // Modifier shouldn't be allowed in a top-level immutable_heads() expression
    let output = work_dir.run_jj([
        "new",
        "--config=revset-aliases.'immutable_heads()'='all:@'",
        "--config=revsets.short-prefixes='none()'",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by:  --> 1:4
      |
    1 | all:@
      |    ^
      |
      = `:` is not an infix operator
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

/// Verifies that the committer_date revset honors the local time zone.
/// This test cannot run on Windows because The TZ env var does not control
/// chrono::Local on that platform.
#[test]
#[cfg(not(target_os = "windows"))]
fn test_revset_committer_date_with_time_zone() {
    // Use these for the test instead of tzdb identifiers like America/New_York
    // because the tz database may not be installed on some build servers
    const NEW_YORK: &str = "EST+5EDT+4,M3.1.0,M11.1.0";
    const CHICAGO: &str = "CST+6CDT+5,M3.1.0,M11.1.0";
    const AUSTRALIA: &str = "AEST-10";
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("TZ", NEW_YORK);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj([
            "--config=debug.commit-timestamp=2023-01-25T11:30:00-05:00",
            "describe",
            "-m",
            "first",
        ])
        .success();
    work_dir
        .run_jj([
            "--config=debug.commit-timestamp=2023-01-25T12:30:00-05:00",
            "new",
            "-m",
            "second",
        ])
        .success();
    work_dir
        .run_jj([
            "--config=debug.commit-timestamp=2023-01-25T13:30:00-05:00",
            "new",
            "-m",
            "third",
        ])
        .success();

    let mut log_commits_before_and_after = |committer_date: &str, now: &str, tz: &str| {
        test_env.add_env_var("TZ", tz);
        let config = format!("debug.commit-timestamp={now}");
        let work_dir = test_env.work_dir("repo");
        let before_log = work_dir.run_jj([
            "--config",
            config.as_str(),
            "log",
            "--no-graph",
            "-T",
            "description.first_line() ++ ' ' ++ committer.timestamp() ++ '\n'",
            "-r",
            format!("committer_date(before:'{committer_date}') ~ root()").as_str(),
        ]);
        let after_log = work_dir.run_jj([
            "--config",
            config.as_str(),
            "log",
            "--no-graph",
            "-T",
            "description.first_line() ++ ' ' ++ committer.timestamp() ++ '\n'",
            "-r",
            format!("committer_date(after:'{committer_date}')").as_str(),
        ]);
        (before_log, after_log)
    };

    let (before_log, after_log) =
        log_commits_before_and_after("2023-01-25 12:00", "2023-02-01T00:00:00-05:00", NEW_YORK);
    insta::assert_snapshot!(before_log, @r"
    first 2023-01-25 11:30:00.000 -05:00
    [EOF]
    ");
    insta::assert_snapshot!(after_log, @r"
    third 2023-01-25 13:30:00.000 -05:00
    second 2023-01-25 12:30:00.000 -05:00
    [EOF]
    ");

    // Switch to DST and ensure we get the same results, because it should
    // evaluate 12:00 on commit date, not the current date
    let (before_log, after_log) =
        log_commits_before_and_after("2023-01-25 12:00", "2023-06-01T00:00:00-04:00", NEW_YORK);
    insta::assert_snapshot!(before_log, @r"
    first 2023-01-25 11:30:00.000 -05:00
    [EOF]
    ");
    insta::assert_snapshot!(after_log, @r"
    third 2023-01-25 13:30:00.000 -05:00
    second 2023-01-25 12:30:00.000 -05:00
    [EOF]
    ");

    // Change the local time zone and ensure the result changes
    let (before_log, after_log) =
        log_commits_before_and_after("2023-01-25 12:00", "2023-06-01T00:00:00-06:00", CHICAGO);
    insta::assert_snapshot!(before_log, @r"
    second 2023-01-25 12:30:00.000 -05:00
    first 2023-01-25 11:30:00.000 -05:00
    [EOF]
    ");
    insta::assert_snapshot!(after_log, @r"
    third 2023-01-25 13:30:00.000 -05:00
    [EOF]
    ");

    // Time zone far outside USA with no DST
    let (before_log, after_log) =
        log_commits_before_and_after("2023-01-26 03:00", "2023-06-01T00:00:00+10:00", AUSTRALIA);
    insta::assert_snapshot!(before_log, @r"
    first 2023-01-25 11:30:00.000 -05:00
    [EOF]
    ");
    insta::assert_snapshot!(after_log, @r"
    third 2023-01-25 13:30:00.000 -05:00
    second 2023-01-25 12:30:00.000 -05:00
    [EOF]
    ");
}
