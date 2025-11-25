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

use crate::common::TestEnvironment;
use crate::common::to_toml_value;

fn set_up_fake_formatter(test_env: &mut TestEnvironment, args: &[&str]) {
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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
    test_env.add_paths_to_normalize(formatter_path, "$FAKE_FORMATTER_PATH");
}

#[test]
fn test_config_no_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "content\n");
    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: No `fix.tools` are configured
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    content
    [EOF]
    ");
}

#[test]
fn test_config_nonexistent_tool() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(
        r###"
        [fix.tools.my-tool]
        command = ["nonexistent-fix-tool-binary"]
        patterns = ["glob:**"]
        "###,
    );

    work_dir.write_file("file", "content\n");
    let output = work_dir.run_jj(["fix"]);
    // We inform the user about the non-existent tool
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Failed to start `nonexistent-fix-tool-binary`
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_config_multiple_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("foo", "Foo\n");
    work_dir.write_file("bar", "Bar\n");
    work_dir.write_file("baz", "Baz\n");

    work_dir.run_jj(["fix"]).success();

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Baz
    [EOF]
    ");
}

#[test]
fn test_config_multiple_tools_with_same_name() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("foo", "Foo\n");
    work_dir.write_file("bar", "Bar\n");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Configuration cannot be parsed as TOML document
    Caused by: TOML parse error at line 6, column 20
      |
    6 |         [fix.tools.my-tool]
      |                    ^^^^^^^
    duplicate key

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    test_env.set_config_path("/dev/null");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Foo
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Bar
    [EOF]
    ");
}

#[test]
fn test_config_disabled_tools() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("foo", "Foo\n");
    work_dir.write_file("bar", "Bar\n");
    work_dir.write_file("baz", "Baz\n");

    work_dir.run_jj(["fix"]).success();

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    Baz
    [EOF]
    ");
}

#[test]
fn test_config_disabled_tools_warning_when_all_tools_are_disabled() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("bar", "Bar\n");

    let output = work_dir.run_jj(["fix"]);
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
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("foo", "foo\n");
    work_dir.write_file("bar", "bar\n");
    work_dir.write_file("baz", "baz\n");

    work_dir.run_jj(["fix"]).success();

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    tool-1[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "bar", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    bar
    tool-1tool-2[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "baz", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    baz
    tool-2[EOF]
    ");
}

#[test]
fn test_config_tables_all_commands_missing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(
        r###"
        [fix.tools.my-tool-missing-command-1]
        patterns = ["foo"]

        [fix.tools.my-tool-missing-command-2]
        patterns = ['glob:"ba*"']
        "###,
    );

    work_dir.write_file("foo", "foo\n");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Config error: Invalid type or value for fix.tools.my-tool-missing-command-1
    Caused by: missing field `command`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_tables_some_commands_missing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("foo", "foo\n");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Config error: Invalid type or value for fix.tools.my-tool-missing-command
    Caused by: missing field `command`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_tables_empty_patterns_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.my-tool-empty-patterns]
        command = [{formatter}, "--uppercase"]
        patterns = []
        "###,
    ));

    work_dir.write_file("foo", "foo\n");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "foo", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    foo
    [EOF]
    ");
}

#[test]
fn test_config_filesets() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.write_file("a1", "a1\n");
    work_dir.write_file("b1", "b1\n");
    work_dir.write_file("b2", "b2\n");

    work_dir.run_jj(["fix"]).success();

    let output = work_dir.run_jj(["file", "show", "a1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    A1
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "b1", "-r", "@"]);
    insta::assert_snapshot!(output, @"1b[EOF]");
    let output = work_dir.run_jj(["file", "show", "b2", "-r", "@"]);
    insta::assert_snapshot!(output, @"2b[EOF]");
}

#[test]
fn test_relative_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());
    test_env.add_config(format!(
        r###"
        [fix.tools.tool]
        command = [{formatter}, "--stdout", "Fixed!"]
        patterns = ['glob:"foo*"']
        "###,
    ));

    let sub_dir = work_dir.create_dir("dir");
    work_dir.write_file("foo1", "unfixed\n");
    work_dir.write_file("foo2", "unfixed\n");
    work_dir.write_file("dir/foo3", "unfixed\n");

    // Positional arguments are cwd-relative, but the configured patterns are
    // repo-relative, so this command fixes the empty intersection of those
    // filesets.
    sub_dir.run_jj(["fix", "foo3"]).success();
    let output = work_dir.run_jj(["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");

    // Positional arguments can specify a subset of the configured fileset.
    sub_dir.run_jj(["fix", "../foo1"]).success();
    let output = work_dir.run_jj(["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = work_dir.run_jj(["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");

    // The current directory does not change the interpretation of the config, so
    // foo2 is fixed but not dir/foo3.
    sub_dir.run_jj(["fix"]).success();
    let output = work_dir.run_jj(["file", "show", "foo1", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = work_dir.run_jj(["file", "show", "foo2", "-r", "@"]);
    insta::assert_snapshot!(output, @"Fixed![EOF]");
    let output = work_dir.run_jj(["file", "show", "dir/foo3", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    unfixed
    [EOF]
    ");

    // The output filtered to a non-existent file should display a warning.
    let output = work_dir.run_jj(["fix", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_relative_tool_path_from_subdirectory() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Copy the fake-formatter into the workspace as a relative tool
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    let formatter_name = formatter_path.file_name().unwrap().to_str().unwrap();
    let tool_dir = work_dir.create_dir("tools");
    let workspace_formatter_path = tool_dir.root().join(formatter_name);
    std::fs::copy(formatter_path, &workspace_formatter_path).unwrap();
    work_dir.write_file(".gitignore", "tools/\n");
    let formatter_relative_path = PathBuf::from_iter(["$root", "tools", formatter_name]);
    test_env.add_config(format!(
        r###"
        [fix.tools.a]
        command = [{path}, "--uppercase"]
        patterns = ['glob:"**/*.txt"']
        "###,
        path = to_toml_value(formatter_relative_path.to_str().unwrap())
    ));

    // Create a test file and subdirectory
    work_dir.write_file("test.txt", "hello world\n");
    let sub_dir = work_dir.create_dir("subdir");
    work_dir.write_file("subdir/nested.txt", "nested content\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // Run fix from workspace root
    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 57a27b36 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "test.txt", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    HELLO WORLD
    [EOF]
    "###);
    let output = work_dir.run_jj(["file", "show", "subdir/nested.txt", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    NESTED CONTENT
    [EOF]
    "###);

    // Reset so the fix tools should have an effect again
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Run fix from the subdirectory
    let output = sub_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 05404d5b (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "test.txt", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    HELLO WORLD
    [EOF]
    "###);
    let output = work_dir.run_jj(["file", "show", "subdir/nested.txt", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    NESTED CONTENT
    [EOF]
    "###);
}

#[test]
fn test_fix_empty_commit() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_fix_leaf_commit() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "unaffected");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "affected");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: rlvkpnrz f5c11961 (no description set)
    Parent commit (@-)      : qpvuntsm b37955c0 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@-"]);
    insta::assert_snapshot!(output, @"unaffected[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"AFFECTED[EOF]");
}

#[test]
fn test_fix_parent_commit() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Using one file name for all commits adds coverage of some possible bugs.
    work_dir.write_file("file", "parent");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "parent"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "child1");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child1"])
        .success();
    work_dir.run_jj(["new", "-r", "parent"]).success();
    work_dir.write_file("file", "child2");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child2"])
        .success();

    let output = work_dir.run_jj(["fix", "-s", "parent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy  (@) now at: mzvwutvl e7ba6d31 child2 | (no description set)
    Parent commit (@-)      : qpvuntsm 49f1ddd5 parent | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "parent"]);
    insta::assert_snapshot!(output, @"PARENT[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "child1"]);
    insta::assert_snapshot!(output, @"CHILD1[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "child2"]);
    insta::assert_snapshot!(output, @"CHILD2[EOF]");
}

#[test]
fn test_fix_sibling_commit() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "parent");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "parent"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "child1");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child1"])
        .success();
    work_dir.run_jj(["new", "-r", "parent"]).success();
    work_dir.write_file("file", "child2");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child2"])
        .success();

    let output = work_dir.run_jj(["fix", "-s", "child1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "parent"]);
    insta::assert_snapshot!(output, @"parent[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "child1"]);
    insta::assert_snapshot!(output, @"CHILD1[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "child2"]);
    insta::assert_snapshot!(output, @"child2[EOF]");
}

#[test]
fn test_fix_descendant_commits() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("parent", "parent");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "parent"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("child1", "child1");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child1"])
        .success();
    work_dir.run_jj(["new", "-r", "parent"]).success();
    work_dir.write_file("child2", "child2");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "child2"])
        .success();

    let output = work_dir.run_jj(["fix", "-s", "parent", "child1", "child2", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Fixed 2 commits of 3 checked.
    Working copy  (@) now at: mzvwutvl afe0ade0 child2 | (no description set)
    Parent commit (@-)      : qpvuntsm c9cb6288 parent | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "parent", "-r", "parent"]);
    insta::assert_snapshot!(output, @"parent[EOF]");
    let output = work_dir.run_jj(["file", "show", "child1", "-r", "child1"]);
    insta::assert_snapshot!(output, @"CHILD1[EOF]");
    let output = work_dir.run_jj(["file", "show", "child2", "-r", "child2"]);
    insta::assert_snapshot!(output, @"CHILD2[EOF]");
}

#[test]
fn test_default_revset() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "trunk1");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "trunk1"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "trunk2");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "trunk2"])
        .success();
    work_dir.run_jj(["new", "trunk1"]).success();
    work_dir.write_file("file", "foo");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    work_dir.run_jj(["new", "trunk1"]).success();
    work_dir.write_file("file", "bar1");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bar1"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar2");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bar2"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar3");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bar3"])
        .success();
    work_dir.run_jj(["edit", "bar2"]).success();

    // With no args and no revset configuration, we fix `reachable(@, mutable())`,
    // which includes bar{1,2,3} and excludes trunk{1,2} (which is immutable) and
    // foo (which is mutable but not reachable).
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "trunk2""#);
    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy  (@) now at: yostqsxw 932b950d bar2 | (no description set)
    Parent commit (@-)      : yqosqzyt 8a37ed67 bar1 | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "trunk1"]);
    insta::assert_snapshot!(output, @"trunk1[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "trunk2"]);
    insta::assert_snapshot!(output, @"trunk2[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "foo"]);
    insta::assert_snapshot!(output, @"foo[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "bar1"]);
    insta::assert_snapshot!(output, @"BAR1[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "bar2"]);
    insta::assert_snapshot!(output, @"BAR2[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "bar3"]);
    insta::assert_snapshot!(output, @"BAR3[EOF]");
}

#[test]
fn test_custom_default_revset() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "foo");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bar"])
        .success();

    // Check out a different commit so that the schema default `reachable(@,
    // mutable())` would behave differently from our customized default.
    work_dir.run_jj(["new", "-r", "foo"]).success();
    test_env.add_config(r#"revsets.fix = "bar""#);

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "foo"]);
    insta::assert_snapshot!(output, @"foo[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "bar"]);
    insta::assert_snapshot!(output, @"BAR[EOF]");
}

#[test]
fn test_fix_immutable_commit() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "immutable");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "immutable"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "mutable");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "mutable"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "immutable""#);

    let output = work_dir.run_jj(["fix", "-s", "immutable"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit a86b2eccaaab is immutable
    Hint: Could not modify commit: qpvuntsm a86b2ecc immutable | (no description set)
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);
    let output = work_dir.run_jj(["file", "show", "file", "-r", "immutable"]);
    insta::assert_snapshot!(output, @"immutable[EOF]");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "mutable"]);
    insta::assert_snapshot!(output, @"mutable[EOF]");
}

#[test]
fn test_fix_empty_file() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_fix_some_paths() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "foo");
    work_dir.write_file("file2", "bar");

    let output = work_dir.run_jj(["fix", "-s", "@", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 0279baba (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file1"]);
    insta::assert_snapshot!(output, @"FOO[EOF]");
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @"bar[EOF]");
}

#[test]
fn test_fix_cyclic() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--reverse"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "content\n");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm ce361156 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"tnetnoc[EOF]");

    let output = work_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 547f589b (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"content[EOF]");
}

#[test]
fn test_deduplication() {
    // Append all fixed content to a log file. Note that fix tools are always run
    // from the workspace root, so this will always write to $root/$path-fixlog.
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase", "--tee", "$path-fixlog"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // There are at least two interesting cases: the content is repeated immediately
    // in the child commit, or later in another descendant.
    work_dir.write_file("file", "foo\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "foo\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    let output = work_dir.run_jj(["fix", "-s", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 4 commits of 4 checked.
    Working copy  (@) now at: yqosqzyt 9849a250 d | (no description set)
    Parent commit (@-)      : mzvwutvl 9544f381 c | (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    BAR
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "c"]);
    insta::assert_snapshot!(output, @r"
    BAR
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "d"]);
    insta::assert_snapshot!(output, @r"
    FOO
    [EOF]
    ");

    // Each new content string only appears once in the log, because all the other
    // inputs (like file name) were identical, and so the results were reused. We
    // sort the log because the order of execution inside `jj fix` is undefined.
    insta::assert_snapshot!(sorted_lines(work_dir.root().join("file-fixlog")), @r"
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
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--tee", "$path-copy"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "content\n");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    content
    [EOF]
    ");
    let copy_content = work_dir.read_file("file-copy");
    insta::assert_snapshot!(copy_content, @"content");

    // fix tools are always run from the workspace root, regardless of working
    // directory at time of invocation.
    let sub_dir = work_dir.create_dir("dir");
    let output = sub_dir.run_jj(["fix"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");

    let copy_content = work_dir.read_file("file-copy");
    insta::assert_snapshot!(copy_content, @r"
    content
    content
    ");
    assert!(!sub_dir.root().join("file-copy").exists());
}

#[test]
fn test_failure() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--fail"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "content");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Fix tool `$FAKE_FORMATTER_PATH` exited with non-zero exit code for `file`
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"content[EOF]");
}

#[test]
fn test_stderr_success() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(
        &mut test_env,
        &["--stderr", "error", "--stdout", "new content"],
    );
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "old content");

    // TODO: Associate the stderr lines with the relevant tool/file/commit instead
    // of passing it through directly.
    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    file:
    error
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm cb75cbcb (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"new content[EOF]");
}

#[test]
fn test_stderr_failure() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(
        &mut test_env,
        &["--stderr", "error", "--stdout", "new content", "--fail"],
    );
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "old content");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    file:
    error
    Warning: Fix tool `$FAKE_FORMATTER_PATH` exited with non-zero exit code for `file`
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"old content[EOF]");
}

#[test]
fn test_missing_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(indoc! {"
        [fix.tools.bad-tool]
        command = ['this_executable_shouldnt_exist']
        patterns = ['all()']
    "});
    // TODO: We should display a warning about invalid tool configurations. When we
    // support multiple tools, we should also keep going to see if any of the other
    // executions succeed.
    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_fix_file_types() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "content");
    work_dir.create_dir("dir");
    try_symlink("file", work_dir.root().join("link")).unwrap();

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 0184b215 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @"CONTENT[EOF]");
}

#[cfg(unix)]
#[test]
fn test_fix_executable() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let path = work_dir.root().join("file");
    work_dir.write_file("file", "content");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    std::fs::set_permissions(&path, permissions).unwrap();

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: qpvuntsm 5293bf26 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"CONTENT[EOF]");
    let executable = std::fs::metadata(&path).unwrap().permissions().mode() & 0o111;
    assert_eq!(executable, 0o111);
}

#[test]
fn test_fix_trivial_merge_commit() {
    // All the changes are attributable to a parent, so none are fixed (in the same
    // way that none would be shown in `jj diff -r @`).
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file_a", "content a");
    work_dir.write_file("file_c", "content c");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.write_file("file_b", "content b");
    work_dir.write_file("file_c", "content c");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(["new", "a", "b"]).success();

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 0 commits of 1 checked.
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file_a", "-r", "@"]);
    insta::assert_snapshot!(output, @"content a[EOF]");
    let output = work_dir.run_jj(["file", "show", "file_b", "-r", "@"]);
    insta::assert_snapshot!(output, @"content b[EOF]");
    let output = work_dir.run_jj(["file", "show", "file_c", "-r", "@"]);
    insta::assert_snapshot!(output, @"content c[EOF]");
}

#[test]
fn test_fix_adding_merge_commit() {
    // None of the changes are attributable to a parent, so they are all fixed (in
    // the same way that they would be shown in `jj diff -r @`).
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file_a", "content a");
    work_dir.write_file("file_c", "content c");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.write_file("file_b", "content b");
    work_dir.write_file("file_c", "content c");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(["new", "a", "b"]).success();
    work_dir.write_file("file_a", "change a");
    work_dir.write_file("file_b", "change b");
    work_dir.write_file("file_c", "change c");
    work_dir.write_file("file_d", "change d");

    let output = work_dir.run_jj(["fix", "-s", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 1 commits of 1 checked.
    Working copy  (@) now at: mzvwutvl 9f580aac (no description set)
    Parent commit (@-)      : qpvuntsm 93f04460 a | (no description set)
    Parent commit (@-)      : kkmpptxz ad4fc36c b | (no description set)
    Added 0 files, modified 4 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file_a", "-r", "@"]);
    insta::assert_snapshot!(output, @"CHANGE A[EOF]");
    let output = work_dir.run_jj(["file", "show", "file_b", "-r", "@"]);
    insta::assert_snapshot!(output, @"CHANGE B[EOF]");
    let output = work_dir.run_jj(["file", "show", "file_c", "-r", "@"]);
    insta::assert_snapshot!(output, @"CHANGE C[EOF]");
    let output = work_dir.run_jj(["file", "show", "file_d", "-r", "@"]);
    insta::assert_snapshot!(output, @"CHANGE D[EOF]");
}

#[test]
fn test_fix_both_sides_of_conflict() {
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "content a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.write_file("file", "content b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(["new", "a", "b"]).success();

    // The conflicts are not different from the merged parent, so they would not be
    // fixed if we didn't fix the parents also.
    let output = work_dir.run_jj(["fix", "-s", "a", "-s", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy  (@) now at: mzvwutvl d4d02bf0 (conflict) (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 0eae0dae a | (no description set)
    Parent commit (@-)      : kkmpptxz eb61ba8d b | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "a"]);
    insta::assert_snapshot!(output, @r"
    CONTENT A
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    CONTENT B
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
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
    let mut test_env = TestEnvironment::default();
    set_up_fake_formatter(&mut test_env, &["--uppercase"]);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file", "Content\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.write_file("file", "cOnTeNt\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(["new", "a", "b"]).success();

    // The conflicts are not different from the merged parent, so they would not be
    // fixed if we didn't fix the parents also.
    let output = work_dir.run_jj(["fix", "-s", "a", "-s", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 3 commits of 3 checked.
    Working copy  (@) now at: mzvwutvl c4e4665e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7a0dbb95 a | (no description set)
    Parent commit (@-)      : kkmpptxz 5d9510ab b | (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "file", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    CONTENT
    [EOF]
    ");
}

#[test]
fn test_all_files() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
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

    work_dir.create_dir("a");
    work_dir.create_dir("b");
    work_dir.create_dir("c");
    work_dir.write_file("a/a", "parent aaa\n");
    work_dir.write_file("b/b", "parent bbb\n");
    work_dir.write_file("c/c", "parent ccc\n");
    work_dir.write_file("ddd", "parent ddd\n");
    work_dir.run_jj(["commit", "-m", "parent"]).success();

    work_dir.write_file("a/a", "child aaa\n");
    work_dir.write_file("ddd", "child ddd\n");
    work_dir.run_jj(["describe", "-m", "child"]).success();

    // Specifying files means exactly those files will be fixed in each revision,
    // although some like file C won't have any tools configured to make changes to
    // them. Specified but unfixed files are silently skipped, whether they lack
    // configuration, are ignored, don't exist, aren't normal files, etc.
    let output = work_dir.run_jj([
        "fix",
        "--include-unchanged-files",
        "b/b",
        "c/c",
        "does_not.exist",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: does_not.exist
    Fixed 2 commits of 2 checked.
    Working copy  (@) now at: rlvkpnrz d8503fee child
    Parent commit (@-)      : qpvuntsm 62c6ee98 parent
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "a/a", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent aaa
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "b/b", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "c/c", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "ddd", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ddd
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "a/a", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child aaa
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "b/b", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "c/c", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "ddd", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child ddd
    [EOF]
    ");

    // Not specifying files means all files will be fixed in each revision.
    let output = work_dir.run_jj(["fix", "--include-unchanged-files"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Fixed 2 commits of 2 checked.
    Working copy  (@) now at: rlvkpnrz 16aeb14c child
    Parent commit (@-)      : qpvuntsm 5257b8ec parent
    Added 0 files, modified 2 files, removed 0 files
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "a/a", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent aaa
    fixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "b/b", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixedfixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "c/c", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "ddd", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    parent ddd
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "a/a", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child aaa
    fixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "b/b", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent bbb
    fixedfixed[EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "c/c", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    parent ccc
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "show", "ddd", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    child ddd
    [EOF]
    ");
}
