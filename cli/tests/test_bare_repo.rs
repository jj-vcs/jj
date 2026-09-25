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

/// `jj git init --bare` creates a bare repo with no workspace.
#[test]
fn test_git_init_bare() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "--bare", "my-project"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized bare repo in "my-project"
    Hint: Add a workspace with: jj workspace add <name>
    [EOF]
    "#);

    assert!(test_env.env_root().join("my-project").join(".jj").is_dir());
    assert!(
        !test_env
            .env_root()
            .join("my-project")
            .join(".jj")
            .join("working_copy")
            .exists()
    );
    assert!(
        test_env
            .env_root()
            .join("my-project")
            .join(".jj")
            .join("bare")
            .exists()
    );
}

/// Running `jj status` from bare root gives a warning and workspace list.
#[test]
fn test_bare_repo_root_error() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");
    bare_dir.run_jj(["workspace", "add", "main"]).success();

    let output = bare_dir.run_jj(["status"]);
    // stdout contains the workspace list
    assert!(
        output.stdout.raw().contains("main:"),
        "expected main workspace in stdout: {}",
        output.stdout.raw()
    );
    // stderr contains the warning and hint
    let stderr = output.stderr.raw();
    assert!(
        stderr.contains("bare repo root"),
        "expected bare repo warning: {stderr}"
    );
    assert!(
        stderr.contains("Registered workspaces"),
        "expected hint: {stderr}"
    );
    // exit code 0
    output.success();
}

/// `jj workspace add` works from a bare repo root.
#[test]
fn test_workspace_add_from_bare_root() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");

    let output = bare_dir.run_jj(["workspace", "add", "main"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Created workspace in "main"
    [EOF]
    "#);

    assert!(
        test_env
            .env_root()
            .join("my-project")
            .join("main")
            .join(".jj")
            .join("working_copy")
            .is_dir()
    );
}

/// `jj workspace list` works from a bare repo root.
#[test]
fn test_workspace_list_from_bare_root() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");
    bare_dir.run_jj(["workspace", "add", "main"]).success();
    bare_dir.run_jj(["workspace", "add", "feat-1"]).success();

    let output = bare_dir.run_jj(["workspace", "list"]);
    let stdout = output.stdout.raw();
    assert!(stdout.contains("feat-1:"), "missing feat-1: {stdout}");
    assert!(stdout.contains("main:"), "missing main: {stdout}");
    let feat_pos = stdout.find("feat-1:").unwrap();
    let main_pos = stdout.find("main:").unwrap();
    assert!(feat_pos < main_pos, "expected feat-1 before main: {stdout}");
}

/// `jj log` works from a bare repo root.
#[test]
fn test_log_from_bare_root() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");
    bare_dir.run_jj(["workspace", "add", "main"]).success();

    let output = bare_dir.run_jj(["log"]).success();
    assert!(
        !output.stdout.raw().is_empty(),
        "expected non-empty log output: {}",
        output.stdout.raw()
    );
}

/// `jj workspace add` then `jj status` inside workspace works end-to-end.
#[test]
fn test_bare_repo_workflow() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");

    bare_dir.run_jj(["workspace", "add", "main"]).success();
    bare_dir.run_jj(["workspace", "add", "feat-1"]).success();

    let main_dir = test_env.work_dir("my-project/main");
    main_dir.run_jj(["status"]).success();

    let feat_dir = test_env.work_dir("my-project/feat-1");
    feat_dir.run_jj(["status"]).success();

    let output = main_dir.run_jj(["workspace", "list"]);
    let stdout = output.stdout.raw();
    assert!(stdout.contains("main"), "missing main: {stdout}");
    assert!(stdout.contains("feat-1"), "missing feat-1: {stdout}");
}

/// Path warning is suppressed from bare root but fires inside a workspace.
#[test]
fn test_workspace_add_path_warning() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--bare", "my-project"])
        .success();
    let bare_dir = test_env.work_dir("my-project");
    bare_dir.run_jj(["workspace", "add", "main"]).success();

    // No warning from bare root — workspace-as-name is the expected workflow.
    let output = bare_dir.run_jj(["workspace", "add", "feat-1"]);
    assert!(
        !output.stderr.raw().contains("unintentional"),
        "unexpected path warning from bare root: {}",
        output.stderr.raw()
    );

    // Warning fires from inside a workspace when no separator is used.
    let main_dir = test_env.work_dir("my-project/main");
    let output = main_dir.run_jj(["workspace", "add", "feat-2"]);
    assert!(
        output.stderr.raw().contains("unintentional"),
        "expected path warning from workspace: {}",
        output.stderr.raw()
    );
}
