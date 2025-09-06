// Copyright 2025 The Jujutsu Authors
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
fn test_post_commit_hook() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(
        ".jj/repo/config.toml",
        r#"hooks.post-commit = ["echo", "Committed successfully!"]"#,
    );
    work_dir.write_file("file", "text");
    let output = work_dir.run_jj(["commit", "-m", "test commit"]);

    insta::assert_snapshot!(output, @r#"
    Committed successfully!
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 6d9b7c28 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 5ef1fd62 test commit
    [EOF]
    "#);
}

#[test]
fn test_post_squash_hook() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m", "test squash"]).success();

    work_dir.write_file(
        ".jj/repo/config.toml",
        r#"hooks.post-squash = ["echo", "Squashed successfully!"]"#,
    );
    work_dir.write_file("file", "text");
    let output = work_dir.run_jj(["squash"]);

    insta::assert_snapshot!(output, @r#"
    Squashed successfully!
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 415c5a42 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm dda690f8 test squash
    [EOF]
    "#);
}

#[test]
fn test_hook_command_not_found() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(
        ".jj/repo/config.toml",
        r#"hooks.post-commit = ["nonexistent-command"]"#,
    );
    work_dir.write_file("file", "text");
    let output = work_dir.run_jj(["commit", "-m", "test commit"]);

    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 6d9b7c28 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 5ef1fd62 test commit
    Error: Hook 'nonexistent-command' failed to run
    Caused by: No such file or directory (os error 2)
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_hook_command_failure() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".jj/repo/config.toml", r#"hooks.post-commit = ["false"]"#);
    work_dir.write_file("file", "text");
    let output = work_dir.run_jj(["commit", "-m", "test commit"]);

    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 6d9b7c28 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 5ef1fd62 test commit
    Error: Hook 'false' exited with code 1
    [EOF]
    [exit status: 1]
    "#);
}
