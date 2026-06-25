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

use std::ffi::OsStr;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

fn normalize_git_hashes(output: CommandOutput) -> CommandOutput {
    output
        .normalize_stdout_with(normalize_git_hashes_in_text)
        .normalize_stderr_with(normalize_git_hashes_in_text)
}

fn normalize_git_hashes_in_text(text: String) -> String {
    let text = regex::Regex::new(r"[0-9a-f]{10,40}")
        .unwrap()
        .replace_all(&text, "<git-commit>")
        .into_owned();
    regex::Regex::new(r"[k-z]{8} [0-9a-f]{8}")
        .unwrap()
        .replace_all(&text, "<jj-commit>")
        .into_owned()
}

fn machine_completion_values(output: CommandOutput) -> Vec<String> {
    let output = output.success();
    let candidates: Vec<serde_json::Value> = serde_json::from_str(output.stdout.raw()).unwrap();
    candidates
        .into_iter()
        .map(|candidate| candidate["value"].as_str().unwrap().to_owned())
        .collect()
}

fn run_git<I, S>(work_dir: &TestWorkDir<'_>, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_submodule_bind_updates_existing_gitlink() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "submodule"])
        .success();
    let submodule_dir = test_env.work_dir("submodule");
    submodule_dir.write_file("file", "first");
    submodule_dir
        .run_jj(["commit", "-m", "first submodule commit"])
        .success();
    let old_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();
    submodule_dir.write_file("file", "second");
    submodule_dir
        .run_jj(["commit", "-m", "second submodule commit"])
        .success();
    let new_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let submodule_url = format!("{}/submodule", test_env.env_root().display());
    run_git(
        &work_dir,
        [
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_url.as_str(),
            "sub",
        ],
    );
    run_git(&work_dir, ["-C", "sub", "checkout", &old_target]);
    run_git(
        &work_dir,
        ["-C", "sub", "branch", "new-target", &new_target],
    );
    run_git(&work_dir, ["add", "sub"]);
    run_git(
        &work_dir,
        [
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test user",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    insta::assert_snapshot!(work_dir.run_jj(["diff", "--summary"]), @r#"
    ------- stderr -------
    ignoring git submodule at "sub"
    Done importing changes from the underlying Git repo.
    [EOF]
    "#);

    let output = work_dir.run_jj(["submodule", "bind", "sub", "-r", "new-target"]);
    insta::assert_snapshot!(normalize_git_hashes(output), @r#"
    ------- stderr -------
    Updated Git submodule sub:
      old: <git-commit>
      new: <git-commit>
    Rewritten superproject commit: <jj-commit> (no description set)
    ignoring git submodule at "sub"
    Working copy  (@) now at: <jj-commit> (no description set)
    Parent commit (@-)      : <jj-commit> master | add submodule
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    "#);

    insta::assert_snapshot!(work_dir.run_jj(["diff", "--summary"]), @r#"
    M sub
    [EOF]
    "#);
    let diff_git = work_dir.run_jj(["diff", "--git"]);
    assert!(diff_git.stdout.raw().contains(&new_target[..10]));
    insta::assert_snapshot!(normalize_git_hashes(diff_git), @r#"
    diff --git a/sub b/sub
    index <git-commit>..<git-commit> 160000
    [EOF]
    "#);
}

#[test]
fn test_submodule_bind_resolves_conflicted_gitlink() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "submodule"])
        .success();
    let submodule_dir = test_env.work_dir("submodule");
    submodule_dir.write_file("file", "first");
    submodule_dir.run_jj(["commit", "-m", "first"]).success();
    let first_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();
    submodule_dir.write_file("file", "second");
    submodule_dir.run_jj(["commit", "-m", "second"]).success();
    let second_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();
    submodule_dir.write_file("file", "third");
    submodule_dir.run_jj(["commit", "-m", "third"]).success();
    let third_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let submodule_url = format!("{}/submodule", test_env.env_root().display());
    run_git(
        &work_dir,
        [
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_url.as_str(),
            "sub",
        ],
    );
    run_git(&work_dir, ["-C", "sub", "checkout", &first_target]);
    run_git(&work_dir, ["add", ".gitmodules", "sub"]);
    run_git(
        &work_dir,
        [
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test user",
            "commit",
            "-m",
            "add submodule",
        ],
    );
    work_dir.run_jj(["status"]).success();

    work_dir
        .run_jj(["new", "@-", "-m", "update sub to second"])
        .success();
    work_dir
        .run_jj(["submodule", "bind", "sub", "-r", &second_target])
        .success();
    let source_change = work_dir
        .run_jj(["log", "-r@", "--no-graph", "-T", "change_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();

    work_dir
        .run_jj(["new", "@-", "-m", "update sub to third"])
        .success();
    work_dir
        .run_jj(["submodule", "bind", "sub", "-r", &third_target])
        .success();
    let destination_change = work_dir
        .run_jj(["log", "-r@", "--no-graph", "-T", "change_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();

    work_dir
        .run_jj(["rebase", "-r", &source_change, "-d", &destination_change])
        .success();
    let diff_git = work_dir
        .run_jj(["diff", "-r", &source_change, "--git"])
        .success();
    assert!(diff_git.stdout.raw().contains("Conflict:"));
    assert!(diff_git.stdout.raw().contains("Git submodule"));

    let output = work_dir.run_jj([
        "submodule",
        "bind",
        "sub",
        "-r",
        &third_target,
        "--change",
        &source_change,
    ]);
    insta::assert_snapshot!(normalize_git_hashes(output), @r#"
    ------- stderr -------
    Updated Git submodule sub:
      old: conflicted
      new: <git-commit>
    Rewritten superproject commit: <jj-commit> (empty) update sub to second
    Existing conflicts were resolved or abandoned from 1 commits.
    [EOF]
    "#);

    let output = work_dir
        .run_jj(["diff", "-r", &source_change, "--summary"])
        .success();
    assert!(output.stdout.raw().is_empty());
    assert!(output.stderr.raw().is_empty());
}

#[test]
fn test_submodule_bind_revision_completion_uses_jj_submodule() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["bookmark", "create", "super-target", "-r", "@"])
        .success();

    work_dir.create_dir("sub");
    test_env
        .run_jj_in("repo/sub", ["git", "init", "--colocate", "."])
        .success();
    let submodule_dir = test_env.work_dir("repo/sub");
    submodule_dir.write_file("file", "contents");
    submodule_dir
        .run_jj(["commit", "-m", "submodule commit"])
        .success();
    submodule_dir
        .run_jj(["bookmark", "create", "sub-target", "-r", "@-"])
        .success();

    let values = machine_completion_values(work_dir.run_jj([
        "util",
        "complete",
        "--index",
        "5",
        "--",
        "jj",
        "submodule",
        "bind",
        "sub",
        "-r",
        "sub-",
    ]));
    assert!(values.contains(&"sub-target".to_owned()));
    assert!(!values.contains(&"super-target".to_owned()));
}

#[test]
fn test_submodule_bind_revision_completion_uses_git_submodule() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["bookmark", "create", "super-target", "-r", "@"])
        .success();

    work_dir.create_dir("sub");
    work_dir.write_file("sub/file", "contents");
    run_git(&work_dir, ["-C", "sub", "init"]);
    run_git(&work_dir, ["-C", "sub", "add", "file"]);
    run_git(
        &work_dir,
        [
            "-C",
            "sub",
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test user",
            "commit",
            "-m",
            "submodule commit",
        ],
    );
    run_git(&work_dir, ["-C", "sub", "branch", "sub-target"]);

    let values = machine_completion_values(work_dir.run_jj([
        "util",
        "complete",
        "--index",
        "5",
        "--",
        "jj",
        "submodule",
        "bind",
        "sub",
        "-r",
        "sub-",
    ]));
    assert!(values.contains(&"sub-target".to_owned()));
    assert!(!values.contains(&"super-target".to_owned()));
}

#[test]
fn test_submodule_bind_at_resolves_in_jj_submodule() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "submodule"])
        .success();
    let submodule_dir = test_env.work_dir("submodule");
    submodule_dir.write_file("file", "contents");
    submodule_dir
        .run_jj(["commit", "-m", "submodule commit"])
        .success();
    let old_target = submodule_dir
        .run_jj(["log", "-r@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let submodule_url = format!("{}/submodule", test_env.env_root().display());
    run_git(
        &work_dir,
        [
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_url.as_str(),
            "sub",
        ],
    );
    run_git(&work_dir, ["-C", "sub", "checkout", &old_target]);
    run_git(&work_dir, ["add", "sub"]);
    run_git(
        &work_dir,
        [
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test user",
            "commit",
            "-m",
            "add submodule",
        ],
    );
    insta::assert_snapshot!(work_dir.run_jj(["diff", "--summary"]), @r#"
    ------- stderr -------
    ignoring git submodule at "sub"
    Done importing changes from the underlying Git repo.
    [EOF]
    "#);

    test_env
        .run_jj_in("repo/sub", ["git", "init", "--git-repo=."])
        .success();
    let jj_wc_target = test_env
        .run_jj_in("repo/sub", ["log", "-r@", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .raw()
        .trim()
        .to_owned();
    assert_ne!(jj_wc_target, old_target);

    let output = work_dir.run_jj(["submodule", "bind", "sub", "-r", "@"]);
    insta::assert_snapshot!(normalize_git_hashes(output), @r#"
    ------- stderr -------
    Updated Git submodule sub:
      old: <git-commit>
      new: <git-commit>
    Rewritten superproject commit: <jj-commit> (no description set)
    ignoring git submodule at "sub"
    Working copy  (@) now at: <jj-commit> (no description set)
    Parent commit (@-)      : <jj-commit> master | add submodule
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    "#);

    let diff_git = work_dir.run_jj(["diff", "--git"]);
    assert!(diff_git.stdout.raw().contains(&jj_wc_target[..10]));
    insta::assert_snapshot!(normalize_git_hashes(diff_git), @r#"
    diff --git a/sub b/sub
    index <git-commit>..<git-commit> 160000
    [EOF]
    "#);
}
