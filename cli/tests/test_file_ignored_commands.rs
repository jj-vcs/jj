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

use insta::assert_snapshot;

use crate::common::TestEnvironment;

#[test]
fn test_file_list_ignored() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.ignored\nignored-root/\n");
    // Untracked files ignored by .gitignore patterns.
    work_dir.write_file("ignored-root/only-ignored/file.txt", "content");
    work_dir.write_file("root-ignored.ignored", "content");

    // Fully ignored directories are listed as a single collapsed entry (with
    // trailing separator), and files matching an ignore pattern are listed
    // individually.
    let output = work_dir.run_jj(["file", "list-ignored"]);
    assert_snapshot!(output, @"
    ignored-root/
    root-ignored.ignored
    [EOF]
    ");
}

#[test]
fn test_file_list_ignored_empty_directory() {
    // Regression test: an empty directory that matches a directory-level
    // `.gitignore` pattern should still be listed, even though it has no
    // content to recurse into.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "empty-ignored/\n");
    work_dir.create_dir("empty-ignored");

    let output = work_dir.run_jj(["file", "list-ignored"]);
    assert_snapshot!(output, @"
    empty-ignored/
    [EOF]
    ");
}

#[test]
fn test_file_list_ignored_tracked_files_not_listed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.ignored\n");
    work_dir.write_file("tracked.ignored", "content");
    work_dir
        .run_jj(["file", "track", "--include-ignored", "tracked.ignored"])
        .success();
    work_dir.run_jj(["commit", "-m", "setup"]).success();

    // A tracked file that matches the ignore patterns is not listed.
    work_dir.write_file("untracked.ignored", "content");
    let output = work_dir.run_jj(["file", "list-ignored"]);
    assert_snapshot!(output, @"
    untracked.ignored
    [EOF]
    ");
}

#[test]
fn test_file_remove_ignored() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.ignored\nignored-root/\n");
    work_dir.write_file("tracked-file.txt", "content");
    work_dir.write_file("ignored-root/only-ignored/file.txt", "content");
    work_dir
        .run_jj(["file", "track", "tracked-file.txt"])
        .success();
    work_dir.run_jj(["commit", "-m", "setup"]).success();

    // Add more ignored files after the commit.
    work_dir.write_file("root-ignored.ignored", "content");
    work_dir.write_file("ignored-root/late.ignored", "content");

    // Removal deletes ignored files and directories from the working copy, but
    // leaves tracked files and .gitignore intact. `ignored-root` has no
    // tracked files underneath, so it's removed as a single collapsed unit
    // (matching what `file list-ignored` shows), rather than only deleting
    // its individual contents and leaving an empty directory behind.
    let output = work_dir.run_jj(["file", "remove-ignored"]);
    assert_snapshot!(output, @"
    ------- stderr -------
    Removed 2 ignored file(s) from the working copy.
    [EOF]
    ");
    assert!(!work_dir.root().join("root-ignored.ignored").exists());
    assert!(!work_dir.root().join("ignored-root").exists());
    assert!(!work_dir.root().join("ignored-root/only-ignored").exists());
    assert!(!work_dir.root().join("ignored-root/late.ignored").exists());
    assert!(work_dir.root().join("tracked-file.txt").exists());
    assert!(work_dir.root().join(".gitignore").exists());

    // A second run reports nothing to remove.
    let output2 = work_dir.run_jj(["file", "remove-ignored"]);
    assert_snapshot!(output2, @"
    ------- stderr -------
    Removed 0 ignored file(s) from the working copy.
    [EOF]
    ");
}

#[test]
fn test_file_remove_ignored_removes_now_empty_directory() {
    // Regression test: a directory that has no dedicated `.gitignore` pattern
    // of its own, but ends up containing only individually-ignored files
    // (e.g. via a `*.log` pattern), should be removed entirely by
    // `remove-ignored` instead of being left behind as an empty directory.
    // The two files are removed individually (not collapsed into a single
    // `logs` entry, since `logs/` isn't itself covered by a `.gitignore`
    // pattern and could in principle contain an unrelated, not-yet-ignored
    // file that must not be touched); the now-empty `logs` directory is then
    // cleaned up as a safe side effect, without being counted separately.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.log\n");
    work_dir.write_file("tracked-file.txt", "content");
    work_dir
        .run_jj(["file", "track", "tracked-file.txt"])
        .success();
    work_dir.run_jj(["commit", "-m", "setup"]).success();

    // `logs/` itself doesn't match any ignore pattern, only the files inside
    // it do.
    work_dir.write_file("logs/one.log", "content");
    work_dir.write_file("logs/two.log", "content");

    let output = work_dir.run_jj(["file", "remove-ignored"]);
    assert_snapshot!(output, @"
    ------- stderr -------
    Removed 2 ignored file(s) from the working copy.
    [EOF]
    ");
    assert!(!work_dir.root().join("logs").exists());
    assert!(work_dir.root().join("tracked-file.txt").exists());
}

#[test]
fn test_file_remove_ignored_removes_empty_directory() {
    // Regression test: an empty directory that matches a directory-level
    // `.gitignore` pattern should be removed by `remove-ignored`, exactly
    // like it is listed by `list-ignored`.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "empty-ignored/\n");
    work_dir.create_dir("empty-ignored");

    let output = work_dir.run_jj(["file", "remove-ignored"]);
    assert_snapshot!(output, @"
    ------- stderr -------
    Removed 1 ignored file(s) from the working copy.
    [EOF]
    ");
    assert!(!work_dir.root().join("empty-ignored").exists());
}

#[test]
fn test_file_remove_ignored_does_not_touch_unrelated_file_in_same_directory() {
    // Safety test: a directory that isn't itself covered by a `.gitignore`
    // pattern, and that contains both an ignored file and an unrelated file
    // that isn't ignored, must only have the ignored file removed. The
    // directory itself must be left behind, since it still has content.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.log\n");
    work_dir.write_file("logs/one.log", "content");
    work_dir.write_file("logs/keep.txt", "content");

    let output = work_dir.run_jj(["file", "remove-ignored"]);
    assert_snapshot!(output, @"
    ------- stderr -------
    Removed 1 ignored file(s) from the working copy.
    [EOF]
    ");
    assert!(!work_dir.root().join("logs/one.log").exists());
    assert!(work_dir.root().join("logs/keep.txt").exists());
    assert!(work_dir.root().join("logs").exists());
}

#[test]
fn test_file_remove_ignored_scoped_to_path() {
    // Passing a path restricts both `list-ignored` and `remove-ignored` to
    // that subset, leaving other ignored content (e.g. a build directory
    // you don't want touched yet) completely untouched. Running
    // `list-ignored` with the same path first lets you preview what
    // `remove-ignored` would do, without a separate `--dry-run` flag.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "/target/\n/rendered-docs\n");
    work_dir.write_file("target/some-build-artifact.bin", "content");
    work_dir.create_dir("rendered-docs");

    let list_output = work_dir.run_jj(["file", "list-ignored", "rendered-docs"]);
    assert_snapshot!(list_output, @"
    rendered-docs/
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "remove-ignored", "rendered-docs"]);
    assert_snapshot!(output, @"
    ------- stderr -------
    Removed 1 ignored file(s) from the working copy.
    [EOF]
    ");
    assert!(!work_dir.root().join("rendered-docs").exists());
    // `target/` was never mentioned, so it's left completely alone.
    assert!(work_dir.root().join("target/some-build-artifact.bin").exists());
}
