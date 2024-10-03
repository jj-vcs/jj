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

use itertools::Itertools;
use regex::Regex;

use crate::common::get_stdout_string;
use crate::common::strip_last_line;
use crate::common::TestEnvironment;

#[test]
fn test_op_log() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "description 0"]);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "op",
            "log",
            "--config-toml",
            "template-aliases.'format_time_range(x)' = 'x'",
        ],
    );
    insta::assert_snapshot!(&stdout, @r###"
    @  c1851f1c3d90 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);
    let op_log_lines = stdout.lines().collect_vec();
    let add_workspace_id = op_log_lines[3].split(' ').nth(2).unwrap();
    let initialize_repo_id = op_log_lines[5].split(' ').nth(2).unwrap();

    // Can load the repo at a specific operation ID
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, initialize_repo_id), @r###"
    ◆  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, add_workspace_id), @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◆  0000000000000000000000000000000000000000
    "###);
    // "@" resolves to the head operation
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, "@"), @r###"
    @  19611c995a342c01f525583e5fcafdd211f6d009
    ◆  0000000000000000000000000000000000000000
    "###);
    // "@-" resolves to the parent of the head operation
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, "@-"), @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◆  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_failure(&repo_path, &["log", "--at-op", "@----"]), @r###"
    Error: The "@----" expression resolved to no operations
    "###);

    // We get a reasonable message if an invalid operation ID is specified
    insta::assert_snapshot!(test_env.jj_cmd_failure(&repo_path, &["log", "--at-op", "foo"]), @r###"
    Error: Operation ID "foo" is not a valid hexadecimal prefix
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log", "--op-diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    @  c1851f1c3d90 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 19611c99 (empty) description 0
    │     - qpvuntsm hidden 230dd059 (empty) (no description set)
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 230dd059 (empty) (no description set)
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    │
    │  Changed commits:
    │  ○  Change zzzzzzzzzzzz
    │     + zzzzzzzz 00000000 (empty) (no description set)
    ○  000000000000 root()
    "#);

    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "description 1"]);
    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "describe",
            "-m",
            "description 2",
            "--at-op",
            add_workspace_id,
        ],
    );
    insta::assert_snapshot!(test_env.jj_cmd_failure(&repo_path, &["log", "--at-op", "@-"]), @r###"
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: c05f56726c27, f9fdb4e100bd
    "###);
}

#[test]
fn test_op_log_with_custom_symbols() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "description 0"]);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "op",
            "log",
            "--config-toml",
            concat!(
                "template-aliases.'format_time_range(x)' = 'x'\n",
                "templates.op_log_node = 'if(current_operation, \"$\", if(root, \"┴\", \"┝\"))'",
            ),
        ],
    );
    insta::assert_snapshot!(&stdout, @r###"
    $  c1851f1c3d90 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    ┝  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ┝  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ┴  000000000000 root()
    "###);
}

#[test]
fn test_op_log_with_no_template() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    let stderr = test_env.jj_cmd_cli_error(&repo_path, &["op", "log", "-T"]);
    insta::assert_snapshot!(stderr, @r#"
    error: a value is required for '--template <TEMPLATE>' but none was supplied

    For more information, try '--help'.
    Hint: The following template aliases are defined:
    - builtin_log_comfortable
    - builtin_log_compact
    - builtin_log_detailed
    - builtin_log_node
    - builtin_log_node_ascii
    - builtin_log_oneline
    - builtin_op_log_comfortable
    - builtin_op_log_compact
    - builtin_op_log_node
    - builtin_op_log_node_ascii
    - commit_summary_separator
    - description_placeholder
    - email_placeholder
    - name_placeholder
    "#);
}

#[test]
fn test_op_log_limit() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log", "-Tdescription", "--limit=1"]);
    insta::assert_snapshot!(stdout, @r###"
    @  add workspace 'default'
    "###);
}

#[test]
fn test_op_log_no_graph() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    let stdout =
        test_env.jj_cmd_success(&repo_path, &["op", "log", "--no-graph", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    [1m[38;5;12mb51416386f26[39m [38;5;3mtest-username@host.example.com[39m [38;5;14m2001-02-03 04:05:07.000 +07:00[39m - [38;5;14m2001-02-03 04:05:07.000 +07:00[39m[0m
    [1madd workspace 'default'[0m
    [38;5;4m9a7d829846af[39m [38;5;3mtest-username@host.example.com[39m [38;5;6m2001-02-03 04:05:07.000 +07:00[39m - [38;5;6m2001-02-03 04:05:07.000 +07:00[39m
    initialize repo
    [38;5;4m000000000000[39m [38;5;2mroot()[39m
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log", "--op-diff", "--no-graph"]);
    insta::assert_snapshot!(&stdout, @r#"
    b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'

    Changed commits:
    Change qpvuntsmwlqt
    + qpvuntsm 230dd059 (empty) (no description set)
    9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    initialize repo

    Changed commits:
    Change zzzzzzzzzzzz
    + zzzzzzzz 00000000 (empty) (no description set)
    000000000000 root()
    "#);
}

#[test]
fn test_op_log_no_graph_null_terminated() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "message1"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "message2"]);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "op",
            "log",
            "--no-graph",
            "--template",
            r#"id.short(4) ++ "\0""#,
        ],
    );
    insta::assert_debug_snapshot!(stdout, @r###""8a30\05cec\0b514\09a7d\00000\0""###);
}

#[test]
fn test_op_log_template() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    let render = |template| test_env.jj_cmd_success(&repo_path, &["op", "log", "-T", template]);

    insta::assert_snapshot!(render(r#"id ++ "\n""#), @r###"
    @  b51416386f2685fd5493f2b20e8eec3c24a1776d9e1a7cb5ed7e30d2d9c88c0c1e1fe71b0b7358cba60de42533d1228ed9878f2f89817d892c803395ccf9fe92
    ○  9a7d829846af88a2f7a1e348fb46ff58729e49632bc9c6a052aec8501563cb0d10f4a4e6010ffde529f84a2b9b5b3a4c211a889106a41f6c076dfdacc79f6af7
    ○  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        render(r#"separate(" ", id.short(5), current_operation, user,
                                time.start(), time.end(), time.duration()) ++ "\n""#), @r###"
    @  b5141 true test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ○  9a7d8 false test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ○  00000 false @ 1970-01-01 00:00:00.000 +00:00 1970-01-01 00:00:00.000 +00:00 less than a microsecond
    "###);

    // Negative length shouldn't cause panic.
    insta::assert_snapshot!(render(r#"id.short(-1) ++ "|""#), @r###"
    @  <Error: out of range integral type conversion attempted>|
    ○  <Error: out of range integral type conversion attempted>|
    ○  <Error: out of range integral type conversion attempted>|
    "###);

    // Test the default template, i.e. with relative start time and duration. We
    // don't generally use that template because it depends on the current time,
    // so we need to reset the time range format here.
    test_env.add_config(
        r#"
[template-aliases]
'format_time_range(time_range)' = 'time_range.start().ago() ++ ", lasted " ++ time_range.duration()'
        "#,
    );
    let regex = Regex::new(r"\d\d years").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(regex.replace_all(&stdout, "NN years"), @r###"
    @  b51416386f26 test-username@host.example.com NN years ago, lasted less than a microsecond
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com NN years ago, lasted less than a microsecond
    │  initialize repo
    ○  000000000000 root()
    "###);
}

#[test]
fn test_op_log_builtin_templates() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    // Render without graph and append "[EOF]" marker to test line ending
    let render = |template| {
        test_env.jj_cmd_success(&repo_path, &["op", "log", "-T", template, "--no-graph"])
            + "[EOF]\n"
    };
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "description 0"]);

    insta::assert_snapshot!(render(r#"builtin_op_log_compact"#), @r###"
    c1851f1c3d90 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    args: jj describe -m 'description 0'
    b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'
    9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    initialize repo
    000000000000 root()
    [EOF]
    "###);

    insta::assert_snapshot!(render(r#"builtin_op_log_comfortable"#), @r###"
    c1851f1c3d90 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    args: jj describe -m 'description 0'

    b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'

    9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    initialize repo

    000000000000 root()

    [EOF]
    "###);
}

#[test]
fn test_op_log_word_wrap() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    std::fs::write(repo_path.join("file1"), "foo\n".repeat(100)).unwrap();
    test_env.jj_cmd_ok(&repo_path, &["debug", "snapshot"]);

    let render = |args: &[&str], columns: u32, word_wrap: bool| {
        let mut args = args.to_vec();
        if word_wrap {
            args.push("--config-toml=ui.log-word-wrap=true");
        }
        let assert = test_env
            .jj_cmd(&repo_path, &args)
            .env("COLUMNS", columns.to_string())
            .assert()
            .success()
            .stderr("");
        get_stdout_string(&assert)
    };

    // ui.log-word-wrap option works
    insta::assert_snapshot!(render(&["op", "log"], 40, false), @r#"
    @  cf25045e0079 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "#);
    insta::assert_snapshot!(render(&["op", "log"], 40, true), @r#"
    @  cf25045e0079
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  b51416386f26
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "#);

    // Nested graph should be wrapped
    insta::assert_snapshot!(render(&["op", "log", "--op-diff"], 40, true), @r#"
    @  cf25045e0079
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm e292def1 (no
    │     description set)
    │     - qpvuntsm hidden 230dd059 (empty)
    │     (no description set)
    ○  b51416386f26
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 230dd059 (empty) (no
    │     description set)
    ○  9a7d829846af
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    │
    │  Changed commits:
    │  ○  Change zzzzzzzzzzzz
    │     + zzzzzzzz 00000000 (empty) (no
    │     description set)
    ○  000000000000 root()
    "#);

    // Nested diff stat shouldn't exceed the terminal width
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--stat"], 40, true), @r#"
    @  cf25045e0079
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm e292def1 (no
    │     description set)
    │     - qpvuntsm hidden 230dd059 (empty)
    │     (no description set)
    │     file1 | 100 +++++++++++++++++++
    │     1 file changed, 100 insertions(+), 0 deletions(-)
    "#);
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--no-graph", "--stat"], 40, true), @r#"
    cf25045e0079
    test-username@host.example.com
    2001-02-03 04:05:08.000 +07:00 -
    2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj debug snapshot

    Changed commits:
    Change qpvuntsmwlqt
    + qpvuntsm e292def1 (no description set)
    - qpvuntsm hidden 230dd059 (empty) (no
    description set)
    file1 | 100 +++++++++++++++++++++++++
    1 file changed, 100 insertions(+), 0 deletions(-)
    "#);

    // Nested graph widths should be subtracted from the term width
    let config = r#"templates.commit_summary='"0 1 2 3 4 5 6 7 8 9"'"#;
    insta::assert_snapshot!(
        render(&["op", "log", "-T''", "--op-diff", "-n1", "--config-toml", config], 15, true), @r#"
    @
    │
    │  Changed
    │  commits:
    │  ○  Change
    │     qpvuntsmwlqt
    │     + 0 1 2 3
    │     4 5 6 7 8
    │     9
    │     - 0 1 2 3
    │     4 5 6 7 8
    │     9
    "#);
}

#[test]
fn test_op_log_configurable() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"operation.hostname = "my-hostname"
        operation.username = "my-username"
        "#,
    );
    test_env
        .jj_cmd(test_env.env_root(), &["git", "init", "repo"])
        .env_remove("JJ_OP_HOSTNAME")
        .env_remove("JJ_OP_USERNAME")
        .assert()
        .success();
    let repo_path = test_env.env_root().join("repo");

    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    assert!(stdout.contains("my-username@my-hostname"));
}

#[test]
fn test_op_abandon_ancestors() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 1"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 2"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  c2878c428b1c test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 81a4ef3dd421f3184289df1c58bd3a16ea1e3d8e
    │  args: jj commit -m 'commit 2'
    ○  5d0ab09ab0fa test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj commit -m 'commit 1'
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);

    // Abandon old operations. The working-copy operation id should be updated.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "..@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 3 operations and reparented 1 descendant operations.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "local-working-copy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("8545e013752445fd845c84eb961dbfbce47e1deb628e4ef20df10f6dc9aae2ef9e47200b0fcc70ca51f050aede05d0fa6dd1db40e20ae740876775738a07d02e")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  8545e0137524 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 81a4ef3dd421f3184289df1c58bd3a16ea1e3d8e
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    "###);

    // Abandon operation range.
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 3"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 4"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 5"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@---..@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 2 operations and reparented 1 descendant operations.
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  d92d0753399f test-username@host.example.com 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    │  commit c5f7dd51add0046405055336ef443f882a0a8968
    │  args: jj commit -m 'commit 5'
    ○  8545e0137524 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 81a4ef3dd421f3184289df1c58bd3a16ea1e3d8e
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    "###);

    // Can't abandon the current operation.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", "..@"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot abandon the current operation d92d0753399f
    Hint: Run `jj undo` to revert the current operation, then use `jj op abandon`
    "###);

    // Can't create concurrent abandoned operations explicitly.
    let stderr = test_env.jj_cmd_cli_error(&repo_path, &["op", "abandon", "--at-op=@-", "@"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: --at-op is not respected
    "###);

    // Abandon the current operation by undoing it first.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 1 operations and reparented 1 descendant operations.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "local-working-copy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("0699d720d0cecd80fb7d765c45955708c61b12feb1d7ed9ff2777ae719471f04ffed3c1dc24efdbf94bdb74426065d6fa9a4f0862a89db2c8c8e359eefc45462")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  0699d720d0ce test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation d92d0753399f732e438bdd88fa7e5214cba2a310d120ec1714028a514c7116bcf04b4a0b26c04dbecf0a917f1d4c8eb05571b8816dd98b0502aaf321e92500b3
    │  args: jj undo
    ○  8545e0137524 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 81a4ef3dd421f3184289df1c58bd3a16ea1e3d8e
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    "###);

    // Abandon empty range.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@-..@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log", "-n1"]), @r###"
    @  0699d720d0ce test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation d92d0753399f732e438bdd88fa7e5214cba2a310d120ec1714028a514c7116bcf04b4a0b26c04dbecf0a917f1d4c8eb05571b8816dd98b0502aaf321e92500b3
    │  args: jj undo
    "###);
}

#[test]
fn test_op_abandon_without_updating_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 1"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 2"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 3"]);

    // Abandon without updating the working copy.
    let (_stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &["op", "abandon", "@-", "--ignore-working-copy"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 1 operations and reparented 1 descendant operations.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "local-working-copy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("cd2b4690faf20cdc477e90c224f15a1f4d62b4d16d0d515fc0f9c998ff91a971cb114d82075c9a7331f3f94d7188c1f93628b7b93e4ca77ac89435a7b536de1e")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["op", "log", "-n1", "--ignore-working-copy"]), @r###"
    @  467d42715f00 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 220cb0b1b5d1c03cc0d351139d824598bb3c1967
    │  args: jj commit -m 'commit 3'
    "###);

    // The working-copy operation id isn't updated if it differs from the repo.
    // It could be updated if the tree matches, but there's no extra logic for
    // that.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 1 operations and reparented 1 descendant operations.
    Warning: The working copy operation cd2b4690faf2 is not updated because it differs from the repo 467d42715f00.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "local-working-copy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("cd2b4690faf20cdc477e90c224f15a1f4d62b4d16d0d515fc0f9c998ff91a971cb114d82075c9a7331f3f94d7188c1f93628b7b93e4ca77ac89435a7b536de1e")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["op", "log", "-n1", "--ignore-working-copy"]), @r###"
    @  050b33d674ff test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 220cb0b1b5d1c03cc0d351139d824598bb3c1967
    │  args: jj commit -m 'commit 3'
    "###);
}

#[test]
fn test_op_abandon_multiple_heads() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Create 1 base operation + 2 operations to be diverged.
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 1"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 2"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 3"]);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#],
    );
    let (head_op_id, prev_op_id) = stdout.lines().next_tuple().unwrap();
    insta::assert_snapshot!(head_op_id, @"cd2b4690faf2");
    insta::assert_snapshot!(prev_op_id, @"c2878c428b1c");

    // Create 1 other concurrent operation.
    test_env.jj_cmd_ok(&repo_path, &["commit", "--at-op=@--", "-m", "commit 4"]);

    // Can't resolve operation relative to @.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", "@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: cd2b4690faf2, 603773cdd351
    "###);
    let (_, other_head_op_id) = stderr.trim_end().rsplit_once(", ").unwrap();
    insta::assert_snapshot!(other_head_op_id, @"603773cdd351");
    assert_ne!(head_op_id, other_head_op_id);

    // Can't abandon one of the head operations.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", head_op_id]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot abandon the current operation cd2b4690faf2
    "###);

    // Can't abandon the other head operation.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", other_head_op_id]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot abandon the current operation 603773cdd351
    "###);

    // Can abandon the operation which is not an ancestor of the other head.
    // This would crash if we attempted to remap the unchanged op in the op
    // heads store.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", prev_op_id]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 1 operations and reparented 2 descendant operations.
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(stdout, @r###"
    @    fb7f33a29cd8 test-username@host.example.com 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  467d42715f00 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  commit 220cb0b1b5d1c03cc0d351139d824598bb3c1967
    │ │  args: jj commit -m 'commit 3'
    │ ○  603773cdd351 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  commit 81a4ef3dd421f3184289df1c58bd3a16ea1e3d8e
    │    args: jj commit '--at-op=@--' -m 'commit 4'
    ○  5d0ab09ab0fa test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj commit -m 'commit 1'
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);
    insta::assert_snapshot!(stderr, @r###"
    Concurrent modification detected, resolving automatically.
    "###);
}

#[test]
fn test_op_recover_from_bad_gc() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo", "--colocate"]);
    let repo_path = test_env.env_root().join("repo");
    let git_object_path = |hex: &str| {
        let (shard, file_name) = hex.split_at(2);
        let mut file_path = repo_path.to_owned();
        file_path.extend([".git", "objects", shard, file_name]);
        file_path
    };

    test_env.jj_cmd_ok(&repo_path, &["describe", "-m1"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m2"]); // victim
    test_env.jj_cmd_ok(&repo_path, &["abandon"]); // break predecessors chain
    test_env.jj_cmd_ok(&repo_path, &["new", "-m3"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m4"]);

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#],
    );
    let (head_op_id, _, _, bad_op_id) = stdout.lines().next_tuple().unwrap();
    insta::assert_snapshot!(head_op_id, @"43d51d9b0c0c");
    insta::assert_snapshot!(bad_op_id, @"e02cba71e65d");

    // Corrupt the repo by removing hidden but reachable commit object.
    let bad_commit_id = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "--at-op",
            bad_op_id,
            "--no-graph",
            "-r@",
            "-Tcommit_id",
        ],
    );
    insta::assert_snapshot!(bad_commit_id, @"ddf84fc5e0dd314092b3dfb13e09e37fa7d04ef9");
    std::fs::remove_file(git_object_path(&bad_commit_id)).unwrap();

    // Do concurrent modification to make the situation even worse. At this
    // point, the index can be loaded, so this command succeeds.
    test_env.jj_cmd_ok(&repo_path, &["--at-op=@-", "describe", "-m4.1"]);

    let stderr =
        test_env.jj_cmd_internal_error(&repo_path, &["--at-op", head_op_id, "debug", "reindex"]);
    insta::assert_snapshot!(strip_last_line(&stderr), @r###"
    Internal error: Failed to index commits at operation e02cba71e65dc9b58bb46e4a31500d94c31ce217f19459883fc85c2d1cfde7124f556565be4f8a3bcd059fe1faee317c3e3eacd3034d419083fd6b85aea396f4
    Caused by:
    1: Object ddf84fc5e0dd314092b3dfb13e09e37fa7d04ef9 of type commit not found
    "###);

    // "op log" should still be usable.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &["op", "log", "--ignore-working-copy", "--at-op", head_op_id],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  43d51d9b0c0c test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  describe commit 37bb762e5dc08073ec4323bdffc023a0f0cc901e
    │  args: jj describe -m4
    ○  13035304f7d6 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  new empty commit
    │  args: jj new -m3
    ○  fcf34f8ae8ac test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  abandon commit ddf84fc5e0dd314092b3dfb13e09e37fa7d04ef9
    │  args: jj abandon
    ○  e02cba71e65d test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit 8b64ddff700dc214dec05d915e85ac692233e6e3
    │  args: jj describe -m2
    ○  0d7016e495d7 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m1
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);
    insta::assert_snapshot!(stderr, @"");

    // "op abandon" should work.
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["op", "abandon", &format!("..{bad_op_id}")]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 4 operations and reparented 4 descendant operations.
    "###);

    // The repo should no longer be corrupt.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["log"]);
    insta::assert_snapshot!(stdout, @r###"
    ○  mzvwutvl?? test.user@example.com 2001-02-03 08:05:15 dc2c6d52
    │  (empty) 4.1
    │ @  mzvwutvl?? test.user@example.com 2001-02-03 08:05:12 6d868f04
    ├─╯  (empty) 4
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:10 HEAD@git f652c321
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    "###);
    insta::assert_snapshot!(stderr, @r###"
    Concurrent modification detected, resolving automatically.
    "###);
}

#[test]
fn test_op_diff() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_bare_git_repo(&git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Overview of op log.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(&stdout, @r###"
    @  ea112f6a02be test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  check out git remote's default branch
    │  args: jj git clone git-repo repo
    ○  cba9d7096849 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  fetch from git remote into empty repo
    │  args: jj git clone git-repo repo
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);

    // Diff between the same operation should be empty.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "diff", "--from", "0000000", "--to", "0000000"],
    );
    insta::assert_snapshot!(&stdout, @r#"
    From operation 000000000000 root()
      To operation 000000000000 root()
    "#);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--from", "@", "--to", "@"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch
      To operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch
    "#);

    // Diff from parent operation to latest operation.
    // `jj op diff --op @` should behave identically to `jj op diff --from
    // @- --to @` (if `@` is not a merge commit).
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--from", "@-", "--to", "@"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation cba9d7096849 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 fetch from git remote into empty repo
      To operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch

    Changed commits:
    ○  Change sqpuoqvxutmz
       + sqpuoqvx 9708515f (empty) (no description set)
    ○  Change qpvuntsmwlqt
       - qpvuntsm hidden 230dd059 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - untracked ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    "#);
    let stdout_without_from_to = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    assert_eq!(stdout, stdout_without_from_to);

    // Diff from root operation to latest operation
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--from", "0000000"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 000000000000 root()
      To operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch

    Changed commits:
    ○  Change sqpuoqvxutmz
    │  + sqpuoqvx 9708515f (empty) (no description set)
    ○  Change ulyvmwyzwuwt
    │  + ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    │ ○  Change tqyxmsztkvot
    ├─╯  + tqyxmszt 3e785984 bookmark-3@origin | Commit 3
    │ ○  Change yuvsmzqkmpws
    ├─╯  + yuvsmzqk 3d9189bc bookmark-2@origin | Commit 2
    ○  Change zzzzzzzzzzzz
       + zzzzzzzz 00000000 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked yuvsmzqk 3d9189bc bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked tqyxmszt 3e785984 bookmark-3@origin | Commit 3
    - untracked (absent)
    "#);

    // Diff from latest operation to root operation
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--to", "0000000"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch
      To operation 000000000000 root()

    Changed commits:
    ○  Change sqpuoqvxutmz
    │  - sqpuoqvx hidden 9708515f (empty) (no description set)
    ○  Change ulyvmwyzwuwt
    │  - ulyvmwyz hidden 1d843d1f Commit 1
    │ ○  Change tqyxmsztkvot
    ├─╯  - tqyxmszt hidden 3e785984 Commit 3
    │ ○  Change yuvsmzqkmpws
    ├─╯  - yuvsmzqk hidden 3d9189bc Commit 2
    ○  Change zzzzzzzzzzzz
       - zzzzzzzz hidden 00000000 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + (absent)
    - ulyvmwyz hidden 1d843d1f Commit 1

    Changed remote bookmarks:
    bookmark-1@origin:
    + untracked (absent)
    - tracked ulyvmwyz hidden 1d843d1f Commit 1
    bookmark-2@origin:
    + untracked (absent)
    - untracked yuvsmzqk hidden 3d9189bc Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked tqyxmszt hidden 3e785984 Commit 3
    "#);

    // Create a conflicted bookmark using a concurrent operation.
    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "bookmark",
            "set",
            "bookmark-1",
            "-r",
            "bookmark-2@origin",
            "--at-op",
            "@-",
        ],
    );
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["log"]);
    insta::assert_snapshot!(&stderr, @r###"
    Concurrent modification detected, resolving automatically.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(&stdout, @r###"
    @    f534dfc3151b test-username@host.example.com 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj log
    ○ │  ea112f6a02be test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │ │  check out git remote's default branch
    │ │  args: jj git clone git-repo repo
    │ ○  9ded7e2755e1 test-username@host.example.com 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00
    ├─╯  point bookmark bookmark-1 to commit 3d9189bc56a1972729350456eb95ec5bf90be2a8
    │    args: jj bookmark set bookmark-1 -r bookmark-2@origin --at-op @-
    ○  cba9d7096849 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  fetch from git remote into empty repo
    │  args: jj git clone git-repo repo
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);
    let op_log_lines = stdout.lines().collect_vec();
    let op_id = op_log_lines[0].split(' ').nth(4).unwrap();
    let first_parent_id = op_log_lines[3].split(' ').nth(3).unwrap();
    let second_parent_id = op_log_lines[6].split(' ').nth(3).unwrap();

    // Diff between the first parent of the merge operation and the merge operation.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "diff", "--from", first_parent_id, "--to", op_id],
    );
    insta::assert_snapshot!(&stdout, @r#"
    From operation ea112f6a02be 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 check out git remote's default branch
      To operation f534dfc3151b 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00 reconcile divergent operations

    Changed local bookmarks:
    bookmark-1:
    + (added) ulyvmwyz 1d843d1f bookmark-1?? bookmark-1@origin | Commit 1
    + (added) yuvsmzqk 3d9189bc bookmark-1?? bookmark-2@origin | Commit 2
    - ulyvmwyz 1d843d1f bookmark-1?? bookmark-1@origin | Commit 1
    "#);

    // Diff between the second parent of the merge operation and the merge
    // operation.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "diff", "--from", second_parent_id, "--to", op_id],
    );
    insta::assert_snapshot!(&stdout, @r#"
    From operation 9ded7e2755e1 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00 point bookmark bookmark-1 to commit 3d9189bc56a1972729350456eb95ec5bf90be2a8
      To operation f534dfc3151b 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00 reconcile divergent operations

    Changed commits:
    ○  Change sqpuoqvxutmz
       + sqpuoqvx 9708515f (empty) (no description set)
    ○  Change qpvuntsmwlqt
       - qpvuntsm hidden 230dd059 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + (added) ulyvmwyz 1d843d1f bookmark-1?? bookmark-1@origin | Commit 1
    + (added) yuvsmzqk 3d9189bc bookmark-1?? bookmark-2@origin | Commit 2
    - yuvsmzqk 3d9189bc bookmark-1?? bookmark-2@origin | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked ulyvmwyz 1d843d1f bookmark-1?? bookmark-1@origin | Commit 1
    - untracked ulyvmwyz 1d843d1f bookmark-1?? bookmark-1@origin | Commit 1
    "#);

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation f534dfc3151b 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00 reconcile divergent operations
      To operation 8a21e297a587 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00 fetch from git remote(s) origin

    Changed commits:
    ○  Change qzxslznxxpoz
       + qzxslznx d487febd bookmark-2@origin | Commit 5
    ○  Change slvtnnzxztqy
       + slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    ○  Change tqyxmsztkvot
       - tqyxmszt hidden 3e785984 Commit 3

    Changed local bookmarks:
    bookmark-1:
    + (added) slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    + (added) yuvsmzqk 3d9189bc bookmark-1?? | Commit 2
    - (added) ulyvmwyz 1d843d1f Commit 1
    - (added) yuvsmzqk 3d9189bc bookmark-1?? | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    - tracked ulyvmwyz 1d843d1f Commit 1
    bookmark-2@origin:
    + untracked qzxslznx d487febd bookmark-2@origin | Commit 5
    - untracked yuvsmzqk 3d9189bc bookmark-1?? | Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked tqyxmszt hidden 3e785984 Commit 3
    "#);

    // Test creation of bookmark.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "bookmark",
            "create",
            "bookmark-2",
            "-r",
            "bookmark-2@origin",
        ],
    );
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Created 1 bookmarks pointing to qzxslznx d487febd bookmark-2 bookmark-2@origin | Commit 5
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 8a21e297a587 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00 fetch from git remote(s) origin
      To operation ef314062f7f5 2001-02-03 04:05:22.000 +07:00 - 2001-02-03 04:05:22.000 +07:00 create bookmark bookmark-2 pointing to commit d487febd08e690ee775a4e0387e30d544307e409

    Changed local bookmarks:
    bookmark-2:
    + qzxslznx d487febd bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    "#);

    // Test tracking of bookmark.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(&stdout, @r###"
     "###);
    insta::assert_snapshot!(&stderr, @r###"
    Started tracking 1 remote bookmarks.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation ef314062f7f5 2001-02-03 04:05:22.000 +07:00 - 2001-02-03 04:05:22.000 +07:00 create bookmark bookmark-2 pointing to commit d487febd08e690ee775a4e0387e30d544307e409
      To operation bc306c9bb67f 2001-02-03 04:05:24.000 +07:00 - 2001-02-03 04:05:24.000 +07:00 track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked qzxslznx d487febd bookmark-2 | Commit 5
    - untracked qzxslznx d487febd bookmark-2 | Commit 5
    "#);

    // Test creation of new commit.
    // Test tracking of bookmark.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation ef314062f7f5 2001-02-03 04:05:22.000 +07:00 - 2001-02-03 04:05:22.000 +07:00 create bookmark bookmark-2 pointing to commit d487febd08e690ee775a4e0387e30d544307e409
      To operation bc306c9bb67f 2001-02-03 04:05:24.000 +07:00 - 2001-02-03 04:05:24.000 +07:00 track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked qzxslznx d487febd bookmark-2 | Commit 5
    - untracked qzxslznx d487febd bookmark-2 | Commit 5
    "#);

    // Test creation of new commit.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &["new", "bookmark-1@origin", "-m", "new commit"],
    );
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: wvuyspvk 358b82d6 (empty) new commit
    Parent commit      : slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    Added 1 files, modified 0 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation bc306c9bb67f 2001-02-03 04:05:24.000 +07:00 - 2001-02-03 04:05:24.000 +07:00 track remote bookmark bookmark-2@origin
      To operation 1ae777a7acc8 2001-02-03 04:05:28.000 +07:00 - 2001-02-03 04:05:28.000 +07:00 new empty commit

    Changed commits:
    ○  Change wvuyspvkupzz
       + wvuyspvk 358b82d6 (empty) new commit
    ○  Change sqpuoqvxutmz
       - sqpuoqvx hidden 9708515f (empty) (no description set)
    "#);

    // Test updating of local bookmark.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Moved 1 bookmarks to wvuyspvk 358b82d6 bookmark-1* | (empty) new commit
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 1ae777a7acc8 2001-02-03 04:05:28.000 +07:00 - 2001-02-03 04:05:28.000 +07:00 new empty commit
      To operation 5728693d7de3 2001-02-03 04:05:30.000 +07:00 - 2001-02-03 04:05:30.000 +07:00 point bookmark bookmark-1 to commit 358b82d6be53fa9b062325abb8bc820a8b34c68d

    Changed local bookmarks:
    bookmark-1:
    + wvuyspvk 358b82d6 bookmark-1* | (empty) new commit
    - (added) slvtnnzx 4f856199 bookmark-1@origin | Commit 4
    - (added) yuvsmzqk 3d9189bc Commit 2
    "#);

    // Test deletion of local bookmark.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["bookmark", "delete", "bookmark-2"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Deleted 1 bookmarks.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 5728693d7de3 2001-02-03 04:05:30.000 +07:00 - 2001-02-03 04:05:30.000 +07:00 point bookmark bookmark-1 to commit 358b82d6be53fa9b062325abb8bc820a8b34c68d
      To operation 0f77d601f1cd 2001-02-03 04:05:32.000 +07:00 - 2001-02-03 04:05:32.000 +07:00 delete bookmark bookmark-2

    Changed local bookmarks:
    bookmark-2:
    + (absent)
    - qzxslznx d487febd bookmark-2@origin | Commit 5
    "#);

    // Test pushing to Git remote.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "push", "--tracked"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r#"
    Changes to push to origin:
      Move bookmark bookmark-1 forward from 4f856199edbf to 358b82d6be53
      Delete bookmark bookmark-2 from d487febd08e6
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy now at: oupztwtk 2f0718a0 (empty) (no description set)
    Parent commit      : wvuyspvk 358b82d6 bookmark-1 | (empty) new commit
    "#);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 0f77d601f1cd 2001-02-03 04:05:32.000 +07:00 - 2001-02-03 04:05:32.000 +07:00 delete bookmark bookmark-2
      To operation 65409e59d36a 2001-02-03 04:05:34.000 +07:00 - 2001-02-03 04:05:34.000 +07:00 push all tracked bookmarks to git remote origin

    Changed commits:
    ○  Change oupztwtkortx
       + oupztwtk 2f0718a0 (empty) (no description set)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked wvuyspvk 358b82d6 bookmark-1 | (empty) new commit
    - tracked slvtnnzx 4f856199 Commit 4
    bookmark-2@origin:
    + untracked (absent)
    - tracked qzxslznx d487febd Commit 5
    "#);
}

#[test]
fn test_op_diff_patch() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Update working copy with a single file and create new commit.
    std::fs::write(repo_path.join("file"), "a\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["new"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: rlvkpnrz 56950632 (empty) (no description set)
    Parent commit      : qpvuntsm 6b1027d2 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--op", "@-", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation b51416386f26 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 add workspace 'default'
      To operation 6188e9d1f7da 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 snapshot working copy

    Changed commits:
    ○  Change qpvuntsmwlqt
       + qpvuntsm 6b1027d2 (no description set)
       - qpvuntsm hidden 230dd059 (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -1,0 +1,1 @@
       +a
    "#);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "--op", "@", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 6188e9d1f7da 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 snapshot working copy
      To operation 8f6a879bef11 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 new empty commit

    Changed commits:
    ○  Change rlvkpnrzqnoo
       + rlvkpnrz 56950632 (empty) (no description set)
    "#);

    // Squash the working copy commit.
    std::fs::write(repo_path.join("file"), "b\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["squash"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: mzvwutvl 9f4fb57f (empty) (no description set)
    Parent commit      : qpvuntsm 2ac85fd1 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation 874d3a8b4c77 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00 snapshot working copy
      To operation c53f5f1afbc6 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00 squash commits into 6b1027d2770cd0a39c468e525e52bf8c47e1464a

    Changed commits:
    ○  Change mzvwutvlkqwt
    │  + mzvwutvl 9f4fb57f (empty) (no description set)
    │ ○  Change rlvkpnrzqnoo
    ├─╯  - rlvkpnrz hidden 1d7f8f94 (no description set)
    │    diff --git a/file b/file
    │    index 7898192261..6178079822 100644
    │    --- a/file
    │    +++ b/file
    │    @@ -1,1 +1,1 @@
    │    -a
    │    +b
    ○  Change qpvuntsmwlqt
       + qpvuntsm 2ac85fd1 (no description set)
       - qpvuntsm hidden 6b1027d2 (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b
    "#);

    // Abandon the working copy commit.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["abandon"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Abandoned commit mzvwutvl 9f4fb57f (empty) (no description set)
    Working copy now at: yqosqzyt 33f321c4 (empty) (no description set)
    Parent commit      : qpvuntsm 2ac85fd1 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r#"
    From operation c53f5f1afbc6 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00 squash commits into 6b1027d2770cd0a39c468e525e52bf8c47e1464a
      To operation e13dc1c7a3b3 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00 abandon commit 9f4fb57fba25a7b47ce5980a5d9a4766778331e8

    Changed commits:
    ○  Change yqosqzytrlsw
       + yqosqzyt 33f321c4 (empty) (no description set)
    ○  Change mzvwutvlkqwt
       - mzvwutvl hidden 9f4fb57f (empty) (no description set)
    "#);
}

#[test]
fn test_op_diff_sibling() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#],
    );
    let base_op_id = stdout.lines().next().unwrap();
    insta::assert_snapshot!(base_op_id, @"b51416386f26");

    // Create merge commit at one operation side. The parent trees will have to
    // be merged when diffing, which requires the commit index of this side.
    test_env.jj_cmd_ok(&repo_path, &["new", "root()", "-mA.1"]);
    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["new", "root()", "-mA.2"]);
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["new", "all:@-+", "-mA"]);

    // Create another operation diverged from the base operation.
    test_env.jj_cmd_ok(&repo_path, &["describe", "--at-op", base_op_id, "-mB"]);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(&stdout, @r###"
    @    2629707e5305 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  b8584dcdb220 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'all:@-+' -mA
    ○ │  f0bb05eb2e7e test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new 'all:@-+' -mA
    ○ │  ee4ec332e269 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.2
    ○ │  971683f333fa test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new 'root()' -mA.2
    ○ │  3df45c2cdcb4 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.1
    │ ○  654f3215449e test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │    args: jj describe --at-op b51416386f26 -mB
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Concurrent modification detected, resolving automatically.
    "###);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#],
    );
    let (head_op_id, p1_op_id, _, _, _, _, p2_op_id) = stdout.lines().next_tuple().unwrap();
    insta::assert_snapshot!(head_op_id, @"2629707e5305");
    insta::assert_snapshot!(p1_op_id, @"b8584dcdb220");
    insta::assert_snapshot!(p2_op_id, @"654f3215449e");

    // Diff between p1 and p2 operations should work no matter if p2 is chosen
    // as a base operation.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "op",
            "diff",
            "--at-op",
            p1_op_id,
            "--from",
            p1_op_id,
            "--to",
            p2_op_id,
            "--summary",
        ],
    );
    insta::assert_snapshot!(&stdout, @r#"
    From operation b8584dcdb220 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00 new empty commit
      To operation 654f3215449e 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00 describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22

    Changed commits:
    ○  Change qpvuntsmwlqt
       + qpvuntsm 02ef2bc4 (empty) B
    ○    Change mzvwutvlkqwt
    ├─╮  - mzvwutvl hidden 270db3d9 (empty) A
    │ ○  Change kkmpptxzrspx
    │    - kkmpptxz hidden 8331e0a3 A.1
    │    A file1
    ○  Change zsuskulnrvyr
       - zsuskuln hidden 8afecaef A.2
       A file2
    "#);
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "op",
            "diff",
            "--at-op",
            p2_op_id,
            "--from",
            p2_op_id,
            "--to",
            p1_op_id,
            "--summary",
        ],
    );
    insta::assert_snapshot!(&stdout, @r#"
    From operation 654f3215449e 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00 describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
      To operation b8584dcdb220 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00 new empty commit

    Changed commits:
    ○    Change mzvwutvlkqwt
    ├─╮  + mzvwutvl 270db3d9 (empty) A
    │ ○  Change kkmpptxzrspx
    │    + kkmpptxz 8331e0a3 A.1
    │    A file1
    ○  Change zsuskulnrvyr
       + zsuskuln 8afecaef A.2
       A file2
    ○  Change qpvuntsmwlqt
       - qpvuntsm hidden 02ef2bc4 (empty) B
    "#);
}

#[test]
fn test_op_diff_word_wrap() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_bare_git_repo(&git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    let render = |args: &[&str], columns: u32, word_wrap: bool| {
        let mut args = args.to_vec();
        if word_wrap {
            args.push("--config-toml=ui.log-word-wrap=true");
        }
        let assert = test_env
            .jj_cmd(&repo_path, &args)
            .env("COLUMNS", columns.to_string())
            .assert()
            .success()
            .stderr("");
        get_stdout_string(&assert)
    };

    // Add some file content changes
    std::fs::write(repo_path.join("file1"), "foo\n".repeat(100)).unwrap();
    test_env.jj_cmd_ok(&repo_path, &["debug", "snapshot"]);

    // ui.log-word-wrap option works, and diff stat respects content width
    insta::assert_snapshot!(render(&["op", "diff", "--from=@---", "--stat"], 40, true), @r#"
    From operation b51416386f26 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 add workspace 'default'
      To operation d12081b11443 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 snapshot working copy

    Changed commits:
    ○  Change sqpuoqvxutmz
    │  + sqpuoqvx 850efc9e (no description
    │  set)
    │  file1 | 100 ++++++++++++++++++++++
    │  1 file changed, 100 insertions(+), 0 deletions(-)
    ○  Change ulyvmwyzwuwt
       + ulyvmwyz 1d843d1f bookmark-1 |
       Commit 1
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  Change tqyxmsztkvot
       + tqyxmszt 3e785984 bookmark-3@origin
       | Commit 3
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  Change yuvsmzqkmpws
       + yuvsmzqk 3d9189bc bookmark-2@origin
       | Commit 2
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  Change qpvuntsmwlqt
       - qpvuntsm hidden 230dd059 (empty)
       (no description set)
       0 files changed, 0 insertions(+), 0 deletions(-)

    Changed local bookmarks:
    bookmark-1:
    + ulyvmwyz 1d843d1f bookmark-1 | Commit
    1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked ulyvmwyz 1d843d1f bookmark-1 |
    Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked yuvsmzqk 3d9189bc
    bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked tqyxmszt 3e785984
    bookmark-3@origin | Commit 3
    - untracked (absent)
    "#);

    // Graph width should be subtracted from the term width
    let config = r#"templates.commit_summary='"0 1 2 3 4 5 6 7 8 9"'"#;
    insta::assert_snapshot!(
        render(&["op", "diff", "--from=@---", "--config-toml", config], 10, true), @r#"
    From operation b51416386f26 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 add workspace 'default'
      To operation d12081b11443 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 snapshot working copy

    Changed
    commits:
    ○  Change
    │  sqpuoqvxutmz
    │  + 0 1 2
    │  3 4 5 6
    │  7 8 9
    ○  Change
       ulyvmwyzwuwt
       + 0 1 2
       3 4 5 6
       7 8 9
    ○  Change
       tqyxmsztkvot
       + 0 1 2
       3 4 5 6
       7 8 9
    ○  Change
       yuvsmzqkmpws
       + 0 1 2
       3 4 5 6
       7 8 9
    ○  Change
       qpvuntsmwlqt
       - 0 1 2
       3 4 5 6
       7 8 9

    Changed
    local
    bookmarks:
    bookmark-1:
    + 0 1 2 3
    4 5 6 7 8
    9
    - (absent)

    Changed
    remote
    bookmarks:
    bookmark-1@origin:
    + tracked
    0 1 2 3 4
    5 6 7 8 9
    -
    untracked
    (absent)
    bookmark-2@origin:
    +
    untracked
    0 1 2 3 4
    5 6 7 8 9
    -
    untracked
    (absent)
    bookmark-3@origin:
    +
    untracked
    0 1 2 3 4
    5 6 7 8 9
    -
    untracked
    (absent)
    "#);
}

#[test]
fn test_op_show() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_bare_git_repo(&git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Overview of op log.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    insta::assert_snapshot!(&stdout, @r###"
    @  ea112f6a02be test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  check out git remote's default branch
    │  args: jj git clone git-repo repo
    ○  cba9d7096849 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  fetch from git remote into empty repo
    │  args: jj git clone git-repo repo
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ○  000000000000 root()
    "###);

    // The root operation is empty.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "0000000"]);
    insta::assert_snapshot!(&stdout, @r###"
    000000000000 root()
    "###);

    // Showing the latest operation.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "@"]);
    insta::assert_snapshot!(&stdout, @r###"
    ea112f6a02be test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    check out git remote's default branch
    args: jj git clone git-repo repo

    Changed commits:
    ○  Change sqpuoqvxutmz
       + sqpuoqvx 9708515f (empty) (no description set)
    ○  Change qpvuntsmwlqt
       - qpvuntsm hidden 230dd059 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    - untracked ulyvmwyz 1d843d1f bookmark-1 | Commit 1
    "###);
    // `jj op show @` should behave identically to `jj op show`.
    let stdout_without_op_id = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    assert_eq!(stdout, stdout_without_op_id);

    // Showing a given operation.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "@-"]);
    insta::assert_snapshot!(&stdout, @r###"
    cba9d7096849 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    fetch from git remote into empty repo
    args: jj git clone git-repo repo

    Changed commits:
    ○  Change tqyxmsztkvot
       + tqyxmszt 3e785984 bookmark-3@origin | Commit 3
    ○  Change yuvsmzqkmpws
       + yuvsmzqk 3d9189bc bookmark-2@origin | Commit 2
    ○  Change ulyvmwyzwuwt
       + ulyvmwyz 1d843d1f bookmark-1@origin | Commit 1

    Changed remote bookmarks:
    bookmark-1@origin:
    + untracked ulyvmwyz 1d843d1f bookmark-1@origin | Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked yuvsmzqk 3d9189bc bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked tqyxmszt 3e785984 bookmark-3@origin | Commit 3
    - untracked (absent)
    "###);

    // Create a conflicted bookmark using a concurrent operation.
    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "bookmark",
            "set",
            "bookmark-1",
            "-r",
            "bookmark-2@origin",
            "--at-op",
            "@-",
        ],
    );
    let (_, stderr) = test_env.jj_cmd_ok(&repo_path, &["log"]);
    insta::assert_snapshot!(&stderr, @r###"
    Concurrent modification detected, resolving automatically.
    "###);
    // Showing a merge operation is empty.
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    5d52ce7672bb test-username@host.example.com 2001-02-03 04:05:14.000 +07:00 - 2001-02-03 04:05:14.000 +07:00
    reconcile divergent operations
    args: jj log
    "###);

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    5eed0f5f5fbc test-username@host.example.com 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    fetch from git remote(s) origin
    args: jj git fetch

    Changed commits:
    ○  Change qzxslznxxpoz
       + qzxslznx d487febd bookmark-2@origin | Commit 5
    ○  Change slvtnnzxztqy
       + slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    ○  Change tqyxmsztkvot
       - tqyxmszt hidden 3e785984 Commit 3

    Changed local bookmarks:
    bookmark-1:
    + (added) slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    + (added) yuvsmzqk 3d9189bc bookmark-1?? | Commit 2
    - (added) ulyvmwyz 1d843d1f Commit 1
    - (added) yuvsmzqk 3d9189bc bookmark-1?? | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    - tracked ulyvmwyz 1d843d1f Commit 1
    bookmark-2@origin:
    + untracked qzxslznx d487febd bookmark-2@origin | Commit 5
    - untracked yuvsmzqk 3d9189bc bookmark-1?? | Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked tqyxmszt hidden 3e785984 Commit 3
    "###);

    // Test creation of bookmark.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "bookmark",
            "create",
            "bookmark-2",
            "-r",
            "bookmark-2@origin",
        ],
    );
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Created 1 bookmarks pointing to qzxslznx d487febd bookmark-2 bookmark-2@origin | Commit 5
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    25b687bb01b6 test-username@host.example.com 2001-02-03 04:05:18.000 +07:00 - 2001-02-03 04:05:18.000 +07:00
    create bookmark bookmark-2 pointing to commit d487febd08e690ee775a4e0387e30d544307e409
    args: jj bookmark create bookmark-2 -r bookmark-2@origin

    Changed local bookmarks:
    bookmark-2:
    + qzxslznx d487febd bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    "###);

    // Test tracking of a bookmark.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(&stdout, @r###"
     "###);
    insta::assert_snapshot!(&stderr, @r###"
    Started tracking 1 remote bookmarks.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    48cf434c463d test-username@host.example.com 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked qzxslznx d487febd bookmark-2 | Commit 5
    - untracked qzxslznx d487febd bookmark-2 | Commit 5
    "###);

    // Test creation of new commit.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    48cf434c463d test-username@host.example.com 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked qzxslznx d487febd bookmark-2 | Commit 5
    - untracked qzxslznx d487febd bookmark-2 | Commit 5
    "###);

    // Test creation of new commit.
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &["new", "bookmark-1@origin", "-m", "new commit"],
    );
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: xznxytkn eb6c2b21 (empty) new commit
    Parent commit      : slvtnnzx 4f856199 bookmark-1?? bookmark-1@origin | Commit 4
    Added 1 files, modified 0 files, removed 1 files
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    89aa6cca51fc test-username@host.example.com 2001-02-03 04:05:24.000 +07:00 - 2001-02-03 04:05:24.000 +07:00
    new empty commit
    args: jj new bookmark-1@origin -m 'new commit'

    Changed commits:
    ○  Change xznxytknoqwo
       + xznxytkn eb6c2b21 (empty) new commit
    ○  Change sqpuoqvxutmz
       - sqpuoqvx hidden 9708515f (empty) (no description set)
    "###);

    // Test updating of local bookmark.
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Moved 1 bookmarks to xznxytkn eb6c2b21 bookmark-1* | (empty) new commit
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    6d6bf9b35d8a test-username@host.example.com 2001-02-03 04:05:26.000 +07:00 - 2001-02-03 04:05:26.000 +07:00
    point bookmark bookmark-1 to commit eb6c2b21ec20a33ab6a1c44bc86c59d84ffd93ac
    args: jj bookmark set bookmark-1 -r @

    Changed local bookmarks:
    bookmark-1:
    + xznxytkn eb6c2b21 bookmark-1* | (empty) new commit
    - (added) slvtnnzx 4f856199 bookmark-1@origin | Commit 4
    - (added) yuvsmzqk 3d9189bc Commit 2
    "###);

    // Test deletion of local bookmark.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["bookmark", "delete", "bookmark-2"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r###"
    Deleted 1 bookmarks.
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    a50c95a6b180 test-username@host.example.com 2001-02-03 04:05:28.000 +07:00 - 2001-02-03 04:05:28.000 +07:00
    delete bookmark bookmark-2
    args: jj bookmark delete bookmark-2

    Changed local bookmarks:
    bookmark-2:
    + (absent)
    - qzxslznx d487febd bookmark-2@origin | Commit 5
    "###);

    // Test pushing to Git remote.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "push", "--tracked"]);
    insta::assert_snapshot!(&stdout, @r###"
    "###);
    insta::assert_snapshot!(&stderr, @r#"
    Changes to push to origin:
      Move bookmark bookmark-1 forward from 4f856199edbf to eb6c2b21ec20
      Delete bookmark bookmark-2 from d487febd08e6
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy now at: pzsxstzt 7ab2d837 (empty) (no description set)
    Parent commit      : xznxytkn eb6c2b21 bookmark-1 | (empty) new commit
    "#);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show"]);
    insta::assert_snapshot!(&stdout, @r###"
    5d994bb7d230 test-username@host.example.com 2001-02-03 04:05:30.000 +07:00 - 2001-02-03 04:05:30.000 +07:00
    push all tracked bookmarks to git remote origin
    args: jj git push --tracked

    Changed commits:
    ○  Change pzsxstztnpkv
       + pzsxstzt 7ab2d837 (empty) (no description set)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked xznxytkn eb6c2b21 bookmark-1 | (empty) new commit
    - tracked slvtnnzx 4f856199 Commit 4
    bookmark-2@origin:
    + untracked (absent)
    - tracked qzxslznx d487febd Commit 5
    "###);
}

#[test]
fn test_op_show_patch() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Update working copy with a single file and create new commit.
    std::fs::write(repo_path.join("file"), "a\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["new"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: rlvkpnrz 56950632 (empty) (no description set)
    Parent commit      : qpvuntsm 6b1027d2 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "@-", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r###"
    6188e9d1f7da test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj new

    Changed commits:
    ○  Change qpvuntsmwlqt
       + qpvuntsm 6b1027d2 (no description set)
       - qpvuntsm hidden 230dd059 (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -1,0 +1,1 @@
       +a
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "@", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r###"
    8f6a879bef11 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    new empty commit
    args: jj new

    Changed commits:
    ○  Change rlvkpnrzqnoo
       + rlvkpnrz 56950632 (empty) (no description set)
    "###);

    // Squash the working copy commit.
    std::fs::write(repo_path.join("file"), "b\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["squash"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Working copy now at: mzvwutvl 9f4fb57f (empty) (no description set)
    Parent commit      : qpvuntsm 2ac85fd1 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r###"
    c53f5f1afbc6 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    squash commits into 6b1027d2770cd0a39c468e525e52bf8c47e1464a
    args: jj squash

    Changed commits:
    ○  Change mzvwutvlkqwt
    │  + mzvwutvl 9f4fb57f (empty) (no description set)
    │ ○  Change rlvkpnrzqnoo
    ├─╯  - rlvkpnrz hidden 1d7f8f94 (no description set)
    │    diff --git a/file b/file
    │    index 7898192261..6178079822 100644
    │    --- a/file
    │    +++ b/file
    │    @@ -1,1 +1,1 @@
    │    -a
    │    +b
    ○  Change qpvuntsmwlqt
       + qpvuntsm 2ac85fd1 (no description set)
       - qpvuntsm hidden 6b1027d2 (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b
    "###);

    // Abandon the working copy commit.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["abandon"]);
    insta::assert_snapshot!(&stdout, @"");
    insta::assert_snapshot!(&stderr, @r###"
    Abandoned commit mzvwutvl 9f4fb57f (empty) (no description set)
    Working copy now at: yqosqzyt 33f321c4 (empty) (no description set)
    Parent commit      : qpvuntsm 2ac85fd1 (no description set)
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(&stdout, @r###"
    e13dc1c7a3b3 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    abandon commit 9f4fb57fba25a7b47ce5980a5d9a4766778331e8
    args: jj abandon

    Changed commits:
    ○  Change yqosqzytrlsw
       + yqosqzyt 33f321c4 (empty) (no description set)
    ○  Change mzvwutvlkqwt
       - mzvwutvl hidden 9f4fb57f (empty) (no description set)
    "###);

    // Try again with "op log".
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log", "--git"]);
    insta::assert_snapshot!(&stdout, @r#"
    @  e13dc1c7a3b3 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  abandon commit 9f4fb57fba25a7b47ce5980a5d9a4766778331e8
    │  args: jj abandon
    │
    │  Changed commits:
    │  ○  Change yqosqzytrlsw
    │     + yqosqzyt 33f321c4 (empty) (no description set)
    │  ○  Change mzvwutvlkqwt
    │     - mzvwutvl hidden 9f4fb57f (empty) (no description set)
    ○  c53f5f1afbc6 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  squash commits into 6b1027d2770cd0a39c468e525e52bf8c47e1464a
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  Change mzvwutvlkqwt
    │  │  + mzvwutvl 9f4fb57f (empty) (no description set)
    │  │ ○  Change rlvkpnrzqnoo
    │  ├─╯  - rlvkpnrz hidden 1d7f8f94 (no description set)
    │  │    diff --git a/file b/file
    │  │    index 7898192261..6178079822 100644
    │  │    --- a/file
    │  │    +++ b/file
    │  │    @@ -1,1 +1,1 @@
    │  │    -a
    │  │    +b
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 2ac85fd1 (no description set)
    │     - qpvuntsm hidden 6b1027d2 (no description set)
    │     diff --git a/file b/file
    │     index 7898192261..6178079822 100644
    │     --- a/file
    │     +++ b/file
    │     @@ -1,1 +1,1 @@
    │     -a
    │     +b
    ○  874d3a8b4c77 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  snapshot working copy
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  Change rlvkpnrzqnoo
    │     + rlvkpnrz 1d7f8f94 (no description set)
    │     - rlvkpnrz hidden 56950632 (empty) (no description set)
    │     diff --git a/file b/file
    │     index 7898192261..6178079822 100644
    │     --- a/file
    │     +++ b/file
    │     @@ -1,1 +1,1 @@
    │     -a
    │     +b
    ○  8f6a879bef11 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new
    │
    │  Changed commits:
    │  ○  Change rlvkpnrzqnoo
    │     + rlvkpnrz 56950632 (empty) (no description set)
    ○  6188e9d1f7da test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj new
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 6b1027d2 (no description set)
    │     - qpvuntsm hidden 230dd059 (empty) (no description set)
    │     diff --git a/file b/file
    │     new file mode 100644
    │     index 0000000000..7898192261
    │     --- /dev/null
    │     +++ b/file
    │     @@ -1,0 +1,1 @@
    │     +a
    ○  b51416386f26 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  Change qpvuntsmwlqt
    │     + qpvuntsm 230dd059 (empty) (no description set)
    ○  9a7d829846af test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    │
    │  Changed commits:
    │  ○  Change zzzzzzzzzzzz
    │     + zzzzzzzz 00000000 (empty) (no description set)
    ○  000000000000 root()
    "#);
}

fn init_bare_git_repo(git_repo_path: &Path) -> git2::Repository {
    let git_repo = git2::Repository::init_bare(git_repo_path).unwrap();
    let git_blob_oid = git_repo.blob(b"some content").unwrap();
    let mut git_tree_builder = git_repo.treebuilder(None).unwrap();
    git_tree_builder
        .insert("some-file", git_blob_oid, 0o100644)
        .unwrap();
    let git_tree_id = git_tree_builder.write().unwrap();
    drop(git_tree_builder);
    let git_tree = git_repo.find_tree(git_tree_id).unwrap();
    let git_signature = git2::Signature::new(
        "Git User",
        "git.user@example.com",
        &git2::Time::new(123, 60),
    )
    .unwrap();
    git_repo
        .commit(
            Some("refs/heads/bookmark-1"),
            &git_signature,
            &git_signature,
            "Commit 1",
            &git_tree,
            &[],
        )
        .unwrap();
    git_repo
        .commit(
            Some("refs/heads/bookmark-2"),
            &git_signature,
            &git_signature,
            "Commit 2",
            &git_tree,
            &[],
        )
        .unwrap();
    git_repo
        .commit(
            Some("refs/heads/bookmark-3"),
            &git_signature,
            &git_signature,
            "Commit 3",
            &git_tree,
            &[],
        )
        .unwrap();
    drop(git_tree);
    git_repo.set_head("refs/heads/bookmark-1").unwrap();
    git_repo
}

fn modify_git_repo(git_repo: git2::Repository) -> git2::Repository {
    let git_blob_oid = git_repo.blob(b"more content").unwrap();
    let mut git_tree_builder = git_repo.treebuilder(None).unwrap();
    git_tree_builder
        .insert("next-file", git_blob_oid, 0o100644)
        .unwrap();
    let git_tree_id = git_tree_builder.write().unwrap();
    drop(git_tree_builder);
    let git_tree = git_repo.find_tree(git_tree_id).unwrap();
    let git_signature = git2::Signature::new(
        "Git User",
        "git.user@example.com",
        &git2::Time::new(123, 60),
    )
    .unwrap();
    let bookmark1_commit = git_repo
        .find_reference("refs/heads/bookmark-1")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    let bookmark2_commit = git_repo
        .find_reference("refs/heads/bookmark-2")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    git_repo
        .commit(
            Some("refs/heads/bookmark-1"),
            &git_signature,
            &git_signature,
            "Commit 4",
            &git_tree,
            &[&bookmark1_commit],
        )
        .unwrap();
    git_repo
        .commit(
            Some("refs/heads/bookmark-2"),
            &git_signature,
            &git_signature,
            "Commit 5",
            &git_tree,
            &[&bookmark2_commit],
        )
        .unwrap();
    git_repo
        .find_reference("refs/heads/bookmark-3")
        .unwrap()
        .delete()
        .unwrap();
    drop(git_tree);
    drop(bookmark1_commit);
    drop(bookmark2_commit);
    git_repo
}

fn get_log_output(test_env: &TestEnvironment, repo_path: &Path, op_id: &str) -> String {
    test_env.jj_cmd_success(
        repo_path,
        &["log", "-T", "commit_id", "--at-op", op_id, "-r", "all()"],
    )
}
