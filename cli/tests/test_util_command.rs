// Copyright 2023 The Jujutsu Authors
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

use std::fs;
use std::process::Command;

use insta::assert_snapshot;
use test_case::test_case;
use testutils::TestRepoBackend;
use testutils::TestResult;
use testutils::TestWorkspace;

use crate::common::TestEnvironment;

#[test_case(TestRepoBackend::Simple, "Simple" ; "simple backend")]
#[test_case(TestRepoBackend::Git, "git" ; "git backend")]
fn test_util_backend_name(backend: TestRepoBackend, expected_name: &str) {
    let test_env = TestEnvironment::default();
    let test_workspace = TestWorkspace::init_with_backend(backend);
    let root = test_workspace.workspace.workspace_root();
    let output = test_env
        .run_jj_in(&root, ["util", "backend", "name"])
        .success();
    assert_eq!(output.stdout.raw(), &[expected_name, "\n"].concat());
}

#[test]
fn test_util_config_schema() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "config-schema"]);
    // Validate partial snapshot, redacting any lines nested 2+ indent levels.
    insta::with_settings!({filters => vec![(r"(?m)(^        .*$\r?\n)+", "        [...]\n")]}, {
        assert_snapshot!(output, @r#"
        {
            "$schema": "http://json-schema.org/draft-04/schema",
            "$comment": "`taplo` and the corresponding VS Code plugins only support version draft-04 of JSON Schema, see <https://taplo.tamasfe.dev/configuration/developing-schemas.html>. draft-07 is mostly compatible with it, newer versions may not be.",
            "title": "Jujutsu config",
            "type": "object",
            "description": "User configuration for Jujutsu VCS. See https://docs.jj-vcs.dev/latest/config/ for details",
            "properties": {
                [...]
            }
        }
        [EOF]
        "#);
    });
}

#[test]
fn test_gc_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The Git object phase must not fall back to a configured Git executable.
    let output = work_dir.run_jj([
        "--config=git.executable-path=/nonexistent-git",
        "util",
        "gc",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Git objects repacked (188 pack bytes, 1128 index bytes); reclamation deferred.
    [EOF]
    ");

    let output = work_dir.run_jj(["util", "gc", "--at-op=@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot garbage collect from a non-head operation
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["util", "gc", "--expire=foobar"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --expire only accepts 'now'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gc_operation_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create an operation.
    work_dir.write_file("file", "a change\n");
    work_dir.run_jj(["commit", "-m", "a change"]).success();
    let op_to_remove = work_dir.current_operation_id();

    // Make another operation the head.
    work_dir.write_file("file", "another change\n");
    work_dir
        .run_jj(["commit", "-m", "another change"])
        .success();

    // This works before the operation is removed.
    work_dir
        .run_jj(["debug", "object", "operation", &op_to_remove])
        .success();

    // Remove some operations.
    work_dir.run_jj(["operation", "abandon", "..@-"]).success();
    work_dir.run_jj(["util", "gc", "--expire=now"]).success();

    // Now this doesn't work.
    let output = work_dir.run_jj(["debug", "object", "operation", &op_to_remove]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @"
    ------- stderr -------
    Internal error: Failed to load an operation
    Caused by:
    1: Object 2bf28acab8bf15817f17f696d4483584440e13d155280691c7dd5abfa2e7e9b67258f06022cfdd5b86295cbfcfd42e9132740b8d3a75d18c2b0718b45f69d8d9 of type operation not found
    2: Cannot access $TEST_ENV/repo/.jj/repo/op_store/operations/2bf28acab8bf15817f17f696d4483584440e13d155280691c7dd5abfa2e7e9b67258f06022cfdd5b86295cbfcfd42e9132740b8d3a75d18c2b0718b45f69d8d9
    [EOF]
    [exit status: 255]
    ");
}

#[test_case("sha1" ; "sha1")]
#[test_case("sha256" ; "sha256")]
fn test_gc_preserves_external_git_object_and_alternate(object_hash: &str) -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "init",
                "--colocate",
                "--object-hash",
                object_hash,
                "repo",
            ],
        )
        .success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("tracked", "jj commit\n");
    work_dir
        .run_jj(["commit", "-m", "Retain this commit"])
        .success();

    // An independent Git writer publishes an object without a Git ref. Another repository
    // borrows the object store, so a jj lock cannot authorize deletion of that object.
    let external = work_dir.root().parent().unwrap().join("external-blob");
    fs::write(&external, b"external Git bytes\0\xff")?;
    let output = Command::new("git")
        .current_dir(work_dir.root())
        .args(["hash-object", "-w"])
        .arg(&external)
        .output()?;
    assert!(output.status.success());
    let object_id = String::from_utf8(output.stdout)?.trim().to_owned();

    let alternate = work_dir.root().parent().unwrap().join("alternate");
    assert!(
        Command::new("git")
            .arg("init")
            .arg(format!("--object-format={object_hash}"))
            .arg(&alternate)
            .output()?
            .status
            .success()
    );
    fs::write(
        alternate.join(".git/objects/info/alternates"),
        format!("{}\n", work_dir.root().join(".git/objects").display()),
    )?;

    let output = work_dir
        .run_jj([
            "--config=git.executable-path=/nonexistent-git",
            "util",
            "gc",
        ])
        .success();
    assert!(output.stderr.raw().contains("reclamation deferred"));
    let output = Command::new("git")
        .current_dir(&alternate)
        .args(["cat-file", "-t", &object_id])
        .output()?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"blob\n");
    assert!(
        Command::new("git")
            .current_dir(work_dir.root())
            .args(["fsck", "--full", "--no-reflogs"])
            .output()?
            .status
            .success()
    );
    work_dir.run_jj(["log", "-r", "@-"]).success();
    Ok(())
}

#[test]
fn test_gc_defers_repack_when_pack_storage_exceeds_limit() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let pack_dir = work_dir.root().join(".git/objects/pack");
    fs::create_dir_all(&pack_dir)?;
    let marker = pack_dir.join("unrelated-storage");
    fs::File::create(&marker)?.set_len(4 * 1024 * 1024 * 1024 + 1)?;

    let output = work_dir
        .run_jj([
            "--config=git.executable-path=/nonexistent-git",
            "util",
            "gc",
        ])
        .success();
    assert!(output.stderr.raw().contains("reclamation deferred"));
    assert!(
        output
            .stderr
            .raw()
            .contains("4 GiB additive-repack threshold"),
        "{}",
        output.stderr.raw()
    );
    assert_eq!(fs::read_dir(&pack_dir)?.count(), 1);
    Ok(())
}

#[test]
fn test_shell_completions() {
    #[track_caller]
    fn test(shell: &str) {
        let test_env = TestEnvironment::default();
        let output = test_env
            .run_jj_in(".", ["util", "completion", shell])
            .success();
        // Ensures only stdout contains text
        assert!(
            !output.stdout.is_empty() && output.stderr.is_empty(),
            "{output}"
        );
    }

    test("bash");
    test("elvish");
    test("fish");
    test("nushell");
    test("power-shell");
    test("zsh");
}

#[test]
fn test_util_diff() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("").create_dir("work");

    // file1 == file2, != file3
    work_dir.write_file("file1", "foo\nbar\n");
    work_dir.write_file("file2", "foo\nbar\n");
    work_dir.write_file("file3", "foo\nbaz\n");

    let output = work_dir.run_jj(["util", "diff", "--git", "file1", "file2"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["util", "diff", "--git", "file1", "file3"]);
    insta::assert_snapshot!(output, @"
    diff --git a/file1 b/file3
    --- file1
    +++ file3
    @@ -1,2 +1,2 @@
     foo
    -bar
    +baz
    [EOF]
    ");

    let output = work_dir.run_jj(["util", "diff", "file1", "file3", "--color=debug"]);
    insta::assert_snapshot!(output, @"
    [38;5;3m<<diff color_words header::Modified file3 (file1 => file3):>>[39m
    [2m[38;5;1m<<diff color_words context removed line_number::   1>>[0m<<diff color_words context:: >>[2m[38;5;2m<<diff color_words context added line_number::   1>>[0m<<diff color_words context::: foo>>
    [38;5;1m<<diff color_words removed line_number::   2>>[39m<<diff color_words:: >>[38;5;2m<<diff color_words added line_number::   2>>[39m<<diff color_words::: >>[4m[38;5;1m<<diff color_words removed token::bar>>[38;5;2m<<diff color_words added token::baz>>[24m[39m<<diff color_words::>>
    [EOF]
    ");
}

#[test]
fn test_util_exec() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--append",
            "hello",
        ],
    );
    // Ensures only stdout contains text
    insta::assert_snapshot!(output, @"hello[EOF]");
}

#[test]
fn test_util_exec_fail() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--badopt",
        ],
    );
    // Ensures only stdout contains text
    insta::assert_snapshot!(output.normalize_stderr_with(|s| s.replace(".exe", "")), @"
    ------- stderr -------
    error: unexpected argument '--badopt' found

      tip: a similar argument exists: '--abort'

    Usage: fake-formatter --abort

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_util_exec_not_found() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "exec", "--", "jj-test-missing-program"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @"
    ------- stderr -------
    Error: Failed to execute external command 'jj-test-missing-program'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_util_exec_crash() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin!("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--abort",
        ],
    );

    if cfg!(unix) {
        insta::assert_snapshot!(output, @"
        ------- stderr -------
        Error: External command was terminated by signal: 15 (SIGTERM)
        [EOF]
        [exit status: 1]
        ");
    } else if cfg!(windows) {
        // abort produces STATUS_STACK_BUFFER_OVERRUN (0xc0000409)
        insta::assert_snapshot!(output, @r"
        [exit status: -1073740791]
        ");
    }
}

#[cfg(unix)]
#[test]
fn test_util_exec_sets_env() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let output = test_env.run_jj_in(
        ".",
        [
            "-R",
            "repo",
            "util",
            "exec",
            "--",
            "/bin/sh",
            "-c",
            r#"echo "$JJ_WORKSPACE_ROOT""#,
        ],
    );
    insta::assert_snapshot!(output, @"
    $TEST_ENV/repo
    [EOF]
    ");
}

#[test]
fn test_install_man_pages() -> TestResult {
    let test_env = TestEnvironment::default();

    // no man pages present
    let man_dir = test_env.env_root().join("man1");
    assert!(!man_dir.exists());

    // install man pages
    let output = test_env.run_jj_in(".", ["util", "install-man-pages", "."]);
    insta::assert_snapshot!(output, @"");

    // confirm something is now present
    assert!(man_dir.is_dir());
    assert!(fs::read_dir(man_dir)?.next().is_some());
    Ok(())
}

#[test]
fn test_util_snapshot() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("foo", "foo");

    let output = work_dir.run_jj(["util", "snapshot"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Snapshot complete.
    [EOF]
    ");
}

#[test]
fn test_util_snapshot_nothing_changed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["util", "snapshot"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No snapshot needed.
    [EOF]
    ");
}
