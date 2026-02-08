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
use testutils::TestResult;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::to_toml_value;

#[test]
fn test_op_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    let op_log_lines = output.stdout.raw().lines().collect_vec();
    let add_workspace_id = op_log_lines[3].split(' ').nth(2).unwrap();

    // Can load the repo at a specific operation ID
    insta::assert_snapshot!(get_log_output(&work_dir, add_workspace_id), @"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // "@" resolves to the head operation
    insta::assert_snapshot!(get_log_output(&work_dir, "@"), @"
    @  3ae22e7f50a15d393e412cca72d09a61165d0c84
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    // "@-" resolves to the parent of the head operation
    insta::assert_snapshot!(get_log_output(&work_dir, "@-"), @"
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
    insta::assert_snapshot!(output, @"
    @  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    │
    │  Changed commits:
    │  ○  + qpvuntsm 3ae22e7f (empty) description 0
    │     - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 3ae22e7f (empty) description 0
    │  - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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

    let output = work_dir.run_jj(["op", "log", "--op-diff", "--color=always"]);
    insta::assert_snapshot!(output, @"
    [1m[38;5;2m@[0m  [1m[38;5;12m69e9fab8ce76[39m [38;5;3mtest-username@host.example.com[39m [38;5;2mdefault@[39m [38;5;14m2001-02-03 04:05:08.000 +07:00[39m - [38;5;14m2001-02-03 04:05:08.000 +07:00[39m[0m
    │  [1mdescribe commit e8849ae12c709f2321908879bc724fdb2ab8a781[0m
    │  [1m[38;5;13margs: jj describe -m 'description 0'[39m[0m
    │
    │  Changed commits:
    │  ○  [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12m3[38;5;8mae22e7f[39m [38;5;10m(empty)[39m description 0[0m
    │     [38;5;1m-[39m [1m[39mq[0m[38;5;8mpvuntsm[1m[39m/1[0m [1m[38;5;4me[0m[38;5;8m8849ae1[39m (hidden) [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    │
    │  Changed working copy [38;5;2mdefault@[39m:
    │  [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12m3[38;5;8mae22e7f[39m [38;5;10m(empty)[39m description 0[0m
    │  [38;5;1m-[39m [1m[39mq[0m[38;5;8mpvuntsm[1m[39m/1[0m [1m[38;5;4me[0m[38;5;8m8849ae1[39m (hidden) [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    ○  [38;5;4me39dc288903d[39m [38;5;3mtest-username@host.example.com[39m [38;5;6m2001-02-03 04:05:07.000 +07:00[39m - [38;5;6m2001-02-03 04:05:07.000 +07:00[39m
    │  add workspace 'default'
    │
    │  Changed commits:
    │  ○  [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12me[38;5;8m8849ae1[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[0m
    │
    │  Changed working copy [38;5;2mdefault@[39m:
    │  [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12me[38;5;8m8849ae1[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[0m
    │  [38;5;1m-[39m (absent)
    ○  [38;5;4m000000000000[39m [38;5;2mroot()[39m
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
    Hint: Try specifying one of the operations by ID: 3e9cf8ff3f23, 4693396e777b
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
    insta::assert_snapshot!(output, @"
    $  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 0'
    ┝  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: a value is required for '--template <TEMPLATE>' but none was supplied

    For more information, try '--help'.
    Hint: The following template aliases are defined:
    - builtin_config_list
    - builtin_config_list_detailed
    - builtin_draft_commit_description
    - builtin_draft_commit_description_with_diff
    - builtin_evolog_compact
    - builtin_log_comfortable
    - builtin_log_compact
    - builtin_log_compact_full_description
    - builtin_log_detailed
    - builtin_log_node
    - builtin_log_node_ascii
    - builtin_log_oneline
    - builtin_log_redacted
    - builtin_op_log_comfortable
    - builtin_op_log_compact
    - builtin_op_log_node
    - builtin_op_log_node_ascii
    - builtin_op_log_oneline
    - builtin_op_log_redacted
    - commit_summary_separator
    - default_commit_description
    - description_placeholder
    - email_placeholder
    - empty_commit_marker
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    [1m[38;5;12me39dc288903d[39m [38;5;3mtest-username@host.example.com[39m [38;5;14m2001-02-03 04:05:07.000 +07:00[39m - [38;5;14m2001-02-03 04:05:07.000 +07:00[39m[0m
    [1madd workspace 'default'[0m
    [38;5;4m000000000000[39m [38;5;2mroot()[39m
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log", "--op-diff", "--no-graph"]);
    insta::assert_snapshot!(output, @"
    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ○  000000000000 root()
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    @  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
       describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
       args: jj describe -m 'description 0'
    [EOF]
    ");

    work_dir
        .run_jj(["describe", "-m", "description 1", "--at-op", "@-"])
        .success();

    // Should be able to display log with fork and branch points
    let output = work_dir.run_jj(["op", "log", "--reversed"]);
    insta::assert_snapshot!(output, @"
    ○  000000000000 root()
    ○    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    ├─╮  add workspace 'default'
    │ ○  4f59a3fcf7fb test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe -m 'description 1' --at-op @-
    ○ │  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe -m 'description 0'
    @  3257abe149b2 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
       reconcile divergent operations
       args: jj op log --reversed
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    // Should work correctly with `--no-graph`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @"
    000000000000 root()
    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'
    4f59a3fcf7fb test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 1' --at-op @-
    69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    3257abe149b2 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    reconcile divergent operations
    args: jj op log --reversed
    [EOF]
    ");

    // Should work correctly with `--limit`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--limit=3"]);
    insta::assert_snapshot!(output, @"
    ○  4f59a3fcf7fb test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'description 1' --at-op @-
    │ ○  69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe -m 'description 0'
    @  3257abe149b2 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
       reconcile divergent operations
       args: jj op log --reversed
    [EOF]
    ");

    // Should work correctly with `--limit` and `--no-graph`
    let output = work_dir.run_jj(["op", "log", "--reversed", "--limit=2", "--no-graph"]);
    insta::assert_snapshot!(output, @"
    69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    3257abe149b2 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
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
    insta::assert_debug_snapshot!(output.stdout.normalized(), @r#""932c\04693\0e39d\00000\0""#);
}

#[test]
fn test_op_log_template() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |template| work_dir.run_jj(["op", "log", "-T", template]);

    insta::assert_snapshot!(render(r#"id ++ "\n""#), @"
    @  e39dc288903d96b1bfe662925e34157fd706fa2c213040ffc2fc535a9e87ff814a603e90cfe79fceed946e4367d302b5401a180d1e3a3946d739c6e0ca7362d1
    ○  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        render(r#"separate(" ", id.short(5), current_operation, user,
                                time.start(), time.end(), time.duration()) ++ "\n""#), @"
    @  e39dc true test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ○  00000 false @ 1970-01-01 00:00:00.000 +00:00 1970-01-01 00:00:00.000 +00:00 less than a microsecond
    [EOF]
    ");

    // Negative length shouldn't cause panic.
    insta::assert_snapshot!(render(r#"id.short(-1) ++ "|""#), @"
    @  <Error: out of range integral type conversion attempted>|
    ○  <Error: out of range integral type conversion attempted>|
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"json(self) ++ "\n""#), @r#"
    @  {"id":"e39dc288903d96b1bfe662925e34157fd706fa2c213040ffc2fc535a9e87ff814a603e90cfe79fceed946e4367d302b5401a180d1e3a3946d739c6e0ca7362d1","parents":["00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"],"time":{"start":"2001-02-03T04:05:07+07:00","end":"2001-02-03T04:05:07+07:00"},"description":"add workspace 'default'","hostname":"host.example.com","username":"test-username","is_snapshot":false,"workspace_name":null,"tags":{}}
    ○  {"id":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","parents":[],"time":{"start":"1970-01-01T00:00:00Z","end":"1970-01-01T00:00:00Z"},"description":"","hostname":"","username":"","is_snapshot":false,"workspace_name":null,"tags":{}}
    [EOF]
    "#);

    // Test the default template, i.e. with relative start time and duration. We
    // don't generally use that template because it depends on the current time,
    // so we need to reset the time range format here.
    test_env.add_config(
        r#"
[template-aliases]
'format_time_range(time_range)' = 'time_range.end().ago() ++ ", lasted " ++ time_range.duration()'
        "#,
    );
    let regex = Regex::new(r"\d\d years")?;
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| regex.replace_all(&s, "NN years").into_owned()), @"
    @  e39dc288903d test-username@host.example.com NN years ago, lasted less than a microsecond
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    Ok(())
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

    insta::assert_snapshot!(render(r#"builtin_op_log_compact"#), @"
    69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'
    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'
    000000000000 root()
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_op_log_comfortable"#), @"
    69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    args: jj describe -m 'description 0'

    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    add workspace 'default'

    000000000000 root()

    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_op_log_oneline"#), @"
    69e9fab8ce76 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00 describe commit e8849ae12c709f2321908879bc724fdb2ab8a781 args: jj describe -m 'description 0'
    e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00 add workspace 'default'
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
    insta::assert_snapshot!(render(&["op", "log"], 40, false), @"
    @  b16a25215f16 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
    insta::assert_snapshot!(render(&["op", "log"], 40, true), @"
    @  b16a25215f16
    │  test-username@host.example.com
    │  default@ 2001-02-03 04:05:08.000
    │  +07:00 - 2001-02-03 04:05:08.000
    │  +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    ○  e39dc288903d
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Nested graph should be wrapped
    insta::assert_snapshot!(render(&["op", "log", "--op-diff"], 40, true), @"
    @  b16a25215f16
    │  test-username@host.example.com
    │  default@ 2001-02-03 04:05:08.000
    │  +07:00 - 2001-02-03 04:05:08.000
    │  +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  + qpvuntsm 79f0968d (no
    │     description set)
    │     - qpvuntsm/1 e8849ae1 (hidden)
    │     (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 79f0968d (no description
    │  set)
    │  - qpvuntsm/1 e8849ae1 (hidden)
    │  (empty) (no description set)
    ○  e39dc288903d
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
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--stat"], 40, true), @"
    @  b16a25215f16
    │  test-username@host.example.com
    │  default@ 2001-02-03 04:05:08.000
    │  +07:00 - 2001-02-03 04:05:08.000
    │  +07:00
    │  snapshot working copy
    │  args: jj debug snapshot
    │
    │  Changed commits:
    │  ○  + qpvuntsm 79f0968d (no
    │     description set)
    │     - qpvuntsm/1 e8849ae1 (hidden)
    │     (empty) (no description set)
    │     file1 | 100 ++++++++++++++++++++++
    │     1 file changed, 100 insertions(+), 0 deletions(-)
    │
    │  Changed working copy default@:
    │  + qpvuntsm 79f0968d (no description
    │  set)
    │  - qpvuntsm/1 e8849ae1 (hidden)
    │  (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(render(&["op", "log", "-n1", "--no-graph", "--stat"], 40, true), @"
    b16a25215f16
    test-username@host.example.com default@
    2001-02-03 04:05:08.000 +07:00 -
    2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj debug snapshot

    Changed commits:
    + qpvuntsm 79f0968d (no description set)
    - qpvuntsm/1 e8849ae1 (hidden) (empty)
    (no description set)
    file1 | 100 ++++++++++++++++++++++++++++
    1 file changed, 100 insertions(+), 0 deletions(-)

    Changed working copy default@:
    + qpvuntsm 79f0968d (no description set)
    - qpvuntsm/1 e8849ae1 (hidden) (empty)
    (no description set)
    [EOF]
    ");

    // Nested graph widths should be subtracted from the term width
    let config = r#"templates.commit_summary='"0 1 2 3 4 5 6 7 8 9"'"#;
    insta::assert_snapshot!(
        render(&["op", "log", "-T''", "--op-diff", "-n1", "--config", config], 15, true), @"
    @
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
    insta::assert_snapshot!(output, @"
    @  bd21e64248ff my-username@my-hostname 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_abandon_invalid() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a merge operation
    work_dir.run_jj(["commit", "-m", "commit 1"]).success();
    work_dir
        .run_jj(["commit", "--at-op=@-", "-m", "commit 2"])
        .success();
    work_dir.run_jj(["commit", "-m", "commit 3"]).success();

    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-T", "description"]), @"
    @  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    ○    reconcile divergent operations
    ├─╮
    ○ │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ ○  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ├─╯
    ○  add workspace 'default'
    ○
    [EOF]
    ");

    // Cannot abandon the root operation
    let output = work_dir.run_jj(["op", "abandon", "000000000000"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon the root operation
    [EOF]
    [exit status: 1]
    ");

    // Cannot abandon merge operations
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon a merge operation
    [EOF]
    [exit status: 1]
    ");

    // Cannot abandon the current operation (specified via "..")
    let output = work_dir.run_jj(["op", "abandon", "@-.."]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon the current operation 331b7704b7c2
    Hint: Run `jj undo` to revert the current operation, then use `jj op abandon`
    [EOF]
    [exit status: 1]
    ");

    // Confirm no change
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-T", "description"]), @"
    @  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    ○    reconcile divergent operations
    ├─╮
    ○ │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ ○  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ├─╯
    ○  add workspace 'default'
    ○
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
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @"
    @  d13443654b1e test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  f2f76be99072 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj commit -m 'commit 1'
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Abandon old operations. The working-copy operation id should be updated.
    let output = work_dir.run_jj(["op", "abandon", "..@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 2 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("24c5b38a3043a5e97d944d18da33b1181bed68aa910289177464870565129edd16b72c3f2e6dfbf1fe14a055df070c648530100c39bf44c595bfa51ea09b5b8d")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")), labels: Unlabeled, .. }
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @"
    @  24c5b38a3043 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 2 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @"
    @  ad81acaa37ed test-username@host.example.com default@ 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    │  commit 2f3e935ade915272ccdce9e43e5a5c82fc336aee
    │  args: jj commit -m 'commit 5'
    ○  24c5b38a3043 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    [EOF]
    ");

    // Can't abandon the current operation.
    let output = work_dir.run_jj(["op", "abandon", "..@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon the current operation ad81acaa37ed
    Hint: Run `jj undo` to revert the current operation, then use `jj op abandon`
    [EOF]
    [exit status: 1]
    ");

    // Can't create concurrent abandoned operations explicitly.
    let output = work_dir.run_jj(["op", "abandon", "--at-op=@-", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");

    // Abandon the current operation by reverting it first.
    work_dir.run_jj(["op", "revert"]).success();
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("74cb936a729d1765e0efc16871f95dfd0433cf9b3b8ee003f7554d631e11c23d24de39ead0c5586a14ec5b0183aaf0728b03b1888cd77e69e2c9b064d1611f5f")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")), labels: Unlabeled, .. }
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log"]), @"
    @  74cb936a729d test-username@host.example.com default@ 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  revert operation ad81acaa37ed00a7d5b872c97c95c454d04bbe95f5fc28ba90e55bba9ef8279a2ff31d04a607fe96ca2d3d14413a0aca0baa2ea0f2235177c87c37b39978a6d1
    │  args: jj op revert
    ○  24c5b38a3043 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │  args: jj commit -m 'commit 2'
    ○  000000000000 root()
    [EOF]
    ");

    // Abandon empty range.
    let output = work_dir.run_jj(["op", "abandon", "@-..@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1"]), @"
    @  74cb936a729d test-username@host.example.com default@ 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  revert operation ad81acaa37ed00a7d5b872c97c95c454d04bbe95f5fc28ba90e55bba9ef8279a2ff31d04a607fe96ca2d3d14413a0aca0baa2ea0f2235177c87c37b39978a6d1
    │  args: jj op revert
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("b860c65efde923bf39f4ad1cbb6f85fb056beff70fa7fcdb83b775b7eca41b60caba41371777e162fbc3c6ea98c7494806c77d96e94d53eb1353a7c2a4dde0a2")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")), labels: Unlabeled, .. }
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1", "--ignore-working-copy"]), @"
    @  18ff0d689ac4 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 4b087e94a5d14530c3953d617623d075a13294c8
    │  args: jj commit -m 'commit 3'
    [EOF]
    ");

    // The working-copy operation id isn't updated if it differs from the repo.
    // It could be updated if the tree matches, but there's no extra logic for
    // that.
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 operations and reparented 1 descendant operations.
    Warning: The working copy operation b860c65efde9 is not updated because it differs from the repo 18ff0d689ac4.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "local-working-copy", "--ignore-working-copy"]), @r#"
    Current operation: OperationId("b860c65efde923bf39f4ad1cbb6f85fb056beff70fa7fcdb83b775b7eca41b60caba41371777e162fbc3c6ea98c7494806c77d96e94d53eb1353a7c2a4dde0a2")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904")), labels: Unlabeled, .. }
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-n1", "--ignore-working-copy"]), @"
    @  ffc35915b2ea test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
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
    insta::assert_snapshot!(head_op_id, @"b860c65efde9");
    insta::assert_snapshot!(prev_op_id, @"d13443654b1e");

    // Create 1 other concurrent operation.
    work_dir
        .run_jj(["commit", "--at-op=@--", "-m", "commit 4"])
        .success();

    // Can't resolve operation relative to @.
    let output = work_dir.run_jj(["op", "abandon", "@-"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: The "@" expression resolved to more than one operation
    Hint: Try specifying one of the operations by ID: b860c65efde9, a93f8407482c
    [EOF]
    [exit status: 1]
    "#);
    let (_, other_head_op_id) = output.stderr.raw().trim_end().rsplit_once(", ").unwrap();
    insta::assert_snapshot!(other_head_op_id, @"a93f8407482c");
    assert_ne!(head_op_id, other_head_op_id);

    // Can't abandon one of the head operations.
    let output = work_dir.run_jj(["op", "abandon", head_op_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon the current operation b860c65efde9
    [EOF]
    [exit status: 1]
    ");

    // Can't abandon the other head operation.
    let output = work_dir.run_jj(["op", "abandon", other_head_op_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot abandon the current operation a93f8407482c
    [EOF]
    [exit status: 1]
    ");

    // Can abandon the operation which is not an ancestor of the other head.
    // This would crash if we attempted to remap the unchanged op in the op
    // heads store.
    let output = work_dir.run_jj(["op", "abandon", prev_op_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 operations and reparented 2 descendant operations.
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    42987ee32a03 test-username@host.example.com default@ 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  18ff0d689ac4 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  commit 4b087e94a5d14530c3953d617623d075a13294c8
    │ │  args: jj commit -m 'commit 3'
    │ ○  a93f8407482c test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  commit 4e0592f3dd52e7a4998a97d9a1f354e2727a856b
    │    args: jj commit '--at-op=@--' -m 'commit 4'
    ○  f2f76be99072 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj commit -m 'commit 1'
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_op_recover_from_bad_gc() -> TestResult {
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
    insta::assert_snapshot!(head_op_id, @"321ce3bc20e7");
    insta::assert_snapshot!(bad_op_id, @"45336b3aedc2");

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
    std::fs::remove_file(git_object_path(&bad_commit_id))?;

    // Do concurrent modification to make the situation even worse. At this
    // point, the index can be loaded, so this command succeeds.
    work_dir
        .run_jj(["--at-op=@-", "describe", "-m4.1"])
        .success();

    let output = work_dir.run_jj(["--at-op", head_op_id, "debug", "reindex"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @"
    ------- stderr -------
    Internal error: Failed to index commits at operation 45336b3aedc28d2cde05878f3b9fd7d77925587acea7e93e21ef8b3aa23fdd7826690920bc22f468616b28ad9587d3408d4e412af079fa610c5783a603151358
    Caused by:
    1: Object 4e123bae951c3216a145dbcd56d60522739d362e of type commit not found
    [EOF]
    [exit status: 255]
    ");

    // "op log" should still be usable.
    let output = work_dir.run_jj(["op", "log", "--ignore-working-copy", "--at-op", head_op_id]);
    insta::assert_snapshot!(output, @"
    @  321ce3bc20e7 test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  describe commit a053bc8736064a739ab73f2c775a6ac2851bf1a3
    │  args: jj describe -m4
    ○  9d43dc5bb4bb test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  new empty commit
    │  args: jj new -m3
    ○  b512a4fedf09 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  abandon commit 4e123bae951c3216a145dbcd56d60522739d362e
    │  args: jj abandon
    ○  45336b3aedc2 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  describe commit 884fe9b9c65602d724c7c0f2a238d5549efbe5e6
    │  args: jj describe -m2
    ○  490da14faac4 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m1
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // "op abandon" should work.
    let output = work_dir.run_jj(["op", "abandon", &format!("..{bad_op_id}")]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 3 operations and reparented 4 descendant operations.
    [EOF]
    ");

    // The repo should no longer be corrupt.
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  mzvwutvl/1 test.user@example.com 2001-02-03 08:05:12 29d07a2d (divergent)
    │  (empty) 4
    │ ○  mzvwutvl/0 test.user@example.com 2001-02-03 08:05:15 bc027e2c (divergent)
    ├─╯  (empty) 4.1
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:10 c2934cfb
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_op_corrupted_operation_file() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let op_store_path = work_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "op_store"]));

    let op_id = work_dir.current_operation_id();
    insta::assert_snapshot!(op_id, @"e39dc288903d96b1bfe662925e34157fd706fa2c213040ffc2fc535a9e87ff814a603e90cfe79fceed946e4367d302b5401a180d1e3a3946d739c6e0ca7362d1");

    let op_file_path = op_store_path.join("operations").join(&op_id);
    assert!(op_file_path.exists());

    // truncated
    std::fs::write(&op_file_path, b"")?;
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Error when reading object e39dc288903d96b1bfe662925e34157fd706fa2c213040ffc2fc535a9e87ff814a603e90cfe79fceed946e4367d302b5401a180d1e3a3946d739c6e0ca7362d1 of type operation
    2: Invalid hash length (expected 64 bytes, got 0 bytes)
    [EOF]
    [exit status: 255]
    ");

    // undecodable
    std::fs::write(&op_file_path, b"\0")?;
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Error when reading object e39dc288903d96b1bfe662925e34157fd706fa2c213040ffc2fc535a9e87ff814a603e90cfe79fceed946e4367d302b5401a180d1e3a3946d739c6e0ca7362d1 of type operation
    2: failed to decode Protobuf message: invalid tag value: 0
    [EOF]
    [exit status: 255]
    ");
    Ok(())
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
    let output = work_dir.run_jj(["op", "revert", "--color=always"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Reverted operation: [38;5;4m5dbabf1331b3[39m ([38;5;6m2001-02-03 08:05:08[39m) new empty commit
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
    insta::assert_snapshot!(output, @"
    From operation: [38;5;4m000000000000[39m [38;5;2mroot()[39m
      To operation: [38;5;4ma8697a0c692a[39m ([38;5;6m2001-02-03 08:05:09[39m) revert operation 5dbabf1331b371cd08b1da6a42e5552107d71a4c364f8ca11c231f025c32746821eb3af957626705160855a65b6d807362f090095f7d5a82b90044bbc8ad8448

    Changed commits:
    ○  [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12me[38;5;8m8849ae1[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[0m

    Changed working copy [38;5;2mdefault@[39m:
    [38;5;2m+[39m [1m[38;5;13mq[38;5;8mpvuntsm[39m [38;5;12me[38;5;8m8849ae1[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[0m
    [38;5;1m-[39m (absent)
    [EOF]
    ");

    // Tests with templates
    work_dir
        .run_jj(["new", "--no-edit", "-m=scratch"])
        .success();
    let output = work_dir.run_jj(["op", "revert", "--color=debug"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Reverted operation: [38;5;4m<<operation id short::c932a0b07f69>>[39m<<operation:: (>>[38;5;6m<<operation time end local format::2001-02-03 08:05:11>>[39m<<operation::) >><<operation description first_line::new empty commit>>
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
    insta::assert_snapshot!(output, @"
    From operation: [38;5;4m<<op_diff operation id short::000000000000>>[39m<<op_diff operation:: >>[38;5;2m<<op_diff operation root::root()>>[39m
      To operation: [38;5;4m<<op_diff operation id short::72c6d72f86b6>>[39m<<op_diff operation:: (>>[38;5;6m<<op_diff operation time end local format::2001-02-03 08:05:12>>[39m<<op_diff operation::) >><<op_diff operation description first_line::revert operation c932a0b07f694b6a23aa5d7c25b9becb01fbf33d1ae2fcf6a39b28d731a08242dcb524c72c8b448fee69bfdd113bd381b8e9c0cc54fc7e42d60745697e5d0f4c>>

    Changed commits:
    ○  [38;5;2m<<diff added::+>>[39m [1m[38;5;13m<<op_diff commit working_copy change_id shortest prefix::q>>[38;5;8m<<op_diff commit working_copy change_id shortest rest::pvuntsm>>[39m<<op_diff commit working_copy:: >>[38;5;12m<<op_diff commit working_copy commit_id shortest prefix::e>>[38;5;8m<<op_diff commit working_copy commit_id shortest rest::8849ae1>>[39m<<op_diff commit working_copy:: >>[38;5;10m<<op_diff commit working_copy empty::(empty)>>[39m<<op_diff commit working_copy:: >>[38;5;10m<<op_diff commit working_copy empty description placeholder::(no description set)>>[0m

    Changed working copy [38;5;2m<<working_copies::default@>>[39m:
    [38;5;2m<<diff added::+>>[39m [1m[38;5;13m<<op_diff commit working_copy change_id shortest prefix::q>>[38;5;8m<<op_diff commit working_copy change_id shortest rest::pvuntsm>>[39m<<op_diff commit working_copy:: >>[38;5;12m<<op_diff commit working_copy commit_id shortest prefix::e>>[38;5;8m<<op_diff commit working_copy commit_id shortest rest::8849ae1>>[39m<<op_diff commit working_copy:: >>[38;5;10m<<op_diff commit working_copy empty::(empty)>>[39m<<op_diff commit working_copy:: >>[38;5;10m<<op_diff commit working_copy empty description placeholder::(no description set)>>[0m
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
    work_dir
        .run_jj(["git", "fetch", "--branch=*", "--tag=*"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "bookmark-1"])
        .success();

    // Overview of op log.
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @  50ecb1b62028 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  track remote bookmark bookmark-1@origin
    │  args: jj bookmark track bookmark-1
    ○  6d2bd2526eb3 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch '--branch=*' '--tag=*'
    ○  ab33a510f80e test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  add git remote origin
    │  args: jj git remote add origin ../git-repo
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // Diff between the same operation should be empty.
    let output = work_dir.run_jj(["op", "diff", "--from", "0000000", "--to", "0000000"]);
    insta::assert_snapshot!(output, @"
    From operation: 000000000000 root()
      To operation: 000000000000 root()
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--from", "@", "--to", "@"]);
    insta::assert_snapshot!(output, @"
    From operation: 50ecb1b62028 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
      To operation: 50ecb1b62028 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
    [EOF]
    ");

    // Diff from parent operation to latest operation.
    // `jj op diff --op @` should behave identically to `jj op diff --from
    // @- --to @` (if `@` is not a merge commit).
    let output = work_dir.run_jj(["op", "diff", "--from", "@-", "--to", "@"]);
    insta::assert_snapshot!(output, @"
    From operation: 6d2bd2526eb3 (2001-02-03 08:05:09) fetch from git remote(s) origin
      To operation: 50ecb1b62028 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin

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
    insta::assert_snapshot!(output, @"
    From operation: 000000000000 root()
      To operation: 50ecb1b62028 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin

    Changed commits:
    ○  + skovwzlu 854c38b8 Commit 4
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

    Changed local tags:
    tag-1:
    + skovwzlu 854c38b8 Commit 4
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

    Changed remote tags:
    tag-1@origin:
    + tracked skovwzlu 854c38b8 Commit 4
    - untracked (absent)
    [EOF]
    ");

    // Diff from latest operation to root operation
    let output = work_dir.run_jj(["op", "diff", "--to", "0000000"]);
    insta::assert_snapshot!(output, @"
    From operation: 50ecb1b62028 (2001-02-03 08:05:10) track remote bookmark bookmark-1@origin
      To operation: 000000000000 root()

    Changed commits:
    ○  - skovwzlu/0 854c38b8 (hidden) Commit 4
    ○  - rnnslrkn/0 4ff62539 (hidden) Commit 2
    ○  - rnnkyono/0 11671e4c (hidden) Commit 3
    ○  - pukowqtp/0 0cb7e07e (hidden) Commit 1
    ○  - qpvuntsm/0 e8849ae1 (hidden) (empty) (no description set)

    Changed working copy default@:
    + (absent)
    - qpvuntsm/0 e8849ae1 (hidden) (empty) (no description set)

    Changed local bookmarks:
    bookmark-1:
    + (absent)
    - pukowqtp/0 0cb7e07e (hidden) Commit 1

    Changed local tags:
    tag-1:
    + (absent)
    - skovwzlu/0 854c38b8 (hidden) Commit 4

    Changed remote bookmarks:
    bookmark-1@origin:
    + untracked (absent)
    - tracked pukowqtp/0 0cb7e07e (hidden) Commit 1
    bookmark-2@origin:
    + untracked (absent)
    - untracked rnnslrkn/0 4ff62539 (hidden) Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked rnnkyono/0 11671e4c (hidden) Commit 3

    Changed remote tags:
    tag-1@origin:
    + untracked (absent)
    - tracked skovwzlu/0 854c38b8 (hidden) Commit 4
    [EOF]
    ");
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    │ ○  pukowqtp someone@example.org 1970-01-01 11:00:00 bookmark-1 0cb7e07e
    ├─╯  Commit 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Create a conflicted bookmark using a concurrent operation.
    // Conflict with a move so the target references change (not just adds)
    work_dir
        .run_jj([
            "bookmark",
            "move",
            "bookmark-1",
            "--to",
            "@",
            "--allow-backwards",
        ])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "set",
            "bookmark-1",
            "-r",
            "bookmark-2@origin",
            "--allow-backwards",
            "--at-op",
            "@-",
        ])
        .success();
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 bookmark-1?? e8849ae1
    │  (empty) (no description set)
    │ ○  pukowqtp someone@example.org 1970-01-01 11:00:00 bookmark-1@origin 0cb7e07e
    ├─╯  Commit 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    eedf6edfaabf test-username@host.example.com default@ 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj log
    ○ │  157150b37ad0 test-username@host.example.com default@ 2001-02-03 04:05:19.000 +07:00 - 2001-02-03 04:05:19.000 +07:00
    │ │  point bookmark bookmark-1 to commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj bookmark move bookmark-1 --to @ --allow-backwards
    │ ○  0009316ed96c test-username@host.example.com default@ 2001-02-03 04:05:20.000 +07:00 - 2001-02-03 04:05:20.000 +07:00
    ├─╯  point bookmark bookmark-1 to commit 4ff6253913375c6ebdddd8423c11df3b3f17e331
    │    args: jj bookmark set bookmark-1 -r bookmark-2@origin --allow-backwards --at-op @-
    ○  50ecb1b62028 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  track remote bookmark bookmark-1@origin
    │  args: jj bookmark track bookmark-1
    ○  6d2bd2526eb3 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch '--branch=*' '--tag=*'
    ○  ab33a510f80e test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  add git remote origin
    │  args: jj git remote add origin ../git-repo
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    insta::assert_snapshot!(output, @"
    From operation: 157150b37ad0 (2001-02-03 08:05:19) point bookmark bookmark-1 to commit e8849ae12c709f2321908879bc724fdb2ab8a781
      To operation: eedf6edfaabf (2001-02-03 08:05:21) reconcile divergent operations

    Changed local bookmarks:
    bookmark-1:
    + (added) qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    + (added) rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2
    + (removed) pukowqtp 0cb7e07e bookmark-1@origin | Commit 1
    - qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    [EOF]
    ");

    // Diff between the second parent of the merge operation and the merge
    // operation.
    let output = work_dir.run_jj(["op", "diff", "--from", second_parent_id, "--to", op_id]);
    insta::assert_snapshot!(output, @"
    From operation: 0009316ed96c (2001-02-03 08:05:20) point bookmark bookmark-1 to commit 4ff6253913375c6ebdddd8423c11df3b3f17e331
      To operation: eedf6edfaabf (2001-02-03 08:05:21) reconcile divergent operations

    Changed local bookmarks:
    bookmark-1:
    + (added) qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    + (added) rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2
    + (removed) pukowqtp 0cb7e07e bookmark-1@origin | Commit 1
    - rnnslrkn 4ff62539 bookmark-1?? bookmark-2@origin | Commit 2
    [EOF]
    ");

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    tag: tag-1@git [new] 
    Abandoned 1 commits that are no longer reachable:
      rnnkyono 11671e4c Commit 3
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: eedf6edfaabf (2001-02-03 08:05:21) reconcile divergent operations
      To operation: aa6262db8536 (2001-02-03 08:05:25) fetch from git remote(s) origin

    Changed commits:
    ○  + kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    ○  + zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    ○  - rnnkyono/0 11671e4c (hidden) Commit 3

    Changed local bookmarks:
    bookmark-1:
    + (added) qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    + (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    + (added) zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    + (removed) pukowqtp 0cb7e07e Commit 1
    + (removed) pukowqtp 0cb7e07e Commit 1
    - (added) qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    - (added) rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    - (removed) pukowqtp 0cb7e07e Commit 1

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    - tracked pukowqtp 0cb7e07e Commit 1
    bookmark-2@origin:
    + untracked kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    - untracked rnnslrkn 4ff62539 bookmark-1?? | Commit 2
    bookmark-3@origin:
    + untracked (absent)
    - untracked rnnkyono/0 11671e4c (hidden) Commit 3
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created 1 bookmarks pointing to kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: aa6262db8536 (2001-02-03 08:05:25) fetch from git remote(s) origin
      To operation: 49e4f3d128a7 (2001-02-03 08:05:27) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af

    Changed local bookmarks:
    bookmark-2:
    + kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    [EOF]
    ");

    // Test tracking of bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 49e4f3d128a7 (2001-02-03 08:05:27) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
      To operation: c1211e01178b (2001-02-03 08:05:29) track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test tracking of bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 49e4f3d128a7 (2001-02-03 08:05:27) create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
      To operation: c1211e01178b (2001-02-03 08:05:29) track remote bookmark bookmark-2@origin

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["new", "bookmark-1@origin", "-m", "new commit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: qmkrwlvp 96f3a57c (empty) new commit
    Parent commit (@-)      : zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: c1211e01178b (2001-02-03 08:05:29) track remote bookmark bookmark-2@origin
      To operation: 4aabb7f1a6cd (2001-02-03 08:05:33) new empty commit

    Changed commits:
    ○  + qmkrwlvp 96f3a57c (empty) new commit

    Changed working copy default@:
    + qmkrwlvp 96f3a57c (empty) new commit
    - qpvuntsm e8849ae1 bookmark-1?? | (empty) (no description set)
    [EOF]
    ");

    // Test updating of local bookmark.
    let output = work_dir.run_jj(["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Moved 1 bookmarks to qmkrwlvp 96f3a57c bookmark-1* | (empty) new commit
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 4aabb7f1a6cd (2001-02-03 08:05:33) new empty commit
      To operation: 299860162381 (2001-02-03 08:05:35) point bookmark bookmark-1 to commit 96f3a57c9a4a4ae7bb45d1eafe32fe3b6e33f458

    Changed local bookmarks:
    bookmark-1:
    + qmkrwlvp 96f3a57c bookmark-1* | (empty) new commit
    - (added) qpvuntsm e8849ae1 (empty) (no description set)
    - (added) rnnslrkn 4ff62539 Commit 2
    - (added) zkmtkqvo 0dee6313 bookmark-1@origin | Commit 4
    - (removed) pukowqtp 0cb7e07e Commit 1
    - (removed) pukowqtp 0cb7e07e Commit 1
    [EOF]
    ");

    // Test deletion of local bookmark.
    let output = work_dir.run_jj(["bookmark", "delete", "bookmark-2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Deleted 1 bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 299860162381 (2001-02-03 08:05:35) point bookmark bookmark-1 to commit 96f3a57c9a4a4ae7bb45d1eafe32fe3b6e33f458
      To operation: 869a48b9de32 (2001-02-03 08:05:37) delete bookmark bookmark-2

    Changed local bookmarks:
    bookmark-2:
    + (absent)
    - kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    [EOF]
    ");

    // Test pushing to Git remote.
    let output = work_dir.run_jj(["git", "push", "--tracked", "--deleted"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark-1 from 0dee631320b1 to 96f3a57c9a4a
      Delete bookmark bookmark-2 from e1a239a57eb1
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 869a48b9de32 (2001-02-03 08:05:37) delete bookmark bookmark-2
      To operation: 8e9b0502a1b2 (2001-02-03 08:05:39) push all tracked bookmarks to git remote origin

    Changed remote bookmarks:
    bookmark-1@origin:
    + tracked qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit
    - tracked zkmtkqvo 0dee6313 Commit 4
    bookmark-2@origin:
    + untracked (absent)
    - tracked kulxwnxm e1a239a5 Commit 5
    [EOF]
    ");

    // Test creation of tag.
    work_dir.run_jj(["tag", "set", "tag1"]).success();
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 8e9b0502a1b2 (2001-02-03 08:05:39) push all tracked bookmarks to git remote origin
      To operation: 18d8235061a1 (2001-02-03 08:05:41) set tag tag1 to commit 96f3a57c9a4a4ae7bb45d1eafe32fe3b6e33f458

    Changed commits:
    ○  + wvmqtotl 56e74c8d (empty) (no description set)

    Changed working copy default@:
    + wvmqtotl 56e74c8d (empty) (no description set)
    - qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit

    Changed local tags:
    tag1:
    + qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit
    - (absent)
    [EOF]
    ");

    // Test tag movement.
    work_dir
        .run_jj(["tag", "set", "tag1", "-r=@-", "--allow-move"])
        .success();
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 8e9b0502a1b2 (2001-02-03 08:05:39) push all tracked bookmarks to git remote origin
      To operation: 18d8235061a1 (2001-02-03 08:05:41) set tag tag1 to commit 96f3a57c9a4a4ae7bb45d1eafe32fe3b6e33f458

    Changed commits:
    ○  + wvmqtotl 56e74c8d (empty) (no description set)

    Changed working copy default@:
    + wvmqtotl 56e74c8d (empty) (no description set)
    - qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit

    Changed local tags:
    tag1:
    + qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit
    - (absent)
    [EOF]
    ");

    // Test tag deletion.
    work_dir.run_jj(["tag", "delete", "tag1"]).success();
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 18d8235061a1 (2001-02-03 08:05:41) set tag tag1 to commit 96f3a57c9a4a4ae7bb45d1eafe32fe3b6e33f458
      To operation: 9fe48b9198b5 (2001-02-03 08:05:45) delete tag tag1

    Changed local tags:
    tag1:
    + (absent)
    - qmkrwlvp 96f3a57c bookmark-1 | (empty) new commit
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz c1c924b8 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--op", "@-", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    From operation: e39dc288903d (2001-02-03 08:05:07) add workspace 'default'
      To operation: f148e2f53b28 (2001-02-03 08:05:08) snapshot working copy

    Changed commits:
    ○  + qpvuntsm 6b57e33c (no description set)
       - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -0,0 +1,1 @@
       +a

    Changed working copy default@:
    + qpvuntsm 6b57e33c (no description set)
    - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "--op", "@", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    From operation: f148e2f53b28 (2001-02-03 08:05:08) snapshot working copy
      To operation: 487462262dfc (2001-02-03 08:05:08) new empty commit

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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 6cbd01ae (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    From operation: 7a92b9c35cdf (2001-02-03 08:05:11) snapshot working copy
      To operation: f9b785eed068 (2001-02-03 08:05:11) squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c

    Changed commits:
    ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    ○  + qpvuntsm 7aa2ec5d (no description set)
       - qpvuntsm/1 6b57e33c (hidden) (no description set)
       - rlvkpnrz/0 05a2969e (hidden) (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b

    Changed working copy default@:
    + mzvwutvl 6cbd01ae (empty) (no description set)
    - rlvkpnrz/0 05a2969e (hidden) (no description set)
    [EOF]
    ");

    // Abandon the working copy commit.
    let output = work_dir.run_jj(["abandon"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      mzvwutvl 6cbd01ae (empty) (no description set)
    Working copy  (@) now at: yqosqzyt c97a8573 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "diff", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    From operation: f9b785eed068 (2001-02-03 08:05:11) squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
      To operation: 345aaf62e45d (2001-02-03 08:05:13) abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe

    Changed commits:
    ○  + yqosqzyt c97a8573 (empty) (no description set)
    ○  - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)

    Changed working copy default@:
    + yqosqzyt c97a8573 (empty) (no description set)
    - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)
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
    insta::assert_snapshot!(base_op_id, @"e39dc288903d");

    // Create merge commit at one operation side. The parent trees will have to
    // be merged when diffing, which requires the commit index of this side.
    work_dir.run_jj(["new", "root()", "-mA.1"]).success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new", "root()", "-mA.2"]).success();
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new", "@-+", "-mA"]).success();

    // Create another operation diverged from the base operation.
    work_dir
        .run_jj(["describe", "--at-op", base_op_id, "-mB"])
        .success();

    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @    0571f8166a8a test-username@host.example.com default@ 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj op log
    ○ │  62224ab1f2ca test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  new empty commit
    │ │  args: jj new '@-+' -mA
    ○ │  4fb5ff2413b4 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new '@-+' -mA
    ○ │  3cccd7ed737f test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.2
    ○ │  980ecdc182d0 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │ │  snapshot working copy
    │ │  args: jj new 'root()' -mA.2
    ○ │  607b11df2cee test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  new empty commit
    │ │  args: jj new 'root()' -mA.1
    │ ○  d330b16f187a test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    ├─╯  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │    args: jj describe --at-op e39dc288903d -mB
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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
    insta::assert_snapshot!(head_op_id, @"0571f8166a8a");
    insta::assert_snapshot!(p1_op_id, @"62224ab1f2ca");
    insta::assert_snapshot!(p2_op_id, @"d330b16f187a");

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
    insta::assert_snapshot!(output, @"
    From operation: 62224ab1f2ca (2001-02-03 08:05:11) new empty commit
      To operation: d330b16f187a (2001-02-03 08:05:12) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781

    Changed commits:
    ○    - mzvwutvl/0 08c63613 (hidden) (empty) A
    ├─╮
    │ ○  - kkmpptxz/0 6c70a4f7 (hidden) A.1
    │    A file1
    ○  - zsuskuln/0 47b9525e (hidden) A.2
       A file2
    ○  + qpvuntsm b1ca67e2 (empty) B
       - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)

    Changed working copy default@:
    + qpvuntsm b1ca67e2 (empty) B
    - mzvwutvl/0 08c63613 (hidden) (empty) A
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
    insta::assert_snapshot!(output, @"
    From operation: d330b16f187a (2001-02-03 08:05:12) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
      To operation: 62224ab1f2ca (2001-02-03 08:05:11) new empty commit

    Changed commits:
    ○  - qpvuntsm/0 b1ca67e2 (hidden) (empty) B
    ○    + mzvwutvl 08c63613 (empty) A
    ├─╮
    │ ○  + kkmpptxz 6c70a4f7 A.1
    │    A file1
    ○  + zsuskuln 47b9525e A.2
       A file2

    Changed working copy default@:
    + mzvwutvl 08c63613 (empty) A
    - qpvuntsm/0 b1ca67e2 (hidden) (empty) B
    [EOF]
    ");

    // no graph
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
        "--no-graph",
    ]);
    insta::assert_snapshot!(output, @"
    From operation: d330b16f187a (2001-02-03 08:05:12) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
      To operation: 62224ab1f2ca (2001-02-03 08:05:11) new empty commit

    Changed commits:
    - qpvuntsm/0 b1ca67e2 (hidden) (empty) B
    + mzvwutvl 08c63613 (empty) A
    + zsuskuln 47b9525e A.2
    A file2
    + kkmpptxz 6c70a4f7 A.1
    A file1

    Changed working copy default@:
    + mzvwutvl 08c63613 (empty) A
    - qpvuntsm/0 b1ca67e2 (hidden) (empty) B
    [EOF]
    ");
}

#[test]
fn test_op_diff_divergent_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Initial change
    work_dir.write_file("file", "1\n");
    work_dir.run_jj(["commit", "-m1"]).success();
    let initial_op_id = work_dir.current_operation_id();

    // Create divergent change
    work_dir.write_file("file", "2a\n1\n");
    work_dir.run_jj(["desc", "-m2a"]).success();
    work_dir.run_jj(["edit", "at_operation(@--, @)"]).success();
    work_dir.write_file("file", "1\n2b\n");
    work_dir.run_jj(["desc", "-m2b"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["log"]), @"
    @  rlvkpnrz/0 test.user@example.com 2001-02-03 08:05:11 c5cad9ab (divergent)
    │  2b
    │ ○  rlvkpnrz/2 test.user@example.com 2001-02-03 08:05:09 f189cafa (divergent)
    ├─╯  2a
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8a06f3b3
    │  1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    let divergent_op_id = work_dir.current_operation_id();

    // Resolve divergence by squashing commits
    work_dir
        .run_jj(["squash", "--from=subject(2a)", "--to=@", "-m2ab"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["log"]), @"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:13 17d68d92
    │  2ab
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8a06f3b3
    │  1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    let resolved_op_id = work_dir.current_operation_id();

    // Diff of new divergence
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &initial_op_id,
        "--to",
        &divergent_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: a5bc1e206f86 (2001-02-03 08:05:08) commit 5d86d4b609080a15077fcd723e537582d5ea6559
      To operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b

    Changed commits:
    ○  + rlvkpnrz/0 c5cad9ab (divergent) 2b
       - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)
    ○  + rlvkpnrz/2 f189cafa (divergent) 2a
       - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)

    Changed working copy default@:
    + rlvkpnrz/0 c5cad9ab (divergent) 2b
    - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)
    [EOF]
    ");

    // Diff of old divergence
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &divergent_op_id,
        "--to",
        &resolved_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b
      To operation: f686e63324ad (2001-02-03 08:05:13) squash commits into c5cad9ab7772714178c158a133a0243908545b48

    Changed commits:
    ○  + rlvkpnrz 17d68d92 2ab
       - rlvkpnrz/1 c5cad9ab (hidden) 2b
       - rlvkpnrz/3 f189cafa (hidden) 2a

    Changed working copy default@:
    + rlvkpnrz 17d68d92 2ab
    - rlvkpnrz/1 c5cad9ab (hidden) 2b
    [EOF]
    ");

    // Diff of new divergence with patch
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--git",
        "--from",
        &initial_op_id,
        "--to",
        &divergent_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: a5bc1e206f86 (2001-02-03 08:05:08) commit 5d86d4b609080a15077fcd723e537582d5ea6559
      To operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b

    Changed commits:
    ○  + rlvkpnrz/0 c5cad9ab (divergent) 2b
       - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)
       diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
       --- JJ-COMMIT-DESCRIPTION
       +++ JJ-COMMIT-DESCRIPTION
       @@ -0,0 +1,1 @@
       +2b
       diff --git a/file b/file
       index d00491fd7e..5e0f51b37b 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,2 @@
        1
       +2b
    ○  + rlvkpnrz/2 f189cafa (divergent) 2a
       - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)
       diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
       --- JJ-COMMIT-DESCRIPTION
       +++ JJ-COMMIT-DESCRIPTION
       @@ -0,0 +1,1 @@
       +2a
       diff --git a/file b/file
       index d00491fd7e..13a46f22fa 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,2 @@
       +2a
        1

    Changed working copy default@:
    + rlvkpnrz/0 c5cad9ab (divergent) 2b
    - rlvkpnrz/4 4f7a567a (hidden) (empty) (no description set)
    [EOF]
    ");

    // Diff of old divergence with patch
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--git",
        "--from",
        &divergent_op_id,
        "--to",
        &resolved_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b
      To operation: f686e63324ad (2001-02-03 08:05:13) squash commits into c5cad9ab7772714178c158a133a0243908545b48

    Changed commits:
    ○  + rlvkpnrz 17d68d92 2ab
       - rlvkpnrz/1 c5cad9ab (hidden) 2b
       - rlvkpnrz/3 f189cafa (hidden) 2a
       diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
       --- JJ-COMMIT-DESCRIPTION
       +++ JJ-COMMIT-DESCRIPTION
       @@ -1,1 +1,1 @@
       -2b
       +2ab
       diff --git a/file b/file
       index 5e0f51b37b..60327514e0 100644
       --- a/file
       +++ b/file
       @@ -1,2 +1,3 @@
       +2a
        1
        2b

    Changed working copy default@:
    + rlvkpnrz 17d68d92 2ab
    - rlvkpnrz/1 c5cad9ab (hidden) 2b
    [EOF]
    ");

    // Reverse diff of old divergence
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &resolved_op_id,
        "--to",
        &divergent_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: f686e63324ad (2001-02-03 08:05:13) squash commits into c5cad9ab7772714178c158a133a0243908545b48
      To operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b

    Changed commits:
    ○  + rlvkpnrz/1 c5cad9ab (divergent) 2b
       - rlvkpnrz/0 17d68d92 (hidden) 2ab
    ○  + rlvkpnrz/3 f189cafa (divergent) 2a
       - rlvkpnrz/0 17d68d92 (hidden) 2ab

    Changed working copy default@:
    + rlvkpnrz/1 c5cad9ab (divergent) 2b
    - rlvkpnrz/0 17d68d92 (hidden) 2ab
    [EOF]
    ");

    // Reverse diff of new divergence
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &divergent_op_id,
        "--to",
        &initial_op_id,
    ]);
    insta::assert_snapshot!(output, @"
    From operation: 00e13cdbe522 (2001-02-03 08:05:11) describe commit 7a72a9ad7f4d8aa8b613a9840313b0ef0632842b
      To operation: a5bc1e206f86 (2001-02-03 08:05:08) commit 5d86d4b609080a15077fcd723e537582d5ea6559

    Changed commits:
    ○  + rlvkpnrz 4f7a567a (empty) (no description set)
       - rlvkpnrz/2 f189cafa (hidden) 2a
       - rlvkpnrz/0 c5cad9ab (hidden) 2b

    Changed working copy default@:
    + rlvkpnrz 4f7a567a (empty) (no description set)
    - rlvkpnrz/0 c5cad9ab (hidden) 2b
    [EOF]
    ");
}

#[test]
fn test_op_diff_at_merge_op_with_rebased_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create merge operation that rebases descendant commits
    work_dir.run_jj(["new", "-m2a"]).success();
    work_dir.run_jj(["desc", "-r@-", "-m1"]).success();
    work_dir.run_jj(["desc", "--at-op=@-", "-m2b"]).success();

    insta::assert_snapshot!(work_dir.run_jj(["log"]), @"
    @  rlvkpnrz/2 test.user@example.com 2001-02-03 08:05:09 7ed5a610 (divergent)
    │  (empty) 2a
    │ ○  rlvkpnrz/0 test.user@example.com 2001-02-03 08:05:11 8f35f6a6 (divergent)
    ├─╯  (empty) 2b
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:09 6666e5c3
    │  (empty) 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Rebased 1 descendant commits onto commits rewritten by other operation
    [EOF]
    ");

    // FIXME: the diff should be empty
    let output = work_dir.run_jj(["op", "diff"]);
    insta::assert_snapshot!(output, @"
    From operation: 68f983fdc890 (2001-02-03 08:05:09) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    From operation: b682ca9ba03c (2001-02-03 08:05:10) describe commit ab92d1a87bebb4300165a16a753c5403bd7bc578
      To operation: 64e09790c2a8 (2001-02-03 08:05:11) reconcile divergent operations

    Changed commits:
    ○  + rlvkpnrz/1 8f35f6a6 (divergent) (empty) 2b
       - rlvkpnrz/0 4545eaf5 (hidden) (empty) 2b
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    64e09790c2a8 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    reconcile divergent operations
    args: jj log
    [EOF]
    ");

    let output = work_dir.run_jj(["op", "log", "--op-diff", "--limit=3"]);
    insta::assert_snapshot!(output, @"
    @    64e09790c2a8 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    ├─╮  reconcile divergent operations
    │ │  args: jj log
    ○ │  68f983fdc890 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │ │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │ │  args: jj describe -r@- -m1
    │ │
    │ │  Changed commits:
    │ │  ○  + rlvkpnrz 7ed5a610 (empty) 2a
    │ │  │  - rlvkpnrz/1 ab92d1a8 (hidden) (empty) 2a
    │ │  ○  + qpvuntsm 6666e5c3 (empty) 1
    │ │     - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    │ │
    │ │  Changed working copy default@:
    │ │  + rlvkpnrz 7ed5a610 (empty) 2a
    │ │  - rlvkpnrz/1 ab92d1a8 (hidden) (empty) 2a
    │ ○  b682ca9ba03c test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    ├─╯  describe commit ab92d1a87bebb4300165a16a753c5403bd7bc578
    │    args: jj describe '--at-op=@-' -m2b
    │
    │    Changed commits:
    │    ○  + rlvkpnrz 50ec12eb (empty) 2b
    │       - rlvkpnrz/1 ab92d1a8 (hidden) (empty) 2a
    │
    │    Changed working copy default@:
    │    + rlvkpnrz 50ec12eb (empty) 2b
    │    - rlvkpnrz/1 ab92d1a8 (hidden) (empty) 2a
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
                .arg("--config=revset-aliases.'immutable_heads()'='root()'")
                .env("COLUMNS", columns.to_string())
        })
    };

    // Add some file content changes
    work_dir.write_file("file1", "foo\n".repeat(100));
    work_dir.run_jj(["debug", "snapshot"]).success();

    // ui.log-word-wrap option works, and diff stat respects content width
    insta::assert_snapshot!(render(&["op", "diff", "--from=@---", "--stat"], 40, true), @"
    From operation: c2d0c49630f8 (2001-02-03 08:05:07) add git remote origin
      To operation: c1ad333a0303 (2001-02-03 08:05:08) snapshot working copy

    Changed commits:
    ○  + sqpuoqvx f6f32c19 (no description
    │  set)
    │  file1 | 100 +++++++++++++++++++++++++
    │  1 file changed, 100 insertions(+), 0 deletions(-)
    ○  + pukowqtp 0cb7e07e bookmark-1 |
       Commit 1
       some-file | 1 +
       1 file changed, 1 insertion(+), 0 deletions(-)
    ○  + skovwzlu 854c38b8 Commit 4
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
    ○  - qpvuntsm/0 e8849ae1 (hidden)
       (empty) (no description set)
       0 files changed, 0 insertions(+), 0 deletions(-)

    Changed working copy default@:
    + sqpuoqvx f6f32c19 (no description set)
    - qpvuntsm/0 e8849ae1 (hidden) (empty)
    (no description set)

    Changed local bookmarks:
    bookmark-1:
    + pukowqtp 0cb7e07e bookmark-1 | Commit
    1
    - (absent)

    Changed local tags:
    tag-1:
    + skovwzlu 854c38b8 Commit 4
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
        render(&["op", "diff", "--from=@---", "--config", config], 10, true), @"
    From operation: c2d0c49630f8 (2001-02-03 08:05:07) add git remote origin
      To operation: c1ad333a0303 (2001-02-03 08:05:08) snapshot working copy

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
    local
    tags:
    tag-1:
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
    work_dir
        .run_jj(["git", "fetch", "--branch=*", "--tag=*"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "bookmark-1"])
        .success();

    // Overview of op log.
    let output = work_dir.run_jj(["op", "log"]);
    insta::assert_snapshot!(output, @"
    @  50ecb1b62028 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  track remote bookmark bookmark-1@origin
    │  args: jj bookmark track bookmark-1
    ○  6d2bd2526eb3 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  fetch from git remote(s) origin
    │  args: jj git fetch '--branch=*' '--tag=*'
    ○  ab33a510f80e test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  add git remote origin
    │  args: jj git remote add origin ../git-repo
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");

    // The root operation is empty.
    let output = work_dir.run_jj(["op", "show", "0000000"]);
    insta::assert_snapshot!(output, @"
    000000000000 root()
    [EOF]
    ");

    // Showing the latest operation.
    let output = work_dir.run_jj(["op", "show", "@"]);
    insta::assert_snapshot!(output, @"
    50ecb1b62028 test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    track remote bookmark bookmark-1@origin
    args: jj bookmark track bookmark-1

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
    insta::assert_snapshot!(output, @"
    6d2bd2526eb3 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    fetch from git remote(s) origin
    args: jj git fetch '--branch=*' '--tag=*'

    Changed commits:
    ○  + skovwzlu 854c38b8 Commit 4
    ○  + rnnslrkn 4ff62539 bookmark-2@origin | Commit 2
    ○  + rnnkyono 11671e4c bookmark-3@origin | Commit 3
    ○  + pukowqtp 0cb7e07e bookmark-1@origin | Commit 1

    Changed local tags:
    tag-1:
    + skovwzlu 854c38b8 Commit 4
    - (absent)

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

    Changed remote tags:
    tag-1@origin:
    + tracked skovwzlu 854c38b8 Commit 4
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    0a111af39a27 test-username@host.example.com default@ 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    reconcile divergent operations
    args: jj log
    [EOF]
    ");

    // Test fetching from git remote.
    modify_git_repo(git_repo);
    let output = work_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    bookmark: bookmark-1@origin [updated] tracked
    bookmark: bookmark-2@origin [updated] untracked
    bookmark: bookmark-3@origin [deleted] untracked
    tag: tag-1@git [new] 
    Abandoned 1 commits that are no longer reachable:
      rnnkyono 11671e4c Commit 3
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    9b94bdc8a99a test-username@host.example.com default@ 2001-02-03 04:05:19.000 +07:00 - 2001-02-03 04:05:19.000 +07:00
    fetch from git remote(s) origin
    args: jj git fetch

    Changed commits:
    ○  + kulxwnxm e1a239a5 bookmark-2@origin | Commit 5
    ○  + zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    ○  - rnnkyono/0 11671e4c (hidden) Commit 3

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
    - untracked rnnkyono/0 11671e4c (hidden) Commit 3
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Created 1 bookmarks pointing to kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    fa6006b6ac16 test-username@host.example.com default@ 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    create bookmark bookmark-2 pointing to commit e1a239a57eb15cefc5910198befbbbe2b43c47af
    args: jj bookmark create bookmark-2 -r bookmark-2@origin

    Changed local bookmarks:
    bookmark-2:
    + kulxwnxm e1a239a5 bookmark-2 bookmark-2@origin | Commit 5
    - (absent)
    [EOF]
    ");

    // Test tracking of a bookmark.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Started tracking 1 remote bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    e66568b72587 test-username@host.example.com default@ 2001-02-03 04:05:23.000 +07:00 - 2001-02-03 04:05:23.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["bookmark", "track", "bookmark-2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Remote bookmark already tracked: bookmark-2@origin
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    e66568b72587 test-username@host.example.com default@ 2001-02-03 04:05:23.000 +07:00 - 2001-02-03 04:05:23.000 +07:00
    track remote bookmark bookmark-2@origin
    args: jj bookmark track bookmark-2

    Changed remote bookmarks:
    bookmark-2@origin:
    + tracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    - untracked kulxwnxm e1a239a5 bookmark-2 | Commit 5
    [EOF]
    ");

    // Test creation of new commit.
    let output = work_dir.run_jj(["new", "bookmark-1@origin", "-m", "new commit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: tlkvzzqu 8f340dd7 (empty) new commit
    Parent commit (@-)      : zkmtkqvo 0dee6313 bookmark-1?? bookmark-1@origin | Commit 4
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    d8d9a7f66c79 test-username@host.example.com default@ 2001-02-03 04:05:27.000 +07:00 - 2001-02-03 04:05:27.000 +07:00
    new empty commit
    args: jj new bookmark-1@origin -m 'new commit'

    Changed commits:
    ○  + tlkvzzqu 8f340dd7 (empty) new commit
    ○  - qpvuntsm/0 e8849ae1 (hidden) (empty) (no description set)

    Changed working copy default@:
    + tlkvzzqu 8f340dd7 (empty) new commit
    - qpvuntsm/0 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Test updating of local bookmark.
    let output = work_dir.run_jj(["bookmark", "set", "bookmark-1", "-r", "@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Moved 1 bookmarks to tlkvzzqu 8f340dd7 bookmark-1* | (empty) new commit
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    b9e2a46e6c69 test-username@host.example.com default@ 2001-02-03 04:05:29.000 +07:00 - 2001-02-03 04:05:29.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Deleted 1 bookmarks.
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    3265d109cb32 test-username@host.example.com default@ 2001-02-03 04:05:31.000 +07:00 - 2001-02-03 04:05:31.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Changes to push to origin:
      Move forward bookmark bookmark-1 from 0dee631320b1 to 8f340dd76dc6
      Delete bookmark bookmark-2 from e1a239a57eb1
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show"]);
    insta::assert_snapshot!(output, @"
    e19c28e79f4c test-username@host.example.com default@ 2001-02-03 04:05:33.000 +07:00 - 2001-02-03 04:05:33.000 +07:00
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

    // Showing a given operation, without graph
    let output = work_dir.run_jj(["op", "show", "--no-graph", "3117c5ac4af3"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: No operation ID matching "3117c5ac4af3"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_op_show_patch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Update working copy with a single file and create new commit.
    work_dir.write_file("file", "a\n");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz c1c924b8 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 6b57e33c (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "@-", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    f148e2f53b28 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    snapshot working copy
    args: jj new

    Changed commits:
    ○  + qpvuntsm 6b57e33c (no description set)
       - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
       diff --git a/file b/file
       new file mode 100644
       index 0000000000..7898192261
       --- /dev/null
       +++ b/file
       @@ -0,0 +1,1 @@
       +a

    Changed working copy default@:
    + qpvuntsm 6b57e33c (no description set)
    - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "@", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    487462262dfc test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 6cbd01ae (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    f9b785eed068 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
    args: jj squash

    Changed commits:
    ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    ○  + qpvuntsm 7aa2ec5d (no description set)
       - qpvuntsm/1 6b57e33c (hidden) (no description set)
       - rlvkpnrz/0 05a2969e (hidden) (no description set)
       diff --git a/file b/file
       index 7898192261..6178079822 100644
       --- a/file
       +++ b/file
       @@ -1,1 +1,1 @@
       -a
       +b

    Changed working copy default@:
    + mzvwutvl 6cbd01ae (empty) (no description set)
    - rlvkpnrz/0 05a2969e (hidden) (no description set)
    [EOF]
    ");

    // Abandon the working copy commit.
    let output = work_dir.run_jj(["abandon"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      mzvwutvl 6cbd01ae (empty) (no description set)
    Working copy  (@) now at: yqosqzyt c97a8573 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 7aa2ec5d (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["op", "show", "-p", "--git"]);
    insta::assert_snapshot!(output, @"
    345aaf62e45d test-username@host.example.com default@ 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe
    args: jj abandon

    Changed commits:
    ○  + yqosqzyt c97a8573 (empty) (no description set)
    ○  - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)

    Changed working copy default@:
    + yqosqzyt c97a8573 (empty) (no description set)
    - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)
    [EOF]
    ");

    // Try again with "op log".
    let output = work_dir.run_jj(["op", "log", "--git"]);
    insta::assert_snapshot!(output, @"
    @  345aaf62e45d test-username@host.example.com default@ 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  abandon commit 6cbd01aefe5ae05a015328311dbd63b7305b8ebe
    │  args: jj abandon
    │
    │  Changed commits:
    │  ○  + yqosqzyt c97a8573 (empty) (no description set)
    │  ○  - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + yqosqzyt c97a8573 (empty) (no description set)
    │  - mzvwutvl/0 6cbd01ae (hidden) (empty) (no description set)
    ○  f9b785eed068 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  squash commits into 6b57e33cc56babbeaa6bcd6e2a296236b52ad93c
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  + mzvwutvl 6cbd01ae (empty) (no description set)
    │  ○  + qpvuntsm 7aa2ec5d (no description set)
    │     - qpvuntsm/1 6b57e33c (hidden) (no description set)
    │     - rlvkpnrz/0 05a2969e (hidden) (no description set)
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
    │  - rlvkpnrz/0 05a2969e (hidden) (no description set)
    ○  7a92b9c35cdf test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  snapshot working copy
    │  args: jj squash
    │
    │  Changed commits:
    │  ○  + rlvkpnrz 05a2969e (no description set)
    │     - rlvkpnrz/1 c1c924b8 (hidden) (empty) (no description set)
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
    │  - rlvkpnrz/1 c1c924b8 (hidden) (empty) (no description set)
    ○  487462262dfc test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  new empty commit
    │  args: jj new
    │
    │  Changed commits:
    │  ○  + rlvkpnrz c1c924b8 (empty) (no description set)
    │
    │  Changed working copy default@:
    │  + rlvkpnrz c1c924b8 (empty) (no description set)
    │  - qpvuntsm 6b57e33c (no description set)
    ○  f148e2f53b28 test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  snapshot working copy
    │  args: jj new
    │
    │  Changed commits:
    │  ○  + qpvuntsm 6b57e33c (no description set)
    │     - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
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
    │  - qpvuntsm/1 e8849ae1 (hidden) (empty) (no description set)
    ○  e39dc288903d test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
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

#[test]
fn test_op_show_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "content\n");
    work_dir.run_jj(["commit", "-m", "first commit"]).success();

    // Test with custom template
    let output = work_dir.run_jj([
        "op",
        "show",
        "-T",
        r#"separate(" ", id.short(), description)"#,
        "--no-op-diff",
    ]);
    insta::assert_snapshot!(output, @"02f1b290fb1f commit 0883ea507656cce545dbba9f23760ff72dff5174[EOF]");

    // Test --no-op-diff flag suppresses the diff
    let output = work_dir.run_jj(["op", "show", "--no-op-diff"]);
    insta::assert_snapshot!(output, @"
    02f1b290fb1f test-username@host.example.com default@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    commit 0883ea507656cce545dbba9f23760ff72dff5174
    args: jj commit -m 'first commit'
    [EOF]
    ");

    // Test with custom template, without --no-op-diff
    let output = work_dir.run_jj([
        "op",
        "show",
        "-T",
        r#"separate(" ", id.short(), description)"#,
    ]);
    insta::assert_snapshot!(output, @"
    02f1b290fb1f commit 0883ea507656cce545dbba9f23760ff72dff5174
    Changed commits:
    ○  + rlvkpnrz e4863b8c (empty) (no description set)
    ○  + qpvuntsm b52b7cb5 first commit
       - qpvuntsm/1 0883ea50 (hidden) (no description set)

    Changed working copy default@:
    + rlvkpnrz e4863b8c (empty) (no description set)
    - qpvuntsm/1 0883ea50 (hidden) (no description set)
    [EOF]
    ");
}

#[test]
fn test_op_log_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    work_dir
        .run_jj(["describe", "-m", "description 1", "--at-op", "@-"])
        .success();
    let template = r#"id.short() ++ "\nP: " ++ parents.len() ++ " " ++ parents.map(|o| o.id().short()) ++ "\n""#;
    let output = work_dir.run_jj(["op", "log", "-T", template]);
    insta::assert_snapshot!(output, @"
    @    f4a47cf52d6d
    ├─╮  P: 2 69e9fab8ce76 a395e6e6a995
    ○ │  69e9fab8ce76
    │ │  P: 1 e39dc288903d
    │ ○  a395e6e6a995
    ├─╯  P: 1 e39dc288903d
    ○  e39dc288903d
    │  P: 1 000000000000
    ○  000000000000
       P: 0
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_op_log_anonymize() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();

    let output = work_dir.run_jj(["op", "log", "-Tbuiltin_op_log_redacted"]);
    insta::assert_snapshot!(output, @"
    @  69e9fab8ce76 user-5910 workspace-ab88@ 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  (redacted)
    ○  e39dc288903d user-5910 workspace-482a@ 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ○  000000000000 root()
    [EOF]
    ");
}

#[test]
fn test_op_immutable_revisions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "tags() | bookmarks()""#);
    test_env.add_config(r#"revsets.op-diff-changes-in = "mutable() | immutable_heads()""#);

    // 1. Basic addition and removal elision
    // Create a stack of 5 commits, all immutable.
    for i in 1..=5 {
        work_dir
            .run_jj(["new", "@", "-m", &format!("commit {i}")])
            .success();
    }
    work_dir.run_jj(["tag", "set", "t1", "-r", "@"]).success();

    // Move working copy away
    work_dir.run_jj(["new", "root()"]).success();

    // Abandon the immutable stack
    work_dir
        .run_jj(["abandon", "--ignore-immutable", "::t1 & ~root()"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    91e586c86bf2 test-username@host.example.com default@ 2001-02-03 04:05:15.000 +07:00 - 2001-02-03 04:05:15.000 +07:00
    abandon commit 9c86781f3fe9097ffc530e65fd2ab4aff1e654bd and 5 more
    args: jj abandon --ignore-immutable '::t1 & ~root()'

    Changed commits:
    ○  - royxmykx/0 9c86781f (hidden) (empty) commit 5
       (Elided 5 newly removed revisions)
    [EOF]
    ");

    // Undo
    work_dir.run_jj(["op", "revert"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    775a2ea85810 test-username@host.example.com default@ 2001-02-03 04:05:17.000 +07:00 - 2001-02-03 04:05:17.000 +07:00
    revert operation 91e586c86bf21b1501b4549f5fe6f35ad1085feb6c05d214aa08871af3ee2991c11105624ae60fbb5c07c73441fdba487eb521ede427a9bb9ef35e7f01817ab2
    args: jj op revert

    Changed commits:
    ○  + royxmykx 9c86781f (empty) commit 5
       (Elided 5 newly added revisions)
    [EOF]
    ");

    // 2. Multiple branches elision
    work_dir.run_jj(["new", "t1", "-m", "f1 1"]).success();
    work_dir.run_jj(["new", "@", "-m", "f1 2"]).success();
    work_dir.run_jj(["new", "@", "-m", "f1 3"]).success();
    work_dir.run_jj(["tag", "set", "f1", "-r", "@"]).success();

    work_dir.run_jj(["new", "t1", "-m", "f2 1"]).success();
    work_dir.run_jj(["new", "@", "-m", "f2 2"]).success();
    work_dir.run_jj(["new", "@", "-m", "f2 3"]).success();
    work_dir.run_jj(["tag", "set", "f2", "-r", "@"]).success();

    // Move WC away
    work_dir.run_jj(["new", "root()"]).success();

    // Abandon both chains
    work_dir
        .run_jj(["abandon", "--ignore-immutable", "(::f1 | ::f2) & ~root()"])
        .success();
    let op_id_for_diff = work_dir.current_operation_id();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    06bb06195de5 test-username@host.example.com default@ 2001-02-03 04:05:28.000 +07:00 - 2001-02-03 04:05:28.000 +07:00
    abandon commit e7f51c58b0862dc0c255d9efd11fb9a89f07eb88 and 11 more
    args: jj abandon --ignore-immutable '(::f1 | ::f2) & ~root()'

    Changed commits:
    ○  - xtnwkqum/0 e7f51c58 (hidden) (empty) f2 3
    ╷ ○  - kxryzmor/0 c800ddaf (hidden) (empty) f1 3
    ╭─╯
    ○  - royxmykx/0 9c86781f (hidden) (empty) commit 5
       (Elided 9 newly removed revisions)
    [EOF]
    ");

    // Use `--show-changes-in none()` to see only elisions
    insta::assert_snapshot!(work_dir.run_jj(["op", "show", "--show-changes-in", "none()"]), @"
    06bb06195de5 test-username@host.example.com default@ 2001-02-03 04:05:28.000 +07:00 - 2001-02-03 04:05:28.000 +07:00
    abandon commit e7f51c58b0862dc0c255d9efd11fb9a89f07eb88 and 11 more
    args: jj abandon --ignore-immutable '(::f1 | ::f2) & ~root()'

    Changed commits:
       (Elided 10+ newly removed revisions)
    [EOF]
    ");

    // 3. Case where both added and removed immutable revisions are elided.
    work_dir.run_jj(["new", "root()", "-m", "mix-a1"]).success();
    for i in 2..=5 {
        work_dir
            .run_jj(["new", "@", "-m", &format!("mix-a{i}")])
            .success();
    }
    work_dir
        .run_jj(["bookmark", "set", "ba", "-r", "@"])
        .success();

    work_dir.run_jj(["new", "root()", "-m", "mix-b1"]).success();
    for i in 2..=5 {
        work_dir
            .run_jj(["new", "@", "-m", &format!("mix-b{i}")])
            .success();
    }
    work_dir
        .run_jj(["bookmark", "set", "bb", "-r", "@"])
        .success();

    // Rebase bb chain onto ba head.
    work_dir
        .run_jj(["rebase", "--ignore-immutable", "-s", "bb----", "-d", "ba"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    076651080ac0 test-username@host.example.com default@ 2001-02-03 04:05:43.000 +07:00 - 2001-02-03 04:05:43.000 +07:00
    rebase commit c09af48da0b4dcbbe6be869823d17bf6cd73a4db and descendants
    args: jj rebase --ignore-immutable -s bb---- -d ba

    Changed commits:
    ○  + nsrwusvy caaf0759 (empty) (no description set)
    │  - nsrwusvy/1 76ebe692 (hidden) (empty) (no description set)
    ○  + wvmqtotl 44827d4a bb | (empty) mix-b5
       - wvmqtotl/1 c117dbab (hidden) (empty) mix-b5
       (Elided 4 newly added and 4 newly removed revisions)

    Changed working copy default@:
    + nsrwusvy caaf0759 (empty) (no description set)
    - nsrwusvy/1 76ebe692 (hidden) (empty) (no description set)

    Changed local bookmarks:
    bb:
    + wvmqtotl 44827d4a bb | (empty) mix-b5
    - wvmqtotl/1 c117dbab (hidden) (empty) mix-b5
    [EOF]
    ");

    // Use `--show-changes-in none()` to see only elisions
    insta::assert_snapshot!(work_dir.run_jj(["op", "show", "--show-changes-in", "none()"]), @"
    076651080ac0 test-username@host.example.com default@ 2001-02-03 04:05:43.000 +07:00 - 2001-02-03 04:05:43.000 +07:00
    rebase commit c09af48da0b4dcbbe6be869823d17bf6cd73a4db and descendants
    args: jj rebase --ignore-immutable -s bb---- -d ba

    Changed commits:
       (Elided 6 newly added and 6 newly removed revisions)

    Changed working copy default@:
    + nsrwusvy caaf0759 (empty) (no description set)
    - nsrwusvy/1 76ebe692 (hidden) (empty) (no description set)

    Changed local bookmarks:
    bb:
    + wvmqtotl 44827d4a bb | (empty) mix-b5
    - wvmqtotl/1 c117dbab (hidden) (empty) mix-b5
    [EOF]
    ");

    // 4. Case where exactly one immutable revision is elided (singular "revision")
    work_dir
        .run_jj(["new", "root()", "-m", "single-1"])
        .success();
    work_dir.run_jj(["new", "@", "-m", "single-2"]).success();
    work_dir
        .run_jj(["tag", "set", "ts", "-r", "@", "--allow-move"])
        .success();
    // Abandon to see single removal elision
    work_dir
        .run_jj(["abandon", "--ignore-immutable", "::ts & ~root()"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    e040ebb4360c test-username@host.example.com default@ 2001-02-03 04:05:49.000 +07:00 - 2001-02-03 04:05:49.000 +07:00
    abandon commit 0a2e24c8d8d8010243ed72f9c50ee69b57291eff and 1 more
    args: jj abandon --ignore-immutable '::ts & ~root()'

    Changed commits:
    ○  + sryyqqkq 46f2f483 (empty) (no description set)
       - sryyqqkq/1 d41cf466 (hidden) (empty) (no description set)
    ○  - ukwxllxp/0 0a2e24c8 (hidden) (empty) single-2
       (Elided 1 newly removed revisions)

    Changed working copy default@:
    + sryyqqkq 46f2f483 (empty) (no description set)
    - sryyqqkq/1 d41cf466 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Undo to see single addition elision
    work_dir.run_jj(["op", "revert"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]), @"
    ab74531a4cfa test-username@host.example.com default@ 2001-02-03 04:05:51.000 +07:00 - 2001-02-03 04:05:51.000 +07:00
    revert operation e040ebb4360ce004dc06b3a6a96603348085bc4644c79bea33f6de9986f5c49e8a125318c8160aa715b70db253da516357b2d23d8af14970111bbbe005d0e651
    args: jj op revert

    Changed commits:
    ○  + sryyqqkq d41cf466 (empty) (no description set)
    │  - sryyqqkq/0 46f2f483 (hidden) (empty) (no description set)
    ○  + ukwxllxp 0a2e24c8 (empty) single-2
       (Elided 1 newly added revisions)

    Changed working copy default@:
    + sryyqqkq d41cf466 (empty) (no description set)
    - sryyqqkq/0 46f2f483 (hidden) (empty) (no description set)
    [EOF]
    ");

    // 5. op diff and op log tests
    insta::assert_snapshot!(work_dir.run_jj(["op", "diff", "--from", &op_id_for_diff]), @"
    From operation: 06bb06195de5 (2001-02-03 08:05:28) abandon commit e7f51c58b0862dc0c255d9efd11fb9a89f07eb88 and 11 more
      To operation: ab74531a4cfa (2001-02-03 08:05:51) revert operation e040ebb4360ce004dc06b3a6a96603348085bc4644c79bea33f6de9986f5c49e8a125318c8160aa715b70db253da516357b2d23d8af14970111bbbe005d0e651

    Changed commits:
    ○  + sryyqqkq d41cf466 (empty) (no description set)
    ○  + ukwxllxp 0a2e24c8 (empty) single-2
    ○  + wvmqtotl 44827d4a bb | (empty) mix-b5
    ○  + pkstwlsy 2e898f29 ba | (empty) mix-a5
    ○  - tlkvzzqu/0 3f6d698d (hidden) (empty) (no description set)
       (Elided 9 newly added revisions)

    Changed working copy default@:
    + sryyqqkq d41cf466 (empty) (no description set)
    - tlkvzzqu/0 3f6d698d (hidden) (empty) (no description set)

    Changed local bookmarks:
    ba:
    + pkstwlsy 2e898f29 ba | (empty) mix-a5
    - (absent)
    bb:
    + wvmqtotl 44827d4a bb | (empty) mix-b5
    - (absent)

    Changed local tags:
    ts:
    + ukwxllxp 0a2e24c8 (empty) single-2
    - (absent)
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-p", "--limit", "1"]), @"
    @  ab74531a4cfa test-username@host.example.com default@ 2001-02-03 04:05:51.000 +07:00 - 2001-02-03 04:05:51.000 +07:00
    │  revert operation e040ebb4360ce004dc06b3a6a96603348085bc4644c79bea33f6de9986f5c49e8a125318c8160aa715b70db253da516357b2d23d8af14970111bbbe005d0e651
    │  args: jj op revert
    │
    │  Changed commits:
    │  ○  + sryyqqkq d41cf466 (empty) (no description set)
    │  │  - sryyqqkq/0 46f2f483 (hidden) (empty) (no description set)
    │  ○  + ukwxllxp 0a2e24c8 (empty) single-2
    │     Modified commit description:
    │             1: single-2
    │     (Elided 1 newly added revisions)
    │
    │  Changed working copy default@:
    │  + sryyqqkq d41cf466 (empty) (no description set)
    │  - sryyqqkq/0 46f2f483 (hidden) (empty) (no description set)
    [EOF]
    ");

    // 6. Accuracy: Show local heads of affected set even if they have immutable
    // descendants elsewhere (e.g. already hidden).
    // root -> c1 -> c2 -> c3 (all immutable)
    work_dir.run_jj(["new", "root()", "-m", "acc-c1"]).success();
    let c1_id = work_dir
        .run_jj(["log", "-T", "commit_id", "-r", "@", "--no-graph"])
        .stdout
        .raw()
        .trim()
        .to_string();
    work_dir.run_jj(["new", "@", "-m", "acc-c2"]).success();
    let c2_id = work_dir
        .run_jj(["log", "-T", "commit_id", "-r", "@", "--no-graph"])
        .stdout
        .raw()
        .trim()
        .to_string();
    work_dir.run_jj(["new", "@", "-m", "acc-c3"]).success();
    let c3_id = work_dir
        .run_jj(["log", "-T", "commit_id", "-r", "@", "--no-graph"])
        .stdout
        .raw()
        .trim()
        .to_string();

    // Use c2_id to ensure the hidden c2 is shown.
    test_env.add_config(format!(
        r#"revsets.op-diff-changes-in = "mutable() | (all() & {c2_id})""#,
    ));

    // Track all with bookmarks.
    work_dir
        .run_jj(["bookmark", "create", "ba1", "-r", &c1_id])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "ba2", "-r", &c2_id])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "ba3", "-r", &c3_id])
        .success();
    work_dir.run_jj(["new", "root()"]).success();

    // Operation A: Hide acc-c3 by abandoning it. c1 and c2 remain visible via
    // bookmarks.
    work_dir
        .run_jj(["abandon", "--ignore-immutable", "-r", &c3_id])
        .success();
    let op_a = work_dir.current_operation_id();

    // Operation B: Abandon c1 and c2. Both become hidden.
    // newly_hidden = {c1, c2}.
    // Option 1 (Fix) shows the head (c2) and elides the parent (c1).
    work_dir
        .run_jj(["abandon", "--ignore-immutable", "-r", &c1_id, "-r", &c2_id])
        .success();
    let op_b = work_dir.current_operation_id();

    // With --show-changes-in all(), the diff should show both c1 and c2 as
    // newly hidden.
    let output = work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &op_a,
        "--to",
        &op_b,
        "--show-changes-in",
        "all()",
    ]);
    insta::assert_snapshot!(output, @"
    From operation: 9bced8e7907b (2001-02-03 08:06:05) abandon commit 5ff32b4e633551c9d0e40760cd4f8b61937395c6
      To operation: 4a42dae3c320 (2001-02-03 08:06:06) abandon commit 61c625114e83805c3b25709d71cebedcf2be7406 and 1 more

    Changed commits:
    ○  - knltnxnu/0 61c62511 (hidden) (empty) acc-c2
    ○  - vkywoywq/0 a35cc4a3 (hidden) (empty) acc-c1

    Changed local bookmarks:
    ba1:
    + (absent)
    - vkywoywq/0 a35cc4a3 (hidden) (empty) acc-c1
    ba2:
    + (absent)
    - knltnxnu/0 61c62511 (hidden) (empty) acc-c2
    [EOF]
    ");

    // Without --show-changes-in, the diff should show c2 as the head
    // of the newly hidden set and elide c1.
    let output = work_dir.run_jj(["op", "diff", "--from", &op_a, "--to", &op_b]);
    insta::assert_snapshot!(output, @"
    From operation: 9bced8e7907b (2001-02-03 08:06:05) abandon commit 5ff32b4e633551c9d0e40760cd4f8b61937395c6
      To operation: 4a42dae3c320 (2001-02-03 08:06:06) abandon commit 61c625114e83805c3b25709d71cebedcf2be7406 and 1 more

    Changed commits:
    ○  - knltnxnu/0 61c62511 (hidden) (empty) acc-c2
       (Elided 1 newly removed revisions)

    Changed local bookmarks:
    ba1:
    + (absent)
    - vkywoywq/0 a35cc4a3 (hidden) (empty) acc-c1
    ba2:
    + (absent)
    - knltnxnu/0 61c62511 (hidden) (empty) acc-c2
    [EOF]
    ");
}

#[test]
fn test_op_show_revset_expression_resolution() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(
        r#"
[templates]
commit_summary = 'commit_id.short() ++ " " ++ description.first_line()'
[template-aliases]
'format_short_id(id)' = 'id.substr(0, 12)'
'format_short_change_id_with_change_offset(commit)' = 'commit.change_id().short()'
"#,
    );

    // 1. Initial commits.
    work_dir.run_jj(["new", "root()", "-m", "base"]).success();

    // 2. Create bookmark_x (op_create).
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark_x"])
        .success();
    let op_create = work_dir.current_operation_id();

    // 3. Create a stack of 2 commits.
    for i in 1..=2 {
        work_dir
            .run_jj(["new", "@", "-m", &format!("stack {i}")])
            .success();
    }
    work_dir
        .run_jj(["bookmark", "set", "bookmark_x", "-r@"])
        .success();

    // 4. Rebase the stack (op_rebase).
    work_dir
        .run_jj(["new", "root()", "-m", "new_base"])
        .success();
    let new_base = "@";
    work_dir
        .run_jj(["rebase", "-s", "bookmark_x-", "-d", new_base])
        .success();
    let op_rebase = work_dir.current_operation_id();

    // Configure op-diff-changes-in to require 'bookmark_x'.
    test_env.add_config(r#"revsets.op-diff-changes-in = "bookmark_x""#);

    // 5. Test op show for op_rebase: should show ELISION summary.
    // bookmark_x exists in both states of the rebase.
    insta::assert_snapshot!(work_dir.run_jj(["op", "show", &op_rebase]), @"
    75842f60dc81 test-username@host.example.com default@ 2001-02-03 04:05:14.000 +07:00 - 2001-02-03 04:05:14.000 +07:00
    rebase commit 0f12cf5c679b373cb1ee0fa3e441c2f5030c4dc9 and descendants
    args: jj rebase -s bookmark_x- -d @

    Changed commits:
    ○  + 3cafca23bb81 stack 2
       - 5456f1af47ed stack 2
       (Elided 1 newly added and 1 newly removed revisions)

    Changed local bookmarks:
    bookmark_x:
    + 3cafca23bb81 stack 2
    - 5456f1af47ed stack 2
    [EOF]
    ");

    // 6. Test op show for op_create: should show WARNING.
    // bookmark_x did not exist in the 'from' state.
    insta::assert_snapshot!(work_dir.run_jj(["op", "show", &op_create]), @"
    a6739a805d51 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    create bookmark bookmark_x pointing to commit 2308e5a241f7a47f186b0686ffb17aa613a727d7
    args: jj bookmark create -r@ bookmark_x

    Warning: Could not resolve revset expression for elision: Revision `bookmark_x` doesn't exist
       (Use --show-changes-in=all() to see all changes)

    Changed local bookmarks:
    bookmark_x:
    + 2308e5a241f7 base
    - (absent)
    [EOF]
    ");

    // 7. Test op show for op_create with the flag: should show all changes and NO
    //    WARNING.
    insta::assert_snapshot!(work_dir.run_jj(["op", "show", &op_create, "--show-changes-in", "all()"]), @"
    a6739a805d51 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    create bookmark bookmark_x pointing to commit 2308e5a241f7a47f186b0686ffb17aa613a727d7
    args: jj bookmark create -r@ bookmark_x

    Changed local bookmarks:
    bookmark_x:
    + 2308e5a241f7 base
    - (absent)
    [EOF]
    ");

    // 8. Test op diff from BEFORE creation to op_rebase: should show WARNING.
    let op_before_create = format!("{op_create}-");
    insta::assert_snapshot!(work_dir.run_jj(["op", "diff", "--from", &op_before_create, "--to", &op_rebase]), @"
    From operation: 06324bd96409 (2001-02-03 08:05:08) new empty commit
      To operation: 75842f60dc81 (2001-02-03 08:05:14) rebase commit 0f12cf5c679b373cb1ee0fa3e441c2f5030c4dc9 and descendants

    Warning: Could not resolve revset expression for elision: Revision `bookmark_x` doesn't exist
       (Use --show-changes-in=all() to see all changes)

    Changed working copy default@:
    + 6b753f7043b4 new_base
    - 2308e5a241f7 base

    Changed local bookmarks:
    bookmark_x:
    + 3cafca23bb81 stack 2
    - (absent)
    [EOF]
    ");

    // 9. Test op diff with the flag: should show all changes and NO WARNING.
    insta::assert_snapshot!(work_dir.run_jj([
        "op",
        "diff",
        "--from",
        &op_before_create,
        "--to",
        &op_rebase,
        "--show-changes-in",
        "all()",
    ]), @"
    From operation: 06324bd96409 (2001-02-03 08:05:08) new empty commit
      To operation: 75842f60dc81 (2001-02-03 08:05:14) rebase commit 0f12cf5c679b373cb1ee0fa3e441c2f5030c4dc9 and descendants

    Changed commits:
    ○  + 3cafca23bb81 stack 2
    ○  + e7bd1678832f stack 1
    ○  + 6b753f7043b4 new_base

    Changed working copy default@:
    + 6b753f7043b4 new_base
    - 2308e5a241f7 base

    Changed local bookmarks:
    bookmark_x:
    + 3cafca23bb81 stack 2
    - (absent)
    [EOF]
    ");

    // 10. Test op log -p: should show BOTH behaviors.
    test_env.add_config(r#"revsets.op-diff-changes-in = "mutable() | bookmark_x""#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "log", "-p", "--limit", "6"]), @"
    @  75842f60dc81 test-username@host.example.com default@ 2001-02-03 04:05:14.000 +07:00 - 2001-02-03 04:05:14.000 +07:00
    │  rebase commit 0f12cf5c679b373cb1ee0fa3e441c2f5030c4dc9 and descendants
    │  args: jj rebase -s bookmark_x- -d @
    │
    │  Changed commits:
    │  ○  + 3cafca23bb81 stack 2
    │  │  - 5456f1af47ed stack 2
    │  ○  + e7bd1678832f stack 1
    │     - 0f12cf5c679b stack 1
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 3cafca23bb81 stack 2
    │  - 5456f1af47ed stack 2
    ○  118a6ee52daa test-username@host.example.com default@ 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  new empty commit
    │  args: jj new 'root()' -m new_base
    │
    │  Changed commits:
    │  ○  + 6b753f7043b4 new_base
    │     Modified commit description:
    │             1: new_base
    │
    │  Changed working copy default@:
    │  + 6b753f7043b4 new_base
    │  - 5456f1af47ed stack 2
    ○  85144a1c6ca9 test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  point bookmark bookmark_x to commit 5456f1af47edb52cfd73d582364cc4dd6ddb08cf
    │  args: jj bookmark set bookmark_x -r@
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 5456f1af47ed stack 2
    │  - 2308e5a241f7 base
    ○  bcc1579c81d7 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  new empty commit
    │  args: jj new @ -m 'stack 2'
    │
    │  Changed commits:
    │  ○  + 5456f1af47ed stack 2
    │     Modified commit description:
    │             1: stack 2
    │
    │  Changed working copy default@:
    │  + 5456f1af47ed stack 2
    │  - 0f12cf5c679b stack 1
    ○  7e6bd9675d9d test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  new empty commit
    │  args: jj new @ -m 'stack 1'
    │
    │  Changed commits:
    │  ○  + 0f12cf5c679b stack 1
    │     Modified commit description:
    │             1: stack 1
    │
    │  Changed working copy default@:
    │  + 0f12cf5c679b stack 1
    │  - 2308e5a241f7 base
    ○  a6739a805d51 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  create bookmark bookmark_x pointing to commit 2308e5a241f7a47f186b0686ffb17aa613a727d7
    │  args: jj bookmark create -r@ bookmark_x
    │
    │  Warning: Could not resolve revset expression for elision: Revision `bookmark_x` doesn't exist
    │     (Use --show-changes-in=all() to see all changes)
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 2308e5a241f7 base
    │  - (absent)
    [EOF]
    ");

    // 11. Test op log -p with the flag: should show all changes and NO WARNING.
    insta::assert_snapshot!(work_dir.run_jj([
        "op",
        "log",
        "-p",
        "--limit",
        "6",
        "--show-changes-in",
        "all()",
    ]), @"
    @  75842f60dc81 test-username@host.example.com default@ 2001-02-03 04:05:14.000 +07:00 - 2001-02-03 04:05:14.000 +07:00
    │  rebase commit 0f12cf5c679b373cb1ee0fa3e441c2f5030c4dc9 and descendants
    │  args: jj rebase -s bookmark_x- -d @
    │
    │  Changed commits:
    │  ○  + 3cafca23bb81 stack 2
    │  │  - 5456f1af47ed stack 2
    │  ○  + e7bd1678832f stack 1
    │     - 0f12cf5c679b stack 1
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 3cafca23bb81 stack 2
    │  - 5456f1af47ed stack 2
    ○  118a6ee52daa test-username@host.example.com default@ 2001-02-03 04:05:13.000 +07:00 - 2001-02-03 04:05:13.000 +07:00
    │  new empty commit
    │  args: jj new 'root()' -m new_base
    │
    │  Changed commits:
    │  ○  + 6b753f7043b4 new_base
    │     Modified commit description:
    │             1: new_base
    │
    │  Changed working copy default@:
    │  + 6b753f7043b4 new_base
    │  - 5456f1af47ed stack 2
    ○  85144a1c6ca9 test-username@host.example.com default@ 2001-02-03 04:05:12.000 +07:00 - 2001-02-03 04:05:12.000 +07:00
    │  point bookmark bookmark_x to commit 5456f1af47edb52cfd73d582364cc4dd6ddb08cf
    │  args: jj bookmark set bookmark_x -r@
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 5456f1af47ed stack 2
    │  - 2308e5a241f7 base
    ○  bcc1579c81d7 test-username@host.example.com default@ 2001-02-03 04:05:11.000 +07:00 - 2001-02-03 04:05:11.000 +07:00
    │  new empty commit
    │  args: jj new @ -m 'stack 2'
    │
    │  Changed commits:
    │  ○  + 5456f1af47ed stack 2
    │     Modified commit description:
    │             1: stack 2
    │
    │  Changed working copy default@:
    │  + 5456f1af47ed stack 2
    │  - 0f12cf5c679b stack 1
    ○  7e6bd9675d9d test-username@host.example.com default@ 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  new empty commit
    │  args: jj new @ -m 'stack 1'
    │
    │  Changed commits:
    │  ○  + 0f12cf5c679b stack 1
    │     Modified commit description:
    │             1: stack 1
    │
    │  Changed working copy default@:
    │  + 0f12cf5c679b stack 1
    │  - 2308e5a241f7 base
    ○  a6739a805d51 test-username@host.example.com default@ 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  create bookmark bookmark_x pointing to commit 2308e5a241f7a47f186b0686ffb17aa613a727d7
    │  args: jj bookmark create -r@ bookmark_x
    │
    │  Changed local bookmarks:
    │  bookmark_x:
    │  + 2308e5a241f7 base
    │  - (absent)
    [EOF]
    ");
}

#[test]
fn test_op_diff_invalid_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Invalid flag value
    insta::assert_snapshot!(work_dir.run_jj(["op", "diff", "--show-changes-in", "invalid("]), @"
    ------- stderr -------
    Error: Invalid `--show-changes-in` expression: invalid(
    Caused by:  --> 1:9
      |
    1 | invalid(
      |         ^---
      |
      = expected <strict_identifier> or <expression>
    [EOF]
    [exit status: 1]
    ");

    // Invalid config value
    test_env.add_config(r#"revsets.op-diff-changes-in = "invalid(""#);
    insta::assert_snapshot!(work_dir.run_jj(["op", "diff"]), @"
    ------- stderr -------
    Config error: Invalid `revsets.op-diff-changes-in`
    Caused by:  --> 1:9
      |
    1 | invalid(
      |         ^---
      |
      = expected <strict_identifier> or <expression>
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
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

    git::add_commit(
        &git_repo,
        "refs/tags/tag-1",
        "some-file",
        b"some tagged content",
        "Commit 4",
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
