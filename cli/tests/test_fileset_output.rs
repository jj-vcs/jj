// Copyright 2026 The Jujutsu Authors
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

use indoc::indoc;

use crate::common::TestEnvironment;

#[test]
fn test_alias() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "");
    work_dir.write_file("file2", "");

    test_env.add_config(indoc! {"
        [fileset-aliases]
        'star' = '*'
        'syntax-error' = 'whatever &'
        'recurse' = 'recurse1'
        'recurse1' = 'recurse2()'
        'recurse2()' = 'recurse'
        'identity(x)' = 'x'
        'not:x' = '~x'
    "});
    let query = |arg: &str| work_dir.run_jj(["file", "list", arg]);

    insta::assert_snapshot!(query("star"), @"
    file1
    file2
    [EOF]
    ");

    insta::assert_snapshot!(query("identity(file1)"), @"
    file1
    [EOF]
    ");

    insta::assert_snapshot!(query("not:file1"), @"
    file2
    [EOF]
    ");

    insta::assert_snapshot!(query("file1 | syntax-error"), @"
    ------- stderr -------
    Error: Failed to parse fileset: In alias `syntax-error`
    Caused by:
    1:  --> 1:9
      |
    1 | file1 | syntax-error
      |         ^----------^
      |
      = In alias `syntax-error`
    2:  --> 1:11
      |
    1 | whatever &
      |           ^---
      |
      = expected `~` or <primary>
    [EOF]
    [exit status: 1]
    ");

    insta::assert_snapshot!(query("identity(unknown:pat)"), @"
    ------- stderr -------
    Error: Failed to parse fileset: In alias `identity(x)`
    Caused by:
    1:  --> 1:1
      |
    1 | identity(unknown:pat)
      | ^-------------------^
      |
      = In alias `identity(x)`
    2:  --> 1:1
      |
    1 | x
      | ^
      |
      = In function parameter `x`
    3:  --> 1:10
      |
    1 | identity(unknown:pat)
      |          ^---------^
      |
      = Invalid file pattern
    4: Invalid file pattern kind `unknown:`
    [EOF]
    [exit status: 1]
    ");

    insta::assert_snapshot!(query("file1 & recurse"), @"
    ------- stderr -------
    Error: Failed to parse fileset: In alias `recurse`
    Caused by:
    1:  --> 1:9
      |
    1 | file1 & recurse
      |         ^-----^
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
fn test_alias_in_revset_or_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "");

    test_env.add_config(indoc! {"
        [fileset-aliases]
        'star' = '*'
    "});

    let output = work_dir.run_jj(["log", "-rfiles(star)", "--summary"]);
    insta::assert_snapshot!(output, @"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 093c3c96
    │  (no description set)
    ~  A file1
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r@", "-Tself.diff('star').summary()"]);
    insta::assert_snapshot!(output, @"
    @  A file1
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_alias_override() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(indoc! {"
        [fileset-aliases]
        'f(x)' = 'user'
    "});

    // 'f(x)' should be overridden by --config 'f(a)'. If aliases were sorted
    // purely by name, 'f(a)' would come first.
    let output = work_dir.run_jj([
        "file",
        "list",
        "f(_)",
        "--config=fileset-aliases.'f(a)'=arg",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: arg
    [EOF]
    ");
}

#[test]
fn test_bad_alias_decl() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "");

    test_env.add_config(indoc! {r#"
        [fileset-aliases]
        'star' = '*'
        '"bad"' = 'root()'
        'badfn(a, a)' = 'root()'
    "#});

    // Invalid declaration should be warned and ignored.
    let output = work_dir.run_jj(["file", "list", "star"]);
    insta::assert_snapshot!(output, @r#"
    file1
    [EOF]
    ------- stderr -------
    Warning: Failed to load `fileset-aliases."bad"`:  --> 1:1
      |
    1 | "bad"
      | ^---
      |
      = expected <strict_identifier> or <function_name>
    Warning: Failed to load `fileset-aliases.badfn(a, a)`:  --> 1:7
      |
    1 | badfn(a, a)
      |       ^--^
      |
      = Redefinition of function parameter
    [EOF]
    "#);
}
