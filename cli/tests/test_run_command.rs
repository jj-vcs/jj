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

use insta::assert_snapshot;

use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_run_simple() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    test_env.add_paths_to_normalize(fake_formatter.clone(), "$FAKE_FORMATTER_PATH");
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "test to replace");
    work_dir.run_jj(&["commit", "-m", "C"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyrovkzqrwmxqlsskqntxvp
    ○  kkmpptxzrspxrzommnulwmwkkqwworplC
    │
    ○  rlvkpnrzqnoowoytxnquwvuryrwnrmlpB
    │
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnuA
    │
    ◆  zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    [EOF]
    ");
    // `--tee touched.txt` creates a file in each working copy, so every commit's
    // tree gets rewritten.
    let stdout = work_dir
        .run_jj(&[
            "run",
            "-r",
            "..@",
            "--",
            &fake_formatter_path,
            "--stdout",
            "x",
            "--tee",
            "touched.txt",
        ])
        .success()
        .stdout;
    insta::assert_snapshot!(stdout, @"xxxx[EOF]");
}

#[test]
fn test_run_on_immutable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy();
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "test to replace");
    work_dir.run_jj(&["commit", "-m", "C"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyrovkzqrwmxqlsskqntxvp
    ○  kkmpptxzrspxrzommnulwmwkkqwworplC
    │
    ○  rlvkpnrzqnoowoytxnquwvuryrwnrmlpB
    │
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnuA
    │
    ◆  zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    [EOF]
    ");
    let output = work_dir.run_jj(&[
        "run",
        "-r",
        "all()",
        "--",
        &fake_formatter_path,
        "--uppercase",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_run_noop() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    test_env.add_paths_to_normalize(fake_formatter.clone(), "$FAKE_FORMATTER_PATH");
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "test to replace");
    work_dir.run_jj(&["commit", "-m", "C"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyrovkzqrwmxqlsskqntxvp
    ○  kkmpptxzrspxrzommnulwmwkkqwworplC
    │
    ○  rlvkpnrzqnoowoytxnquwvuryrwnrmlpB
    │
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnuA
    │
    ◆  zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    [EOF]
    ");
    // `--stdout foo` writes to the subprocess's stdout, which `jj run` buffers
    // and emits to its own stdout. No tracked files in the working copy change,
    // so no commits get rewritten. Using a fixed string keeps the per-commit
    // output identical, so the concatenated stdout is stable regardless of the
    // (non-deterministic) order in which the parallel jobs finish.
    let output = work_dir
        .run_jj(&[
            "run",
            "-r",
            "..@",
            "--",
            &fake_formatter_path,
            "--stdout",
            "foo",
        ])
        .success();
    insta::assert_snapshot!(output.stdout, @"foofoofoofoo[EOF]");
    insta::assert_snapshot!(output.stderr, @r"
    No commits were rewritten as the command did not modify any tracked files
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_run_sets_env_vars() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Show the change_id and commit_id so the reader can match them against
    // the values the subprocess writes into the per-commit working copy.
    let log_template = r#"change_id ++ " " ++ commit_id ++ " " ++ description ++ "\n""#;
    insta::assert_snapshot!(
        work_dir.run_jj(&["log", "-T", log_template]),
        @r"
    @  rlvkpnrzqnoowoytxnquwvuryrwnrmlp fc4c875c9bc90128cbb9e8084dd5f5f336b383d9
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu 5fbe90560fed1c39d46a46a672ba98abd53bdc6d seed
    │
    ◆  zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz 0000000000000000000000000000000000000000
    [EOF]
    "
    );

    // Each subprocess echoes its JJ_CHANGE_ID and JJ_COMMIT_ID into files in
    // the per-commit working copy, modifying the tree so the commit gets
    // rewritten with those files.
    let jj_args: &[&str] = if cfg!(windows) {
        &[
            "run",
            "-r",
            "@-",
            "--",
            "cmd",
            "/c",
            "echo %JJ_CHANGE_ID%>change_id.txt && echo %JJ_COMMIT_ID%>commit_id.txt",
        ]
    } else {
        &[
            "run",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "echo $JJ_CHANGE_ID > change_id.txt && echo $JJ_COMMIT_ID > commit_id.txt",
        ]
    };
    work_dir.run_jj(jj_args).success();

    let normalize_whitespace = |s: String| {
        s.replace("\r\n", "\n")
            .lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "show", "-r", "@-", "change_id.txt"])
            .normalize_stdout_with(normalize_whitespace),
        @r"
    qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "show", "-r", "@-", "commit_id.txt"])
            .normalize_stdout_with(normalize_whitespace),
        @r"
    5fbe90560fed1c39d46a46a672ba98abd53bdc6d
    [EOF]
    "
    );
}

#[test]
fn test_run_from_subdir_skips_commits_without_it() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // `fake-formatter --tee ran.txt` is a portable way to create an empty
    // `ran.txt`, equivalent to `touch ran.txt` but available on all platforms.
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    test_env.add_paths_to_normalize(fake_formatter.clone(), "$FAKE_FORMATTER_PATH");
    let work_dir = test_env.work_dir("repo");

    // First commit has only root-level files; no `sub/` exists yet.
    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "no-sub"]).success();
    // Second commit adds `sub/file.txt`, so `sub/` exists from here on.
    work_dir.write_file("sub/file.txt", "x");
    work_dir.run_jj(&["commit", "-m", "with-sub"]).success();

    // Run from inside sub/ on both ancestors. The command creates `ran.txt`
    // in cwd, so we can later tell where it ran. The `no-sub` commit has no
    // `sub/` directory and should be skipped; the `with-sub` commit has
    // `sub/` and should be rewritten with `sub/ran.txt` added.
    let sub_dir = work_dir.dir("sub");
    let output = sub_dir
        .run_jj(&[
            "run",
            "-r",
            "@-|@--",
            "--",
            &fake_formatter_path,
            "--tee",
            "ran.txt",
        ])
        .success()
        .normalize_backslash();
    insta::assert_snapshot!(output.stderr, @r"
    Skipped commit 3bb1f1ca3c09a8e6be46ef48515803464b16b426: directory does not exist: sub
    Rewrote 1 commits with $FAKE_FORMATTER_PATH --tee ran.txt
    Working copy  (@) now at: kkmpptxz 3548431a (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 3aa9a235 with-sub
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // The rewritten `with-sub` commit has `sub/ran.txt`, alongside the
    // pre-existing `sub/file.txt`.
    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "list", "-r", "@-"])
            .normalize_backslash(),
        @r"
    seed.txt
    sub/file.txt
    sub/ran.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_root_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // `fake-formatter --tee ran.txt` is a portable way to create an empty
    // `ran.txt`, equivalent to `touch ran.txt` but available on all platforms.
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("sub/file.txt", "x");
    work_dir.run_jj(&["commit", "-m", "with-sub"]).success();

    // Invoke `jj run` from inside sub/, but pass `--root` so the command
    // executes from the workspace root and `ran.txt` lands at the top level.
    let sub_dir = work_dir.dir("sub");
    sub_dir
        .run_jj(&[
            "run",
            "--root",
            "-r",
            "@-",
            "--",
            &fake_formatter_path,
            "--tee",
            "ran.txt",
        ])
        .success();

    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "list", "-r", "@-"])
            .normalize_backslash(),
        @r"
    ran.txt
    sub/file.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_uses_revsets_run_as_default() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // `fake-formatter --tee ran.txt` is a portable way to create an empty
    // `ran.txt`, equivalent to `touch ran.txt` but available on all platforms.
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");

    // Two sibling commits, `foo` and `bar`.
    work_dir.write_file("file", "foo");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file", "bar");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bar"])
        .success();
    work_dir.run_jj(["edit", "foo"]).success();

    test_env.add_config(r#"revsets.run = "bar""#);

    // Running `jj run` with `revsets.run=bar` should only modify bar
    work_dir
        .run_jj([
            "--config=revsets.run=\"bar\"",
            "run",
            "--",
            &fake_formatter_path,
            "--tee",
            "ran.txt",
        ])
        .success();

    insta::assert_snapshot!(
        work_dir.run_jj(["file", "list", "-r", "foo"]),
        @r"
    file
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir.run_jj(["file", "list", "-r", "bar"]),
        @r"
    file
    ran.txt
    [EOF]
    "
    );

    // Run again but now with foo in the config
    work_dir.run_jj(["undo"]).success();
    work_dir
        .run_jj([
            "--config=revsets.run=\"foo\"",
            "run",
            "--",
            &fake_formatter_path,
            "--tee",
            "ran.txt",
        ])
        .success();

    insta::assert_snapshot!(
        work_dir.run_jj(["file", "list", "-r", "foo"]),
        @r"
    file
    ran.txt
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir.run_jj(["file", "list", "-r", "bar"]),
        @r"
    file
    [EOF]
    "
    );
}

#[test]
fn test_run_failure_rewrites_nothing() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    let log_before = get_log_output(&work_dir);
    insta::assert_snapshot!(log_before, @r"
    @  kkmpptxzrspxrzommnulwmwkkqwworpl
    ○  rlvkpnrzqnoowoytxnquwvuryrwnrmlpB
    │
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnuA
    │
    ◆  zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    [EOF]
    ");

    // Fail on commit B; succeed (modify the tree) on every other commit. If
    // any subprocess fails, `jj run` must roll back: no commit gets rewritten,
    // even the ones whose commands ran to completion before B's failure
    // propagated.
    let cmd = "if [ \"$JJ_CHANGE_ID\" = 'rlvkpnrzqnoowoytxnquwvuryrwnrmlp' ]; then exit 1; fi; \
               touch ran.txt";
    let output = work_dir.run_jj(&["run", "-r", "..@", "--", "sh", "-c", cmd]);
    assert!(!output.status.success(), "expected `jj run` to fail");

    // Log is unchanged: same change_ids, same shape, no descendants of B got
    // rebased onto a new commit.
    assert_eq!(get_log_output(&work_dir), log_before);
}

#[test]
fn test_run_recovers_after_failure() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    // `fake-formatter --fail` exits non-zero (like `false`) and
    // `fake-formatter --tee ran.txt` creates an empty `ran.txt` (like `touch`);
    // both are portable across platforms.
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();

    // First run fails outright on every commit, leaving the per-commit
    // working copies in `.jj/run/default/` behind.
    let first = work_dir.run_jj(&["run", "-r", "..@", "--", &fake_formatter_path, "--fail"]);
    assert!(!first.status.success(), "expected first `jj run` to fail");

    // A second run with a working command must succeed despite those leftover
    // directories — `jj run` clears each per-commit dir before reusing it.
    work_dir
        .run_jj(&[
            "run",
            "-r",
            "..@",
            "--",
            &fake_formatter_path,
            "--tee",
            "ran.txt",
        ])
        .success();

    // Both commits in `..@` now carry `ran.txt`.
    insta::assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "@-"]),
        @r"
    A.txt
    b.txt
    ran.txt
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "@--"]),
        @r"
    A.txt
    ran.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_shell_command() {
    // The new positional-args interface means users have to invoke a shell
    // explicitly to use shell features. This verifies that path works
    // end-to-end: each per-commit subprocess sees its `JJ_COMMIT_ID` and the
    // shell echoes it to stdout.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "test to replace");
    work_dir.run_jj(&["commit", "-m", "C"]).success();

    // Show the commit_ids so the reader can match them against the values
    // the snapshot below was captured with.
    let log_template = r#"change_id ++ " " ++ commit_id ++ " " ++ description ++ "\n""#;
    insta::assert_snapshot!(
        work_dir.run_jj(&["log", "-T", log_template, "-r", "..@"]),
        @r"
    @  zsuskulnrvyrovkzqrwmxqlsskqntxvp 8d0cb96bac2cfefd56a8691b9301ef44cc94a368
    ○  kkmpptxzrspxrzommnulwmwkkqwworpl 3406218c99ce8076f3a28434ebda109cbd84de9e C
    │
    ○  rlvkpnrzqnoowoytxnquwvuryrwnrmlp 9453b0f03bbda20fa849b10eb051d1e3eed1ec5d B
    │
    ○  qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu 26d8ff9bba4faa4da6735ced959c57280e49afa7 A
    │
    ~
    [EOF]
    "
    );

    let jj_args: &[&str] = if cfg!(windows) {
        &["run", "-r", "..@", "--", "cmd", "/c", "echo %JJ_COMMIT_ID%"]
    } else {
        &[
            "run",
            "-r",
            "..@",
            "--",
            "bash",
            "-c",
            r#"echo "$JJ_COMMIT_ID""#,
        ]
    };
    let output = work_dir.run_jj(jj_args).success();

    // Parallel jobs finish in non-deterministic order, so sort before
    // asserting.
    let mut lines: Vec<&str> = output.stdout.raw().lines().collect();
    lines.sort_unstable();
    let sorted_stdout = lines.join("\n");
    insta::assert_snapshot!(sorted_stdout, @r"
    26d8ff9bba4faa4da6735ced959c57280e49afa7
    3406218c99ce8076f3a28434ebda109cbd84de9e
    8d0cb96bac2cfefd56a8691b9301ef44cc94a368
    9453b0f03bbda20fa849b10eb051d1e3eed1ec5d
    ");
}

#[test]
fn test_run_sets_workspace_root_env_var() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Each subprocess writes $JJ_WORKSPACE_ROOT into a file so we can assert
    // it equals the actual workspace root (not the per-commit working copy).
    let jj_args: &[&str] = if cfg!(windows) {
        &[
            "run",
            "-r",
            "@-",
            "--",
            "cmd",
            "/c",
            "echo %JJ_WORKSPACE_ROOT%>workspace_root.txt",
        ]
    } else {
        &[
            "run",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "echo $JJ_WORKSPACE_ROOT > workspace_root.txt",
        ]
    };
    work_dir.run_jj(jj_args).success();

    // Trim trailing whitespace per line and normalize CRLF to LF so the
    // snapshot is identical on Windows and Unix.
    let normalize_whitespace = |s: String| {
        s.replace("\r\n", "\n")
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    // $TEST_ENV is the normalized placeholder for the test environment's temp
    // root directory. JJ_WORKSPACE_ROOT should point to the per-commit
    // isolated working copy under .jj/run/default/<commit-id>/working_copy,
    // not to the original workspace.
    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "show", "-r", "@-", "workspace_root.txt"])
            .normalize_stdout_with(normalize_whitespace),
        @r"
    $TEST_ENV/repo/.jj/run/default/5fbe90560fed1c39d46a46a672ba98abd53bdc6d/working_copy
    [EOF]
    "
    );
}

#[test]
fn test_run_failure_shows_output() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();

    let output = work_dir.run_jj(&[
        "run",
        "-r",
        "@-",
        "--",
        &fake_formatter_path,
        "--stdout",
        "hello stdout\n",
        "--stderr",
        "hello stderr\n",
        "--fail",
    ]);
    assert!(!output.status.success());
    insta::with_settings!({
        filters => [
            ("exit code", "exit status"), // Windows
        ],
    }, {
        insta::assert_snapshot!(&output.normalize_stderr_with(|stderr| stderr.replace(&fake_formatter_path.clone(), "fake-formatter")), @r"
        hello stdout
        [EOF]
        ------- stderr -------
        hello stderr
        Error: the command 'fake-formatter --stdout hello stdout
         --stderr hello stderr
         --fail' failed with exit status: 1 for commit 26d8ff9bba4faa4da6735ced959c57280e49afa7
        [EOF]
        [exit status: 1]
        ");
    });
}

/// Changes made to an ancestor commit must still propagate into descendant
/// commits after rebasing.
#[test]
fn test_run_parallel_changes_propagate_to_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("a.txt", "a");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "c");
    work_dir.run_jj(&["commit", "-m", "C"]).success();

    // Add a unique file to each change
    work_dir
        .run_jj(&[
            "run",
            "--jobs=3",
            "-r=..@",
            "--",
            "sh",
            "-c",
            "touch newfile-$JJ_CHANGE_ID.txt",
        ])
        .success();

    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r=@---"]).success().stdout,@r"
    a.txt
    newfile-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
    [EOF]
    ");
    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r=@--"]).success().stdout,@r"
    a.txt
    b.txt
    newfile-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
    newfile-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
    [EOF]
    ");
    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r=@-"]).success().stdout,@r"
    a.txt
    b.txt
    c.txt
    newfile-kkmpptxzrspxrzommnulwmwkkqwworpl.txt
    newfile-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
    newfile-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
    [EOF]
    ");
}

fn get_log_output(work_dir: &TestWorkDir) -> String {
    work_dir
        .run_jj(&["log", "-T", r#"change_id ++ description ++ "\n""#])
        .success()
        .stdout
        .to_string()
}
