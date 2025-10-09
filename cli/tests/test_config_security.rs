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

use std::path::Path;

use jj_lib::protos::user_config::RepoConfig;
use jj_lib::user_config::ConfigType as _;

use crate::common::TestEnvironment;

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[test]
// Here we simulate an attacker attempting to send you a zip file containing a
// malicious jj repo
fn test_attacker_without_signature() {
    // A test environment representing the user under attack.
    let mut victim_env = TestEnvironment::default();
    // A test environment with a different home directory, representing a malicious
    // actor.
    let evil_env = TestEnvironment::default();
    let work_dir = evil_env.work_dir("").create_dir("repo");
    evil_env.run_jj_in(".", ["git", "init", "repo"]).success();

    // We see that the config was created by someone else, but it was empty so
    // it doesn't pose a security risk.
    let d = work_dir.root().canonicalize().unwrap();
    let output = victim_env.run_jj_in(&d, ["status"]);
    insta::assert_snapshot!(output, @r###"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    "###);

    evil_env
        .run_jj_in(&d, ["config", "set", "--repo", "repo-key", "repo-val"])
        .success();
    let output = victim_env.run_jj_in(&d, ["status"]);
    insta::assert_snapshot!(output, @r###"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Warning: The repo appears to have been created by someone else. For security reasons, we have disabled the repo config.
    Hint: Run `jj config edit --repo` to review and re-enable
    [EOF]
    "###);

    evil_env
        .run_jj_in(
            &d,
            [
                "config",
                "set",
                "--workspace",
                "workspace-key",
                "workspace-val",
            ],
        )
        .success();

    let output = victim_env.run_jj_in(&d, ["config", "get", "workspace-key"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Warning: The repo appears to have been created by someone else. For security reasons, we have disabled the repo config.
    Hint: Run `jj config edit --repo` to review and re-enable
    Warning: The workspace appears to have been created by someone else. For security reasons, we have disabled the workspace config.
    Hint: Run `jj config edit --workspace` to review and re-enable
    Config error: Value not found for workspace-key
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    "###);

    // The victim now reviews the repo config and finds it to not be a security
    // risk.
    let edit_script = victim_env.set_up_fake_editor();
    victim_env
        .run_jj_in(&d, ["config", "edit", "--repo"])
        .success();
    let output = victim_env.run_jj_in(&d, ["config", "get", "repo-key"]);
    insta::assert_snapshot!(output, @r###"
    repo-val
    [EOF]
    ------- stderr -------
    Warning: The workspace appears to have been created by someone else. For security reasons, we have disabled the workspace config.
    Hint: Run `jj config edit --workspace` to review and re-enable
    [EOF]
    "###);

    // The victim now reviews the workspace config and finds it to be a security
    // risk, so makes some changes.
    std::fs::write(&edit_script, "write\nworkspace-key = \"new-workspace-val\"").unwrap();
    victim_env
        .run_jj_in(&d, ["config", "edit", "--workspace"])
        .success();
    let output = victim_env.run_jj_in(&d, ["config", "get", "workspace-key"]);
    insta::assert_snapshot!(output, @r###"
    new-workspace-val
    [EOF]
    "###);
}

#[test]
// Here, we simulate you having sent the attacker a zip file.
// They then send you another repo with that signature and you download it to a
// different directory.
fn test_attacker_has_signature() {
    let test_env = TestEnvironment::default();
    let repo_dir = test_env.work_dir("").create_dir("repo");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();

    let evil_dir = test_env.work_dir("evil");
    copy_dir_all(repo_dir.root(), evil_dir.root()).unwrap();

    // The file has moved but there's no config, so we consider it safe.
    let output = evil_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r###"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    "###);

    repo_dir
        .run_jj(["config", "set", "--repo", "repo-key", "repo-val"])
        .success();
    repo_dir
        .run_jj([
            "config",
            "set",
            "--workspace",
            "workspace-key",
            "workspace-val",
        ])
        .success();

    std::fs::remove_dir_all(evil_dir.root()).unwrap();
    copy_dir_all(repo_dir.root(), evil_dir.root()).unwrap();

    // It's no longer safe because it now has config.
    let output = test_env.run_jj_in(evil_dir.root(), ["config", "get", "repo-key"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Warning: The repo has moved from $TEST_ENV/repo/.jj/repo to $TEST_ENV/evil/.jj/repo. For security reasons, we have disabled the repo config.
    Hint: Run `jj config edit --repo` to review and re-enable
    Warning: The workspace has moved from $TEST_ENV/repo/.jj to $TEST_ENV/evil/.jj. For security reasons, we have disabled the workspace config.
    Hint: Run `jj config edit --workspace` to review and re-enable
    Config error: Value not found for repo-key
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    "###);
}

#[test]
fn test_legacy_config_migration() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("").create_dir("repo");
    work_dir.run_jj(["git", "init"]).success();

    // Convert it to a legacy repo.
    let legacy_config = work_dir.root().join(".jj/repo/config.toml");
    let new_config = work_dir
        .root()
        .join(".jj/repo")
        .join(RepoConfig::filename());
    std::fs::write(&legacy_config, "foo = \"bar\"").unwrap();
    std::fs::remove_file(&new_config).unwrap();

    let output = work_dir.run_jj(["config", "get", "foo"]);
    insta::assert_snapshot!(output, @r###"
    bar
    [EOF]
    "###);

    assert!(!legacy_config.exists());
    assert!(new_config.exists());
}
