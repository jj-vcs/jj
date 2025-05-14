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
use std::path::PathBuf;

use itertools::Itertools as _;
use regex::Regex;
use testutils::git;

use crate::common::to_toml_value;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_op_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    let op_log_lines = output.stdout.raw().lines().collect_vec();
    let add_workspace_id = op_log_lines[3].split(' ').nth(2).unwrap();

    // Can load the repo at a specific operation ID
    insta::assert_snapshot!(get_log_output(&work_dir, add_workspace_id), @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // "@" resolves to the head operation
    insta::assert_snapshot!(get_log_output(&work_dir, "@"), @r"
    @  3ae22e7f50a15d393e412cca72d09a61165d0c84
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // "@-" resolves to the parent of the head operation
    insta::assert_snapshot!(get_log_output(&work_dir, "@-"), @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["log", "--at-op", "@---"]), @r#"
    ------- stderr -------
    Error: The "@---" expression resolved to no operations
    [EOF]
    [exit status: 1]
    "#);

    // We get a reasonable message if an invalid operation ID is specified
    insta::assert_snapshot!(work_dir.run_jj(["log", "--at-op", "foo"]), @r#"
    ------- stderr -------
    Error: Operation ID "foo" is not a valid hexadecimal prefix
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["op", "log", "--op-diff"]);
    insta::assert_snapshot!(output, @r"
    @  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    │
    │  Changed commits:
    │  ○  + qpvuntsm 3ae22e7f (empty) description 0
    │     - qpvuntsm hidden e8849ae1 (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 3ae22e7f (empty) description 0
    │  - qpvuntsm hidden e8849ae1 (empty) (no description set)
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  + qpvuntsm e8849ae1 (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm e8849ae1 (empty) (no description set)
    │  - (absent)
    ○  000000000000 root()
    [EOF]
    ");

    work_dir
        .run_jj(["describe", "-m", "description 1"])
        .success();
    work_dir
        .run_jj([
            "describe",
            "-m",
            "description 2",
            "--at-op",
            add_workspace_id,
        ])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["log", "--at-op", "@-"]), @r#"
    ------- stderr -------
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: ad1b3bd7fb02, 9e17e47612d5
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_op_log_with_custom_symbols() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    let output = work_dir.run_jj([
        "op",
        "log",
        "--config=templates.op_log_node='if(current_operation, \"$\", if(root, \"┴\", \"┝\"))'",
    ]);
    insta::assert_snapshot!(output, @r"
    $  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    ┝  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ┴  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_log_with_no_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "log", "-T"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: a value is required for '--template <TEMPLATE>' but none was supplied

    For more information, try '--help'.
    Hint: The following template aliases are defined:
    - builtin_config_list
    - builtin_config_list_detailed
    - builtin_draft_commit_description
    - builtin_log_comfortable
    - builtin_log_compact
    - builtin_log_compact_full_description
    - builtin_log_detailed
    - builtin_log_node
    - builtin_log_node_ascii
    - builtin_log_oneline
    - builtin_op_log_comfortable
    - builtin_op_log_compact
    - builtin_op_log_node
    - builtin_op_log_node_ascii
    - builtin_op_log_oneline
    - commit_summary_separator
    - default_commit_description
    - description_placeholder
    - email_placeholder
    - git_format_patch_email_headers
    - name_placeholder
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_op_log_limit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "log", "-Tdescription", "--limit=1"]);
    insta::assert_snapshot!(output, @r"
    @  add workspace 'default'
    [EOF]
    ");
}

#[test]
fn test_op_log_no_graph() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "log", "--no-graph", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;12m2affa7025254[39m [38;5;3mtest-username@host.example.com[39m [38;5;14m2001-02-03 04:05:07.000 +07:00[39m - [38;5;14m2001-02-03 04:05:07.000 +07:00[39m[0m
    [1madd workspace 'default'[0m
    [38;5;4m000000000000[39m [38;5;2mroot()[39m
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log", "--op-diff", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'

    Changed commits:
    + qpvuntsm e8849ae1 (empty) (no description set)

    Changed working copy default@:
    + qpvuntsm e8849ae1 (empty) (no description set)
    - (absent)
    000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_log_reversed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    let output = work_dir.run_jj(["op", "log", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  000000000000 root()
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    @  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
       describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
       args: jj describe -m 'description 0'
    [EOF]
    ");

    work_dir
        .run_jj(["describe", "-m", "description 1", "--at-op", "@-"])
        .success();

    // Should be able to display log with fork and branch points
    let output = work_dir.run_jj(["op", "log", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  000000000000 root()
    ○    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    ├─╮  add workspace 'default'
    │ ○  c04227b01598 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe -m 'description 1' --at-op @-
    ○ │  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe -m 'description 0'
    @  6238cd3bc6e9 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
       reconcile divergent operations
       args: jj op log --reversed
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    // Should work correctly with `--no-graph`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    000000000000 root()
    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'
    c04227b01598 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 1' --at-op @-
    09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    6238cd3bc6e9 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    reconcile divergent operations
    args: jj op log --reversed
    [EOF]
    ");

    // Should work correctly with `--limit`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--limit=3"]);
    insta::assert_snapshot!(output, @r"
    ○  c04227b01598 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 1' --at-op @-
    │ ○  09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe -m 'description 0'
    @  6238cd3bc6e9 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
       reconcile divergent operations
       args: jj op log --reversed
    [EOF]
    ");

    // Should work correctly with `--limit` and `--no-graph`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--limit=2", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    6238cd3bc6e9 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    reconcile divergent operations
    args: jj op log --reversed
    [EOF]
    ");
}

#[test]
fn test_op_log_no_graph_null_terminated() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "message1"]).success();
    work_dir.run_jj(["commit", "-m", "message2"]).success();

    let output = work_dir
        .run_jj([
            "op",
            "log",
            "--no-graph",
            "--template",
            r#"id.short(4) ++ "\0""#,
        ])
        .success();
    insta::assert_debug_snapshot!(output.stdout.normalized(), @r#""4dc1\07ad0\02aff\00000\0""#);
}

#[test]
fn test_op_log_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |template| work_dir.run_jj(["op", "log", "-T", template]);

    insta::assert_snapshot!(render(r#"id ++ "\n""#), @r"
    @  2affa702525487ca490c4bc8a9a365adf75f972efb5888dd58716de7603e822ba1ed1ed0a50132ee44572bb9d819f37589d0ceb790b397ddcc88c976fde2bf02
    ○  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        render(r#"separate(" ", id.short(5), current_operation, user,
                                time.start(), time.end(), time.duration()) ++ "\n""#), @r"
    @  2affa true test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ○  00000 false @ 1970-01-01 00:00:00.000 +00:00 1970-01-01 00:00:00.000 +00:00 less than a microsecond
    [EOF]
    ");

    // Negative length shouldn't cause panic.
    insta::assert_snapshot!(render(r#"id.short(-1) ++ "|""#), @r"
    @  <Error: out of range integral type conversion attempted>|
    ○  <Error: out of range integral type conversion attempted>|
    [EOF]
    ");

    // Test the default template, i.e. with relative start time and duration. We
    // don't generally use that template because it depends on the current time,
    // so we need to reset the time range format here.
    test_env.add_config(
        r#"
[template-aliases]
'format_time_range(time_range)' = 'time_range.end().ago() ++ ", lasted " ++ time_range.duration()'
        "#,
    );
    let regex = Regex::new(r"\d\d years").unwrap();
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| regex.replace_all(&s, "NN years").into_owned()), @r"
    @  2affa7025254 test-username@host.example.com NN years ago, lasted less than a microsecond
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_log_builtin_templates() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Render without graph to test line ending
    let render = |template| work_dir.run_jj(["op", "log", "-T", template, "--no-graph"]);
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    insta::assert_snapshot!(render(r#"builtin_op_log_compact"#), @r"
    09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'
    000000000000 root()
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_op_log_comfortable"#), @r"
    09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'

    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'

    000000000000 root()

    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_op_log_oneline"#), @r"
    09a518cf68a5 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 describe commit e8849ae12c709f2321908879bc724fdb2ab8a781 args: jj describe -m 'description 0'
    2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 add workspace 'default'
    000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_log_word_wrap() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "foo\n".repeat(100));
    work_dir.run_jj(["debug", "snapshot"]).success();

    let render = |args: &[&str], columns: u32, word_wrap: bool| {
        let word_wrap = to_toml_value(word_wrap);
        work_dir.run_jj_with(|cmd| {
            cmd.args(args)
                .arg(format!("--config=ui.log-word-wrap={word_wrap}"))
                .env("COLUMNS", columns.to_string())
        })
    };

    // ui.log-word-wrap option works
    insta::assert_snapshot!(render(&["op", "log"], 40, false), @r"
    @  fc2ccf751a9d test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    insta::assert_snapshot!(render(&["op", "log"], 40, true), @r"
    @  fc2ccf751a9d
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  2affa7025254
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Nested graph should be wrapped
    insta::assert_snapshot!(render(&["op", "log", "--op-diff"], 40, true), @r"
    @  fc2ccf751a9d
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  + qpvuntsm 79f0968d (no
    │     description set)
    │     - qpvuntsm hidden e8849ae1 (empty)
    │     (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 79f0968d (no description
    │  set)
    │  - qpvuntsm hidden e8849ae1 (empty)
    │  (no description set)
    ○  2affa7025254
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  + qpvuntsm e8849ae1 (empty) (no
    │     description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm e8849ae1 (empty) (no
    │  description set)
    │  - (absent)
    ○  000000000000 root()
    [EOF]
    ");

    // Nested diff stat shouldn't exceed the terminal width
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--stat"], 40, true), @r"
    @  fc2ccf751a9d
    │  test-username@host.example.com
    │  2001-02-03 04:05:08.000 +07:00 -
    │  2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  + qpvuntsm 79f0968d (no
    │     description set)
    │     - qpvuntsm hidden e8849ae1 (empty)
    │     (no description set)
    │     file1 | 100 +++++++++++++++++++
    │     1 file changed, 100 insertions(+), 0 deletions(-)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 79f0968d (no description
    │  set)
    │  - qpvuntsm hidden e8849ae1 (empty)
    │  (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--no-graph", "--stat"], 40, true), @r"
    fc2ccf751a9d
    test-username@host.example.com
    2001-02-03 04:05:08.000 +07:00 -
    2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj debug snapshot

    Changed commits:
    + qpvuntsm 79f0968d (no description set)
    - qpvuntsm hidden e8849ae1 (empty) (no
    description set)
    file1 | 100 +++++++++++++++++++++++++
    1 file changed, 100 insertions(+), 0 deletions(-)

    Changed working copy default@:
    + qpvuntsm 79f0968d (no description set)
    - qpvuntsm hidden e8849ae1 (empty) (no
    description set)
    [EOF]
    ");

    // Nested graph widths should be subtracted from the term width
    let config = r#"templates.commit_summary='"0 1 2 3 4 5 6 7 8 9"'"#;
    insta::assert_snapshot!(
        render(&["op", "log", "-T''", "--op-diff", "-n1", "--config", config], 15, true), @r"
    @
    │
    │  Changed
    │  commits:
    │  ○  + 0 1 2 3
    │     4 5 6 7 8
    │     9
    │     - 0 1 2 3
    │     4 5 6 7 8
    │     9
    │
    │  Changed
    │  working copy
    │  default@:
    │  + 0 1 2 3 4
    │  5 6 7 8 9
    │  - 0 1 2 3 4
    │  5 6 7 8 9
    [EOF]
    ");
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
        .run_jj_with(|cmd| {
            cmd.args(["git", "init", "repo"])
                .env_remove("JJ_OP_HOSTNAME")
                .env_remove("JJ_OP_USERNAME")
        })
        .success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  84cd00547aac my-username@my-hostname 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_abandon_ancestors() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "commit 1"]).success();
    work_dir.run_jj(["commit", "-m", "commit 2"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @r"
    @  d9fb2978c3b3 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  2808265c0ab7 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj commit -m 'commit 1'
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Abandon old operations. The working-copy operation id should be updated.
    let output = work_dir.run_jj(["op", "abandon", "..@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 2 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("bd36ef949d8184d6cb0846eadac00951b2fbd35b6fccc11bf800c9af8874b83dd4de3e2a34466b39b96a82f1ad2410454e061c56292b2550fc6635ece65bcf9f")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @r"
    @  bd36ef949d81 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    [EOF]
    ");

    // Abandon operation range.
    work_dir.run_jj(["commit", "-m", "commit 3"]).success();
    work_dir.run_jj(["commit", "-m", "commit 4"]).success();
    work_dir.run_jj(["commit", "-m", "commit 5"]).success();
    let output = work_dir.run_jj(["op", "abandon", "@---..@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 2 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @r"
    @  ebbdcdbf0e20 test-username@host.example.com 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    │  commit 2f3e935ade915272ccdce9e43e5a5c82fc336aee
    │  args: jj commit -m 'commit 5'
    ○  bd36ef949d81 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    [EOF]
    ");

    // Can't abandon the current operation.
    let output = work_dir.run_jj(["op", "abandon", "..@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot abandon the current operation ebbdcdbf0e20
    Hint: Run `jj undo` to revert the current operation, then use `jj op abandon`
    [EOF]
    [exit status: 1]
    ");

    // Can't create concurrent abandoned operations explicitly.
    let output = work_dir.run_jj(["op", "abandon", "--at-op=@-", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");

    // Abandon the current operation by undoing it first.
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("c9115c490c0e2c45bebed8af0ca748885a636f33dfdd543e71693dafb9c773e42dd001c3c76054026e3ca0f6f5b778c6377997dca65ec99ed8a0b17f4aa8998c")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @r"
    @  c9115c490c0e test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation ebbdcdbf0e202b089f99e95477270a87d72dda2660942e16c75f700307384d88374a3d0b8a3e717dcb5f398ac524f5c7a8f4a87a677a167d52f258dd9a3daceb
    │  args: jj undo
    ○  bd36ef949d81 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    [EOF]
    ");

    // Abandon empty range.
    let output = work_dir.run_jj(["op", "abandon", "@-..@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1"]), @r"
    @  c9115c490c0e test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation ebbdcdbf0e202b089f99e95477270a87d72dda2660942e16c75f700307384d88374a3d0b8a3e717dcb5f398ac524f5c7a8f4a87a677a167d52f258dd9a3daceb
    │  args: jj undo
    [EOF]
    ");
}

#[test]
fn test_op_abandon_without_updating_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "commit 1"]).success();
    work_dir.run_jj(["commit", "-m", "commit 2"]).success();
    work_dir.run_jj(["commit", "-m", "commit 3"]).success();

    // Abandon without updating the working copy.
    let output = work_dir.run_jj(["op", "abandon", "@-", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("b440809cf32014e1c2bc327377fe51286a6dab610841337110f0b795510668ff4e44d3d53cea1efb31730355529a8eb0dd5a718277d80d6be51d917ff0fcb392")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1", "--ignore-working-copy"]), @r"
    @  e9fed929c83b test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 4b087e94a5d14530c3953d617623d075a13294c8
    │  args: jj commit -m 'commit 3'
    [EOF]
    ");

    // The working-copy operation id isn't updated if it differs from the repo.
    // It could be updated if the tree matches, but there's no extra logic for
    // that.
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    Warning: The working copy operation b440809cf320 is not updated because it differs from the repo e9fed929c83b.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("b440809cf32014e1c2bc327377fe51286a6dab610841337110f0b795510668ff4e44d3d53cea1efb31730355529a8eb0dd5a718277d80d6be51d917ff0fcb392")
    Current tree: Merge(Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")))
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1", "--ignore-working-copy"]), @r"
    @  8733d4bde6d4 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 4b087e94a5d14530c3953d617623d075a13294c8
    │  args: jj commit -m 'commit 3'
    [EOF]
    ");
}

#[test]
fn test_op_abandon_multiple_heads() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create 1 base operation + 2 operations to be diverged.
    work_dir.run_jj(["commit", "-m", "commit 1"]).success();
    work_dir.run_jj(["commit", "-m", "commit 2"]).success();
    work_dir.run_jj(["commit", "-m", "commit 3"]).success();
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#])
        .success();
    let [head_op_id, prev_op_id] = output.stdout.raw().lines().next_array().unwrap();
    insta::assert_snapshot!(head_op_id, @"b440809cf320");
    insta::assert_snapshot!(prev_op_id, @"d9fb2978c3b3");

    // Create 1 other concurrent operation.
    work_dir
        .run_jj(["commit", "--at-op=@--", "-m", "commit 4"])
        .success();

    // Can't resolve operation relative to @.
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: b440809cf320, 7d1bec914282
    [EOF]
    [exit status: 1]
    "#);
    let (_, other_head_op_id) = output.stderr.raw().trim_end().rsplit_once(", ").unwrap();
    insta::assert_snapshot!(other_head_op_id, @"7d1bec914282");
    assert_ne!(head_op_id, other_head_op_id);

    // Can't abandon one of the head operations.
    let output = work_dir.run_jj(["op", "abandon", head_op_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot abandon the current operation b440809cf320
    [EOF]
    [exit status: 1]
    ");

    // Can't abandon the other head operation.
    let output = work_dir.run_jj(["op", "abandon", other_head_op_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot abandon the current operation 7d1bec914282
    [EOF]
    [exit status: 1]
    ");

    // Can abandon the operation which is not an ancestor of the other head.
    // This would crash if we attempted to remap the unchanged op in the op
    // heads store.
    let output = work_dir.run_jj(["op", "abandon", prev_op_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 operations and reparented 2 descendant operations.
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @    a266ee1233f7 test-username@host.example.com 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  e9fed929c83b test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  commit 4b087e94a5d14530c3953d617623d075a13294c8
    │ │  args: jj commit -m 'commit 3'
    │ ○  7d1bec914282 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │    args: jj commit '--at-op=@--' -m 'commit 4'
    ○  2808265c0ab7 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj commit -m 'commit 1'
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_op_recover_from_bad_gc() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "repo", "--colocate"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let git_object_path = |hex: &str| {
        let (shard, file_name) = hex.split_at(2);
        let mut file_path = work_dir.root().to_owned();
        file_path.extend([".git", "objects", shard, file_name]);
        file_path
    };

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.run_jj(["describe", "-m2"]).success(); // victim
    work_dir.run_jj(["abandon"]).success(); // break predecessors chain
    work_dir.run_jj(["new", "-m3"]).success();
    work_dir.run_jj(["describe", "-m4"]).success();

    let output = work_dir
        .run_jj(["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#])
        .success();
    let [head_op_id, _, _, bad_op_id] = output.stdout.raw().lines().next_array().unwrap();
    insta::assert_snapshot!(head_op_id, @"b4fa5b4308c8");
    insta::assert_snapshot!(bad_op_id, @"361e02cfaa19");

    // Corrupt the repo by removing hidden but reachable commit object.
    let output = work_dir
        .run_jj([
            "log",
            "--at-op",
            bad_op_id,
            "--no-graph",
            "-r@",
            "-Tcommit_id",
        ])
        .success();
    let bad_commit_id = output.stdout.into_raw();
    insta::assert_snapshot!(bad_commit_id, @"4e123bae951c3216a145dbcd56d60522739d362e");
    std::fs::remove_file(git_object_path(&bad_commit_id)).unwrap();

    // Do concurrent modification to make the situation even worse. At this
    // point, the index can be loaded, so this command succeeds.
    work_dir
        .run_jj(["--at-op=@-", "describe", "-m4.1"])
        .success();

    let output = work_dir.run_jj(["--at-op", head_op_id, "debug", "reindex"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Internal error: Failed to index commits at operation 361e02cfaa1994f71140d7d08a588829f5ca72f856eefae48ed8d0c809727003713a772aab1faf7c6e9efd315879251f0386ad2b5a9f59791991265af3c354a8
    Caused by:
    1: Object 4e123bae951c3216a145dbcd56d60522739d362e of type commit not found
    [EOF]
    [exit status: 255]
    ");

    // "op log" should still be usable.
    let output = work_dir.run_jj(["op", "log", "--ignore-working-copy", "--at-op", head_op_id]);
    insta::assert_snapshot!(output, @r"
    @  b4fa5b4308c8 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  describe commit a053bc8736064a739ab73f2c775a6ac2851bf1a3
    │  args: jj describe -m4
    ○  212c79e1e844 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  new empty commit
    │  args: jj new -m3
    ○  70899ef22ee9 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  abandon commit 4e123bae951c3216a145dbcd56d60522739d362e
    │  args: jj abandon
    ○  361e02cfaa19 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit 884fe9b9c65602d724c7c0f2a238d5549efbe5e6
    │  args: jj describe -m2
    ○  541be72f8ee2 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m1
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // "op abandon" should work.
    let output = work_dir.run_jj(["op", "abandon", &format!("..{bad_op_id}")]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 3 operations and reparented 4 descendant operations.
    [EOF]
    ");

    // The repo should no longer be corrupt.
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  mzvwutvl?? test.user@example.com 2001-02-03 08:05:12 29d07a2d
    │  (empty) 4
    │ ○  mzvwutvl?? test.user@example.com 2001-02-03 08:05:15 bc027e2c
    ├─╯  (empty) 4.1
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:10 git_head() c2934cfb
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_op_corrupted_operation_file() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let op_store_path = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "op_store"]));

    let op_id = work_dir.current_operation_id();
    insta::assert_snapshot!(op_id, @"2affa702525487ca490c4bc8a9a365adf75f972efb5888dd58716de7603e822ba1ed1ed0a50132ee44572bb9d819f37589d0ceb790b397ddcc88c976fde2bf02");

    let op_file_path = op_store_path.join("operations").join(&op_id);
    assert!(op_file_path.exists());

    // truncated
    std::fs::write(&op_file_path, b"").unwrap();
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Error when reading object 2affa702525487ca490c4bc8a9a365adf75f972efb5888dd58716de7603e822ba1ed1ed0a50132ee44572bb9d819f37589d0ceb790b397ddcc88c976fde2bf02 of type operation
    2: Invalid hash length (expected 64 bytes, got 0 bytes)
    [EOF]
    [exit status: 255]
    ");

    // undecodable
    std::fs::write(&op_file_path, b"\0").unwrap();
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Error when reading object 2affa702525487ca490c4bc8a9a365adf75f972efb5888dd58716de7603e822ba1ed1ed0a50132ee44572bb9d819f37589d0ceb790b397ddcc88c976fde2bf02 of type operation
    2: failed to decode Protobuf message: invalid tag value: 0
    [EOF]
    [exit status: 255]
    ");
}

#[test]
fn test_op_summary_diff_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Tests in color (easier to read with `less -R`)
    work_dir
        .run_jj(["new", "--no-edit", "-m=scratch"])
        .success();
    let output = work_dir.run_jj(["op", "undo", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: [38;5;4m5881546f5a5c[39m ([38;5;6m2001-02-03 08:05:08[39m) new empty commit
    [EOF]
    ");
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        "0000000",
        "--to",
        "@",
        "--color=always",
    ]);
    insta::assert_snapshot!(output, @r"
    From operation: [38;5;4m000000000000[39m [38;5;2mroot()[39m
      To operation: [38;5;4m4d601e03331c[39m ([38;5;6m2001-02-03 08:05:09[39m) undo operation 5881546f5a5c322f0f5ced5216d4eb1110570617786292c2e3c102fabb6eb74c3a1183349eee2371ba24ebda7801bf43b6382957756040198384e3a0deeb34fa

    Changed commits:
    ○  [38;5;2m+[39m [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m [38;5;2m(empty)[39m [38;5;2m(no description set)[39m

    Changed working copy [38;5;2mdefault@[39m:
    [38;5;2m+[39m [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    [38;5;1m-[39m (absent)
    [EOF]
    ");

    // Tests with templates
    work_dir
        .run_jj(["new", "--no-edit", "-m=scratch"])
        .success();
    let output = work_dir.run_jj(["op", "undo", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: [38;5;4m<<operation id short::496308a90c9d>>[39m<<operation:: (>>[38;5;6m<<operation time end local format::2001-02-03 08:05:11>>[39m<<operation::) >><<operation description first_line::new empty commit>>
    [EOF]
    ");
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        "0000000",
        "--to",
        "@",
        "--color=debug",
    ]);
    insta::assert_snapshot!(output, @r"
    From operation: [38;5;4m<<operation id short::000000000000>>[39m<<operation:: >>[38;5;2m<<operation root::root()>>[39m
      To operation: [38;5;4m<<operation id short::dfdb600231fe>>[39m<<operation:: (>>[38;5;6m<<operation time end local format::2001-02-03 08:05:12>>[39m<<operation::) >><<operation description first_line::undo operation 496308a90c9da4609359f773ea4b4eae56ee1939b00bc9c5a52d4ce96517e7d936b5c3f4b76d6539f873f71908c84c72e7840f2e16a127a0cd6d79b83016ea96>>

    Changed commits:
    ○  [38;5;2m<<diff added::+>>[39m [1m[38;5;5m<<change_id shortest prefix::q>>[0m[38;5;8m<<change_id shortest rest::pvuntsm>>[39m [1m[38;5;4m<<commit_id shortest prefix::e>>[0m[38;5;8m<<commit_id shortest rest::8849ae1>>[39m [38;5;2m<<empty::(empty)>>[39m [38;5;2m<<empty description placeholder::(no description set)>>[39m

    Changed working copy [38;5;2m<<working_copies::default@>>[39m:
    [38;5;2m<<diff added::+>>[39m [1m[38;5;5m<<change_id shortest prefix::q>>[0m[38;5;8m<<change_id shortest rest::pvuntsm>>[39m [1m[38;5;4m<<commit_id shortest prefix::e>>[0m[38;5;8m<<commit_id shortest rest::8849ae1>>[39m [38;5;2m<<empty::(empty)>>[39m [38;5;2m<<empty description placeholder::(no description set)>>[39m
    [38;5;1m<<diff removed::->>[39m (absent)
    [EOF]
    ");
}

#[test]
fn test_op_diff() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_bare_git_repo(&git_repo_path);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "bookmark-1@origin"])
        .success();

    // Overview of op log.
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  845057a2b174 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  track remote bookmark bookmark-1@origin
    │  args: jj bookmark track bookmark-1@origin
    ○  5446f7f2752a test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Diff between the same operation should be empty.
    let output = work_dir.run_jj(["op", "diff", "--from", "0000000", "--to", "0000000"]);
    insta::assert_snapshot!(output, @r"
    From operation: 000000000000 root()
      To operation: 000000000000 root()
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--from", "@", "--to", "@"]);
    insta::assert_snapshot!(output, @r"
    From operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
      To operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
    [EOF]
    ");

    // Diff from parent operation to latest operation.
    // `jj op diff --op @` should behave identically to `jj op diff --from
    // @- --to @` (if `@` is not a merge commit).
    let output = work_dir.run_jj(["op", "diff", "--from", "@-", "--to", "@"]);
    insta::assert_snapshot!(output, @r"
    From operation: 5446f7f2752a (2001-02-03 08:05:09) fetch from git remote(s) origin
      To operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin

    Changed local bookmarks:
    bookmark-1:
    + pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - untracked pukowqtp 0cb7e07e bookmark-1 | Commit 1
    [EOF]
    ");
    let output_without_from_to = work_dir.run_jj(["op", "diff"]);
    assert_eq!(output, output_without_from_to);

    // Diff from root operation to latest operation
    let output = work_dir.run_jj(["op", "diff", "--from", "0000000"]);
    insta::assert_snapshot!(output, @r"
    From operation: 000000000000 root()
      To operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin

    Changed commits:
    ○  + rnnslrkn 4ff62539 bookmark-2@origin | Commit 2
    ○  + rnnkyono 11671e4c bookmark-3@origin | Commit 3
    ○  + pukowqtp 0cb7e07e bookmark-1 | Commit 1
    ○  + qpvuntsm e8849ae1 (empty) (no description set)

    Changed working copy default@:
    + qpvuntsm e8849ae1 (empty) (no description set)
    - (absent)

    Changed local bookmarks:
    bookmark-1:
    + pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked rnnslrkn 4ff62539 bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked rnnkyono 11671e4c bookmark-3@origin | Commit 3
    - untracked (absent)
    [EOF]
    ");

    // Diff from latest operation to root operation
    let output = work_dir.run_jj(["op", "diff", "--to", "0000000"]);
    insta::assert_snapshot!(output, @r"
    From operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
      To operation: 000000000000 root()

    Changed commits:
    ○  - rnnslrkn hidden 4ff62539 Commit 2
    ○  - rnnkyono hidden 11671e4c Commit 3
    ○  - pukowqtp hidden 0cb7e07e Commit 1
    ○  - qpvuntsm hidden e8849ae1 (empty) (no description set)

    Changed working copy default@:
    + (absent)
    - qpvuntsm hidden e8849ae1 (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + (absent)
    - pukowqtp hidden 0cb7e07e Commit 1

    Changed remote bookmarks:
    bookmark-1@origin:
    + untracked (absent)
    - tracked pukowqtp hidden 0cb7e07e Commit 1
    bookmark-2@origin:
    + untracked (absent)
    - untracked rnnslrkn hidden 4ff62539 Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked rnnkyono hidden 11671e4c Commit 3
    [EOF]
    ");

    // Create a conflicted bookmark using a concurrent operation.
    work_dir
        .run_jj([
            "bookmark",
            "set",
            "bookmark-1",
            "-r",
            "bookmark-2@origin",
            "--at-op",
            "@-",
        ])
        .success();
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    │ ○  pukowqtp someone@example.org 1970-01-01 11:00:00 bookmark-1?? bookmark-1@origin 0cb7e07e
    ├─╯  Commit 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @    ef4d1dc2a922 test-username@host.example.com 2001-02-03 04:05:19.000 +07:00 - 2001-02-03 04:05:19.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj log
    ○ │  845057a2b174 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  track remote bookmark bookmark-1@origin
    │ │  args: jj bookmark track bookmark-1@origin
    │ ○  6dae22d68667 test-username@host.example.com 2001-02-03 04:05:18.000 +07:00 - 2001-02-03 04:05:18.000 +07:00
    ├─╯  point bookmark bookmark-1 to commit 4ff6253913375c6ebdddd8423c11df3b3f17e331
    │    args: jj bookmark set bookmark-1 -r bookmark-2@origin --at-op @-
    ○  5446f7f2752a test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    let op_log_lines = output.stdout.raw().lines().collect_vec();
    let op_id = op_log_lines[0].split(' ').nth(4).unwrap();
    let first_parent_id = op_log_lines[3].split(' ').nth(3).unwrap();
    let second_parent_id = op_log_lines[6].split(' ').nth(3).unwrap();

    // Diff between the first parent of the merge operation and the merge operation.
    let output = work_dir.run_jj(["op", "diff", "--from", first_parent_id, "--to", op_id]);
    insta::assert_snapshot!(output, @r"
    From operation: 845057a2b174 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
      To operation: ef4d1dc2a922 (2001-02-03 08:05:19) reconcile divergent operations

    Changed local bookmarks:
    bookmark-1:
    + (added) pukowqtp 0cb7e07e bookmark-1?? bookmark-1@origin | Commit 1
    + (added) rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2
    - pukowqtp 0cb7e07e bookmark-1?? bookmark-1@origin | Commit 1
    [EOF]
    ");

    // Diff between the second parent of the merge operation and the merge
    // operation.
    let output = work_dir.run_jj(["op", "diff", "--from", second_parent_id, "--to", op_id]);
    insta::assert_snapshot!(output, @r"
    From operation: 6dae22d68667 (2001-02-03 08:05:18) point bookmark bookmark-1 to commit 4ff6253913375c6ebdddd8423c11df3b3f17e331
      To operation: ef4d1dc2a922 (2001-02-03 08:05:19) reconcile divergent operations

    Changed local bookmarks:
    bookmark-1:
    + (added) pukowqtp 0cb7e07e bookmark-1?? bookmark-1@origin | Commit 1
    + (added) rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2
    - rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked pukowqtp 0cb7e07e bookmark-1?? bookmark-1@origin | Commit 1
    - untracked pukowqtp 0cb7e07e bookmark-1?? bookmark-1@origin | Commit 1
    [EOF]
    ");

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: ef4d1dc2a922 (2001-02-03 08:05:19) reconcile divergent operations
      To operation: 1656b0111f05 (2001-02-03 08:05:23) fetch from git remote(s) origin

    Changed commits:
    ○  + kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    ○  + zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    ○  - rnnkyono hidden 11671e4c Commit 3

    Changed local bookmarks:
    bookmark-1:
    + (added) zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    + (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    - (added) pukowqtp 0cb7e07e Commit 1
    - (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    - tracked pukowqtp 0cb7e07e Commit 1
    bookmark-2@origin:
    + untracked kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    - untracked rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked rnnkyono hidden 11671e4c Commit 3
    [EOF]
    ");

    // Test creation of bookmark.
    let output = work_dir.run_jj([
        "bookmark",
        "create",
        "bookmark-2",
        "-r",
        "bookmark-2@origin",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: 1656b0111f05 (2001-02-03 08:05:23) fetch from git remote(s) origin
      To operation: d21b0264dff8 (2001-02-03 08:05:25) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af

    Changed local bookmarks:
    bookmark-2:
    + kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    [EOF]
    ");

    // Test tracking of bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: d21b0264dff8 (2001-02-03 08:05:25) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
      To operation: 5ca5744f2259 (2001-02-03 08:05:27) track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    // Test tracking of bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: d21b0264dff8 (2001-02-03 08:05:25) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
      To operation: 5ca5744f2259 (2001-02-03 08:05:27) track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["new", "bookmark-1@origin", "-m", "new commit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: xlzxqlsl 731ab199 (empty) new commit
    Parent commit (@-)      : zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: 5ca5744f2259 (2001-02-03 08:05:27) track remote bookmark bookmark-2@origin
      To operation: f20c0544f339 (2001-02-03 08:05:31) new empty commit

    Changed commits:
    ○  + xlzxqlsl 731ab199 (empty) new commit
    ○  - qpvuntsm hidden e8849ae1 (empty) (no description set)

    Changed working copy default@:
    + xlzxqlsl 731ab199 (empty) new commit
    - qpvuntsm hidden e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Test updating of local bookmark.
    let output = work_dir.run_jj(["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to xlzxqlsl 731ab199 bookmark-1* | (empty) new commit
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: f20c0544f339 (2001-02-03 08:05:31) new empty commit
      To operation: 6a8bb2bf7d13 (2001-02-03 08:05:33) point bookmark bookmark-1 to commit 731ab19950fc6fc1199b9ea73cb8b9016f22e8f3

    Changed local bookmarks:
    bookmark-1:
    + xlzxqlsl 731ab199 bookmark-1* | (empty) new commit
    - (added) zkmtkqvo 0dee6313 bookmark-1@origin | Commit 4
    - (added) rnnslrkn 4ff62539 Commit 2
    [EOF]
    ");

    // Test deletion of local bookmark.
    let output = work_dir.run_jj(["bookmark", "delete", "bookmark-2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 1 bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: 6a8bb2bf7d13 (2001-02-03 08:05:33) point bookmark bookmark-1 to commit 731ab19950fc6fc1199b9ea73cb8b9016f22e8f3
      To operation: 7d9da7123281 (2001-02-03 08:05:35) delete bookmark bookmark-2

    Changed local bookmarks:
    bookmark-2:
    + (absent)
    - kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    [EOF]
    ");

    // Test pushing to Git remote.
    let output = work_dir.run_jj(["git", "push", "--tracked", "--deleted"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark-1 from 0dee631320b1 to 731ab19950fc
      Delete bookmark bookmark-2 from e1a239a57eb1
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @r"
    From operation: 7d9da7123281 (2001-02-03 08:05:35) delete bookmark bookmark-2
      To operation: 04f37ad84c4b (2001-02-03 08:05:37) push all tracked bookmarks to git remote origin

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked xlzxqlsl 731ab199 bookmark-1 | (empty) new commit
    - tracked zkmtkqvo 0dee6313 Commit 4
    bookmark-2@origin:
    + untracked (absent)
    - tracked kulxwnxm e1a239a5 Commit 5
    [EOF]
    ");
}

#[test]
fn test_op_diff_patch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Update working copy with a single file and create new commit.
    work_dir.write_file("file", "a\n");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz c1c924b8 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--op", "@-", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    From operation: 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
      To operation: 7829688c6706 (2001-02-03 08:05:08) snapshot working copy

    Changed commits:
    ○  + qpvuntsm 6b57e33c (no description set)
       - qpvuntsm hidden e8849ae1 (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -0,0 +1,1 @@
       +a

    Changed working copy default@:
    + qpvuntsm 6b57e33c (no description set)
    - qpvuntsm hidden e8849ae1 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--op", "@", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    From operation: 7829688c6706 (2001-02-03 08:05:08) snapshot working copy
      To operation: 94a56ee3a1fe (2001-02-03 08:05:08) new empty commit

    Changed commits:
    ○  + rlvkpnrz c1c924b8 (empty) (no description set)

    Changed working copy default@:
    + rlvkpnrz c1c924b8 (empty) (no description set)
    - qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");

    // Squash the working copy commit.
    work_dir.write_file("file", "b\n");
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 6cbd01ae (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    From operation: c9a2a852af45 (2001-02-03 08:05:11) snapshot working copy
      To operation: 08dd88f26b33 (2001-02-03 08:05:11) squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c

    Changed commits:
    ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    │ ○  - rlvkpnrz hidden 05a2969e (no description set)
    ├─╯  diff --git a/file b/file
    │    index 7898192261..6178079822 100644
    │    --- a/file
    │    +++ b/file
    │    @@ -1,1 +1,1 @@
    │    -a
    │    +b
    ○  + qpvuntsm 7aa2ec5d (no description set)
       - qpvuntsm hidden 6b57e33c (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b

    Changed working copy default@:
    + mzvwutvl 6cbd01ae (empty) (no description set)
    - rlvkpnrz hidden 05a2969e (no description set)
    [EOF]
    ");

    // Abandon the working copy commit.
    let output = work_dir.run_jj(["abandon"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      mzvwutvl 6cbd01ae (empty) (no description set)
    Working copy  (@) now at: yqosqzyt c97a8573 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    From operation: 08dd88f26b33 (2001-02-03 08:05:11) squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
      To operation: 515e816ea876 (2001-02-03 08:05:13) abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe

    Changed commits:
    ○  + yqosqzyt c97a8573 (empty) (no description set)
    ○  - mzvwutvl hidden 6cbd01ae (empty) (no description set)

    Changed working copy default@:
    + yqosqzyt c97a8573 (empty) (no description set)
    - mzvwutvl hidden 6cbd01ae (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_op_diff_sibling() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir
        .run_jj(["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#])
        .success();
    let base_op_id = output.stdout.raw().lines().next().unwrap();
    insta::assert_snapshot!(base_op_id, @"2affa7025254");

    // Create merge commit at one operation side. The parent trees will have to
    // be merged when diffing, which requires the commit index of this side.
    work_dir.run_jj(["new", "root()", "-mA.1"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new", "root()", "-mA.2"]).success();
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new", "all:@-+", "-mA"]).success();

    // Create another operation diverged from the base operation.
    work_dir
        .run_jj(["describe", "--at-op", base_op_id, "-mB"])
        .success();

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @    f44cd62223a1 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  b62d3f39feb4 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'all:@-+' -mA
    ○ │  539020bb231c test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new 'all:@-+' -mA
    ○ │  d84dbfe11c65 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.2
    ○ │  6087b15e5d31 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new 'root()' -mA.2
    ○ │  47d7f09c45b7 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.1
    │ ○  b948d5e4b967 test-username@host.example.com 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe --at-op 2affa7025254 -mB
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", r#"-Tid.short() ++ "\n""#])
        .success();
    let [head_op_id, p1_op_id, _, _, _, _, p2_op_id] =
        output.stdout.raw().lines().next_array().unwrap();
    insta::assert_snapshot!(head_op_id, @"f44cd62223a1");
    insta::assert_snapshot!(p1_op_id, @"b62d3f39feb4");
    insta::assert_snapshot!(p2_op_id, @"b948d5e4b967");

    // Diff between p1 and p2 operations should work no matter if p2 is chosen
    // as a base operation.
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--at-op",
        p1_op_id,
        "--from",
        p1_op_id,
        "--to",
        p2_op_id,
        "--summary",
    ]);
    insta::assert_snapshot!(output, @r"
    From operation: b62d3f39feb4 (2001-02-03 08:05:11) new empty commit
      To operation: b948d5e4b967 (2001-02-03 08:05:12) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781

    Changed commits:
    ○  + qpvuntsm b1ca67e2 (empty) B
    ○    - mzvwutvl hidden 08c63613 (empty) A
    ├─╮
    │ ○  - kkmpptxz hidden 6c70a4f7 A.1
    │    A file1
    ○  - zsuskuln hidden 47b9525e A.2
       A file2

    Changed working copy default@:
    + qpvuntsm b1ca67e2 (empty) B
    - mzvwutvl hidden 08c63613 (empty) A
    [EOF]
    ");
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--at-op",
        p2_op_id,
        "--from",
        p2_op_id,
        "--to",
        p1_op_id,
        "--summary",
    ]);
    insta::assert_snapshot!(output, @r"
    From operation: b948d5e4b967 (2001-02-03 08:05:12) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
      To operation: b62d3f39feb4 (2001-02-03 08:05:11) new empty commit

    Changed commits:
    ○    + mzvwutvl 08c63613 (empty) A
    ├─╮
    │ ○  + kkmpptxz 6c70a4f7 A.1
    │    A file1
    ○  + zsuskuln 47b9525e A.2
       A file2
    ○  - qpvuntsm hidden b1ca67e2 (empty) B

    Changed working copy default@:
    + mzvwutvl 08c63613 (empty) A
    - qpvuntsm hidden b1ca67e2 (empty) B
    [EOF]
    ");
}

#[test]
fn test_op_diff_word_wrap() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_bare_git_repo(&git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let render = |args: &[&str], columns: u32, word_wrap: bool| {
        let word_wrap = to_toml_value(word_wrap);
        work_dir.run_jj_with(|cmd| {
            cmd.args(args)
                .arg(format!("--config=ui.log-word-wrap={word_wrap}"))
                .env("COLUMNS", columns.to_string())
        })
    };

    // Add some file content changes
    work_dir.write_file("file1", "foo\n".repeat(100));
    work_dir.run_jj(["debug", "snapshot"]).success();

    // ui.log-word-wrap option works, and diff stat respects content width
    insta::assert_snapshot!(render(&["op", "diff", "--from=@---", "--stat"], 40, true), @r"
    From operation: 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
      To operation: 5d3e8c16cd1f (2001-02-03 08:05:08) snapshot working copy

    Changed commits:
    ○  + sqpuoqvx f6f32c19 (no description
    │  set)
    │  file1 | 100 ++++++++++++++++++++++
    │  1 file changed, 100 insertions(+), 0 deletions(-)
    ○  + pukowqtp 0cb7e07e bookmark-1 |
       Commit 1
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  + rnnslrkn 4ff62539 bookmark-2@origin
       | Commit 2
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  + rnnkyono 11671e4c bookmark-3@origin
       | Commit 3
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  - qpvuntsm hidden e8849ae1 (empty)
       (no description set)
       0 files changed, 0 insertions(+), 0 deletions(-)

    Changed working copy default@:
    + sqpuoqvx f6f32c19 (no description set)
    - qpvuntsm hidden e8849ae1 (empty) (no
    description set)

    Changed local bookmarks:
    bookmark-1:
    + pukowqtp 0cb7e07e bookmark-1 | Commit
    1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked pukowqtp 0cb7e07e bookmark-1 |
    Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked rnnslrkn 4ff62539
    bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked rnnkyono 11671e4c
    bookmark-3@origin | Commit 3
    - untracked (absent)
    [EOF]
    ");

    // Graph width should be subtracted from the term width
    let config = r#"templates.commit_summary='"0 1 2 3 4 5 6 7 8 9"'"#;
    insta::assert_snapshot!(
        render(&["op", "diff", "--from=@---", "--config", config], 10, true), @r"
    From operation: 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
      To operation: 5d3e8c16cd1f (2001-02-03 08:05:08) snapshot working copy

    Changed
    commits:
    ○  + 0 1 2
    │  3 4 5 6
    │  7 8 9
    ○  + 0 1 2
       3 4 5 6
       7 8 9
    ○  + 0 1 2
       3 4 5 6
       7 8 9
    ○  + 0 1 2
       3 4 5 6
       7 8 9
    ○  - 0 1 2
       3 4 5 6
       7 8 9

    Changed
    working
    copy
    default@:
    + 0 1 2 3
    4 5 6 7 8
    9
    - 0 1 2 3
    4 5 6 7 8
    9

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
    [EOF]
    ");
}

#[test]
fn test_op_show() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_bare_git_repo(&git_repo_path);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../git-repo"])
        .success();
    work_dir.run_jj(["git", "fetch"]).success();
    work_dir
        .run_jj(["bookmark", "track", "bookmark-1@origin"])
        .success();

    // Overview of op log.
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @r"
    @  845057a2b174 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  track remote bookmark bookmark-1@origin
    │  args: jj bookmark track bookmark-1@origin
    ○  5446f7f2752a test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // The root operation is empty.
    let output = work_dir.run_jj(["op", "show", "0000000"]);
    insta::assert_snapshot!(output, @r"
    000000000000 root()
    [EOF]
    ");

    // Showing the latest operation.
    let output = work_dir.run_jj(["op", "show", "@"]);
    insta::assert_snapshot!(output, @r"
    845057a2b174 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    track remote bookmark bookmark-1@origin
    args: jj bookmark track bookmark-1@origin

    Changed local bookmarks:
    bookmark-1:
    + pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - (absent)

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked pukowqtp 0cb7e07e bookmark-1 | Commit 1
    - untracked pukowqtp 0cb7e07e bookmark-1 | Commit 1
    [EOF]
    ");
    // `jj op show @` should behave identically to `jj op show`.
    let output_without_op_id = work_dir.run_jj(["op", "show"]);
    assert_eq!(output, output_without_op_id);

    // Showing a given operation.
    let output = work_dir.run_jj(["op", "show", "@-"]);
    insta::assert_snapshot!(output, @r"
    5446f7f2752a test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    fetch from git remote(s) origin
    args: jj git fetch

    Changed commits:
    ○  + rnnslrkn 4ff62539 bookmark-2@origin | Commit 2
    ○  + rnnkyono 11671e4c bookmark-3@origin | Commit 3
    ○  + pukowqtp 0cb7e07e bookmark-1@origin | Commit 1

    Changed remote bookmarks:
    bookmark-1@origin:
    + untracked pukowqtp 0cb7e07e bookmark-1@origin | Commit 1
    - untracked (absent)
    bookmark-2@origin:
    + untracked rnnslrkn 4ff62539 bookmark-2@origin | Commit 2
    - untracked (absent)
    bookmark-3@origin:
    + untracked rnnkyono 11671e4c bookmark-3@origin | Commit 3
    - untracked (absent)
    [EOF]
    ");

    // Create a conflicted bookmark using a concurrent operation.
    work_dir
        .run_jj([
            "bookmark",
            "set",
            "bookmark-1",
            "-r",
            "bookmark-2@origin",
            "--at-op",
            "@-",
        ])
        .success();
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    │ ○  pukowqtp someone@example.org 1970-01-01 11:00:00 bookmark-1?? bookmark-1@origin 0cb7e07e
    ├─╯  Commit 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    // Showing a merge operation is empty.
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    ef2d9a73a2ef test-username@host.example.com 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    reconcile divergent operations
    args: jj log
    [EOF]
    ");

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    Abandoned 1 commits that are no longer reachable.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    1279f6806f48 test-username@host.example.com 2001-02-03 04:05:19.000 +07:00 - 2001-02-03 04:05:19.000 +07:00
    fetch from git remote(s) origin
    args: jj git fetch

    Changed commits:
    ○  + kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    ○  + zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    ○  - rnnkyono hidden 11671e4c Commit 3

    Changed local bookmarks:
    bookmark-1:
    + (added) zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    + (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    - (added) pukowqtp 0cb7e07e Commit 1
    - (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    - tracked pukowqtp 0cb7e07e Commit 1
    bookmark-2@origin:
    + untracked kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    - untracked rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked rnnkyono hidden 11671e4c Commit 3
    [EOF]
    ");

    // Test creation of bookmark.
    let output = work_dir.run_jj([
        "bookmark",
        "create",
        "bookmark-2",
        "-r",
        "bookmark-2@origin",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created 1 bookmarks pointing to kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    414022582096 test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
    args: jj bookmark create bookmark-2 -r bookmark-2@origin

    Changed local bookmarks:
    bookmark-2:
    + kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    [EOF]
    ");

    // Test tracking of a bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    469e72ad2feb test-username@host.example.com 2001-02-03 04:05:23.000 +07:00 - 2001-02-03 04:05:23.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2@origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    469e72ad2feb test-username@host.example.com 2001-02-03 04:05:23.000 +07:00 - 2001-02-03 04:05:23.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["new", "bookmark-1@origin", "-m", "new commit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: tlkvzzqu 8f340dd7 (empty) new commit
    Parent commit (@-)      : zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    e15c5bb81b65 test-username@host.example.com 2001-02-03 04:05:27.000 +07:00 - 2001-02-03 04:05:27.000 +07:00
    new empty commit
    args: jj new bookmark-1@origin -m 'new commit'

    Changed commits:
    ○  + tlkvzzqu 8f340dd7 (empty) new commit
    ○  - qpvuntsm hidden e8849ae1 (empty) (no description set)

    Changed working copy default@:
    + tlkvzzqu 8f340dd7 (empty) new commit
    - qpvuntsm hidden e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Test updating of local bookmark.
    let output = work_dir.run_jj(["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to tlkvzzqu 8f340dd7 bookmark-1* | (empty) new commit
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    cddf6d16be85 test-username@host.example.com 2001-02-03 04:05:29.000 +07:00 - 2001-02-03 04:05:29.000 +07:00
    point bookmark bookmark-1 to commit 8f340dd76dc637e4deac17f30056eef7d8eaf682
    args: jj bookmark set bookmark-1 -r @

    Changed local bookmarks:
    bookmark-1:
    + tlkvzzqu 8f340dd7 bookmark-1* | (empty) new commit
    - (added) zkmtkqvo 0dee6313 bookmark-1@origin | Commit 4
    - (added) rnnslrkn 4ff62539 Commit 2
    [EOF]
    ");

    // Test deletion of local bookmark.
    let output = work_dir.run_jj(["bookmark", "delete", "bookmark-2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Deleted 1 bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    093a28c30650 test-username@host.example.com 2001-02-03 04:05:31.000 +07:00 - 2001-02-03 04:05:31.000 +07:00
    delete bookmark bookmark-2
    args: jj bookmark delete bookmark-2

    Changed local bookmarks:
    bookmark-2:
    + (absent)
    - kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    [EOF]
    ");

    // Test pushing to Git remote.
    let output = work_dir.run_jj(["git", "push", "--tracked", "--deleted"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark-1 from 0dee631320b1 to 8f340dd76dc6
      Delete bookmark bookmark-2 from e1a239a57eb1
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @r"
    f9e1dd8479de test-username@host.example.com 2001-02-03 04:05:33.000 +07:00 - 2001-02-03 04:05:33.000 +07:00
    push all tracked bookmarks to git remote origin
    args: jj git push --tracked --deleted

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked tlkvzzqu 8f340dd7 bookmark-1 | (empty) new commit
    - tracked zkmtkqvo 0dee6313 Commit 4
    bookmark-2@origin:
    + untracked (absent)
    - tracked kulxwnxm e1a239a5 Commit 5
    [EOF]
    ");
}

#[test]
fn test_op_show_patch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Update working copy with a single file and create new commit.
    work_dir.write_file("file", "a\n");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz c1c924b8 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "@-", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    7829688c6706 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj new

    Changed commits:
    ○  + qpvuntsm 6b57e33c (no description set)
       - qpvuntsm hidden e8849ae1 (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -0,0 +1,1 @@
       +a

    Changed working copy default@:
    + qpvuntsm 6b57e33c (no description set)
    - qpvuntsm hidden e8849ae1 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "@", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    94a56ee3a1fe test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    new empty commit
    args: jj new

    Changed commits:
    ○  + rlvkpnrz c1c924b8 (empty) (no description set)

    Changed working copy default@:
    + rlvkpnrz c1c924b8 (empty) (no description set)
    - qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");

    // Squash the working copy commit.
    work_dir.write_file("file", "b\n");
    let output = work_dir.run_jj(["squash"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 6cbd01ae (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    08dd88f26b33 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
    args: jj squash

    Changed commits:
    ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    │ ○  - rlvkpnrz hidden 05a2969e (no description set)
    ├─╯  diff --git a/file b/file
    │    index 7898192261..6178079822 100644
    │    --- a/file
    │    +++ b/file
    │    @@ -1,1 +1,1 @@
    │    -a
    │    +b
    ○  + qpvuntsm 7aa2ec5d (no description set)
       - qpvuntsm hidden 6b57e33c (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b

    Changed working copy default@:
    + mzvwutvl 6cbd01ae (empty) (no description set)
    - rlvkpnrz hidden 05a2969e (no description set)
    [EOF]
    ");

    // Abandon the working copy commit.
    let output = work_dir.run_jj(["abandon"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      mzvwutvl 6cbd01ae (empty) (no description set)
    Working copy  (@) now at: yqosqzyt c97a8573 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    515e816ea876 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe
    args: jj abandon

    Changed commits:
    ○  + yqosqzyt c97a8573 (empty) (no description set)
    ○  - mzvwutvl hidden 6cbd01ae (empty) (no description set)

    Changed working copy default@:
    + yqosqzyt c97a8573 (empty) (no description set)
    - mzvwutvl hidden 6cbd01ae (empty) (no description set)
    [EOF]
    ");

    // Try again with "op log".
    let output = work_dir.run_jj(["op", "log", "--git"]);
    insta::assert_snapshot!(output, @r"
    @  515e816ea876 test-username@host.example.com 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe
    │  args: jj abandon
    │
    │  Changed commits:
    │  ○  + yqosqzyt c97a8573 (empty) (no description set)
    │  ○  - mzvwutvl hidden 6cbd01ae (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + yqosqzyt c97a8573 (empty) (no description set)
    │  - mzvwutvl hidden 6cbd01ae (empty) (no description set)
    ○  08dd88f26b33 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    │  │ ○  - rlvkpnrz hidden 05a2969e (no description set)
    │  ├─╯  diff --git a/file b/file
    │  │    index 7898192261..6178079822 100644
    │  │    --- a/file
    │  │    +++ b/file
    │  │    @@ -1,1 +1,1 @@
    │  │    -a
    │  │    +b
    │  ○  + qpvuntsm 7aa2ec5d (no description set)
    │     - qpvuntsm hidden 6b57e33c (no description set)
    │     diff --git a/file b/file
    │     index 7898192261..6178079822 100644
    │     --- a/file
    │     +++ b/file
    │     @@ -1,1 +1,1 @@
    │     -a
    │     +b
    │
    │  Changed working copy default@:
    │  + mzvwutvl 6cbd01ae (empty) (no description set)
    │  - rlvkpnrz hidden 05a2969e (no description set)
    ○  c9a2a852af45 test-username@host.example.com 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  snapshot working copy
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  + rlvkpnrz 05a2969e (no description set)
    │     - rlvkpnrz hidden c1c924b8 (empty) (no description set)
    │     diff --git a/file b/file
    │     index 7898192261..6178079822 100644
    │     --- a/file
    │     +++ b/file
    │     @@ -1,1 +1,1 @@
    │     -a
    │     +b
    │
    │  Changed working copy default@:
    │  + rlvkpnrz 05a2969e (no description set)
    │  - rlvkpnrz hidden c1c924b8 (empty) (no description set)
    ○  94a56ee3a1fe test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new
    │
    │  Changed commits:
    │  ○  + rlvkpnrz c1c924b8 (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + rlvkpnrz c1c924b8 (empty) (no description set)
    │  - qpvuntsm 6b57e33c (no description set)
    ○  7829688c6706 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj new
    │
    │  Changed commits:
    │  ○  + qpvuntsm 6b57e33c (no description set)
    │     - qpvuntsm hidden e8849ae1 (empty) (no description set)
    │     diff --git a/file b/file
    │     new file mode 100644
    │     index 0000000000..7898192261
    │     --- /dev/null
    │     +++ b/file
    │     @@ -0,0 +1,1 @@
    │     +a
    │
    │  Changed working copy default@:
    │  + qpvuntsm 6b57e33c (no description set)
    │  - qpvuntsm hidden e8849ae1 (empty) (no description set)
    ○  2affa7025254 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  + qpvuntsm e8849ae1 (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm e8849ae1 (empty) (no description set)
    │  - (absent)
    ○  000000000000 root()
    [EOF]
    ");
}

fn init_bare_git_repo(git_repo_path: &Path) -> gix::Repository {
    let git_repo = git::init_bare(git_repo_path);
    let commit_result = git::add_commit(
        &git_repo,
        "refs/heads/bookmark-1",
        "some-file",
        b"some content",
        "Commit 1",
        &[],
    );
    git::write_commit(
        &git_repo,
        "refs/heads/bookmark-2",
        commit_result.tree_id,
        "Commit 2",
        &[],
    );
    git::write_commit(
        &git_repo,
        "refs/heads/bookmark-3",
        commit_result.tree_id,
        "Commit 3",
        &[],
    );

    git::set_head_to_id(&git_repo, commit_result.commit_id);
    git_repo
}

fn modify_git_repo(git_repo: gix::Repository) -> gix::Repository {
    let bookmark1_commit = git_repo
        .find_reference("refs/heads/bookmark-1")
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id();
    let bookmark2_commit = git_repo
        .find_reference("refs/heads/bookmark-2")
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id();

    let commit_result = git::add_commit(
        &git_repo,
        "refs/heads/bookmark-1",
        "next-file",
        b"more content",
        "Commit 4",
        &[bookmark1_commit.detach()],
    );
    git::write_commit(
        &git_repo,
        "refs/heads/bookmark-2",
        commit_result.tree_id,
        "Commit 5",
        &[bookmark2_commit.detach()],
    );

    git_repo
        .find_reference("refs/heads/bookmark-3")
        .unwrap()
        .delete()
        .unwrap();
    git_repo
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir, op_id: &str) -> CommandOutput {
    work_dir.run_jj(["log", "-T", "commit_id", "--at-op", op_id, "-r", "all()"])
}
