// Copyright 2024 The Jujutsu Authors
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

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use indoc::formatdoc;
use indoc::indoc;
use jj_lib::file_util::try_symlink;

use crate::common::to_toml_value;
use crate::common::TestEnvironment;

/// Set up a repo where the `jj fix` command uses the fake editor with the given
/// flags.
fn init_with_fake_formatter(args: &[&str]) -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    set_up_fake_formatter(&test_env, args);
    (test_env, repo_path)
}

fn set_up_fake_formatter(test_env: &TestEnvironment, args: &[&str]) {
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    test_env.add_config(formatdoc! {"
        [fix.tools.fake-formatter]
        command = {command}
        patterns = ['all()']
        ",
        command = toml_edit::Value::from_iter(
            [formatter_path.to_str().unwrap()]
                .iter()
                .chain(args)
                .copied()
        )
    });
}

#[test]
fn test_config_no_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "content\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: No `fix.tools` are configured
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    content
    [EOF]
    ");
}

#[test]
fn test_config_multiple_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool-1]
        command = [{formatter}, "--uppercase"]
        patterns = ["foo"]

        [fix.tools.tool-2]
        command = [{formatter}, "--lowercase"]
        patterns = ["bar"]
        "###,
    ));

    std::fs::write(repo_path.join("foo"), "Foo\n").unwrap();
    std::fs::write(repo_path.join("bar"), "Bar\n").unwrap();
    std::fs::write(repo_path.join("baz"), "Baz\n").unwrap();

    test_env.run_jj_in(&repo_path, ["fix"]).success();

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Baz
    [EOF]
    ");
}

#[test]
fn test_config_multiple_tools_with_same_name() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());

    // Multiple definitions with the same `name` are not allowed, because it is
    // likely to be a mistake, and mistakes are risky when they rewrite files.
    test_env.add_config(format!(
        r###"
        [fix.tools.my-tool]
        command = [{formatter}, "--uppercase"]
        patterns = ["foo"]

        [fix.tools.my-tool]
        command = [{formatter}, "--lowercase"]
        patterns = ["bar"]
        "###,
    ));

    std::fs::write(repo_path.join("foo"), "Foo\n").unwrap();
    std::fs::write(repo_path.join("bar"), "Bar\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Configuration cannot be parsed as TOML document
    Caused by: TOML parse error at line 6, column 9
      |
    6 |         [fix.tools.my-tool]
      |         ^
    invalid table header
    duplicate key `my-tool` in table `fix.tools`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    test_env.set_config_path("/dev/null");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Foo
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Bar
    [EOF]
    ");
}

#[test]
fn test_config_disabled_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool-1]
        # default is enabled
        command = [{formatter}, "--uppercase"]
        patterns = ["foo"]

        [fix.tools.tool-2]
        enabled = true
        command = [{formatter}, "--lowercase"]
        patterns = ["bar"]

        [fix.tools.tool-3]
        enabled = false
        command = [{formatter}, "--lowercase"]
        patterns = ["baz"]
        "###
    ));

    std::fs::write(repo_path.join("foo"), "Foo\n").unwrap();
    std::fs::write(repo_path.join("bar"), "Bar\n").unwrap();
    std::fs::write(repo_path.join("baz"), "Baz\n").unwrap();

    test_env.run_jj_in(&repo_path, ["fix"]).success();

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Baz
    [EOF]
    ");
}

#[test]
fn test_config_disabled_tools_warning_when_all_tools_are_disabled() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool-2]
        enabled = false
        command = [{formatter}, "--lowercase"]
        patterns = ["bar"]
        "###
    ));

    std::fs::write(repo_path.join("bar"), "Bar\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: At least one entry of `fix.tools` must be enabled.
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_config_tables_overlapping_patterns() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());

    test_env.add_config(format!(
        r###"
        [fix.tools.tool-1]
        command = [{formatter}, "--append", "tool-1"]
        patterns = ["foo", "bar"]

        [fix.tools.tool-2]
        command = [{formatter}, "--append", "tool-2"]
        patterns = ["bar", "baz"]
        "###,
    ));

    std::fs::write(repo_path.join("foo"), "foo\n").unwrap();
    std::fs::write(repo_path.join("bar"), "bar\n").unwrap();
    std::fs::write(repo_path.join("baz"), "baz\n").unwrap();

    test_env.run_jj_in(&repo_path, ["fix"]).success();

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    tool-1[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    tool-1
    tool-2[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    baz
    tool-2[EOF]
    ");
}

#[test]
fn test_config_tables_all_commands_missing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(
        r###"
        [fix.tools.my-tool-missing-command-1]
        patterns = ["foo"]

        [fix.tools.my-tool-missing-command-2]
        patterns = ['glob:"ba*"']
        "###,
    );

    std::fs::write(repo_path.join("foo"), "foo\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Config error: Invalid type or value for fix.tools.my-tool-missing-command-1
    Caused by: missing field `command`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_tables_some_commands_missing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool-1]
        command = [{formatter}, "--uppercase"]
        patterns = ["foo"]

        [fix.tools.my-tool-missing-command]
        patterns = ['bar']
        "###,
    ));

    std::fs::write(repo_path.join("foo"), "foo\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Config error: Invalid type or value for fix.tools.my-tool-missing-command
    Caused by: missing field `command`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_tables_empty_patterns_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.my-tool-empty-patterns]
        command = [{formatter}, "--uppercase"]
        patterns = []
        "###,
    ));

    std::fs::write(repo_path.join("foo"), "foo\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_filesets() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.my-tool-match-one]
        command = [{formatter}, "--uppercase"]
        patterns = ['glob:"a*"']

        [fix.tools.my-tool-match-two]
        command = [{formatter}, "--reverse"]
        patterns = ['glob:"b*"']

        [fix.tools.my-tool-match-none]
        command = [{formatter}, "--append", "SHOULD NOT APPEAR"]
        patterns = ['glob:"this-doesnt-match-anything-*"']
        "###,
    ));

    std::fs::write(repo_path.join("a1"), "a1\n").unwrap();
    std::fs::write(repo_path.join("b1"), "b1\n").unwrap();
    std::fs::write(repo_path.join("b2"), "b2\n").unwrap();

    test_env.run_jj_in(&repo_path, ["fix"]).success();

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "a1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    A1
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    1b
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b2", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    2b
    [EOF]
    ");
}

#[test]
fn test_relative_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool]
        command = [{formatter}, "--stdout", "Fixed!"]
        patterns = ['glob:"foo*"']
        "###,
    ));

    std::fs::create_dir(repo_path.join("dir")).unwrap();
    std::fs::write(repo_path.join("foo1"), "unfixed\n").unwrap();
    std::fs::write(repo_path.join("foo2"), "unfixed\n").unwrap();
    std::fs::write(repo_path.join("dir/foo3"), "unfixed\n").unwrap();

    // Positional arguments are cwd-relative, but the configured patterns are
    // repo-relative, so this command fixes the empty intersection of those
    // filesets.
    test_env
        .run_jj_in(&repo_path.join("dir"), ["fix", "foo3"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");

    // Positional arguments can specify a subset of the configured fileset.
    test_env
        .run_jj_in(&repo_path.join("dir"), ["fix", "../foo1"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");

    // The current directory does not change the interpretation of the config, so
    // foo2 is fixed but not dir/foo3.
    test_env
        .run_jj_in(&repo_path.join("dir"), ["fix"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
}

#[test]
fn test_fix_empty_commit() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_fix_leaf_commit() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "unaffected").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "affected").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: rlvkpnrz 85ce8924 (no description set)
    Parent commit      : qpvuntsm b2ca2bc5 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@-"]);
    insta::assert_snapshot!(output, @"unaffected[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    AFFECTED
    [EOF]
    ");
}

#[test]
fn test_fix_parent_commit() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    // Using one file name for all commits adds coverage of some possible bugs.
    std::fs::write(repo_path.join("file"), "parent").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "parent"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "child1").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "child1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "-r", "parent"])
        .success();
    std::fs::write(repo_path.join("file"), "child2").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "child2"])
        .success();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "parent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy now at: mzvwutvl d30c8ae2 child2 | (no description set)
    Parent commit      : qpvuntsm 70a4dae2 parent | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "parent"]);
    insta::assert_snapshot!(output, @r"
    PARENT
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "child1"]);
    insta::assert_snapshot!(output, @r"
    CHILD1
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "child2"]);
    insta::assert_snapshot!(output, @r"
    CHILD2
    [EOF]
    ");
}

#[test]
fn test_fix_sibling_commit() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "parent").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "parent"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "child1").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "child1"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "-r", "parent"])
        .success();
    std::fs::write(repo_path.join("file"), "child2").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "child2"])
        .success();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "child1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "parent"]);
    insta::assert_snapshot!(output, @"parent[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "child1"]);
    insta::assert_snapshot!(output, @r"
    CHILD1
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "child2"]);
    insta::assert_snapshot!(output, @"child2[EOF]");
}

#[test]
fn test_default_revset() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "trunk1").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "trunk1"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "trunk2").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "trunk2"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "trunk1"]).success();
    std::fs::write(repo_path.join("file"), "foo").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "trunk1"]).success();
    std::fs::write(repo_path.join("file"), "bar1").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar1"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "bar2").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar2"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "bar3").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar3"])
        .success();
    test_env.run_jj_in(&repo_path, ["edit", "bar2"]).success();

    // With no args and no revset configuration, we fix `reachable(@, mutable())`,
    // which includes bar{1,2,3} and excludes trunk{1,2} (which is immutable) and
    // foo (which is mutable but not reachable).
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "trunk2""#);
    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy now at: yostqsxw dabc47b2 bar2 | (no description set)
    Parent commit      : yqosqzyt 984b5924 bar1 | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "trunk1"]);
    insta::assert_snapshot!(output, @"trunk1[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "trunk2"]);
    insta::assert_snapshot!(output, @"trunk2[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "foo"]);
    insta::assert_snapshot!(output, @"foo[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "bar1"]);
    insta::assert_snapshot!(output, @r"
    BAR1
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "bar2"]);
    insta::assert_snapshot!(output, @r"
    BAR2
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "bar3"]);
    insta::assert_snapshot!(output, @r"
    BAR3
    [EOF]
    ");
}

#[test]
fn test_custom_default_revset() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);

    std::fs::write(repo_path.join("file"), "foo").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "foo"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "bar").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "bar"])
        .success();

    // Check out a different commit so that the schema default `reachable(@,
    // mutable())` would behave differently from our customized default.
    test_env
        .run_jj_in(&repo_path, ["new", "-r", "foo"])
        .success();
    test_env.add_config(r#"revsets.fix = "bar""#);

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "foo"]);
    insta::assert_snapshot!(output, @"foo[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "bar"]);
    insta::assert_snapshot!(output, @r"
    BAR
    [EOF]
    ");
}

#[test]
fn test_fix_immutable_commit() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "immutable").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "immutable"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "mutable").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "mutable"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "immutable""#);

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "immutable"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit e4b41a3ce243 is immutable
    Hint: Could not modify commit: qpvuntsm e4b41a3c immutable | (no description set)
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "immutable"]);
    insta::assert_snapshot!(output, @"immutable[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "mutable"]);
    insta::assert_snapshot!(output, @"mutable[EOF]");
}

#[test]
fn test_fix_empty_file() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_fix_some_paths() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file1"), "foo").unwrap();
    std::fs::write(repo_path.join("file2"), "bar").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm 54a90d2b (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file1"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"bar[EOF]");
}

#[test]
fn test_fix_cyclic() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--reverse"]);
    std::fs::write(repo_path.join("file"), "content\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm bf5e6a5a (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    tnetnoc
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm 0e2d20d6 (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    content
    [EOF]
    ");
}

#[test]
fn test_deduplication() {
    // Append all fixed content to a log file. Note that fix tools are always run
    // from the workspace root, so this will always write to $root/$path-fixlog.
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase", "--tee", "$path-fixlog"]);

    // There are at least two interesting cases: the content is repeated immediately
    // in the child commit, or later in another descendant.
    std::fs::write(repo_path.join("file"), "foo\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "bar\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "bar\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "c"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "foo\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "d"])
        .success();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 4 commits of 4 checked.
    Working copy now at: yqosqzyt cf770245 d | (no description set)
    Parent commit      : mzvwutvl 370615a5 c | (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    BAR
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "c"]);
    insta::assert_snapshot!(output, @r"
    BAR
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "d"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");

    // Each new content string only appears once in the log, because all the other
    // inputs (like file name) were identical, and so the results were reused. We
    // sort the log because the order of execution inside `jj fix` is undefined.
    insta::assert_snapshot!(sorted_lines(repo_path.join("file-fixlog")), @r"
    BAR
    FOO
    ");
}

fn sorted_lines(path: PathBuf) -> String {
    let mut log: Vec<_> = std::fs::read_to_string(path.as_os_str())
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    log.sort();
    log.join("\n")
}

#[test]
fn test_executed_but_nothing_changed() {
    // Show that the tool ran by causing a side effect with --tee, and test that we
    // do the right thing when the tool's output is exactly equal to its input.
    let (test_env, repo_path) = init_with_fake_formatter(&["--tee", "$path-copy"]);
    std::fs::write(repo_path.join("file"), "content\n").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    content
    [EOF]
    ");
    let copy_content = std::fs::read_to_string(repo_path.join("file-copy").as_os_str()).unwrap();
    insta::assert_snapshot!(copy_content, @"content");

    // fix tools are always run from the workspace root, regardless of working
    // directory at time of invocation.
    std::fs::create_dir(repo_path.join("dir")).unwrap();
    let output = test_env.run_jj_in(&repo_path.join("dir"), ["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");

    let copy_content = std::fs::read_to_string(repo_path.join("file-copy").as_os_str()).unwrap();
    insta::assert_snapshot!(copy_content, @r"
    content
    content
    ");
    assert!(!repo_path.join("dir").join("file-copy").exists());
}

#[test]
fn test_failure() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--fail"]);
    std::fs::write(repo_path.join("file"), "content").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"content[EOF]");
}

#[test]
fn test_stderr_success() {
    let (test_env, repo_path) =
        init_with_fake_formatter(&["--stderr", "error", "--stdout", "new content"]);
    std::fs::write(repo_path.join("file"), "old content").unwrap();

    // TODO: Associate the stderr lines with the relevant tool/file/commit instead
    // of passing it through directly.
    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    errorFixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm 487808ba (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"new content[EOF]");
}

#[test]
fn test_stderr_failure() {
    let (test_env, repo_path) =
        init_with_fake_formatter(&["--stderr", "error", "--stdout", "new content", "--fail"]);
    std::fs::write(repo_path.join("file"), "old content").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    errorFixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"old content[EOF]");
}

#[test]
fn test_missing_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(indoc! {"
        [fix.tools.bad-tool]
        command = ['this_executable_shouldnt_exist']
        patterns = ['all()']
    "});
    // TODO: We should display a warning about invalid tool configurations. When we
    // support multiple tools, we should also keep going to see if any of the other
    // executions succeed.
    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_fix_file_types() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "content").unwrap();
    std::fs::create_dir(repo_path.join("dir")).unwrap();
    try_symlink("file", repo_path.join("link")).unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm 6836a9e4 (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CONTENT
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_fix_executable() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    let path = repo_path.join("file");
    std::fs::write(&path, "content").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    std::fs::set_permissions(&path, permissions).unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: qpvuntsm fee78e99 (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CONTENT
    [EOF]
    ");
    let executable = std::fs::metadata(&path).unwrap().permissions().mode() & 0o111;
    assert_eq!(executable, 0o111);
}

#[test]
fn test_fix_trivial_merge_commit() {
    // All the changes are attributable to a parent, so none are fixed (in the same
    // way that none would be shown in `jj diff -r @`).
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file_a"), "content a").unwrap();
    std::fs::write(repo_path.join("file_c"), "content c").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@-"]).success();
    std::fs::write(repo_path.join("file_b"), "content b").unwrap();
    std::fs::write(repo_path.join("file_c"), "content c").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "a", "b"]).success();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_a", "-r", "@"]);
    insta::assert_snapshot!(output, @"content a[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_b", "-r", "@"]);
    insta::assert_snapshot!(output, @"content b[EOF]");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_c", "-r", "@"]);
    insta::assert_snapshot!(output, @"content c[EOF]");
}

#[test]
fn test_fix_adding_merge_commit() {
    // None of the changes are attributable to a parent, so they are all fixed (in
    // the same way that they would be shown in `jj diff -r @`).
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file_a"), "content a").unwrap();
    std::fs::write(repo_path.join("file_c"), "content c").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@-"]).success();
    std::fs::write(repo_path.join("file_b"), "content b").unwrap();
    std::fs::write(repo_path.join("file_c"), "content c").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "a", "b"]).success();
    std::fs::write(repo_path.join("file_a"), "change a").unwrap();
    std::fs::write(repo_path.join("file_b"), "change b").unwrap();
    std::fs::write(repo_path.join("file_c"), "change c").unwrap();
    std::fs::write(repo_path.join("file_d"), "change d").unwrap();

    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy now at: mzvwutvl f93eb5a9 (no description set)
    Parent commit      : qpvuntsm 6e64e7a7 a | (no description set)
    Parent commit      : kkmpptxz c536f264 b | (no description set)
    Added 0 files, modified 4 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_a", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CHANGE A
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_b", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CHANGE B
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_c", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CHANGE C
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file_d", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CHANGE D
    [EOF]
    ");
}

#[test]
fn test_fix_both_sides_of_conflict() {
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "content a\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@-"]).success();
    std::fs::write(repo_path.join("file"), "content b\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "a", "b"]).success();

    // The conflicts are not different from the merged parent, so they would not be
    // fixed if we didn't fix the parents also.
    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "a", "-s", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy now at: mzvwutvl a55c6ec2 (conflict) (empty) (no description set)
    Parent commit      : qpvuntsm 8e8aad69 a | (no description set)
    Parent commit      : kkmpptxz 91f9b284 b | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    CONTENT A
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    CONTENT B
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    +CONTENT A
    +++++++ Contents of side #2
    CONTENT B
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}

#[test]
fn test_fix_resolve_conflict() {
    // If both sides of the conflict look the same after being fixed, the conflict
    // will be resolved.
    let (test_env, repo_path) = init_with_fake_formatter(&["--uppercase"]);
    std::fs::write(repo_path.join("file"), "Content\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "@-"]).success();
    std::fs::write(repo_path.join("file"), "cOnTeNt\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "a", "b"]).success();

    // The conflicts are not different from the merged parent, so they would not be
    // fixed if we didn't fix the parents also.
    let output = test_env.run_jj_in(&repo_path, ["fix", "-s", "a", "-s", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy now at: mzvwutvl 50fd048d (empty) (no description set)
    Parent commit      : qpvuntsm dd2721f1 a | (no description set)
    Parent commit      : kkmpptxz 07c27a8e b | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CONTENT
    [EOF]
    ");
}

#[test]
fn test_all_files() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());

    // Consider a few cases:
    // File A:     in patterns,     changed in child
    // File B:     in patterns, NOT changed in child
    // File C: NOT in patterns, NOT changed in child
    // File D: NOT in patterns,     changed in child
    // Some files will be in subdirectories to make sure we're covering that aspect
    // of matching.
    test_env.add_config(format!(
        r###"
        [fix.tools.tool]
        command = [{formatter}, "--append", "fixed"]
        patterns = ["a/a", "b/b"]
        "###,
    ));

    std::fs::create_dir(repo_path.join("a")).unwrap();
    std::fs::create_dir(repo_path.join("b")).unwrap();
    std::fs::create_dir(repo_path.join("c")).unwrap();
    std::fs::write(repo_path.join("a/a"), "parent aaa\n").unwrap();
    std::fs::write(repo_path.join("b/b"), "parent bbb\n").unwrap();
    std::fs::write(repo_path.join("c/c"), "parent ccc\n").unwrap();
    std::fs::write(repo_path.join("ddd"), "parent ddd\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "parent"])
        .success();

    std::fs::write(repo_path.join("a/a"), "child aaa\n").unwrap();
    std::fs::write(repo_path.join("ddd"), "child ddd\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m", "child"])
        .success();

    // Specifying files means exactly those files will be fixed in each revision,
    // although some like file C won't have any tools configured to make changes to
    // them. Specified but unfixed files are silently skipped, whether they lack
    // configuration, are ignored, don't exist, aren't normal files, etc.
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "fix",
            "--include-unchanged-files",
            "b/b",
            "c/c",
            "does_not.exist",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 2 commits of 2 checked.
    Working copy now at: rlvkpnrz c098d165 child
    Parent commit      : qpvuntsm 0bb31627 parent
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "a/a", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent aaa
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b/b", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "c/c", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "ddd", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ddd
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "a/a", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child aaa
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b/b", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "c/c", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "ddd", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child ddd
    [EOF]
    ");

    // Not specifying files means all files will be fixed in each revision.
    let output = test_env.run_jj_in(&repo_path, ["fix", "--include-unchanged-files"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 2 commits of 2 checked.
    Working copy now at: rlvkpnrz c5d0aa1d child
    Parent commit      : qpvuntsm b4d02ca9 parent
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "a/a", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent aaa
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b/b", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "c/c", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "ddd", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ddd
    [EOF]
    ");

    let output = test_env.run_jj_in(&repo_path, ["file", "show", "a/a", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child aaa
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "b/b", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed
    fixed[EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "c/c", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "ddd", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child ddd
    [EOF]
    ");
}
