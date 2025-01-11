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
//

use std::path::PathBuf;

use jj_lib::repo_path;

use crate::common::TestEnvironment;

/// Sets up a repo with three initial commits, containing some text.
fn init_test_env() -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    (test_env, repo_path)
}
#[test]
fn test_simple_run_invocation() {
    let (test_env, repo_path) = init_test_env();
    std::fs::write(repo_path.join("A.txt"), "A").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "A"]);
    std::fs::write(repo_path.join("b.txt"), "b").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m ", "B"]);
    std::fs::write(repo_path.join("c.txt"), "test to replace").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "C"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "run",
            "'fake-formatter --uppercase'",
            "-r",
            "'mutable()..@'",
        ],
    );
    // all commits should be modified
    insta::assert_snapshot!(stdout, r#""#);
}

#[test]
fn test_run_on_immutable() {
    let (test_env, repo_path) = init_test_env();
    std::fs::write(repo_path.join("A.txt"), "A").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "A"]);
    std::fs::write(repo_path.join("b.txt"), "b").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m ", "B"]);
    std::fs::write(repo_path.join("c.txt"), "test to replace").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "C"]);
    let (stdout, _) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "run",
            "'fake-formatter --uppercase'",
            "-r",
            "'root()'", // Running on the root commit is nonsensical.
        ],
    );

    insta::assert_snapshot!(stdout, r#""#);
}

#[test]
fn test_run_noop() {
    let (test_env, repo_path) = init_test_env();
    std::fs::write(repo_path.join("A.txt"), "A").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "A"]);
    std::fs::write(repo_path.join("b.txt"), "b").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m ", "B"]);
    std::fs::write(repo_path.join("c.txt"), "test to replace").unwrap();
    test_env.jj_cmd_ok(&repo_path, &["commit", "-m", "C"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &["run", "'fake-formatter --echo'", "-r", "'mutable()..@'"],
    );
    insta::assert_snapshot!(stdout, r#""#);
}
