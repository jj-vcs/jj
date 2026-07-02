// Copyright 2026 The Jujutsu Authors
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
fn test_os_args_repeated_consecutive() {
    let test_env = TestEnvironment::default();
    let jj_cmd = assert_cmd::cargo::cargo_bin!("jj")
        .as_os_str()
        .to_str()
        .unwrap();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Consecutive repetitions of `jj_cmd` should be coalesced into a single
    // `jj_cmd`.
    let output = work_dir.run_jj(&[jj_cmd, jj_cmd, jj_cmd, jj_cmd, "--help"]);
    // We have to replace the whole line, instead of matching for `jj_cmd` and
    // replacing it, because this fails on Windows.
    let normalized_warning =
        "Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.\n";
    let output = output
        .normalize_stdout_with(|s| s.split_inclusive('\n').take(7).collect())
        .normalize_stderr_with(|s| {
            s.lines()
                .map(|l| {
                    if l.starts_with("Warning: Ignoring repeated consecutive") {
                        normalized_warning
                    } else {
                        l
                    }
                })
                .collect()
        });
    insta::assert_snapshot!(output, @"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://docs.jj-vcs.dev/latest/tutorial/

    Usage: jj [OPTIONS] <COMMAND>
    [EOF]
    ------- stderr -------
    Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.
    [EOF]
    ");
}

#[test]
fn test_os_args_repeated_non_consecutive() {
    let test_env = TestEnvironment::default();
    let jj_cmd = assert_cmd::cargo::cargo_bin!("jj")
        .as_os_str()
        .to_str()
        .unwrap();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Make sure that default command is not --help.
    test_env.add_config(r#"ui.default-command = "log""#);
    // Only *consecutive* repetitions of `jj_cmd` should be coalesced,
    // everything else passed as arguments.
    let output = work_dir.run_jj(&[jj_cmd, jj_cmd, jj_cmd, "--help", jj_cmd]);
    // We have to replace the whole line, instead of matching for `jj_cmd` and
    // replacing it, because this fails on Windows.
    let normalized_warning =
        "Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.\n";
    let output = output
        .normalize_stdout_with(|s| s.split_inclusive('\n').take(7).collect())
        .normalize_stderr_with(|s| {
            s.lines()
                .map(|l| {
                    if l.starts_with("Warning: Ignoring repeated consecutive") {
                        normalized_warning
                    } else {
                        l
                    }
                })
                .collect()
        });
    insta::assert_snapshot!(output, @"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://docs.jj-vcs.dev/latest/tutorial/

    Usage: jj [OPTIONS] <COMMAND>
    [EOF]
    ------- stderr -------
    Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.
    [EOF]
    ");
}

#[test]
fn test_os_args_repeated_consecutive_no_cmd() {
    let test_env = TestEnvironment::default();
    let jj_cmd = assert_cmd::cargo::cargo_bin!("jj")
        .as_os_str()
        .to_str()
        .unwrap();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Make sure that default command is --help.
    test_env.add_config(r#"ui.default-command = "--help""#);
    // When there's only consecutive repeated `jj_cmd` this should be treated
    // as a single `jj_cmd` and run the default command.
    let output = work_dir.run_jj(&[jj_cmd, jj_cmd, jj_cmd, jj_cmd]);
    // We have to replace the whole line, instead of matching for `jj_cmd` and
    // replacing it, because this fails on Windows.
    let normalized_warning =
        "Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.\n";
    let output = output
        .normalize_stdout_with(|s| s.split_inclusive('\n').take(7).collect())
        .normalize_stderr_with(|s| {
            s.lines()
                .map(|l| {
                    if l.starts_with("Warning: Ignoring repeated consecutive") {
                        normalized_warning
                    } else {
                        l
                    }
                })
                .collect()
        });
    insta::assert_snapshot!(output, @"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://docs.jj-vcs.dev/latest/tutorial/

    Usage: jj [OPTIONS] <COMMAND>
    [EOF]
    ------- stderr -------
    Warning: Ignoring repeated consecutive '@@JJ_EXECUTABLE_NAME@@' passed as arguments.
    [EOF]
    ");
}
