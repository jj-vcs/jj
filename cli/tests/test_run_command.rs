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

use std::fs;

use insta::assert_snapshot;

use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit_with_files;

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
    Rewrote 1 commits
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
fn test_run_ignore_errors_rewrites_successes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    create_commit_with_files(&work_dir, "a", &[], &[("a.txt", "a")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("b.txt", "b")]);

    // Fail on A; succeed on B. A should be untouched, B should be rewritten.
    let jj_args: &[&str] = if cfg!(windows) {
        &[
            "run",
            "--ignore-errors",
            "-r",
            "..b",
            "--",
            "cmd",
            "/c",
            "(if exist a.txt if not exist b.txt exit 1) & type nul >ran.txt",
        ]
    } else {
        &[
            "run",
            "--ignore-errors",
            "-r",
            "..b",
            "--",
            "sh",
            "-c",
            "if [ -f a.txt -a ! -f b.txt ]; then exit 1; fi; touch ran.txt",
        ]
    };
    let output = work_dir.run_jj(jj_args);
    assert_snapshot!(
        output.success(), @r"
    ------- stderr -------
    Rewrote 1 commits
    Working copy  (@) now at: zsuskuln f3c3b6ac b | b
    Parent commit (@-)      : rlvkpnrz 2385de9f a | a
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r=a"]),
        @r"
    a.txt
    [EOF]
    "
    );
    insta::assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r=b"]),
        @r"
    a.txt
    b.txt
    ran.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_ignore_errors_all_fail() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("A.txt", "A");
    work_dir.run_jj(&["commit", "-m", "A"]).success();

    let log_before = get_log_output(&work_dir);

    let output = work_dir.run_jj(&[
        "run",
        "--ignore-errors",
        "-r",
        "@-",
        "--",
        &fake_formatter_path,
        "--fail",
    ]);
    assert_snapshot!(
        output.success(), @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    "
    );

    // Log is unchanged: no commits were rewritten.
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

    // First run fails; snapshot+persist still runs, leaving each slot with a
    // valid tree_state on disk.
    let first = work_dir.run_jj(&["run", "-r", "..@", "--", &fake_formatter_path, "--fail"]);
    assert!(!first.status.success(), "expected first `jj run` to fail");

    // A second run with a working command must succeed: the persisted
    // tree_state lets each slot be reused without a wipe-and-reinit.
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
    // root directory. JJ_WORKSPACE_ROOT should point to the slot working copy
    // under .jj/run/default/1/working_copy, not to the original workspace.
    insta::assert_snapshot!(
        work_dir
            .run_jj(&["file", "show", "-r", "@-", "workspace_root.txt"])
            .normalize_stdout_with(normalize_whitespace),
        @r"
    $TEST_ENV/repo/.jj/run/default/1/working_copy
    [EOF]
    "
    );
}

/// After a failed `jj run`, the pool slot must be reusable: ignored files
/// (build artifacts) survive and non-ignored stray files are cleaned up.
#[test]
fn test_run_pool_reuses_slot_after_failure() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "cache/\n");
    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Failing command writes an ignored artifact and a non-ignored stray file.
    let first = work_dir.run_jj(&[
        "run",
        "--config",
        "run.jobs=1",
        "-r",
        "@-",
        "--",
        "sh",
        "-c",
        &format!(
            "mkdir -p cache && echo artifact > cache/artifact.txt && touch stray.txt && \
             {fake_formatter_path} --fail"
        ),
    ]);
    assert!(!first.status.success(), "expected first run to fail");

    let slot_state = work_dir.root().join(".jj/run/default/1/state/tree_state");
    assert!(
        slot_state.exists(),
        "tree_state must be persisted even after failure"
    );

    // Succeeding second run: ignored artifact must still be present on disk;
    // the non-ignored stray.txt must have been cleaned up (not leaked into
    // the new commit).
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "if [ -f cache/artifact.txt ]; then cp cache/artifact.txt result.txt; else echo \
             MISSING > result.txt; fi",
        ])
        .success();

    assert_snapshot!(
        work_dir.run_jj(&["file", "show", "-r", "@-", "result.txt"]),@r"
    artifact
    [EOF]
    "
    );
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "@-"]),
        @r"
    .gitignore
    result.txt
    seed.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_pool_persists_between_runs() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "touch",
            "ran.txt",
        ])
        .success();

    // The pool slot directory survives the run.
    let pool_slot = work_dir.root().join(".jj/run/default/1");
    assert!(
        pool_slot.exists(),
        "expected pool slot 1 to persist between runs at {pool_slot:?}",
    );
    assert!(pool_slot.join("working_copy").exists());
    assert!(pool_slot.join("state").exists());

    // A second run reuses the existing slot rather than recreating it from
    // scratch.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "touch",
            "ran2.txt",
        ])
        .success();
    assert!(pool_slot.join("working_copy").exists());
}

#[test]
fn test_run_pool_size_from_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("a.txt", "a");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "c");
    work_dir.run_jj(&["commit", "-m", "C"]).success();

    // `run.jobs = 1` forces all three commits to share a single slot
    // and be processed sequentially.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=2",
            "-r",
            "..@",
            "--",
            "sh",
            "-c",
            "touch ran-$JJ_CHANGE_ID.txt",
        ])
        .success();

    let pool_dir = work_dir.root().join(".jj/run/default");
    assert!(pool_dir.join("1").exists(), "pool/1 should exist");
    assert!(pool_dir.join("2").exists(), "pool/2 should exist");
    assert!(
        !pool_dir.join("3").exists(),
        "pool/3 should NOT exist with size=2",
    );

    // All three commits picked up `ran.txt`.
    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r", "@---"]), @r"
        a.txt
        ran-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
        [EOF]
        ");
    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r", "@--"]), @r"
        a.txt
        b.txt
        ran-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
        ran-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
        [EOF]
        ");
    assert_snapshot!(work_dir.run_jj(&["file", "list", "-r", "@-"]), @r"
        a.txt
        b.txt
        c.txt
        ran-kkmpptxzrspxrzommnulwmwkkqwworpl.txt
        ran-qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu.txt
        ran-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
        [EOF]
        ");
}

/// `--jobs N` controls pool size when `run.jobs` is not set.
#[test]
fn test_run_pool_size_from_jobs_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("a.txt", "a");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    work_dir.write_file("b.txt", "b");
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    work_dir.write_file("c.txt", "c");
    work_dir.run_jj(&["commit", "-m", "C"]).success();

    // `--jobs 2` with pool: expect exactly 2 slots, no more.
    work_dir
        .run_jj(&["run", "--jobs", "2", "-r", "..@", "--", "touch", "ran.txt"])
        .success();

    let pool_dir = work_dir.root().join(".jj/run/default");
    assert!(pool_dir.join("1").exists(), "pool/1 should exist");
    assert!(pool_dir.join("2").exists(), "pool/2 should exist");
    assert!(
        !pool_dir.join("3").exists(),
        "pool/3 must NOT exist with --jobs 2",
    );
}

#[test]
fn test_run_pool_preserves_untracked_artifacts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // The .gitignore keeps `cache/` out of the snapshot's tracking set, so
    // files there persist on disk between jobs without being committed.
    work_dir.write_file(".gitignore", "cache/\n");
    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Run 1: drop a marker into the gitignored cache directory.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "mkdir -p cache && echo run1 > cache/marker && touch ran1.txt",
        ])
        .success();

    // Run 2: assert the marker is still there, write its contents into a
    // tracked file so we can verify from outside.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "if [ -f cache/marker ]; then cp cache/marker result.txt; else echo MISSING > \
             result.txt; fi",
        ])
        .success();

    assert_snapshot!(
        work_dir
        .run_jj(&["file", "show", "-r", "@-", "result.txt"])
        .success()
        .stdout,
        @r"
        run1
        [EOF]
        ",
    );
}

/// Pool correctly removes a file that is present in one commit's tree but
/// absent in the next commit processed by the same slot.
#[test]
fn test_run_pool_removes_file_absent_in_next_commit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Commit A has only_in_a.txt.
    work_dir.write_file("only_in_a.txt", "a");
    work_dir.run_jj(&["commit", "-m", "A"]).success();
    // After commit -m A the WC inherits A's files. Delete the file so that
    // commit B's tree is empty (no only_in_a.txt).
    std::fs::remove_file(work_dir.root().join("only_in_a.txt")).unwrap();
    work_dir.run_jj(&["commit", "-m", "B"]).success();
    // Stack: root → A (only_in_a.txt) → B ({}) → @

    // Pool size 1 forces both commits through the same slot sequentially.
    // @-- = A, @- = B (WC is above B).
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@--::@-",
            "--",
            "touch",
            "ran.txt",
        ])
        .success();

    // A's rewrite (@--) keeps only_in_a.txt (and gains ran.txt).
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "@--"])
        .success()
        .stdout,
        @r"
        only_in_a.txt
        ran.txt
        [EOF]
        ",
    );

    // B's rewrite (@-) must NOT have only_in_a.txt leaked from A's slot.
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "@-"])
        .success()
        .stdout,
        @r"
        ran.txt
        [EOF]
        ",
    );
}

/// When a pool slot is reused, files left on disk that the previous commit's
/// .gitignore excluded must be removed before the next commit runs. Otherwise
/// the next commit (which may not ignore the same paths) would pick them up at
/// snapshot time and incorrectly add them to its rewritten tree.
#[test]
fn test_run_pool_removes_previously_ignored_files() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    let work_dir = test_env.work_dir("repo");

    // Commit a: no .gitignore.
    work_dir.write_file("seed.txt", "seed");
    work_dir.write_file(".gitignore", "always_ignored.txt\n");
    work_dir
        .run_jj(&["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.run_jj(&["commit", "-m", "a"]).success();
    // Commit b: adds a .gitignore that excludes `ignored.txt`.
    work_dir.write_file(".gitignore", "always_ignored.txt\nignored.txt\n");
    work_dir
        .run_jj(&["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.run_jj(&["commit", "-m", "b"]).success();

    let run_wc = work_dir.root().join(".jj/run/default/1/working_copy");

    // Leave two ignored files in the run workspace
    work_dir
        .run_jj(&[
            "run",
            "-r=b",
            "--",
            &fake_formatter_path,
            "--tee=always_ignored.txt",
            "--tee=ignored.txt",
        ])
        .success();
    assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "b"]),
        @r"
    .gitignore
    seed.txt
    [EOF]
    "
    );
    assert!(fs::exists(run_wc.join("always_ignored.txt")).unwrap());
    assert!(fs::exists(run_wc.join("ignored.txt")).unwrap());

    // Run against a version that only ignores always_ignored.txt, ensure that
    // ignored.txt is removed, not amended into 'a'.
    work_dir
        .run_jj(&[
            "run",
            "-r=a",
            "--",
            &fake_formatter_path,
            "--stdout",
            "done",
        ])
        .success();
    assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "a"]),
        @r"
    .gitignore
    seed.txt
    [EOF]
    "
    );
    // Remove files that are no longer ignored in the new revision, but don't
    // unnecessarily remove files that are still ignored
    assert!(fs::exists(run_wc.join("always_ignored.txt")).unwrap());
    assert!(!fs::exists(run_wc.join("ignored.txt")).unwrap());

    // Passing --clean removes even always-ignored files
    work_dir
        .run_jj(&[
            "run",
            "-r=a",
            "--clean",
            "--",
            &fake_formatter_path,
            "--stdout",
            "done",
        ])
        .success();
    assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "a"]),
        @r"
    .gitignore
    seed.txt
    [EOF]
    "
    );
    assert!(!fs::exists(run_wc.join("always_ignored.txt")).unwrap());
    assert!(!fs::exists(run_wc.join("ignored.txt")).unwrap());
}

/// A slot whose tree_state is absent (simulating a crash mid-job) should be
/// wiped and reinitialized on the next acquisition, not produce garbage.
#[test]
fn test_run_pool_recovers_from_missing_tree_state() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Prime the slot so working_copy/ exists.
    work_dir
        .run_jj(&["run", "--config", "run.jobs=1", "-r", "@-", "--", "true"])
        .success();

    // Simulate a crash: plant a stale file in the slot and delete tree_state.
    let slot = work_dir.root().join(".jj/run/default/1");
    std::fs::write(slot.join("working_copy/stale.txt"), "crash leftovers").unwrap();
    std::fs::remove_file(slot.join("state/tree_state")).unwrap();

    // A second run should wipe the stale state and produce a clean rewrite.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "touch",
            "ran.txt",
        ])
        .success();

    assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "@-"]).success().stdout,
        @r"
        ran.txt
        seed.txt
        [EOF]
        ",
    );
}

/// A failed command (non-zero exit) must not poison the pool slot for the
/// next `jj run` invocation.
#[test]
fn test_run_pool_failed_command_does_not_poison_slot() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Run 1: command fails, but writes a file first.
    drop(work_dir.run_jj(&[
        "run",
        "--config",
        "run.jobs=1",
        "-r",
        "@-",
        "--",
        "sh",
        "-c",
        "touch poison.txt; exit 1",
    ]));

    // Run 2: a clean command. poison.txt must not appear in the rewrite.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "touch",
            "ran.txt",
        ])
        .success();

    assert_snapshot!(
        work_dir.run_jj(&["file", "list", "-r", "@-"]).success().stdout,
        @r"
        ran.txt
        seed.txt
        [EOF]
        ",
    );
}

#[test]
fn test_run_restore_descendants_preserves_content() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    test_env.add_paths_to_normalize(fake_formatter.clone(), "$FAKE_FORMATTER_PATH");
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file", "a\n")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("file", "b\n")]);
    create_commit_with_files(&work_dir, "c", &["b"], &[("file", "c\n")]);

    let command = if cfg!(windows) {
        format!("{fake_formatter_path} --tee ran-%JJ_CHANGE_ID%.txt")
    } else {
        format!("{fake_formatter_path} --tee ran-$JJ_CHANGE_ID.txt")
    };
    let args: &[&str] = if cfg!(windows) {
        &[
            "run",
            "-r",
            "a::b",
            "--restore-descendants",
            "--",
            "cmd",
            "/c",
            command.as_str(),
        ]
    } else {
        &[
            "run",
            "-r",
            "a::b",
            "--restore-descendants",
            "--",
            "sh",
            "-c",
            command.as_str(),
        ]
    };
    let output = work_dir.run_jj(args).success();
    assert_snapshot!(output.stderr, @r"
    Rewrote 2 commits
    Rebased 1 descendant commits (while preserving their content)
    Working copy  (@) now at: royxmykx a741a7d3 c | c
    Parent commit (@-)      : zsuskuln 43c5a714 b | b
    [EOF]
    ");

    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "a"])
        .success()
        .stdout,
        @r"
    file
    ran-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
    [EOF]
    "
    );
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "b"])
        .success()
        .stdout,
        @r"
    file
    ran-zsuskulnrvyrovkzqrwmxqlsskqntxvp.txt
    [EOF]
    "
    );
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "c"])
        .success()
        .stdout,
        @r"
    file
    [EOF]
    "
    );

    assert_snapshot!(
        work_dir.run_jj(&["diff", "--from=a", "--to=b", "--git"]).success().stdout,
        @r"
    diff --git a/file b/file
    index 7898192261..6178079822 100644
    --- a/file
    +++ b/file
    @@ -1,1 +1,1 @@
    -a
    +b
    diff --git a/ran-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt b/ran-rlvkpnrzqnoowoytxnquwvuryrwnrmlp.txt
    deleted file mode 100644
    index e69de29bb2..0000000000
    diff --git a/ran-zsuskulnrvyrovkzqrwmxqlsskqntxvp.txt b/ran-zsuskulnrvyrovkzqrwmxqlsskqntxvp.txt
    new file mode 100644
    index 0000000000..e69de29bb2
    [EOF]
    "
    );
    assert_snapshot!(
        work_dir.run_jj(&["diff", "--from=b", "--to=c", "--git"]).success().stdout,
        @r"
    diff --git a/file b/file
    index 6178079822..f2ad6c76f0 100644
    --- a/file
    +++ b/file
    @@ -1,1 +1,1 @@
    -b
    +c
    diff --git a/ran-zsuskulnrvyrovkzqrwmxqlsskqntxvp.txt b/ran-zsuskulnrvyrovkzqrwmxqlsskqntxvp.txt
    deleted file mode 100644
    index e69de29bb2..0000000000
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

/// `--clean` wipes each slot before running the command so untracked artifacts
/// left by a previous run do not survive into the next run's working copy.
#[test]
fn test_run_clean_wipes_slot() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    let fake_formatter_path = fake_formatter.to_string_lossy();

    work_dir.write_file("seed.txt", "seed");
    work_dir.write_file(".gitignore", "ignored.txt");
    work_dir.run_jj(&["commit", "-m", "seed"]).success();

    // Prime slot 1 with a leftover untracked file to simulate a build artifact.
    work_dir
        .run_jj(&[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            &fake_formatter_path,
            "--tee",
            "ignored.txt",
        ])
        .success();

    // Without --clean, the leftover file is in the working copy
    let jj_args_no_clean: &[&str] = if cfg!(windows) {
        &[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "cmd",
            "/c",
            "if exist ignored.txt (echo PRESENT > saw_it.txt)",
        ]
    } else {
        &[
            "run",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "[ -e ignored.txt ] && echo PRESENT > saw_it.txt || true",
        ]
    };
    work_dir.run_jj(jj_args_no_clean).success();
    let files_no_clean = work_dir.run_jj(&["file", "list", "-r", "@-"]);
    // The run command saw it, and the leftover file is added
    assert_snapshot!(
        files_no_clean,
        @r"
    .gitignore
    saw_it.txt
    seed.txt
    [EOF]
    ",
    );

    // With --clean, the ignored file is gone
    work_dir.run_jj(&["undo"]).success();

    let jj_args_clean: &[&str] = if cfg!(windows) {
        &[
            "run",
            "--clean",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "cmd",
            "/c",
            "if exist ignored.txt (echo PRESENT > saw_it.txt)",
        ]
    } else {
        &[
            "run",
            "--clean",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "sh",
            "-c",
            "[ -e ignored.txt ] && echo PRESENT > saw_it.txt || true",
        ]
    };
    work_dir.run_jj(jj_args_clean).success();

    let files_clean = work_dir.run_jj(&["file", "list", "-r", "@-"]);
    assert_snapshot!(
        files_clean,
        @r"
    .gitignore
    seed.txt
    [EOF]
    ",
    );
}

#[test]
fn test_run_no_edit_does_not_rewrite() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let fake_formatter = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(fake_formatter.is_file());
    let fake_formatter_path = fake_formatter.to_string_lossy().into_owned();
    test_env.add_paths_to_normalize(fake_formatter.clone(), "$FAKE_FORMATTER_PATH");
    let work_dir = test_env.work_dir("repo");

    crate::common::create_commit_with_files(&work_dir, "a", &[], &[("file.txt", "a\n")]);
    crate::common::create_commit_with_files(&work_dir, "b", &["a"], &[("file.txt", "b\n")]);

    let log_before = get_log_output(&work_dir);

    // --tee touched.txt writes a new file in each working copy. Without
    // --no-edit this would amend the commits; with it nothing is rewritten.
    let output = work_dir
        .run_jj(&[
            "run",
            "--no-edit",
            "--config",
            "run.jobs=1",
            "-r",
            "a",
            "--",
            &fake_formatter_path,
            "--tee",
            "touched.txt",
        ])
        .success();
    insta::assert_snapshot!(output.stderr, @r"
    Nothing changed.
    [EOF]
    ");

    // Log must be identical to before.
    assert_eq!(get_log_output(&work_dir), log_before);

    // touched.txt must not appear in commit a's tree.
    assert_snapshot!(
        work_dir
        .run_jj(&["file", "list", "-r", "a"])
        .success(),
        @r"
    file.txt
    [EOF]
    "
    );
}

#[test]
fn test_run_no_edit_does_not_leak_between_invocations() {
    // Mirrors test_run_pool_no_file_leak_between_invocations but run 1 uses
    // --no-edit. The pool slot's tree_state is still updated by the snapshot
    // during run 1, so run 2 correctly removes artifact.txt from the slot
    // before checking out commit1.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("seed.txt", "seed");
    work_dir.run_jj(&["commit", "-m", "commit1"]).success();
    work_dir.run_jj(&["commit", "-m", "commit2"]).success();

    // Run 1 (--no-edit): writes artifact.txt in the pool slot for commit2 but
    // does not amend any commit. The slot's tree_state must record artifact.txt
    // so run 2 can remove it when checking out a different tree.
    let touch_cmd: &[&str] = if cfg!(windows) {
        &[
            "run",
            "--no-edit",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "cmd",
            "/c",
            "echo. > artifact.txt",
        ]
    } else {
        &[
            "run",
            "--no-edit",
            "--config",
            "run.jobs=1",
            "-r",
            "@-",
            "--",
            "touch",
            "artifact.txt",
        ]
    };
    work_dir.run_jj(touch_cmd).success();

    // Run 2: no-op on commit1 (@--) using the same pool slot that held
    // artifact.txt. The checkout diff should remove artifact.txt from the slot.
    work_dir
        .run_jj(&["run", "--config", "run.jobs=1", "-r", "@--", "--", "true"])
        .success();

    let files = work_dir
        .run_jj(&["file", "list", "-r", "@--"])
        .success()
        .stdout
        .to_string();
    assert!(
        !files.contains("artifact.txt"),
        "artifact.txt must not leak into commit1 from --no-edit slot state; got:\n{files}",
    );
    assert!(
        files.contains("seed.txt"),
        "expected seed.txt in commit1; got:\n{files}",
    );
}

#[test]
fn test_run_no_edit_works_on_immutable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commits and bookmark BEFORE configuring immutable_heads, so
    // setup commands don't trip over the missing revset.
    work_dir.write_file("file.txt", "content");
    work_dir.run_jj(&["commit", "-m", "the commit"]).success();
    work_dir
        .run_jj(&["bookmark", "create", "-r", "@-", "main"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);

    // Without --no-edit, running on an immutable commit must fail.
    let err_output = work_dir.run_jj(&["run", "-r", "main", "--", "true"]);
    assert!(
        !err_output.status.success(),
        "expected failure on immutable commit"
    );
    let stderr = err_output.stderr.to_string();
    assert!(
        stderr.contains("immutable"),
        "expected 'immutable' in error; got:\n{stderr}",
    );

    // With --no-edit the same commit is allowed.
    work_dir
        .run_jj(&["run", "--no-edit", "-r", "main", "--", "true"])
        .success();
}

#[test]
fn test_run_no_edit_conflicts_with_restore_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(&["run", "--no-edit", "--restore-descendants", "--", "true"]);
    assert!(!output.status.success());
    insta::assert_snapshot!(output.stderr, @r"
    error: the argument '--no-edit' cannot be used with '--restore-descendants'

    Usage: jj run --no-edit <COMMAND> [ARGS]...

    For more information, try '--help'.
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
