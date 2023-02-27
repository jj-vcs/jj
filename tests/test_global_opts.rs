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

use std::ffi::OsString;

use itertools::Itertools;

use crate::common::{get_stderr_string, TestEnvironment};

pub mod common;

#[test]
fn test_non_utf8_arg() {
    let test_env = TestEnvironment::default();
    #[cfg(unix)]
    let invalid_utf = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f])
    };
    #[cfg(windows)]
    let invalid_utf = {
        use std::os::windows::prelude::*;
        OsString::from_wide(&[0x0066, 0x006f, 0xD800, 0x006f])
    };
    let assert = test_env
        .jj_cmd(test_env.env_root(), &[])
        .args(&[invalid_utf])
        .assert()
        .code(2);
    let stderr = get_stderr_string(&assert);
    insta::assert_snapshot!(stderr, @r###"
    Error: Non-utf8 argument
    "###);
}

#[test]
fn test_no_subcommand() {
    let test_env = TestEnvironment::default();

    let stderr = test_env.jj_cmd_cli_error(test_env.env_root(), &[]);
    insta::assert_snapshot!(stderr.lines().next().unwrap(), @"Jujutsu (An experimental VCS)");

    let stderr = test_env.jj_cmd_cli_error(test_env.env_root(), &["-R."]);
    insta::assert_snapshot!(stderr.lines().next().unwrap(), @"error: 'jj' requires a subcommand but one was not provided");

    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["--version"]);
    insta::assert_snapshot!(stdout.replace(|c: char| c.is_ascii_digit(), "?"), @r###"
    jj ?.?.?
    "###);

    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["--help"]);
    insta::assert_snapshot!(stdout.lines().next().unwrap(), @"Jujutsu (An experimental VCS)");
}

#[test]
fn test_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);

    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "initial").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  438471f3fbf1004298d8fb01eeb13663a051a643
    o  0000000000000000000000000000000000000000
    "###);

    // Modify the file. With --ignore-working-copy, we still get the same commit
    // ID.
    std::fs::write(repo_path.join("file"), "modified").unwrap();
    let stdout_again = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", "commit_id", "--ignore-working-copy"],
    );
    assert_eq!(stdout_again, stdout);

    // But without --ignore-working-copy, we get a new commit ID.
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  fab22d1acf5bb9c5aa48cb2c3dd2132072a359ca
    o  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_repo_arg_with_init() {
    let test_env = TestEnvironment::default();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["init", "-R=.", "repo"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: '--repository' cannot be used with 'init'
    "###);
}

#[test]
fn test_repo_arg_with_git_clone() {
    let test_env = TestEnvironment::default();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "-R=.", "remote"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: '--repository' cannot be used with 'git clone'
    "###);
}

#[test]
fn test_resolve_workspace_directory() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    let subdir = repo_path.join("dir").join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();

    // Ancestor of cwd
    let stdout = test_env.jj_cmd_success(&subdir, &["status"]);
    insta::assert_snapshot!(stdout, @r###"
    Parent commit: 000000000000 (no description set)
    Working copy : 230dd059e1b0 (no description set)
    The working copy is clean
    "###);

    // Explicit subdirectory path
    let stderr = test_env.jj_cmd_failure(&subdir, &["status", "-R", "."]);
    insta::assert_snapshot!(stderr, @r###"
    Error: There is no jj repo in "."
    "###);

    // Valid explicit path
    let stdout = test_env.jj_cmd_success(&subdir, &["status", "-R", "../.."]);
    insta::assert_snapshot!(stdout, @r###"
    Parent commit: 000000000000 (no description set)
    Working copy : 230dd059e1b0 (no description set)
    The working copy is clean
    "###);

    // "../../..".ancestors() contains "../..", but it should never be looked up.
    let stderr = test_env.jj_cmd_failure(&subdir, &["status", "-R", "../../.."]);
    insta::assert_snapshot!(stderr, @r###"
    Error: There is no jj repo in "../../.."
    "###);
}

#[test]
fn test_no_workspace_directory() {
    let test_env = TestEnvironment::default();
    let repo_path = test_env.env_root().join("repo");
    std::fs::create_dir(&repo_path).unwrap();

    let stderr = test_env.jj_cmd_failure(&repo_path, &["status"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: There is no jj repo in "."
    "###);

    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["status", "-R", "repo"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: There is no jj repo in "repo"
    "###);

    std::fs::create_dir(repo_path.join(".git")).unwrap();
    let stderr = test_env.jj_cmd_failure(&repo_path, &["status"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: There is no jj repo in "."
    Hint: It looks like this is a git repo. You can create a jj repo backed by it by running this:
    jj init --git-repo=.
    "###);
}

#[test]
fn test_broken_repo_structure() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let store_type_path = repo_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("type");

    // Test the error message when the commit backend is of unknown type.
    std::fs::write(&store_type_path, "unknown").unwrap();
    let stderr = test_env.jj_cmd_internal_error(&repo_path, &["log"]);
    insta::assert_snapshot!(stderr, @r###"
    Internal error: This version of the jj binary doesn't support this type of repo: Unsupported commit backend type 'unknown'
    "###);

    // Test the error message when the file indicating the commit backend type
    // cannot be read.
    std::fs::remove_file(&store_type_path).unwrap();
    std::fs::create_dir(&store_type_path).unwrap();
    let stderr = test_env.jj_cmd_internal_error(&repo_path, &["log"]);
    // Trim off the OS-specific error message.
    let stderr = stderr.split(':').take(3).join(":");
    insta::assert_snapshot!(stderr, @"Internal error: The repository appears broken or inaccessible: Failed to read commit backend type");
}

#[test]
fn test_color_config() {
    let mut test_env = TestEnvironment::default();

    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    // Test that --color=always is respected.
    let stdout = test_env.jj_cmd_success(&repo_path, &["--color=always", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [38;5;4m230dd059e1b059aefc0da06a2e5a7dbf22362f22[39m
    o  [38;5;4m0000000000000000000000000000000000000000[39m
    "###);

    // Test that color is used if it's requested in the config file
    test_env.add_config(r#"ui.color="always""#);
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [38;5;4m230dd059e1b059aefc0da06a2e5a7dbf22362f22[39m
    o  [38;5;4m0000000000000000000000000000000000000000[39m
    "###);

    // Test that --color=never overrides the config.
    let stdout = test_env.jj_cmd_success(&repo_path, &["--color=never", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    o  0000000000000000000000000000000000000000
    "###);

    // Test that --color=auto overrides the config.
    let stdout = test_env.jj_cmd_success(&repo_path, &["--color=auto", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    o  0000000000000000000000000000000000000000
    "###);

    // Test that --config-toml 'ui.color="never"' overrides the config.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "--config-toml",
            "ui.color=\"never\"",
            "log",
            "-T",
            "commit_id",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    o  0000000000000000000000000000000000000000
    "###);

    // --color overrides --config-toml 'ui.color=...'.
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "--color",
            "never",
            "--config-toml",
            "ui.color=\"always\"",
            "log",
            "-T",
            "commit_id",
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    o  0000000000000000000000000000000000000000
    "###);

    // Test that NO_COLOR does NOT override the request for color in the config file
    test_env.add_env_var("NO_COLOR", "");
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [38;5;4m230dd059e1b059aefc0da06a2e5a7dbf22362f22[39m
    o  [38;5;4m0000000000000000000000000000000000000000[39m
    "###);

    // Test that per-repo config overrides the user config.
    std::fs::write(
        repo_path.join(".jj/repo/config.toml"),
        r#"ui.color = "never""#,
    )
    .unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id"]);
    insta::assert_snapshot!(stdout, @r###"
    @  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    o  0000000000000000000000000000000000000000
    "###);
}

#[test]
fn test_early_args() {
    // Test that help output parses early args
    let test_env = TestEnvironment::default();

    // The default is no color.
    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["help"]);
    insta::assert_snapshot!(stdout.lines().next().unwrap(), @"Jujutsu (An experimental VCS)");

    // Check that output is colorized.
    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["--color=always", "help"]);
    insta::assert_snapshot!(stdout.lines().next().unwrap(), @"[0mJujutsu (An experimental VCS)");

    // Early args are parsed with clap's ignore_errors(), but there is a known
    // bug that causes defaults to be unpopulated. Test that the early args are
    // tolerant of this bug and don't cause a crash.
    test_env.jj_cmd_success(test_env.env_root(), &["--no-pager", "help"]);
    test_env.jj_cmd_success(
        test_env.env_root(),
        &["--config-toml", "ui.color = 'always'", "help"],
    );
}

#[test]
fn test_invalid_config() {
    // Test that we get a reasonable error if the config is invalid (#55)
    let test_env = TestEnvironment::default();

    test_env.add_config("[section]key = value-missing-quotes");
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["init", "repo"]);
    insta::assert_snapshot!(stderr.replace('\\', "/"), @r###"
    Config error: expected newline, found an identifier at line 1 column 10 in config/config0001.toml
    "###);
}

#[test]
fn test_no_user_configured() {
    // Test that the user is reminded if they haven't configured their name or email
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    let assert = test_env
        .jj_cmd(&repo_path, &["describe", "-m", "without name"])
        .env_remove("JJ_USER")
        .assert()
        .success();
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Name and email not configured. Add something like the following to $HOME/.jjconfig.toml:
      user.name = "Some One"
      user.email = "someone@example.com"
    "###);
    let assert = test_env
        .jj_cmd(&repo_path, &["describe", "-m", "without email"])
        .env_remove("JJ_EMAIL")
        .assert()
        .success();
    insta::assert_snapshot!(get_stderr_string(&assert), @r###"
    Name and email not configured. Add something like the following to $HOME/.jjconfig.toml:
      user.name = "Some One"
      user.email = "someone@example.com"
    "###);
}

#[test]
fn test_help() {
    // Test that global options are separated out in the help output
    let test_env = TestEnvironment::default();

    let stdout = test_env.jj_cmd_success(test_env.env_root(), &["diffedit", "-h"]);
    insta::assert_snapshot!(stdout, @r###"
    Touch up the content changes in a revision with a diff editor

    Usage: jj diffedit [OPTIONS]

    Options:
      -r, --revision <REVISION>  The revision to touch up. Defaults to @ if --to/--from are not specified
          --from <FROM>          Show changes from this revision. Defaults to @ if --to is specified
          --to <TO>              Edit changes in this revision. Defaults to @ if --from is specified
      -h, --help                 Print help information (use `--help` for more detail)

    Global Options:
      -R, --repository <REPOSITORY>      Path to repository to operate on
          --ignore-working-copy          Don't snapshot the working copy, and don't update it
          --at-operation <AT_OPERATION>  Operation to load the repo at [default: @] [aliases: at-op]
      -v, --verbose                      Enable verbose logging
          --color <WHEN>                 When to colorize output (always, never, auto)
          --no-pager                     Disable the pager
          --config-toml <TOML>           Additional configuration options
    "###);
}

#[test]
fn test_verbose_logging_enabled() {
    // Test that the verbose flag enabled verbose logging
    let test_env = TestEnvironment::default();

    let assert = test_env
        .jj_cmd(test_env.env_root(), &["version", "-v"])
        .assert()
        .success();

    let stderr = get_stderr_string(&assert);
    // Split the first log line into a timestamp and the rest.
    // The timestamp is constant sized so this is a robust operation.
    // Example timestamp: 2022-11-20T06:24:05.477703Z
    let (_timestamp, log_line) = stderr
        .lines()
        .next()
        .expect("verbose logging on first line")
        .split_at(36);
    // The log format is currently Pretty so we include the terminal markup.
    // Luckily, insta will print this in colour when reviewing.
    insta::assert_snapshot!(log_line, @"[34mDEBUG[0m [2mjujutsu::cli_util[0m[2m:[0m verbose logging enabled");
}
