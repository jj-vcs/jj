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

use std::env::join_paths;
use std::path::PathBuf;

use indoc::indoc;
use itertools::Itertools as _;
use regex::Regex;

use crate::common::TestEnvironment;
use crate::common::default_config_from_schema;
use crate::common::fake_editor_path;
use crate::common::force_interactive;
use crate::common::to_toml_value;

#[test]
fn test_config_list_single() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    [test-table]
    somekey = "some value"
    "#,
    );

    let output = test_env.run_jj_in(".", ["config", "list", "test-table.somekey"]);
    insta::assert_snapshot!(output, @r#"
    test-table.somekey = "some value"
    [EOF]
    "#);

    let output = test_env.run_jj_in(
        ".",
        ["config", "list", r#"-Tname ++ "\n""#, "test-table.somekey"],
    );
    insta::assert_snapshot!(output, @r"
    test-table.somekey
    [EOF]
    ");
}

#[test]
fn test_config_list_nonexistent() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["config", "list", "nonexistent-test-key"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching config key for nonexistent-test-key
    [EOF]
    ");
}

#[test]
fn test_config_list_table() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    [test-table]
    x = true
    y.foo = "abc"
    y.bar = 123
    "z"."with space"."function()" = 5
    "#,
    );
    let output = test_env.run_jj_in(".", ["config", "list", "test-table"]);
    insta::assert_snapshot!(output, @r#"
    test-table.x = true
    test-table.y.foo = "abc"
    test-table.y.bar = 123
    test-table.z."with space"."function()" = 5
    [EOF]
    "#);
}

#[test]
fn test_config_list_inline_table() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    test-table = { x = true, y = 1 }
    "#,
    );
    // Inline tables are expanded
    let output = test_env.run_jj_in(".", ["config", "list", "test-table"]);
    insta::assert_snapshot!(output, @r"
    test-table.x = true
    test-table.y = 1
    [EOF]
    ");
    // Inner value can also be addressed by a dotted name path
    let output = test_env.run_jj_in(".", ["config", "list", "test-table.x"]);
    insta::assert_snapshot!(output, @r"
    test-table.x = true
    [EOF]
    ");
}

#[test]
fn test_config_list_array() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    test-array = [1, "b", 3.4]
    "#,
    );
    let output = test_env.run_jj_in(".", ["config", "list", "test-array"]);
    insta::assert_snapshot!(output, @r#"
    test-array = [1, "b", 3.4]
    [EOF]
    "#);
}

#[test]
fn test_config_list_array_of_tables() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
        [[test-table]]
        x = 1
        [[test-table]]
        y = ["z"]
        z."key=with whitespace" = []
    "#,
    );
    // Array is a value, so is array of tables
    let output = test_env.run_jj_in(".", ["config", "list", "test-table"]);
    insta::assert_snapshot!(output, @r#"
    test-table = [{ x = 1 }, { y = ["z"], z = { "key=with whitespace" = [] } }]
    [EOF]
    "#);
}

#[test]
fn test_config_list_all() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    test-val = [1, 2, 3]
    [test-table]
    x = true
    y.foo = "abc"
    y.bar = 123
    "#,
    );

    let output = test_env.run_jj_in(".", ["config", "list"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| find_stdout_lines(r"(test-val|test-table\b[^=]*)", &s)), @r#"
    test-val = [1, 2, 3]
    test-table.x = true
    test-table.y.foo = "abc"
    test-table.y.bar = 123
    [EOF]
    "#);
}

#[test]
fn test_config_list_multiline_string() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    multiline = '''
foo
bar
'''
    "#,
    );

    let output = test_env.run_jj_in(".", ["config", "list", "multiline"]);
    insta::assert_snapshot!(output, @r"
    multiline = '''
    foo
    bar
    '''
    [EOF]
    ");

    let output = test_env.run_jj_in(
        ".",
        [
            "config",
            "list",
            "multiline",
            "--include-overridden",
            "--config=multiline='single'",
        ],
    );
    insta::assert_snapshot!(output, @r"
    # multiline = '''
    # foo
    # bar
    # '''
    multiline = 'single'
    [EOF]
    ");
}

#[test]
fn test_config_list_layer() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    // User
    work_dir
        .run_jj(["config", "set", "--user", "test-key", "test-val"])
        .success();

    work_dir
        .run_jj([
            "config",
            "set",
            "--user",
            "test-layered-key",
            "test-original-val",
        ])
        .success();

    let output = work_dir.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r#"
    test-key = "test-val"
    test-layered-key = "test-original-val"
    [EOF]
    "#);

    // Repo
    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "test-layered-key",
            "test-layered-val",
        ])
        .success();

    let output = work_dir.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r#"
    test-key = "test-val"
    [EOF]
    "#);

    let output = work_dir.run_jj(["config", "list", "--repo"]);
    insta::assert_snapshot!(output, @r#"
    test-layered-key = "test-layered-val"
    [EOF]
    "#);

    // Workspace (new scope takes precedence over repo)
    // Add a workspace-level setting
    work_dir
        .run_jj([
            "config",
            "set",
            "--workspace",
            "test-layered-wks-key",
            "ws-val",
        ])
        .success();

    // Listing user shouldn't include workspace
    let output = work_dir.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r#"
    test-key = "test-val"
    [EOF]
    "#);

    // Workspace
    let output = work_dir.run_jj(["config", "list", "--workspace"]);
    insta::assert_snapshot!(output, @r#"
    test-layered-wks-key = "ws-val"
    [EOF]
    "#);
}

#[test]
fn test_config_list_origin() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    // User
    work_dir
        .run_jj(["config", "set", "--user", "test-key", "test-val"])
        .success();

    work_dir
        .run_jj([
            "config",
            "set",
            "--user",
            "test-layered-key",
            "test-original-val",
        ])
        .success();

    // Repo
    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "test-layered-key",
            "test-layered-val",
        ])
        .success();

    let output = work_dir.run_jj([
        "config",
        "list",
        "-Tbuiltin_config_list_detailed",
        "--config",
        "test-cli-key=test-cli-val",
    ]);
    insta::assert_snapshot!(output, @r#"
    test-key = "test-val" # user $TEST_ENV/config/config.toml
    test-layered-key = "test-layered-val" # repo $TEST_ENV/repo/.jj/repo/config.toml
    user.name = "Test User" # env
    user.email = "test.user@example.com" # env
    debug.commit-timestamp = "2001-02-03T04:05:11+07:00" # env
    debug.randomness-seed = 5 # env
    debug.operation-timestamp = "2001-02-03T04:05:11+07:00" # env
    operation.hostname = "host.example.com" # env
    operation.username = "test-username" # env
    test-cli-key = "test-cli-val" # cli
    [EOF]
    "#);

    let output = work_dir.run_jj([
        "config",
        "list",
        "-Tbuiltin_config_list_detailed",
        "--color=debug",
        "--include-defaults",
        "--include-overridden",
        "--config=test-key=test-cli-val",
        "test-key",
    ]);
    insta::assert_snapshot!(output, @r#"
    [38;5;8m<<config_list overridden name::# test-key>><<config_list overridden:: = >><<config_list overridden value::"test-val">><<config_list overridden:: # >><<config_list overridden source::user>><<config_list overridden:: >><<config_list overridden path::$TEST_ENV/config/config.toml>><<config_list overridden::>>[39m
    [38;5;2m<<config_list name::test-key>>[39m<<config_list:: = >>[38;5;3m<<config_list value::"test-cli-val">>[39m<<config_list:: # >>[38;5;4m<<config_list source::cli>>[39m<<config_list::>>
    [EOF]
    "#);

    let output = work_dir.run_jj([
        "config",
        "list",
        r#"-Tjson(self) ++ "\n""#,
        "--include-defaults",
        "--include-overridden",
        "--config=test-key=test-cli-val",
        "test-key",
    ]);
    insta::with_settings!({
        // Windows paths will be escaped in JSON syntax, which cannot be
        // normalized as $TEST_ENV.
        filters => [(r#""path":"[^"]*","#, r#""path":"<redacted>","#)],
    }, {
        insta::assert_snapshot!(output, @r#"
        {"name":"test-key","value":"test-val","source":"user","path":"<redacted>","is_overridden":true}
        {"name":"test-key","value":"test-cli-val","source":"cli","path":null,"is_overridden":false}
        [EOF]
        "#);
    });
}

#[test]
fn test_config_layer_override_default() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let config_key = "merge-tools.vimdiff.program";

    // Default
    let output = work_dir.run_jj(["config", "list", config_key, "--include-defaults"]);
    insta::assert_snapshot!(output, @r#"
    merge-tools.vimdiff.program = "vim"
    [EOF]
    "#);

    // User
    test_env.add_config(format!(
        "{config_key} = {value}\n",
        value = to_toml_value("user")
    ));
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    merge-tools.vimdiff.program = "user"
    [EOF]
    "#);

    // Repo
    work_dir.write_file(
        ".jj/repo/config.toml",
        format!("{config_key} = {value}\n", value = to_toml_value("repo")),
    );
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    merge-tools.vimdiff.program = "repo"
    [EOF]
    "#);

    // Command argument
    let output = work_dir.run_jj([
        "config",
        "list",
        config_key,
        "--config",
        &format!("{config_key}={value}", value = to_toml_value("command-arg")),
    ]);
    insta::assert_snapshot!(output, @r#"
    merge-tools.vimdiff.program = "command-arg"
    [EOF]
    "#);

    // Allow printing overridden values
    let output = work_dir.run_jj([
        "config",
        "list",
        config_key,
        "--include-overridden",
        "--config",
        &format!("{config_key}={value}", value = to_toml_value("command-arg")),
    ]);
    insta::assert_snapshot!(output, @r##"
    # merge-tools.vimdiff.program = "user"
    # merge-tools.vimdiff.program = "repo"
    merge-tools.vimdiff.program = "command-arg"
    [EOF]
    "##);

    let output = work_dir.run_jj([
        "config",
        "list",
        "--color=always",
        config_key,
        "--include-overridden",
    ]);
    insta::assert_snapshot!(output, @r#"
    [38;5;8m# merge-tools.vimdiff.program = "user"[39m
    [38;5;2mmerge-tools.vimdiff.program[39m = [38;5;3m"repo"[39m
    [EOF]
    "#);
}

#[test]
fn test_config_layer_override_env() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let config_key = "ui.editor";

    // Environment base
    test_env.add_env_var("EDITOR", "env-base");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "env-base"
    [EOF]
    "#);

    // User
    test_env.add_config(format!(
        "{config_key} = {value}\n",
        value = to_toml_value("user")
    ));
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "user"
    [EOF]
    "#);

    // Repo
    work_dir.write_file(
        ".jj/repo/config.toml",
        format!("{config_key} = {value}\n", value = to_toml_value("repo")),
    );
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "repo"
    [EOF]
    "#);

    // Environment override
    test_env.add_env_var("JJ_EDITOR", "env-override");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "env-override"
    [EOF]
    "#);

    // Command argument
    let output = work_dir.run_jj([
        "config",
        "list",
        config_key,
        "--config",
        &format!("{config_key}={value}", value = to_toml_value("command-arg")),
    ]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "command-arg"
    [EOF]
    "#);

    // Allow printing overridden values
    let output = work_dir.run_jj([
        "config",
        "list",
        config_key,
        "--include-overridden",
        "--config",
        &format!("{config_key}={value}", value = to_toml_value("command-arg")),
    ]);
    insta::assert_snapshot!(output, @r##"
    # ui.editor = "env-base"
    # ui.editor = "user"
    # ui.editor = "repo"
    # ui.editor = "env-override"
    ui.editor = "command-arg"
    [EOF]
    "##);
}

#[test]
fn test_config_layer_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");
    let config_key = "ui.editor";

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["new"]).success();
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();

    // Repo
    main_dir.write_file(
        ".jj/repo/config.toml",
        format!(
            "{config_key} = {value}\n",
            value = to_toml_value("main-repo")
        ),
    );
    let output = main_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "main-repo"
    [EOF]
    "#);
    let output = secondary_dir.run_jj(["config", "list", config_key]);
    insta::assert_snapshot!(output, @r#"
    ui.editor = "main-repo"
    [EOF]
    "#);
}

#[test]
fn test_config_set_bad_opts() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["config", "set"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--user|--repo|--workspace>
      <NAME>
      <VALUE>

    Usage: jj config set <--user|--repo|--workspace> <NAME> <VALUE>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(".", ["config", "set", "--user", "", "x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '' for '<NAME>': TOML parse error at line 1, column 1
      |
    1 | 
      | ^
    unquoted keys cannot be empty, expected letters, numbers, `-`, `_`


    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(".", ["config", "set", "--user", "x", "['typo'}"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '['typo'}' for '<VALUE>': TOML parse error at line 1, column 8
      |
    1 | ['typo'}
      |        ^
    missing comma between array elements, expected `,`


    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_config_set_for_user() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "test-key", "test-val"])
        .success();
    work_dir
        .run_jj(["config", "set", "--user", "test-table.foo", "true"])
        .success();
    work_dir
        .run_jj(["config", "set", "--user", "test-table.'bar()'", "0"])
        .success();

    // Ensure test-key successfully written to user config.
    let user_config_toml = std::fs::read_to_string(&user_config_path)
        .unwrap_or_else(|_| panic!("Failed to read file {}", user_config_path.display()));
    insta::assert_snapshot!(user_config_toml, @r#"
    #:schema https://jj-vcs.github.io/jj/latest/config-schema.json

    test-key = "test-val"

    [test-table]
    foo = true
    'bar()' = 0
    "#);
}

#[test]
fn test_config_set_for_user_directory() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["config", "set", "--user", "test-key", "test-val"])
        .success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.last_config_file_path()).unwrap(),
        @r#"
    test-key = "test-val"

    [template-aliases]
    'format_time_range(time_range)' = 'time_range.start() ++ " - " ++ time_range.end()'

    [git]
    colocate = false
    "#);

    // Add one more config file to the directory
    test_env.add_config("");
    let output = test_env.run_jj_in(
        ".",
        ["config", "set", "--user", "test-key", "test-other-val"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    1: $TEST_ENV/config/config0001.toml
    2: $TEST_ENV/config/config0002.toml
    Choose a config file (default 1): 1
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.first_config_file_path()).unwrap(),
        @r#"
    test-key = "test-other-val"

    [template-aliases]
    'format_time_range(time_range)' = 'time_range.start() ++ " - " ++ time_range.end()'

    [git]
    colocate = false
    "#);

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.last_config_file_path()).unwrap(),
        @"");
}

#[test]
fn test_config_set_for_repo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["config", "set", "--repo", "test-key", "test-val"])
        .success();
    work_dir
        .run_jj(["config", "set", "--repo", "test-table.foo", "true"])
        .success();
    // Ensure test-key successfully written to user config.
    let repo_config_toml = work_dir.read_file(".jj/repo/config.toml");
    insta::assert_snapshot!(repo_config_toml, @r#"
    #:schema https://jj-vcs.github.io/jj/latest/config-schema.json

    test-key = "test-val"

    [test-table]
    foo = true
    "#);
}

#[test]
fn test_config_set_for_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    let work_dir = test_env.work_dir("secondary");

    // set in workspace
    work_dir
        .run_jj(["config", "set", "--workspace", "test-key", "ws-val"])
        .success();

    // Read workspace config
    let workspace_config = work_dir.read_file(".jj/workspace-config.toml");
    insta::assert_snapshot!(workspace_config, @r#"
    #:schema https://jj-vcs.github.io/jj/latest/config-schema.json

    test-key = "ws-val"
    "#);
}

#[test]
fn test_config_set_toml_types() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    let set_value = |key, value| {
        work_dir
            .run_jj(["config", "set", "--user", key, value])
            .success();
    };
    set_value("test-table.integer", "42");
    set_value("test-table.float", "3.14");
    set_value("test-table.array", r#"["one", "two"]"#);
    set_value("test-table.boolean", "true");
    set_value("test-table.string", r#""foo""#);
    set_value("test-table.invalid", r"a + b");
    insta::assert_snapshot!(std::fs::read_to_string(&user_config_path).unwrap(), @r#"
    #:schema https://jj-vcs.github.io/jj/latest/config-schema.json

    [test-table]
    integer = 42
    float = 3.14
    array = ["one", "two"]
    boolean = true
    string = "foo"
    invalid = "a + b"
    "#);
}

#[test]
fn test_config_set_type_mismatch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "test-table.foo", "test-val"])
        .success();
    let output = work_dir.run_jj(["config", "set", "--user", "test-table", "not-a-table"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to set test-table
    Caused by: Would overwrite entire table test-table
    [EOF]
    [exit status: 1]
    ");

    // But it's fine to overwrite arrays and inline tables
    work_dir
        .run_jj(["config", "set", "--user", "test-table.array", "[1,2,3]"])
        .success();
    work_dir
        .run_jj(["config", "set", "--user", "test-table.array", "[4,5,6]"])
        .success();
    work_dir
        .run_jj(["config", "set", "--user", "test-table.inline", "{ x = 42}"])
        .success();
    work_dir
        .run_jj(["config", "set", "--user", "test-table.inline", "42"])
        .success();
}

#[test]
fn test_config_set_nontable_parent() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "test-nontable", "test-val"])
        .success();
    let output = work_dir.run_jj(["config", "set", "--user", "test-nontable.foo", "test-val"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to set test-nontable.foo
    Caused by: Would overwrite non-table value with parent table test-nontable
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_config_unset_non_existent_key() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["config", "unset", "--user", "nonexistent"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: "nonexistent" doesn't exist
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_config_unset_inline_table_key() {
    let mut test_env = TestEnvironment::default();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "inline-table", "{ foo = true }"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--user", "inline-table.foo"])
        .success();
    let user_config_toml = std::fs::read_to_string(&user_config_path).unwrap();
    insta::assert_snapshot!(user_config_toml, @r"
    #:schema https://jj-vcs.github.io/jj/latest/config-schema.json

    inline-table = {}
    ");
}

#[test]
fn test_config_unset_table_like() {
    let mut test_env = TestEnvironment::default();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);

    std::fs::write(
        &user_config_path,
        indoc! {b"
            inline-table = { foo = true }
            [non-inline-table]
            foo = true
        "},
    )
    .unwrap();

    // Inline table is syntactically a "value", so it can be deleted.
    test_env
        .run_jj_in(".", ["config", "unset", "--user", "inline-table"])
        .success();
    // Non-inline table cannot be deleted.
    let output = test_env.run_jj_in(".", ["config", "unset", "--user", "non-inline-table"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to unset non-inline-table
    Caused by: Would delete entire table non-inline-table
    [EOF]
    [exit status: 1]
    ");

    let user_config_toml = std::fs::read_to_string(&user_config_path).unwrap();
    insta::assert_snapshot!(user_config_toml, @r"
    [non-inline-table]
    foo = true
    ");
}

#[test]
fn test_config_unset_for_user() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.config_path().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "foo", "true"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--user", "foo"])
        .success();

    work_dir
        .run_jj(["config", "set", "--user", "table.foo", "true"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--user", "table.foo"])
        .success();

    work_dir
        .run_jj(["config", "set", "--user", "table.inline", "{ foo = true }"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--user", "table.inline"])
        .success();

    let user_config_toml = std::fs::read_to_string(&user_config_path).unwrap();
    insta::assert_snapshot!(user_config_toml, @"[table]");
}

#[test]
fn test_config_unset_for_repo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--repo", "test-key", "test-val"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--repo", "test-key"])
        .success();

    let repo_config_toml = work_dir.read_file(".jj/repo/config.toml");
    insta::assert_snapshot!(repo_config_toml, @"");
}

#[test]
fn test_config_unset_for_workspace() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    let work_dir = test_env.work_dir("secondary");

    // set then unset
    work_dir
        .run_jj(["config", "set", "--workspace", "foo", "bar"])
        .success();
    work_dir
        .run_jj(["config", "unset", "--workspace", "foo"])
        .success();

    let workspace_config = work_dir.read_file(".jj/workspace-config.toml");
    insta::assert_snapshot!(workspace_config, @"");
}

#[test]
fn test_config_edit_missing_opt() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["config", "edit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <--user|--repo|--workspace>

    Usage: jj config edit <--user|--repo|--workspace>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_config_edit_user() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Remove one of the config file to disambiguate
    std::fs::remove_file(test_env.last_config_file_path()).unwrap();
    let edit_script = test_env.set_up_fake_editor();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["config", "edit", "--user"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    assert_eq!(
        edited_path,
        dunce::simplified(&test_env.last_config_file_path())
    );
}

#[test]
fn test_config_edit_user_new_file() {
    let mut test_env = TestEnvironment::default();
    let user_config_path = test_env.config_path().join("config").join("file.toml");
    test_env.set_up_fake_editor(); // set $EDIT_SCRIPT, but added configuration is ignored
    test_env.add_env_var("EDITOR", fake_editor_path());
    test_env.set_config_path(&user_config_path);
    assert!(!user_config_path.exists());

    test_env
        .run_jj_in(".", ["config", "edit", "--user"])
        .success();
    assert!(
        user_config_path.exists(),
        "new file and directory should be created"
    );
}

#[test]
fn test_config_edit_repo() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let repo_config_path = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "config.toml"]));
    assert!(!repo_config_path.exists());

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["config", "edit", "--repo"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    assert_eq!(edited_path, dunce::simplified(&repo_config_path));
    assert!(repo_config_path.exists(), "new file should be created");
}

#[test]
fn test_config_edit_invalid_config() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();

    // Test re-edit
    std::fs::write(
        &edit_script,
        "write\ninvalid config here\0next invocation\n\0write\ntest=\"success\"",
    )
    .unwrap();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj_with(|cmd| {
        force_interactive(cmd)
            .args(["config", "edit", "--repo"])
            .write_stdin("Y\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Editing file: $TEST_ENV/repo/.jj/repo/config.toml
    Warning: An error has been found inside the config:
    Caused by:
    1: Configuration cannot be parsed as TOML document
    2: TOML parse error at line 1, column 9
      |
    1 | invalid config here
      |         ^
    key with no value, expected `=`

    Do you want to keep editing the file? If not, previous config will be restored. (Yn): [EOF]
    ");

    let output = work_dir.run_jj(["config", "get", "test"]);
    insta::assert_snapshot!(output, @r"
    success
    [EOF]"
    );

    // Test the restore previous config
    std::fs::write(&edit_script, "write\ninvalid config here").unwrap();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj_with(|cmd| {
        force_interactive(cmd)
            .args(["config", "edit", "--repo"])
            .write_stdin("n\n")
    });
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Editing file: $TEST_ENV/repo/.jj/repo/config.toml
    Warning: An error has been found inside the config:
    Caused by:
    1: Configuration cannot be parsed as TOML document
    2: TOML parse error at line 1, column 9
      |
    1 | invalid config here
      |         ^
    key with no value, expected `=`

    Do you want to keep editing the file? If not, previous config will be restored. (Yn): [EOF]
    ");

    let output = work_dir.run_jj(["config", "get", "test"]);
    insta::assert_snapshot!(output, @r"
    success
    [EOF]"
    );
}

#[test]
fn test_config_path() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let user_config_path = test_env.env_root().join("config.toml");
    let repo_config_path = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "config.toml"]));
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["config", "path", "--user"]), @r"
    $TEST_ENV/config.toml
    [EOF]
    ");
    assert!(
        !user_config_path.exists(),
        "jj config path shouldn't create new file"
    );

    insta::assert_snapshot!(work_dir.run_jj(["config", "path", "--repo"]), @r"
    $TEST_ENV/repo/.jj/repo/config.toml
    [EOF]
    ");
    assert!(
        !repo_config_path.exists(),
        "jj config path shouldn't create new file"
    );

    insta::assert_snapshot!(test_env.run_jj_in(".", ["config", "path", "--repo"]), @r"
    ------- stderr -------
    Error: No repo config path found
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_config_path_multiple() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let config_path = test_env.config_path().join("config.toml");
    let work_config_path = test_env.config_path().join("conf.d");
    let user_config_path = join_paths([config_path, work_config_path]).unwrap();
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");
    insta::assert_snapshot!(work_dir.run_jj(["config", "path", "--user"]), @r"
    $TEST_ENV/config/config.toml
    $TEST_ENV/config/conf.d
    [EOF]
    ");
}

#[test]
fn test_config_only_loads_toml_files() {
    let mut test_env = TestEnvironment::default();
    test_env.set_up_fake_editor();
    std::fs::File::create(test_env.config_path().join("is-not.loaded")).unwrap();
    insta::assert_snapshot!(test_env.run_jj_in(".", ["config", "edit", "--user"]), @r"
    ------- stderr -------
    1: $TEST_ENV/config/config0001.toml
    2: $TEST_ENV/config/config0002.toml
    Choose a config file (default 1): 1
    Editing file: $TEST_ENV/config/config0001.toml
    [EOF]
    ");
}

#[test]
fn test_config_edit_repo_outside_repo() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["config", "edit", "--repo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No repo config path found to edit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_config_get() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    [table]
    string = "some value 1"
    int = 123
    list = ["list", "value"]
    overridden = "foo"
    "#,
    );
    test_env.add_config(
        r#"
    [table]
    overridden = "bar"
    "#,
    );

    let output = test_env.run_jj_in(".", ["config", "get", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Value not found for nonexistent
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(".", ["config", "get", "table.string"]);
    insta::assert_snapshot!(output, @r"
    some value 1
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["config", "get", "table.int"]);
    insta::assert_snapshot!(output, @r"
    123
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["config", "get", "table.list"]);
    insta::assert_snapshot!(output, @r#"
    ["list", "value"]
    [EOF]
    "#);

    let output = test_env.run_jj_in(".", ["config", "get", "table"]);
    insta::assert_snapshot!(output, @r#"
    { string = "some value 1", int = 123, list = ["list", "value"], overridden = "bar" }
    [EOF]
    "#);

    let output = test_env.run_jj_in(".", ["config", "get", "table.overridden"]);
    insta::assert_snapshot!(output, @r"
    bar
    [EOF]
    ");
}

#[test]
fn test_config_get_yields_values_consistent_with_schema_defaults() {
    let mut test_env = TestEnvironment::default();

    // The default test environment may already contain configuration that's
    // different from the true default, e.g. `git.colocate = false`. So we
    // explicitly set the config to an empty one in order to test the true
    // default config values.
    let config_dir = test_env.env_root().join("empty-config");
    std::fs::create_dir(&config_dir).unwrap();
    test_env.set_config_path(&config_dir);

    let get_true_default = move |key: &str| {
        let output = test_env.run_jj_in(".", ["config", "get", key]).success();
        let output_doc = toml_edit::Document::parse(format!("test={}", output.stdout.normalized()))
            .unwrap_or_else(|_| {
                // Unfortunately for this test, `config get` is "lossy" and does not print
                // quoted strings. This means that e.g. `false` and `"false"` are not
                // distinguishable. If value couldn't be parsed, it's probably a string, so
                // let's parse its Debug string instead.
                toml_edit::Document::parse(format!("test={:?}", output.stdout.normalized().trim()))
                    .unwrap()
            });
        output_doc.get("test").unwrap().as_value().unwrap().clone()
    };

    let mut schema_defaults = toml_edit::ser::to_document(&default_config_from_schema()).unwrap();

    // Ensure that `get_values()` flattens the entire configuration.
    struct SetDotted;
    impl toml_edit::visit_mut::VisitMut for SetDotted {
        fn visit_table_like_mut(&mut self, node: &mut dyn toml_edit::TableLike) {
            node.set_dotted(true);
            toml_edit::visit_mut::visit_table_like_mut(self, node);
        }
    }
    toml_edit::visit_mut::visit_document_mut(&mut SetDotted, &mut schema_defaults);

    for (key, schema_default) in schema_defaults.into_table().get_values() {
        let key = key.iter().join(".");
        match key.as_str() {
            // These keys technically don't have a default value, but they exhibit a default
            // behavior consistent with the value claimed by the schema. When these defaults are
            // used, a hint is printed to stdout.
            "ui.default-command" => insta::assert_snapshot!(schema_default, @r#""log""#),
            "ui.diff-editor" => insta::assert_snapshot!(schema_default, @r#"":builtin""#),
            "ui.merge-editor" => insta::assert_snapshot!(schema_default, @r#"":builtin""#),
            "git.fetch" => insta::assert_snapshot!(schema_default, @r#""origin""#),
            "git.push" => insta::assert_snapshot!(schema_default, @r#""origin""#),

            // When no `short-prefixes` revset is explicitly configured, the revset for `log` is
            // used instead, even if that has a value different from the default. The schema
            // represents this behavior with a symbolic default value.
            "revsets.short-prefixes" => {
                insta::assert_snapshot!(schema_default, @r#""<revsets.log>""#);
            }

            // The default for `ui.pager` is a table; `ui.pager.command` is an array and `jj config
            // get` currently cannot print that. The schema default omits the env variable
            // `LESSCHARSET` and gives the default as a plain string.
            "ui.pager" => insta::assert_snapshot!(schema_default, @r#""less -FRX""#),

            // The `immutable_heads()` revset actually defaults to `builtin_immutable_heads()` but
            // this would be a poor starting point for a custom revset, so the schema "inlines"
            // `builtin_immutable_heads()`.
            r#"revset-aliases."immutable_heads()""# => {
                let builtin_default =
                    get_true_default("revset-aliases.'builtin_immutable_heads()'");
                assert!(
                    builtin_default.to_string() == schema_default.to_string(),
                    "{key}: the schema claims a default ({schema_default}) which is different \
                     from what builtin_immutable_heads() resolves to ({builtin_default})"
                );
            }

            _ => {
                let true_default = get_true_default(&key);
                assert!(
                    true_default.to_string() == schema_default.to_string(),
                    "{key}: true default value ({true_default}) is not consistent with default \
                     claimed by schema ({schema_default})"
                );
            }
        }
    }
}

#[test]
fn test_config_path_syntax() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
    a.'b()' = 0
    'b c'.d = 1
    'b c'.e.'f[]' = 2
    - = 3
    _ = 4
    '.' = 5
    "#,
    );

    let output = test_env.run_jj_in(".", ["config", "list", "a.'b()'"]);
    insta::assert_snapshot!(output, @r"
    a.'b()' = 0
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "list", "'b c'"]);
    insta::assert_snapshot!(output, @r#"
    'b c'.d = 1
    'b c'.e."f[]" = 2
    [EOF]
    "#);
    let output = test_env.run_jj_in(".", ["config", "list", "'b c'.d"]);
    insta::assert_snapshot!(output, @r"
    'b c'.d = 1
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "list", "'b c'.e.'f[]'"]);
    insta::assert_snapshot!(output, @r"
    'b c'.e.'f[]' = 2
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "get", "'b c'.e.'f[]'"]);
    insta::assert_snapshot!(output, @r"
    2
    [EOF]
    ");

    // Not a table
    let output = test_env.run_jj_in(".", ["config", "list", "a.'b()'.x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching config key for a.'b()'.x
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "get", "a.'b()'.x"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Value not found for a.'b()'.x
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    // "-" and "_" are valid TOML keys
    let output = test_env.run_jj_in(".", ["config", "list", "-"]);
    insta::assert_snapshot!(output, @r"
    - = 3
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "list", "_"]);
    insta::assert_snapshot!(output, @r"
    _ = 4
    [EOF]
    ");

    // "." requires quoting
    let output = test_env.run_jj_in(".", ["config", "list", "'.'"]);
    insta::assert_snapshot!(output, @r"
    '.' = 5
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "get", "'.'"]);
    insta::assert_snapshot!(output, @r"
    5
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "get", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '.' for '<NAME>': TOML parse error at line 1, column 1
      |
    1 | .
      | ^
    unquoted keys cannot be empty, expected letters, numbers, `-`, `_`


    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Invalid TOML keys
    let output = test_env.run_jj_in(".", ["config", "list", "b c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'b c' for '[NAME]': TOML parse error at line 1, column 3
      |
    1 | b c
      |   ^
    unexpected content, expected nothing


    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
    let output = test_env.run_jj_in(".", ["config", "list", ""]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '' for '[NAME]': TOML parse error at line 1, column 1
      |
    1 | 
      | ^
    unquoted keys cannot be empty, expected letters, numbers, `-`, `_`


    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
#[cfg_attr(windows, ignore = "dirs::home_dir() can't be overridden by $HOME")] // TODO
fn test_config_conditional() {
    let mut test_env = TestEnvironment::default();
    let home_dir = test_env.work_dir(test_env.home_dir());
    home_dir.run_jj(["git", "init", "repo1"]).success();
    home_dir.run_jj(["git", "init", "repo2"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.env_root().join("config.toml");
    test_env.set_config_path(&user_config_path);
    std::fs::write(
        &user_config_path,
        indoc! {"
            foo = 'global'
            baz = 'global'
            qux = 'global'

            [[--scope]]
            --when.repositories = ['~/repo1']
            foo = 'repo1'
            [[--scope]]
            --when.repositories = ['~/repo2']
            foo = 'repo2'
            [[--scope]]
            --when.workspaces = ['~/repo2']
            foo2 = 'repo2'
            [[--scope]]
            --when.workspaces = ['~/repo2_1']
            foo2 = 'repo2_1'

            [[--scope]]
            --when.commands = ['config']
            baz = 'config'
            [[--scope]]
            --when.commands = ['config get']
            qux = 'get'
            [[--scope]]
            --when.commands = ['config list']
            qux = 'list'
        "},
    )
    .unwrap();
    let home_dir = test_env.work_dir(test_env.home_dir());
    let work_dir1 = home_dir.dir("repo1");
    let work_dir2 = home_dir.dir("repo2");
    let work_dir2_1 = home_dir.dir("repo2_1");
    work_dir2
        .run_jj(&["workspace", "add", "../repo2_1"])
        .success();

    // get and list should refer to the resolved config
    let output = test_env.run_jj_in(".", ["config", "get", "foo"]);
    insta::assert_snapshot!(output, @r"
    global
    [EOF]
    ");
    let output = work_dir1.run_jj(["config", "get", "foo"]);
    insta::assert_snapshot!(output, @r"
    repo1
    [EOF]
    ");
    // baz should be the same for `jj config get` and `jj config list`
    // qux should be different
    let output = work_dir1.run_jj(["config", "get", "baz"]);
    insta::assert_snapshot!(output, @r"
    config
    [EOF]
    ");
    let output = work_dir1.run_jj(["config", "get", "qux"]);
    insta::assert_snapshot!(output, @r"
    get
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r"
    foo = 'global'
    baz = 'config'
    qux = 'list'
    [EOF]
    ");
    let output = work_dir1.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r"
    foo = 'repo1'
    baz = 'config'
    qux = 'list'
    [EOF]
    ");
    let output = work_dir2.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r"
    foo = 'repo2'
    foo2 = 'repo2'
    baz = 'config'
    qux = 'list'
    [EOF]
    ");
    let output = work_dir2_1.run_jj(["config", "list", "--user"]);
    insta::assert_snapshot!(output, @r"
    foo = 'repo2'
    foo2 = 'repo2_1'
    baz = 'config'
    qux = 'list'
    [EOF]
    ");

    // relative workspace path
    let output = work_dir2.run_jj(["config", "list", "--user", "-R../repo1"]);
    insta::assert_snapshot!(output, @r"
    foo = 'repo1'
    baz = 'config'
    qux = 'list'
    [EOF]
    ");

    // set and unset should refer to the source config
    // (there's no option to update scoped table right now.)
    let output = test_env.run_jj_in(".", ["config", "set", "--user", "bar", "new value"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(std::fs::read_to_string(&user_config_path).unwrap(), @r#"
    foo = 'global'
    baz = 'global'
    qux = 'global'
    bar = "new value"

    [[--scope]]
    --when.repositories = ['~/repo1']
    foo = 'repo1'
    [[--scope]]
    --when.repositories = ['~/repo2']
    foo = 'repo2'
    [[--scope]]
    --when.workspaces = ['~/repo2']
    foo2 = 'repo2'
    [[--scope]]
    --when.workspaces = ['~/repo2_1']
    foo2 = 'repo2_1'

    [[--scope]]
    --when.commands = ['config']
    baz = 'config'
    [[--scope]]
    --when.commands = ['config get']
    qux = 'get'
    [[--scope]]
    --when.commands = ['config list']
    qux = 'list'
    "#);
    let output = work_dir1.run_jj(["config", "unset", "--user", "foo"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(std::fs::read_to_string(&user_config_path).unwrap(), @r#"
    baz = 'global'
    qux = 'global'
    bar = "new value"

    [[--scope]]
    --when.repositories = ['~/repo1']
    foo = 'repo1'
    [[--scope]]
    --when.repositories = ['~/repo2']
    foo = 'repo2'
    [[--scope]]
    --when.workspaces = ['~/repo2']
    foo2 = 'repo2'
    [[--scope]]
    --when.workspaces = ['~/repo2_1']
    foo2 = 'repo2_1'

    [[--scope]]
    --when.commands = ['config']
    baz = 'config'
    [[--scope]]
    --when.commands = ['config get']
    qux = 'get'
    [[--scope]]
    --when.commands = ['config list']
    qux = 'list'
    "#);
}

// Minimal test for Windows where the home directory can't be switched.
// (Can be removed if test_config_conditional() is enabled on Windows.)
#[test]
fn test_config_conditional_without_home_dir() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // Test with fresh new config file
    let user_config_path = test_env.env_root().join("config.toml");
    test_env.set_config_path(&user_config_path);
    let work_dir = test_env.work_dir("repo");
    std::fs::write(
        &user_config_path,
        format!(
            indoc! {"
                foo = 'global'
                [[--scope]]
                --when.repositories = [{repo_path}]
                foo = 'repo'
            "},
            // "\\?\" paths shouldn't be required on Windows
            repo_path = to_toml_value(dunce::simplified(work_dir.root()).to_str().unwrap())
        ),
    )
    .unwrap();

    let output = test_env.run_jj_in(".", ["config", "get", "foo"]);
    insta::assert_snapshot!(output, @r"
    global
    [EOF]
    ");
    let output = work_dir.run_jj(["config", "get", "foo"]);
    insta::assert_snapshot!(output, @r"
    repo
    [EOF]
    ");
}

#[test]
fn test_config_show_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["config", "set", "--user", "ui.paginate", ":builtin"])
        .success();
    let output = test_env.run_jj_in(".", ["st"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid type or value for ui.paginate
    Caused by: unknown variant `:builtin`, expected `never` or `auto`

    Hint: Check the config file: $TEST_ENV/config/config0001.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_config_author_change_warning() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["config", "set", "--repo", "user.email", "'Foo'"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: This setting will only impact future commits.
    The author of the working copy will stay "Test User <test.user@example.com>".
    To change the working copy author, use "jj metaedit --update-author"
    [EOF]
    "#);

    // test_env.run_jj*() resets state for every invocation
    // for this test, the state (user.email) is needed
    work_dir
        .run_jj_with(|cmd| {
            cmd.args(["metaedit", "--update-author"])
                .env_remove("JJ_EMAIL")
        })
        .success();

    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm Foo 2001-02-03 08:05:09 c2090b51
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_config_author_change_warning_root_env() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["config", "set", "--user", "user.email", "'Foo'"]);
    insta::assert_snapshot!(output, @"");
}

fn find_stdout_lines(keyname_pattern: &str, stdout: &str) -> String {
    let key_line_re = Regex::new(&format!(r"(?m)^{keyname_pattern} = .*\n")).unwrap();
    key_line_re.find_iter(stdout).map(|m| m.as_str()).collect()
}
