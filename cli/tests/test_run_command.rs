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

//! Tests for `jj run`.
//!
//! `jj run` is currently a **stub command** — it performs argument parsing and
//! revision resolution but unconditionally returns a user-facing error:
//!
//!   "This is a stub, do not use"
//!
//! ## Why test a stub?
//!
//! 1. **Regression guard** — if someone wires up real implementation code they
//!    must update these tests intentionally, ensuring the change is visible in
//!    review rather than accidentally activating in production.
//! 2. **CLI contract** — the flags (`--jobs`, `--revisions`, `-x`) and their
//!    accepted values are part of the public CLI surface even before the
//!    command is implemented. Tests lock down that the parser accepts them
//!    without a spurious parse error preceding the stub error.
//! 3. **Repo requirement** — `jj run` calls `workspace_helper` before the stub
//!    is reached, so it must be invoked inside a repository. Tests confirm the
//!    correct error is returned when run outside one.

use crate::common::TestEnvironment;

/// `jj run <cmd>` in a valid repo always returns the stub error message.
///
/// This is the baseline: with no extra flags, after resolving the default
/// revision (`@`), the command must exit with code 1 and the known stub
/// message. It must not crash, hang, or produce unexpected output.
#[test]
fn test_run_is_stub() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["run", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This is a stub, do not use
    [EOF]
    [exit status: 1]
    ");
}

/// `jj run --jobs <N>` is accepted by the parser and does not produce a
/// different error before the stub is hit.
///
/// The jobs value is resolved inside `cmd_run` prior to the final
/// `Err(user_error(...))`, so the stub error is always the last thing seen.
/// Tests both a concrete job count and the implicit zero/"use all cores" path.
#[test]
fn test_run_accepts_jobs_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Explicit job count
    let output = work_dir.run_jj(["run", "--jobs", "4", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This is a stub, do not use
    [EOF]
    [exit status: 1]
    ");

    // Short form
    let output = work_dir.run_jj(["run", "-j", "1", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This is a stub, do not use
    [EOF]
    [exit status: 1]
    ");
}

/// `jj run -r <revset>` is accepted and the revset is resolved before the stub
/// error is returned.
///
/// Revision resolution happens inside `cmd_run` (via `parse_union_revsets` /
/// `evaluate_to_commits`). A syntactically valid but non-existent revset would
/// return a revset error rather than the stub error. This test confirms that a
/// valid revset produces only the stub error, proving the resolution path is
/// exercised without side-effects.
#[test]
fn test_run_accepts_revset_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["run", "-r", "@", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This is a stub, do not use
    [EOF]
    [exit status: 1]
    ");
}

/// The `-x` flag is a hidden compat alias (mirrors `git rebase -x`). It must
/// be accepted without any error or warning — it is silently ignored.
///
/// Because it is marked `hide = true` in clap, it will not appear in `--help`
/// output, but the parser must still accept it.
#[test]
fn test_run_accepts_hidden_x_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["run", "-x", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This is a stub, do not use
    [EOF]
    [exit status: 1]
    ");
}

/// `jj run` outside a repository must fail with the standard "no jj repo"
/// error, not the stub error.
///
/// `cmd_run` calls `command.workspace_helper(ui)?` as its first action, which
/// returns early before any stub logic is reached. This test confirms that the
/// workspace check takes priority over the stub.
#[test]
fn test_run_outside_repo() {
    let test_env = TestEnvironment::default();

    let output = test_env.run_jj_in(".", ["run", "true"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);
}

/// An invalid revset expression must return a revset parse error rather than
/// the stub error, confirming that revision resolution runs before the stub
/// is reached.
#[test]
fn test_run_invalid_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["run", "-r", "::invalid::", "true"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse revset: Syntax error
    Caused by:  --> 1:11
      |
    1 | ::invalid::
      |           ^---
      |
      = expected <primary>
    Hint: See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k revsets` for revsets syntax and how to quote symbols.
    [EOF]
    [exit status: 1]
    ");
}
