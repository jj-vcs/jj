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
use crate::common::TestWorkDir;

#[cfg(unix)]
fn write_hook(work_dir: &TestWorkDir<'_>, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    let path = format!(".jj-hooks/{name}");
    work_dir.write_file(&path, format!("#!/bin/sh\n{body}\n"));
    std::fs::set_permissions(
        work_dir.root().join(&path),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
}

#[test]
fn test_hook_status_nothing_configured() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["hook", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    git-pre-push:
      global: none
      local: none
      would run: (nothing)
    git-post-push:
      global: none
      local: none
      would run: (nothing)
    [EOF]
    ");
}

#[test]
fn test_hook_enable_no_hooks_present() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["hook", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No hooks found under .jj-hooks/
    [EOF]
    [exit status: 1]
    ");
}

#[cfg(unix)]
#[test]
fn test_hook_enable_lists_and_trusts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    write_hook(&work_dir, "git-pre-push", "exit 0");

    let output = work_dir.run_jj(["hook", "enable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    jj will now execute the following hooks automatically in this checkout:
      git-pre-push
    [EOF]
    ");

    let output = work_dir.run_jj(["hook", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    git-pre-push:
      global: none
      local: present, trusted
      would run: local
    git-post-push:
      global: none
      local: none
      would run: (nothing)
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_hook_enable_not_run_twice_is_idempotent() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    write_hook(&work_dir, "git-pre-push", "exit 0");

    work_dir.run_jj(["hook", "enable"]).success();
    work_dir.run_jj(["hook", "enable"]).success();
    let output = work_dir.run_jj(["hook", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    git-pre-push:
      global: none
      local: present, trusted
      would run: local
    git-post-push:
      global: none
      local: none
      would run: (nothing)
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_hook_disable_clears_enabled_and_hashes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    write_hook(&work_dir, "git-pre-push", "exit 0");
    work_dir.run_jj(["hook", "enable"]).success();

    let output = work_dir.run_jj(["hook", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Repository-scoped hooks are now disabled.
    [EOF]
    ");

    let output = work_dir.run_jj(["hook", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    git-pre-push:
      global: none
      local: present, not enabled (`jj hook enable`)
      would run: (nothing)
    git-post-push:
      global: none
      local: none
      would run: (nothing)
    [EOF]
    ");
}

#[test]
fn test_hook_disable_without_enable_is_a_noop() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["hook", "disable"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Repository-scoped hooks were not enabled.
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_hook_status_stale_after_edit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    write_hook(&work_dir, "git-pre-push", "exit 0");
    work_dir.run_jj(["hook", "enable"]).success();

    write_hook(&work_dir, "git-pre-push", "exit 1");
    let output = work_dir.run_jj(["hook", "status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    git-pre-push:
      global: none
      local: present, untrusted (`jj hook enable` to re-approve)
      would run: (nothing)
    git-post-push:
      global: none
      local: none
      would run: (nothing)
    [EOF]
    ");
}
