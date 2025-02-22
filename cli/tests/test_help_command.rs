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

use crate::common::TestEnvironment;

#[test]
fn test_help() {
    let test_env = TestEnvironment::default();

    let help_cmd = test_env.run_jj_in(test_env.env_root(), ["help"]).success();
    // The help command output should be equal to the long --help flag
    let help_flag = test_env
        .run_jj_in(test_env.env_root(), ["--help"])
        .success();
    assert_eq!(help_cmd.stdout.raw(), help_flag.stdout.raw());

    // Help command should work with commands
    let help_cmd = test_env
        .run_jj_in(test_env.env_root(), ["help", "log"])
        .success();
    let help_flag = test_env
        .run_jj_in(test_env.env_root(), ["log", "--help"])
        .success();
    assert_eq!(help_cmd.stdout.raw(), help_flag.stdout.raw());

    // Help command should work with subcommands
    let help_cmd = test_env
        .run_jj_in(test_env.env_root(), ["help", "workspace", "root"])
        .success();
    let help_flag = test_env
        .run_jj_in(test_env.env_root(), ["workspace", "root", "--help"])
        .success();
    assert_eq!(help_cmd.stdout.raw(), help_flag.stdout.raw());

    // Help command should not work recursively
    let output = test_env.run_jj_in(test_env.env_root(), ["workspace", "help", "root"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unrecognized subcommand 'help'

    Usage: jj workspace [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(test_env.env_root(), ["workspace", "add", "help"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    let output = test_env.run_jj_in(test_env.env_root(), ["new", "help", "main"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    // Help command should output the same as --help for nonexistent commands
    let help_cmd = test_env.run_jj_in(test_env.env_root(), ["help", "nonexistent"]);
    let help_flag = test_env.run_jj_in(test_env.env_root(), ["nonexistent", "--help"]);
    assert_eq!(help_cmd.status.code(), Some(2), "{help_cmd}");
    assert_eq!(help_flag.status.code(), Some(2), "{help_flag}");
    assert_eq!(help_cmd.stderr.raw(), help_flag.stderr.raw());

    // Some edge cases
    let help_cmd = test_env
        .run_jj_in(test_env.env_root(), ["help", "help"])
        .success();
    let help_flag = test_env
        .run_jj_in(test_env.env_root(), ["help", "--help"])
        .success();
    assert_eq!(help_cmd.stdout.raw(), help_flag.stdout.raw());

    let output = test_env.run_jj_in(test_env.env_root(), ["help", "unknown"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unrecognized subcommand 'unknown'

      tip: a similar subcommand exists: 'undo'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    let output = test_env.run_jj_in(test_env.env_root(), ["help", "log", "--", "-r"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: a value is required for '--revisions <REVSETS>' but none was supplied

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_help_keyword() {
    let test_env = TestEnvironment::default();

    // It should show help for a certain keyword if the `--keyword` flag is present
    let help_cmd = test_env
        .run_jj_in(test_env.env_root(), ["help", "--keyword", "revsets"])
        .success();
    // It should be equal to the docs
    assert_eq!(help_cmd.stdout.raw(), include_str!("../../docs/revsets.md"));

    // It should show help for a certain keyword if the `-k` flag is present
    let help_cmd = test_env
        .run_jj_in(test_env.env_root(), ["help", "-k", "revsets"])
        .success();
    // It should be equal to the docs
    assert_eq!(help_cmd.stdout.raw(), include_str!("../../docs/revsets.md"));

    // It should give hints if a similar keyword is present
    let output = test_env.run_jj_in(test_env.env_root(), ["help", "-k", "rev"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'rev' for '--keyword <KEYWORD>'
      [possible values: bookmarks, config, filesets, glossary, revsets, templates, tutorial]

      tip: a similar value exists: 'revsets'

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // It should give error with a hint if no similar keyword is found
    let output = test_env.run_jj_in(test_env.env_root(), ["help", "-k", "<no-similar-keyword>"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value '<no-similar-keyword>' for '--keyword <KEYWORD>'
      [possible values: bookmarks, config, filesets, glossary, revsets, templates, tutorial]

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // The keyword flag with no argument should error with a hint
    let output = test_env.run_jj_in(test_env.env_root(), ["help", "-k"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: a value is required for '--keyword <KEYWORD>' but none was supplied
      [possible values: bookmarks, config, filesets, glossary, revsets, templates, tutorial]

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // It shouldn't show help for a certain keyword if the `--keyword` is not
    // present
    let output = test_env.run_jj_in(test_env.env_root(), ["help", "revsets"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: unrecognized subcommand 'revsets'

      tip: some similar subcommands exist: 'resolve', 'prev', 'restore', 'rebase', 'revert'

    Usage: jj [OPTIONS] <COMMAND>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}
