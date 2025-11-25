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

use indoc::indoc;
use itertools::Itertools as _;
use regex::Regex;

use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_non_utf8_arg() {
    let test_env = TestEnvironment::default();
    #[cfg(unix)]
    let invalid_utf = {
        use std::os::unix::ffi::OsStringExt as _;
        OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f])
    };
    #[cfg(windows)]
    let invalid_utf = {
        use std::os::windows::prelude::*;
        OsString::from_wide(&[0x0066, 0x006f, 0xD800, 0x006f])
    };
    let output = test_env.run_jj_in(".", [&invalid_utf]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Non-UTF-8 argument
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_version() {
    let test_env = TestEnvironment::default();

    let output = test_env.run_jj_in(".", ["--version"]).success();
    let stdout = output.stdout.into_raw();
    let sanitized = stdout.replace(|c: char| c.is_ascii_hexdigit(), "?");
    let expected = [
        "jj ?.??.?\n",
        "jj ?.??.?-????????????????????????????????????????\n",
        // This test could be made to succeed when `jj` is compiled at a merge commit by adding a
        // few entries like "jj ?.??.?-????????????????????????????????????????-?????????????????
        // ???????????????????????\n" here. However, it might be best to keep it failing, so that
        // we avoid releasing `jj` with such `--version` output.
    ];
    assert!(
        expected.contains(&sanitized.as_str()),
        "`jj version` output: {stdout:?}.\nSanitized: {sanitized:?}\nExpected one of: {expected:?}"
    );
}

#[test]
fn test_no_subcommand() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Outside of a repo.
    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Hint: Use `jj -h` for a list of available commands.
    Run `jj config set --user ui.default-command log` to disable this message.
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    test_env.add_config(r#"ui.default-command="log""#);
    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    let output = test_env.run_jj_in(".", ["--help"]).success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().next().unwrap(),
        @"Jujutsu (An experimental VCS)");
    insta::assert_snapshot!(output.stderr, @"");

    let output = test_env.run_jj_in(".", ["-R", "repo"]).success();
    assert_eq!(output, work_dir.run_jj(["log"]));

    // Inside of a repo.
    let output = work_dir.run_jj([""; 0]).success();
    assert_eq!(output, work_dir.run_jj(["log"]));

    // Command argument that looks like a command name.
    work_dir
        .run_jj(["bookmark", "create", "-r@", "help"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "log"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "show"])
        .success();
    // TODO: test_env.run_jj(["-r", "help"]).success()
    insta::assert_snapshot!(work_dir.run_jj(["-r", "log"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 help log show e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["-r", "show"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 help log show e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");

    // Multiple default command strings work.
    test_env.add_config(r#"ui.default-command=["commit", "-m", "foo"]"#);
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file.txt", "file");
    let output = work_dir.run_jj([""; 0]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kxryzmor 8db1ba9a (empty) (no description set)
    Parent commit (@-)      : lylxulpl 19f3adb2 foo
    [EOF]
    ");
}

#[test]
fn test_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "initial");
    let output = work_dir.run_jj(["log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  82a10a4d9ef783fd68b661f40ce10dd80d599d9e
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Modify the file. With --ignore-working-copy, we still get the same commit
    // ID.
    work_dir.write_file("file", "modified");
    let output_again = work_dir.run_jj(["log", "-T", "commit_id", "--ignore-working-copy"]);
    assert_eq!(output_again, output);

    // But without --ignore-working-copy, we get a new commit ID.
    let output = work_dir.run_jj(["log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  00fc09f48ccf5c8b025a0f93b0ec3b0e4294a598
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_repo_arg_with_git_init() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "-R=.", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_repo_arg_with_git_clone() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "clone", "-R=.", "remote"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_resolve_workspace_directory() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let sub_dir = work_dir.create_dir_all("dir/subdir");

    // Ancestor of cwd
    let output = sub_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Explicit subdirectory path
    let output = sub_dir.run_jj(["status", "-R", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    // Valid explicit path
    let output = sub_dir.run_jj(["status", "-R", "../.."]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // "../../..".ancestors() contains "../..", but it should never be looked up.
    let output = sub_dir.run_jj(["status", "-R", "../../.."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "../../.."
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_no_workspace_directory() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    work_dir.create_dir_all("");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);

    let output = test_env.run_jj_in(".", ["status", "-R", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "repo"
    [EOF]
    [exit status: 1]
    "#);

    work_dir.create_dir_all(".git");
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    Hint: It looks like this is a git repo. You can create a jj repo backed by it by running this:
    jj git init
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_bad_path() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let sub_dir = work_dir.create_dir_all("dir");

    // cwd == workspace_root
    let output = work_dir.run_jj(["file", "show", "../out"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse fileset: Invalid file pattern
    Caused by:
    1:  --> 1:1
      |
    1 | ../out
      | ^----^
      |
      = Invalid file pattern
    2: Path "../out" is not in the repo "."
    3: Invalid component ".." in repo-relative path "../out"
    [EOF]
    [exit status: 1]
    "#);

    // cwd != workspace_root, can't be parsed as repo-relative path
    let output = sub_dir.run_jj(["file", "show", "../.."]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse fileset: Invalid file pattern
    Caused by:
    1:  --> 1:1
      |
    1 | ../..
      | ^---^
      |
      = Invalid file pattern
    2: Path "../.." is not in the repo "../"
    3: Invalid component ".." in repo-relative path "../"
    [EOF]
    [exit status: 1]
    "#);

    // cwd != workspace_root, can be parsed as repo-relative path
    let output = test_env.run_jj_in(".", ["file", "show", "-Rrepo", "out"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse fileset: Invalid file pattern
    Caused by:
    1:  --> 1:1
      |
    1 | out
      | ^-^
      |
      = Invalid file pattern
    2: Path "out" is not in the repo "repo"
    3: Invalid component ".." in repo-relative path "../out"
    Hint: Consider using root:"out" to specify repo-relative path
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_invalid_filesets_looking_like_filepaths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["file", "show", "abc~"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse fileset: Syntax error
    Caused by:  --> 1:5
      |
    1 | abc~
      |     ^---
      |
      = expected `~` or <primary>
    Hint: See https://docs.jj-vcs.dev/latest/filesets/ or use `jj help -k filesets` for filesets syntax and how to match file paths.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_broken_repo_structure() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let store_path = work_dir.root().join(".jj").join("repo").join("store");
    let store_type_path = store_path.join("type");

    // Test the error message when the git repository can't be located.
    work_dir.remove_file(store_path.join("git_target"));
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Internal error: The repository appears broken or inaccessible
    Caused by:
    1: Cannot access $TEST_ENV/repo/.jj/repo/store/git_target
    [EOF]
    [exit status: 255]
    ");

    // Test the error message when the commit backend is of unknown type.
    work_dir.write_file(&store_type_path, "unknown");
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: This version of the jj binary doesn't support this type of repo
    Caused by: Unsupported commit backend type 'unknown'
    [EOF]
    [exit status: 255]
    ");

    // Test the error message when the file indicating the commit backend type
    // cannot be read.
    work_dir.remove_file(&store_type_path);
    work_dir.create_dir(&store_type_path);
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Internal error: The repository appears broken or inaccessible
    Caused by:
    1: Failed to read commit backend type
    2: Cannot access $TEST_ENV/repo/.jj/repo/store/type
    [EOF]
    [exit status: 255]
    ");

    // Test when the .jj directory is empty. The error message is identical to
    // the previous one, but writing the default type file would also fail.
    work_dir.remove_dir_all(".jj");
    work_dir.create_dir(".jj");
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Internal error: The repository appears broken or inaccessible
    Caused by:
    1: Failed to read commit backend type
    2: Cannot access $TEST_ENV/repo/.jj/repo/store/type
    [EOF]
    [exit status: 255]
    ");
}

#[test]
fn test_color_config() {
    let mut test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Test that --color=always is respected.
    let output = work_dir.run_jj(["--color=always", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [38;5;4me8849ae12c709f2321908879bc724fdb2ab8a781[39m
    [1m[38;5;14m◆[0m  [38;5;4m0000000000000000000000000000000000000000[39m
    [EOF]
    ");

    // Test that color is used if it's requested in the config file
    test_env.add_config(r#"ui.color="always""#);
    let output = work_dir.run_jj(["log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [38;5;4me8849ae12c709f2321908879bc724fdb2ab8a781[39m
    [1m[38;5;14m◆[0m  [38;5;4m0000000000000000000000000000000000000000[39m
    [EOF]
    ");

    // Test that --color=never overrides the config.
    let output = work_dir.run_jj(["--color=never", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Test that --color=auto overrides the config.
    let output = work_dir.run_jj(["--color=auto", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Test that --config 'ui.color=never' overrides the config.
    let output = work_dir.run_jj(["--config=ui.color=never", "log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // --color overrides --config 'ui.color=...'.
    let output = work_dir.run_jj([
        "--color",
        "never",
        "--config=ui.color=always",
        "log",
        "-T",
        "commit_id",
    ]);
    insta::assert_snapshot!(output, @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Test that NO_COLOR does NOT override the request for color in the config file
    test_env.add_env_var("NO_COLOR", "1");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [38;5;4me8849ae12c709f2321908879bc724fdb2ab8a781[39m
    [1m[38;5;14m◆[0m  [38;5;4m0000000000000000000000000000000000000000[39m
    [EOF]
    ");

    // Test that per-repo config overrides the user config.
    work_dir.write_file(".jj/repo/config.toml", r#"ui.color = "never""#);
    let output = work_dir.run_jj(["log", "-T", "commit_id"]);
    insta::assert_snapshot!(output, @r"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Invalid --color
    let output = work_dir.run_jj(["log", "--color=foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: invalid value 'foo' for '--color <WHEN>'
      [possible values: always, never, debug, auto]

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
    // Invalid ui.color
    let stderr = work_dir.run_jj(["log", "--config=ui.color=true"]);
    insta::assert_snapshot!(stderr, @r"
    ------- stderr -------
    Config error: Invalid type or value for ui.color
    Caused by: wanted string or table

    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_color_ui_messages() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config("ui.color = 'always'");

    // hint and error
    let output = test_env.run_jj_in(".", ["-R."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    [1m[38;5;6mHint: [0m[39mUse `jj -h` for a list of available commands.[39m
    [39mRun `jj config set --user ui.default-command log` to disable this message.[39m
    [1m[38;5;1mError: [39mThere is no jj repo in "."[0m
    [EOF]
    [exit status: 1]
    "#);

    // error source
    let output = work_dir.run_jj(["log", ".."]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    [1m[38;5;1mError: [39mFailed to parse fileset: Invalid file pattern[0m
    [1m[39mCaused by:[0m
    [1m[39m1: [0m[39m --> 1:1[39m
    [39m  |[39m
    [39m1 | ..[39m
    [39m  | ^^[39m
    [39m  |[39m
    [39m  = Invalid file pattern[39m
    [1m[39m2: [0m[39mPath ".." is not in the repo "."[39m
    [1m[39m3: [0m[39mInvalid component ".." in repo-relative path "../"[39m
    [EOF]
    [exit status: 1]
    "#);

    // warning
    let output = work_dir.run_jj(["log", "@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    [1m[38;5;3mWarning: [39mNo matching entries for paths: @[0m
    [1m[38;5;3mWarning: [39mThe argument "@" is being interpreted as a fileset expression. To specify a revset, pass -r "@" instead.[0m
    [EOF]
    "#);

    // error inlined in template output
    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj([
        "log",
        "-r@|@--",
        "--config=templates.log_node=commit_id",
        "-Tdescription",
    ]);
    insta::assert_snapshot!(output, @r"
    [38;5;4m8afc18ff677d32e40043e1bc8c1683c2f9c2e916[39m
    [1m[39m<[38;5;1mError: [39mNo Commit available>[0m  [38;5;8m(elided revisions)[39m
    [38;5;4m0000000000000000000000000000000000000000[39m
    [EOF]
    ");

    // formatted hint
    let output = work_dir.run_jj(["edit", ".."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    [1m[38;5;1mError: [39mRevset `..` resolved to more than one revision[0m
    [1m[38;5;6mHint: [0m[39mThe revset `..` resolved to these revisions:[39m
    [39m  [1m[38;5;13mm[38;5;8mzvwutvl[39m [38;5;12m8[38;5;8mafc18ff[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[0m[39m[39m
    [39m  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m [38;5;2m(empty)[39m [38;5;2m(no description set)[39m[39m
    [EOF]
    [exit status: 1]
    ");

    // commit_summary template with debugging colors
    let output = work_dir.run_jj(["st", "--color", "debug"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : [1m[38;5;13m<<commit working_copy change_id shortest prefix::m>>[38;5;8m<<commit working_copy change_id shortest rest::zvwutvl>>[39m<<commit working_copy:: >>[38;5;12m<<commit working_copy commit_id shortest prefix::8>>[38;5;8m<<commit working_copy commit_id shortest rest::afc18ff>>[39m<<commit working_copy:: >>[38;5;10m<<commit working_copy empty::(empty)>>[39m<<commit working_copy:: >>[38;5;10m<<commit working_copy empty description placeholder::(no description set)>>[0m
    Parent commit (@-): [1m[38;5;5m<<commit change_id shortest prefix::q>>[0m[38;5;8m<<commit change_id shortest rest::pvuntsm>>[39m<<commit:: >>[1m[38;5;4m<<commit commit_id shortest prefix::e>>[0m[38;5;8m<<commit commit_id shortest rest::8849ae1>>[39m<<commit:: >>[38;5;2m<<commit empty::(empty)>>[39m<<commit:: >>[38;5;2m<<commit empty description placeholder::(no description set)>>[39m
    [EOF]
    ");

    // commit_summary template in transaction
    let output = work_dir.run_jj(["revert", "--color=debug", "-r@", "-d@"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Reverted 1 commits as follows:
      [1m[38;5;5m<<commit change_id shortest prefix::y>>[0m[38;5;8m<<commit change_id shortest rest::ostqsxw>>[39m<<commit:: >>[1m[38;5;4m<<commit commit_id shortest prefix::8b>>[0m[38;5;8m<<commit commit_id shortest rest::f82eec>>[39m<<commit:: >>[38;5;2m<<commit empty::(empty)>>[39m<<commit:: >><<commit description first_line::Revert "">>
    [EOF]
    "#);
}

#[test]
fn test_quiet() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Can skip message about new working copy with `--quiet`
    work_dir.write_file("file1", "contents");
    let output = work_dir.run_jj(["--quiet", "describe", "-m=new description"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_early_args() {
    // Test that help output parses early args
    let test_env = TestEnvironment::default();

    // The default is no color.
    let output = test_env.run_jj_in(".", ["help"]).success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().find(|l| l.contains("Commands:")).unwrap(),
        @"Commands:");

    // Check that output is colorized.
    let output = test_env
        .run_jj_in(".", ["--color=always", "help"])
        .success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().find(|l| l.contains("Commands:")).unwrap(),
        @"[1m[33mCommands:[0m");

    // Check that early args are accepted after the help command
    let output = test_env
        .run_jj_in(".", ["help", "--color=always"])
        .success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().find(|l| l.contains("Commands:")).unwrap(),
        @"[1m[33mCommands:[0m");

    // Check that early args are accepted after -h/--help
    let output = test_env.run_jj_in(".", ["-h", "--color=always"]).success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().find(|l| l.contains("Usage:")).unwrap(),
        @"[1m[33mUsage:[0m [1m[32mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m");
    let output = test_env
        .run_jj_in(".", ["log", "--help", "--color=always"])
        .success();
    insta::assert_snapshot!(
        output.stdout.normalized().lines().find(|l| l.contains("Usage:")).unwrap(),
        @"[1m[33mUsage:[0m [1m[32mjj log[0m [32m[OPTIONS][0m [32m[FILESETS]...[0m");

    // Early args are parsed with clap's ignore_errors(), but there is a known
    // bug that causes defaults to be unpopulated. Test that the early args are
    // tolerant of this bug and don't cause a crash.
    test_env.run_jj_in(".", ["--no-pager", "help"]).success();
    test_env
        .run_jj_in(".", ["--config=ui.color=always", "help"])
        .success();
}

#[test]
fn test_config_args() {
    let test_env = TestEnvironment::default();
    let list_config = |args: &[&str]| {
        test_env.run_jj_in(
            ".",
            [&["config", "list", "--include-overridden", "test"], args].concat(),
        )
    };

    std::fs::write(
        test_env.env_root().join("file1.toml"),
        indoc! {"
            test.key1 = 'file1'
            test.key2 = 'file1'
        "},
    )
    .unwrap();
    std::fs::write(
        test_env.env_root().join("file2.toml"),
        indoc! {"
            test.key3 = 'file2'
        "},
    )
    .unwrap();

    let output = list_config(&["--config=test.key1=arg1"]);
    insta::assert_snapshot!(output, @r#"
    test.key1 = "arg1"
    [EOF]
    "#);
    let output = list_config(&["--config-file=file1.toml"]);
    insta::assert_snapshot!(output, @r"
    test.key1 = 'file1'
    test.key2 = 'file1'
    [EOF]
    ");

    // --config items are inserted to a single layer internally
    let output = list_config(&[
        "--config=test.key1='arg1'",
        "--config=test.key2.sub=true",
        "--config=test.key1=arg3",
    ]);
    insta::assert_snapshot!(output, @r#"
    test.key1 = "arg3"
    test.key2.sub = true
    [EOF]
    "#);

    // --config* arguments are processed in order of appearance
    let output = list_config(&[
        "--config=test.key1=arg1",
        "--config-file=file1.toml",
        "--config=test.key2=arg3",
        "--config-file=file2.toml",
    ]);
    insta::assert_snapshot!(output, @r#"
    # test.key1 = "arg1"
    test.key1 = 'file1'
    # test.key2 = 'file1'
    test.key2 = "arg3"
    test.key3 = 'file2'
    [EOF]
    "#);

    let output = test_env.run_jj_in(".", ["config", "list", "--config=foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: --config must be specified as NAME=VALUE
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    let output = test_env.run_jj_in(".", ["config", "list", "--config-file=unknown.toml"]);
    insta::with_settings!({
        filters => [("(?m)^([2-9]): .*", "$1: <redacted>")],
    }, {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Config error: Failed to read configuration file
        Caused by:
        1: Cannot access unknown.toml
        2: <redacted>
        For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
        [EOF]
        [exit status: 1]
        ");
    });
}

#[test]
fn test_invalid_config() {
    // Test that we get a reasonable error if the config is invalid (#55)
    let test_env = TestEnvironment::default();

    test_env.add_config("[section]key = value-missing-quotes");
    let output = test_env.run_jj_in(".", ["git", "init", "repo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Configuration cannot be parsed as TOML document
    Caused by: TOML parse error at line 1, column 10
      |
    1 | [section]key = value-missing-quotes
      |          ^
    unexpected key or value, expected newline, `#`

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_invalid_config_value() {
    // Test that we get a reasonable error if a config value is invalid
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["status", "--config=snapshot.auto-track=[0]"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid type or value for snapshot.auto-track
    Caused by: invalid type: sequence, expected a string

    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
#[cfg_attr(windows, ignore = "dirs::home_dir() can't be overridden by $HOME")] // TODO
fn test_conditional_config() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.home_dir(), ["git", "init", "repo1"])
        .success();
    test_env
        .run_jj_in(test_env.home_dir(), ["git", "init", "repo2"])
        .success();
    test_env.add_config(indoc! {"
        aliases.foo = ['new', 'root()', '-mglobal']
        [[--scope]]
        --when.repositories = ['~']
        aliases.foo = ['new', 'root()', '-mhome']
        [[--scope]]
        --when.repositories = ['~/repo1']
        aliases.foo = ['new', 'root()', '-mrepo1']
    "});

    // Sanity check
    let output = test_env.run_jj_in(".", ["config", "list", "--include-overridden", "aliases"]);
    insta::assert_snapshot!(output, @r"
    aliases.foo = ['new', 'root()', '-mglobal']
    [EOF]
    ");
    let output = test_env.run_jj_in(
        &test_env.home_dir().join("repo1"),
        ["config", "list", "--include-overridden", "aliases"],
    );
    insta::assert_snapshot!(output, @r"
    # aliases.foo = ['new', 'root()', '-mglobal']
    # aliases.foo = ['new', 'root()', '-mhome']
    aliases.foo = ['new', 'root()', '-mrepo1']
    [EOF]
    ");
    let output = test_env.run_jj_in(
        &test_env.home_dir().join("repo2"),
        ["config", "list", "--include-overridden", "aliases"],
    );
    insta::assert_snapshot!(output, @r"
    # aliases.foo = ['new', 'root()', '-mglobal']
    aliases.foo = ['new', 'root()', '-mhome']
    [EOF]
    ");

    // Aliases can be expanded by using the conditional tables
    let output = test_env.run_jj_in(&test_env.home_dir().join("repo1"), ["foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: royxmykx 7c486962 (empty) repo1
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    let output = test_env.run_jj_in(&test_env.home_dir().join("repo2"), ["foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: yqosqzyt 072741b8 (empty) home
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

/// Test that `jj` command works with the default configuration.
#[test]
fn test_default_config() {
    let mut test_env = TestEnvironment::default();
    let config_dir = test_env.env_root().join("empty-config");
    std::fs::create_dir(&config_dir).unwrap();
    test_env.set_config_path(&config_dir);

    let envs_to_drop = test_env
        .new_jj_cmd()
        .get_envs()
        .filter_map(|(name, _)| name.to_str())
        .filter(|&name| name.starts_with("JJ_") && name != "JJ_CONFIG")
        .map(|name| name.to_owned())
        .sorted_unstable()
        .collect_vec();
    insta::assert_debug_snapshot!(envs_to_drop, @r#"
    [
        "JJ_EMAIL",
        "JJ_OP_HOSTNAME",
        "JJ_OP_TIMESTAMP",
        "JJ_OP_USERNAME",
        "JJ_RANDOMNESS_SEED",
        "JJ_TIMESTAMP",
        "JJ_TZ_OFFSET_MINS",
        "JJ_USER",
    ]
    "#);
    let run_jj = |work_dir: &TestWorkDir, args: &[&str]| {
        work_dir.run_jj_with(|cmd| {
            for name in &envs_to_drop {
                cmd.env_remove(name);
            }
            cmd.args(args)
        })
    };

    let mut insta_settings = insta::Settings::clone_current();
    insta_settings.add_filter(r"\b[a-zA-Z0-9\-._]+@[a-zA-Z0-9\-._]+\b", "<user>@<host>");
    insta_settings.add_filter(
        r"\b[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}\b",
        "<date-time>",
    );
    insta_settings.add_filter(r"\b[k-z]{8,12}\b", "<change-id>");
    insta_settings.add_filter(r"\b[0-9a-f]{8,12}\b", "<id>");
    let _guard = insta_settings.bind_to_scope();

    let maskable_op_user = {
        let maskable_re = Regex::new(r"^[a-zA-Z0-9\-._]+$").unwrap();
        let hostname = whoami::fallible::hostname().expect("hostname should be set");
        let username = whoami::fallible::username().expect("username should be set");
        maskable_re.is_match(&hostname) && maskable_re.is_match(&username)
    };

    let output = run_jj(
        &test_env.work_dir(""),
        &["config", "list", r#"-Tname ++ "\n""#],
    );
    insta::assert_snapshot!(output, @r"
    operation.hostname
    operation.username
    [EOF]
    ");

    let output = run_jj(&test_env.work_dir(""), &["git", "init", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    let work_dir = test_env.work_dir("repo");
    let output = run_jj(&work_dir, &["new"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: <change-id> <id> (empty) (no description set)
    Parent commit (@-)      : <change-id> <id> (empty) (no description set)
    Warning: Name and email not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.name "Some One"
      jj config set --user user.email "<user>@<host>"
    [EOF]
    "#);

    let output = run_jj(&work_dir, &["log"]);
    insta::assert_snapshot!(output, @r"
    @  <change-id> (no email set) <date-time> <id>
    │  (empty) (no description set)
    ○  <change-id> (no email set) <date-time> git_head() <id>
    │  (empty) (no description set)
    ◆  <change-id> root() <id>
    [EOF]
    ");

    let time_config =
        "--config=template-aliases.'format_time_range(t)'='format_timestamp(t.end())'";
    let output = run_jj(&work_dir, &["op", "log", time_config]);
    if maskable_op_user {
        insta::assert_snapshot!(output, @r"
        @  <id> <user>@<host> <date-time>
        │  new empty commit
        │  args: jj new
        ○  <id> <user>@<host> <date-time>
        │  add workspace 'default'
        ○  <id> root()
        [EOF]
        ");
    }
}

#[test]
fn test_no_user_configured() {
    // Test that the user is reminded if they haven't configured their name or email
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["describe", "-m", "without name"])
            .env_remove("JJ_USER")
    });
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 7e7014c2 (empty) without name
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: Name not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.name "Some One"
    [EOF]
    "#);
    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["describe", "-m", "without email"])
            .env_remove("JJ_EMAIL")
    });
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 876580e3 (empty) without email
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: Email not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.email "someone@example.com"
    [EOF]
    "#);
    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(["describe", "-m", "without name and email"])
            .env_remove("JJ_USER")
            .env_remove("JJ_EMAIL")
    });
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm f87356a1 (empty) without name and email
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: Name and email not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.name "Some One"
      jj config set --user user.email "someone@example.com"
    [EOF]
    "#);
}

#[test]
fn test_help() {
    // Test that global options are separated out in the help output
    let test_env = TestEnvironment::default();

    let output = test_env.run_jj_in(".", ["diffedit", "-h"]);
    insta::assert_snapshot!(output, @r"
    Touch up the content changes in a revision with a diff editor

    Usage: jj diffedit [OPTIONS] [FILESETS]...

    Arguments:
      [FILESETS]...  Edit only these paths (unmatched paths will remain unchanged)

    Options:
      -r, --revision <REVSET>    The revision to touch up
      -f, --from <REVSET>        Show changes from this revision
      -t, --to <REVSET>          Edit changes in this revision
          --tool <NAME>          Specify diff editor to be used
          --restore-descendants  Preserve the content (not the diff) when rebasing descendants
      -h, --help                 Print help (see more with '--help')

    Global Options:
      -R, --repository <REPOSITORY>      Path to repository to operate on
          --ignore-working-copy          Don't snapshot the working copy, and don't update it
          --ignore-immutable             Allow rewriting immutable commits
          --at-operation <AT_OPERATION>  Operation to load the repo at [aliases: --at-op]
          --debug                        Enable debug logging
          --color <WHEN>                 When to colorize output [possible values: always, never, debug,
                                         auto]
          --quiet                        Silence non-primary command output
          --no-pager                     Disable the pager
          --config <NAME=VALUE>          Additional configuration options (can be repeated)
          --config-file <PATH>           Additional configuration files (can be repeated)
    [EOF]
    ");
}

#[test]
fn test_debug_logging_enabled() {
    // Test that the debug flag enabled debug logging
    let test_env = TestEnvironment::default();

    let output = test_env.run_jj_in(".", ["version", "--debug"]).success();
    // Split the first log line into a timestamp and the rest.
    // The timestamp is constant sized so this is a robust operation.
    // Example timestamp: 2022-11-20T06:24:05.477703Z
    let (_timestamp, log_line) = output
        .stderr
        .normalized()
        .lines()
        .next()
        .expect("debug logging on first line")
        .split_at(36);
    // The log format is currently Pretty so we include the terminal markup.
    // Luckily, insta will print this in color when reviewing.
    insta::assert_snapshot!(log_line, @"[32m INFO[0m [2mjj_cli::cli_util[0m[2m:[0m debug logging enabled");
}
