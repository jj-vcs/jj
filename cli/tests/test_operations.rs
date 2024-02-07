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

use crate::common::{get_stdout_string, TestEnvironment};

#[test]
fn test_op_log() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
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
    @  826c45dd2457 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    ◉  27143b59c690 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ◉  0e8aee02e242 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ◉  000000000000 root()
    "###);
    let op_log_lines = stdout.lines().collect_vec();
    let add_workspace_id = op_log_lines[3].split(' ').nth(2).unwrap();
    let initialize_repo_id = op_log_lines[5].split(' ').nth(2).unwrap();

    // Can load the repo at a specific operation ID
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, initialize_repo_id), @r###"
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, add_workspace_id), @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◉  0000000000000000000000000000000000000000
    "###);
    // "@" resolves to the head operation
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, "@"), @r###"
    @  bc8f18aa6f396a93572811632313cbb5625d475d
    ◉  0000000000000000000000000000000000000000
    "###);
    // "@-" resolves to the parent of the head operation
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path, "@-"), @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◉  0000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_failure(&repo_path, &["log", "--at-op", "@----"]), @r###"
    Error: The "@----" expression resolved to no operations
    "###);

    // We get a reasonable message if an invalid operation ID is specified
    insta::assert_snapshot!(test_env.jj_cmd_failure(&repo_path, &["log", "--at-op", "foo"]), @r###"
    Error: Operation ID "foo" is not a valid hexadecimal prefix
    "###);

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
    "###);
}

#[test]
fn test_op_log_limit() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log", "-Tdescription", "--limit=1"]);
    insta::assert_snapshot!(stdout, @r###"
    @  add workspace 'default'
    "###);
}

#[test]
fn test_op_log_no_graph() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let stdout =
        test_env.jj_cmd_success(&repo_path, &["op", "log", "--no-graph", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    [1m[38;5;12m27143b59c690[39m [38;5;3mtest-username@host.example.com[39m [38;5;14m2001-02-03 04:05:07.000 +07:00[39m - [38;5;14m2001-02-03 04:05:07.000 +07:00[39m[0m
    [1madd workspace 'default'[0m
    [38;5;4m0e8aee02e242[39m [38;5;3mtest-username@host.example.com[39m [38;5;6m2001-02-03 04:05:07.000 +07:00[39m - [38;5;6m2001-02-03 04:05:07.000 +07:00[39m
    initialize repo
    [38;5;4m000000000000[39m [38;5;2mroot()[39m
    "###);
}

#[test]
fn test_op_log_no_graph_null_terminated() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
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
    insta::assert_debug_snapshot!(stdout, @r###""f5e4\05ff2\02714\00e8a\00000\0""###);
}

#[test]
fn test_op_log_template() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    let render = |template| test_env.jj_cmd_success(&repo_path, &["op", "log", "-T", template]);

    insta::assert_snapshot!(render(r#"id ++ "\n""#), @r###"
    @  27143b59c6904046f6be83ad6fe145d819944f9abbd7247ea9c57848d1d2c678ea8265598a156fe8aeef31d24d958bf6cfa0c2eb3afef40bdae2c5e98d73d0ee
    ◉  0e8aee02e24230c99d6d90d469c582a60fdb2ae8329341bbdb09f4a0beceba1ce7c84fc9ba6c7657d6d275b392b89b825502475ad2501be1ddebd4a09b07668c
    ◉  00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
    "###);
    insta::assert_snapshot!(
        render(r#"separate(" ", id.short(5), current_operation, user,
                                time.start(), time.end(), time.duration()) ++ "\n""#), @r###"
    @  27143 true test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ◉  0e8ae false test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 2001-02-03 04:05:07.000 +07:00 less than a microsecond
    ◉  00000 false @ 1970-01-01 00:00:00.000 +00:00 1970-01-01 00:00:00.000 +00:00 less than a microsecond
    "###);

    // Negative length shouldn't cause panic (and is clamped.)
    // TODO: If we add runtime error, this will probably error out.
    insta::assert_snapshot!(render(r#"id.short(-1) ++ "|""#), @r###"
    @  |
    ◉  |
    ◉  |
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
    @  27143b59c690 test-username@host.example.com NN years ago, lasted less than a microsecond
    │  add workspace 'default'
    ◉  0e8aee02e242 test-username@host.example.com NN years ago, lasted less than a microsecond
    │  initialize repo
    ◉  000000000000 root()
    "###);
}

#[test]
fn test_op_log_builtin_templates() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    let render = |template| test_env.jj_cmd_success(&repo_path, &["op", "log", "-T", template]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "description 0"]);

    insta::assert_snapshot!(render(r#"builtin_op_log_compact"#), @r###"
    @  826c45dd2457 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    ◉  27143b59c690 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ◉  0e8aee02e242 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ◉  000000000000 root()
    "###);

    insta::assert_snapshot!(render(r#"builtin_op_log_comfortable"#), @r###"
    @  826c45dd2457 test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj describe -m 'description 0'
    │
    ◉  27143b59c690 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    │
    ◉  0e8aee02e242 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    │
    ◉  000000000000 root()
    "###);
}

#[test]
fn test_op_log_word_wrap() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
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

    // ui.log-word-wrap option works
    insta::assert_snapshot!(render(&["op", "log"], 40, false), @r###"
    @  27143b59c690 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ◉  0e8aee02e242 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ◉  000000000000 root()
    "###);
    insta::assert_snapshot!(render(&["op", "log"], 40, true), @r###"
    @  27143b59c690
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ◉  0e8aee02e242
    │  test-username@host.example.com
    │  2001-02-03 04:05:07.000 +07:00 -
    │  2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ◉  000000000000 root()
    "###);
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
        .jj_cmd(test_env.env_root(), &["init", "repo", "--git"])
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
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 1"]);
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "commit 2"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  85137561ef60 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit a8ac27b29a157ae7dabc0deb524df68823505730
    │  args: jj commit -m 'commit 2'
    ◉  db27d55e457f test-username@host.example.com 2001-02-03 04:05:08.000 +07:00 - 2001-02-03 04:05:08.000 +07:00
    │  commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  args: jj commit -m 'commit 1'
    ◉  27143b59c690 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  add workspace 'default'
    ◉  0e8aee02e242 test-username@host.example.com 2001-02-03 04:05:07.000 +07:00 - 2001-02-03 04:05:07.000 +07:00
    │  initialize repo
    ◉  000000000000 root()
    "###);

    // Abandon old operations. The working-copy operation id should be updated.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "..@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 3 operations and reparented 1 descendant operations.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "workingcopy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("1c88fada5b95d13ca136baa13c4b4aae1b79f3d453fe2f56539dbcd5d779642439314a15f685d6737eb414fffb3e53519f65f1d5cc5947a7821331137f4a91e2")
    Current tree: Legacy(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904"))
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  1c88fada5b95 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit a8ac27b29a157ae7dabc0deb524df68823505730
    │  args: jj commit -m 'commit 2'
    ◉  000000000000 root()
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
    @  459e01910446 test-username@host.example.com 2001-02-03 04:05:16.000 +07:00 - 2001-02-03 04:05:16.000 +07:00
    │  commit e184d62c9ab118b0f62de91959b857550a9273a5
    │  args: jj commit -m 'commit 5'
    ◉  1c88fada5b95 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit a8ac27b29a157ae7dabc0deb524df68823505730
    │  args: jj commit -m 'commit 2'
    ◉  000000000000 root()
    "###);

    // Can't abandon the current operation.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", "..@"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Cannot abandon the current operation
    Hint: Run `jj undo` to revert the current operation, then use `jj op abandon`
    "###);

    // Can't create concurrent abandoned operations explicitly.
    let stderr = test_env.jj_cmd_failure(&repo_path, &["op", "abandon", "--at-op=@-", "@"]);
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
        test_env.jj_cmd_success(&repo_path, &["debug", "workingcopy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("dbe91445f72184c82b2ad4c13063a20a764dd5a3bab42fc22652a0ca5d9f5a811d2b051560ca8b45c31f29e9d17ea408b224f761165b30f6df5f68ca74c90435")
    Current tree: Legacy(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904"))
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log"]), @r###"
    @  dbe91445f721 test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation 459e01910446fb8f6f6447112abc2227cb3b054b7831eca45bd527145719152931b1707d299f5e4e8484352e67d3622265a0baa58bfa01c459fd753f58791e0d
    │  args: jj undo
    ◉  1c88fada5b95 test-username@host.example.com 2001-02-03 04:05:09.000 +07:00 - 2001-02-03 04:05:09.000 +07:00
    │  commit a8ac27b29a157ae7dabc0deb524df68823505730
    │  args: jj commit -m 'commit 2'
    ◉  000000000000 root()
    "###);

    // Abandon empty range.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@-..@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["op", "log", "-l1"]), @r###"
    @  dbe91445f721 test-username@host.example.com 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    │  undo operation 459e01910446fb8f6f6447112abc2227cb3b054b7831eca45bd527145719152931b1707d299f5e4e8484352e67d3622265a0baa58bfa01c459fd753f58791e0d
    │  args: jj undo
    "###);
}

#[test]
fn test_op_abandon_without_updating_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["init", "repo", "--git"]);
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
        test_env.jj_cmd_success(&repo_path, &["debug", "workingcopy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("0229bff5a5244b0804cc677c77e318887b6d00422257de108158ef41fa5edcff38aa1683a3c00364835e0565b841eb41c5092300ec23a1aa5e393914d18fef32")
    Current tree: Legacy(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904"))
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["op", "log", "-l1", "--ignore-working-copy"]), @r###"
    @  0173d6fbe6a2 test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 268f5f16139313ff25bef31280b2ec2e675200f3
    │  args: jj commit -m 'commit 3'
    "###);

    // The working-copy operation id isn't updated if it differs from the repo.
    // It could be updated if the tree matches, but there's no extra logic for
    // that.
    let (_stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "abandon", "@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Abandoned 1 operations and reparented 1 descendant operations.
    The working copy operation 0229bff5a524 is not updated because it differs from the repo 0173d6fbe6a2.
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["debug", "workingcopy", "--ignore-working-copy"]), @r###"
    Current operation: OperationId("0229bff5a5244b0804cc677c77e318887b6d00422257de108158ef41fa5edcff38aa1683a3c00364835e0565b841eb41c5092300ec23a1aa5e393914d18fef32")
    Current tree: Legacy(TreeId("4b825dc642cb6eb9a060e54bf8d69288fbee4904"))
    "###);
    insta::assert_snapshot!(
        test_env.jj_cmd_success(&repo_path, &["op", "log", "-l1", "--ignore-working-copy"]), @r###"
    @  b66dc21d579b test-username@host.example.com 2001-02-03 04:05:10.000 +07:00 - 2001-02-03 04:05:10.000 +07:00
    │  commit 268f5f16139313ff25bef31280b2ec2e675200f3
    │  args: jj commit -m 'commit 3'
    "###);
}

fn get_log_output(test_env: &TestEnvironment, repo_path: &Path, op_id: &str) -> String {
    test_env.jj_cmd_success(
        repo_path,
        &["log", "-T", "commit_id", "--at-op", op_id, "-r", "all()"],
    )
}
