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

use std::fs;

use crate::common::TestEnvironment;

#[test]
fn test_alias_basic() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(r#"aliases.bk = ["log", "-r", "@", "-T", "bookmarks"]"#);
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "my-bookmark", "-r", "@"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bk"]);
    insta::assert_snapshot!(output, @r"
    @  my-bookmark
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_alias_bad_name() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&repo_path, ["foo."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unrecognized subcommand 'foo.'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_alias_calls_empty_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(
        r#"
    aliases.empty = []
    aliases.empty_command_with_opts = ["--no-pager"]
    "#,
    );

    let output = test_env.run_jj_in(&repo_path, ["empty"]);
    insta::assert_snapshot!(
        output.normalize_stderr_with(|s| s.split_inclusive('\n').take(3).collect()), @r"
    ------- stderr -------
    Jujutsu (An experimental VCS)

    Usage: jj [OPTIONS] <COMMAND>
    [EOF]
    [exit status: 2]
    ");
    let output = test_env.run_jj_in(&repo_path, ["empty", "--no-pager"]);
    insta::assert_snapshot!(
        output.normalize_stderr_with(|s| s.split_inclusive('\n').take(1).collect()), @r"
    ------- stderr -------
    error: 'jj' requires a subcommand but one was not provided
    [EOF]
    [exit status: 2]
    ");
    let output = test_env.run_jj_in(&repo_path, ["empty_command_with_opts"]);
    insta::assert_snapshot!(
        output.normalize_stderr_with(|s| s.split_inclusive('\n').take(1).collect()), @r"
    ------- stderr -------
    error: 'jj' requires a subcommand but one was not provided
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_alias_calls_unknown_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(r#"aliases.foo = ["nonexistent"]"#);
    let output = test_env.run_jj_in(&repo_path, ["foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unrecognized subcommand 'nonexistent'

      tip: a similar subcommand exists: 'next'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_alias_calls_command_with_invalid_option() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(r#"aliases.foo = ["log", "--nonexistent"]"#);
    let output = test_env.run_jj_in(&repo_path, ["foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unexpected argument '--nonexistent' found

      tip: to pass '--nonexistent' as a value, use '-- --nonexistent'

    Usage: jj log [OPTIONS] [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_alias_calls_help() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(r#"aliases.h = ["--help"]"#);
    let output = test_env.run_jj_in(&repo_path, &["h"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| s.split_inclusive('\n').take(7).collect()), @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    Usage: jj [OPTIONS] <COMMAND>
    [EOF]
    ");
}

#[test]
fn test_alias_cannot_override_builtin() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(r#"aliases.log = ["rebase"]"#);
    // Alias should give a warning
    let output = test_env.run_jj_in(&repo_path, ["log", "-r", "root()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Warning: Cannot define an alias that overrides the built-in command 'log'
    [EOF]
    ");
}

#[test]
fn test_alias_recursive() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    test_env.add_config(
        r#"[aliases]
    foo = ["foo"]
    bar = ["baz"]
    baz = ["bar"]
    "#,
    );
    // Alias should not cause infinite recursion or hang
    let output = test_env.run_jj_in(&repo_path, ["foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Recursive alias definition involving `foo`
    [EOF]
    [exit status: 1]
    ");
    // Also test with mutual recursion
    let output = test_env.run_jj_in(&repo_path, ["bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Recursive alias definition involving `bar`
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_alias_global_args_before_and_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(r#"aliases.l = ["log", "-T", "commit_id", "-r", "all()"]"#);
    // Test the setup
    let output = test_env.run_jj_in(&repo_path, ["l"]);
    insta::assert_snapshot!(output, @r"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Can pass global args before
    let output = test_env.run_jj_in(&repo_path, ["l", "--at-op", "@-"]);
    insta::assert_snapshot!(output, @r"
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // Can pass global args after
    let output = test_env.run_jj_in(&repo_path, ["--at-op", "@-", "l"]);
    insta::assert_snapshot!(output, @r"
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // Test passing global args both before and after
    let output = test_env.run_jj_in(&repo_path, ["--at-op", "abc123", "l", "--at-op", "@-"]);
    insta::assert_snapshot!(output, @r"
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["-R", "../nonexistent", "l", "-R", "."]);
    insta::assert_snapshot!(output, @r"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_alias_global_args_in_definition() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(
        r#"aliases.l = ["log", "-T", "commit_id", "--at-op", "@-", "-r", "all()", "--color=always"]"#,
    );

    // The global argument in the alias is respected
    let output = test_env.run_jj_in(&repo_path, ["l"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;14m◆[0m  [38;5;4m0000000000000000000000000000000000000000[39m
    [EOF]
    ");
}

#[test]
fn test_alias_invalid_definition() {
    let test_env = TestEnvironment::default();

    test_env.add_config(
        r#"[aliases]
    non-list = 5
    non-string-list = [0]
    "#,
    );
    let output = test_env.run_jj_in(".", ["non-list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Config error: Invalid type or value for aliases.non-list
    Caused by: invalid type: integer `5`, expected a sequence

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(".", ["non-string-list"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid type or value for aliases.non-string-list
    Caused by: invalid type: integer `0`, expected a string

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_alias_in_repo_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo1"]).success();
    let repo1_path = test_env.env_root().join("repo1");
    fs::create_dir(repo1_path.join("sub")).unwrap();
    test_env.run_jj_in(".", ["git", "init", "repo2"]).success();
    let repo2_path = test_env.env_root().join("repo2");
    fs::create_dir(repo2_path.join("sub")).unwrap();

    test_env.add_config(r#"aliases.l = ['log', '-r@', '--no-graph', '-T"user alias\n"']"#);
    fs::write(
        repo1_path.join(".jj/repo/config.toml"),
        r#"aliases.l = ['log', '-r@', '--no-graph', '-T"repo1 alias\n"']"#,
    )
    .unwrap();

    // In repo1 sub directory, aliases can be loaded from the repo1 config.
    let output = test_env.run_jj_in(&repo1_path.join("sub"), ["l"]);
    insta::assert_snapshot!(output, @r"
    repo1 alias
    [EOF]
    ");

    // In repo2 directory, no repo-local aliases exist.
    let output = test_env.run_jj_in(&repo2_path, ["l"]);
    insta::assert_snapshot!(output, @r"
    user alias
    [EOF]
    ");

    // Aliases can't be loaded from the -R path due to chicken and egg problem.
    let output = test_env.run_jj_in(&repo2_path, ["l", "-R", repo1_path.to_str().unwrap()]);
    insta::assert_snapshot!(output, @r"
    user alias
    [EOF]
    ------- stderr -------
    Warning: Command aliases cannot be loaded from -R/--repository path or --config/--config-file arguments.
    [EOF]
    ");

    // Aliases are loaded from the cwd-relative workspace even with -R.
    let output = test_env.run_jj_in(&repo1_path, ["l", "-R", repo2_path.to_str().unwrap()]);
    insta::assert_snapshot!(output, @r"
    repo1 alias
    [EOF]
    ------- stderr -------
    Warning: Command aliases cannot be loaded from -R/--repository path or --config/--config-file arguments.
    [EOF]
    ");

    // No warning if the expanded command is identical.
    let output = test_env.run_jj_in(
        &repo1_path,
        ["file", "list", "-R", repo2_path.to_str().unwrap()],
    );
    insta::assert_snapshot!(output, @"");

    // Config loaded from the cwd-relative workspace shouldn't persist. It's
    // used only for command arguments expansion.
    let output = test_env.run_jj_in(
        &repo1_path,
        [
            "config",
            "list",
            "aliases",
            "-R",
            repo2_path.to_str().unwrap(),
        ],
    );
    insta::assert_snapshot!(output, @r#"
    aliases.l = ['log', '-r@', '--no-graph', '-T"user alias\n"']
    [EOF]
    "#);
}

#[test]
fn test_alias_in_config_arg() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    test_env.add_config(r#"aliases.l = ['log', '-r@', '--no-graph', '-T"user alias\n"']"#);

    let output = test_env.run_jj_in(&repo_path, ["l"]);
    insta::assert_snapshot!(output, @r"
    user alias
    [EOF]
    ");

    let alias_arg = r#"--config=aliases.l=['log', '-r@', '--no-graph', '-T"arg alias\n"']"#;
    let output = test_env.run_jj_in(&repo_path, [alias_arg, "l"]);
    insta::assert_snapshot!(output, @r"
    user alias
    [EOF]
    ------- stderr -------
    Warning: Command aliases cannot be loaded from -R/--repository path or --config/--config-file arguments.
    [EOF]
    ");
    // should print warning about aliases even if cli parsing fails
    let alias_arg = r#"--config=aliases.this-command-not-exist=['log', '-r@', '--no-graph', '-T"arg alias\n"']"#;
    let output = test_env.run_jj_in(&repo_path, [alias_arg, "this-command-not-exist"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Command aliases cannot be loaded from -R/--repository path or --config/--config-file arguments.
    error: unrecognized subcommand 'this-command-not-exist'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_aliases_overriding_friendly_errors() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["init", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: unrecognized subcommand 'init'

      tip: a similar subcommand exists: 'git'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    Hint: You probably want `jj git init`. See also `jj help git`.
    Hint: You can configure `aliases.init = ["git", "init"]` if you want `jj init` to work and always use the Git backend.
    [EOF]
    [exit status: 2]
    "#);
    let output = test_env.run_jj_in(".", ["clone", "https://example.org/repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: unrecognized subcommand 'clone'

      tip: a similar subcommand exists: 'config'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    Hint: You probably want `jj git clone`. See also `jj help git`.
    Hint: You can configure `aliases.clone = ["git", "clone"]` if you want `jj clone` to work and always use the Git backend.
    [EOF]
    [exit status: 2]
    "#);
    let output = test_env.run_jj_in(".", ["init", "--help"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: unrecognized subcommand 'init'

      tip: a similar subcommand exists: 'git'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    Hint: You probably want `jj git init`. See also `jj help git`.
    Hint: You can configure `aliases.init = ["git", "init"]` if you want `jj init` to work and always use the Git backend.
    [EOF]
    [exit status: 2]
    "#);

    // Test that `init` can be overridden as an alias. (We use `jj config get`
    // as a command with a predictable output)
    test_env.add_config(r#"aliases.init=["config", "get", "user.name"]"#);
    let output = test_env.run_jj_in(".", ["init"]);
    insta::assert_snapshot!(output, @r"
    Test User
    [EOF]
    ");
}
