// Copyright 2023 The Jujutsu Authors
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

use insta::assert_snapshot;

use crate::common::TestEnvironment;

#[test]
fn test_clear_predecessors() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "version 2"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "version 3"]);

    let stdout = test_env.jj_cmd_success(&repo_path, &["evolog"]);
    insta::assert_snapshot!(stdout, @r#"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:09 ba508502
    │  (empty) version 3
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 24c5d33a
    │  (empty) version 2
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    "#);

    let (stdout, stderr) =
        test_env.jj_cmd_ok(&repo_path, &["util", "clear-predecessors", "-r", "@"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r#"
    Cleared predecessors for 1 commits.
    Working copy now at: qpvuntsm 1c89a3cb (empty) version 3
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    "#);

    let stdout = test_env.jj_cmd_success(&repo_path, &["evolog"]);
    insta::assert_snapshot!(stdout, @r#"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:11 1c89a3cb
       (empty) version 3
    "#);
}

#[test]
fn test_util_config_schema() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "config-schema"]);
    // Validate partial snapshot, redacting any lines nested 2+ indent levels.
    insta::with_settings!({filters => vec![(r"(?m)(^        .*$\r?\n)+", "        [...]\n")]}, {
        assert_snapshot!(output, @r#"
        {
            "$schema": "http://json-schema.org/draft-04/schema",
            "$comment": "`taplo` and the corresponding VS Code plugins only support version draft-04 of JSON Schema, see <https://taplo.tamasfe.dev/configuration/developing-schemas.html>. draft-07 is mostly compatible with it, newer versions may not be.",
            "title": "Jujutsu config",
            "type": "object",
            "description": "User configuration for Jujutsu VCS. See https://jj-vcs.github.io/jj/latest/config/ for details",
            "properties": {
                [...]
            }
        }
        [EOF]
        "#);
    });
}

#[test]
fn test_gc_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["util", "gc"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["util", "gc", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot garbage collect from a non-head operation
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["util", "gc", "--expire=foobar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --expire only accepts 'now'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gc_operation_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create an operation.
    work_dir.write_file("file", "a change\n");
    work_dir.run_jj(["commit", "-m", "a change"]).success();
    let op_to_remove = work_dir.current_operation_id();

    // Make another operation the head.
    work_dir.write_file("file", "another change\n");
    work_dir
        .run_jj(["commit", "-m", "another change"])
        .success();

    // This works before the operation is removed.
    work_dir
        .run_jj(["debug", "operation", &op_to_remove])
        .success();

    // Remove some operations.
    work_dir.run_jj(["operation", "abandon", "..@-"]).success();
    work_dir.run_jj(["util", "gc", "--expire=now"]).success();

    // Now this doesn't work.
    let output = work_dir.run_jj(["debug", "operation", &op_to_remove]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: No operation ID matching "31c378f62742a80562c8fe790e46895882ed618d1ac06ceeddc182b3f676fc926f07f577cc505059b5345a5111982a193bb25b0601c518b88fd5c0fdac4e229d"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_shell_completions() {
    #[track_caller]
    fn test(shell: &str) {
        let test_env = TestEnvironment::default();
        // Use the local backend because GitBackend::gc() depends on the git CLI.
        let output = test_env
            .run_jj_in(".", ["util", "completion", shell])
            .success();
        // Ensures only stdout contains text
        assert!(
            !output.stdout.is_empty() && output.stderr.is_empty(),
            "{output}"
        );
    }

    test("bash");
    test("fish");
    test("nushell");
    test("zsh");
}

#[test]
fn test_util_exec() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--append",
            "hello",
        ],
    );
    // Ensures only stdout contains text
    insta::assert_snapshot!(output, @"hello[EOF]");
}

#[test]
fn test_util_exec_fail() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "exec", "--", "jj-test-missing-program"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to execute external command 'jj-test-missing-program'
    [EOF]
    [exit status: 1]
    ");
}
