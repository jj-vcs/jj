// Copyright 2022 The Jujutsu Authors
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
fn test_track_untrack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "initial");
    work_dir.write_file("file1.bak", "initial");
    work_dir.write_file("file2.bak", "initial");
    let target_dir = work_dir.create_dir("target");
    target_dir.write_file("file2", "initial");
    target_dir.write_file("file3", "initial");

    // Run a command so all the files get tracked, then add "*.bak" to the ignore
    // patterns
    work_dir.run_jj(["st"]).success();
    work_dir.write_file(".gitignore", "*.bak\n");
    let files_before = work_dir.run_jj(["file", "list"]).success();

    // Errors out when not run at the head operation
    let output = work_dir.run_jj(["file", "untrack", "file1", "--at-op", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command must be able to update the working copy.
    Hint: Don't use --at-op.
    [EOF]
    [exit status: 1]
    ");
    // Errors out when no path is specified
    let output = work_dir.run_jj(["file", "untrack"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <FILESETS>...

    Usage: jj file untrack <FILESETS>...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
    // Errors out when a specified file is not ignored
    let output = work_dir.run_jj(["file", "untrack", "file1", "file1.bak"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: 'file1' is not ignored.
    Hint: Files that are not ignored will be added back by the next command.
    Make sure they're ignored, then try again.
    [EOF]
    [exit status: 1]
    ");
    let files_after = work_dir.run_jj(["file", "list"]).success();
    // There should be no changes to the state when there was an error
    assert_eq!(files_after, files_before);

    // Can untrack a single file
    assert!(files_before.stdout.raw().contains("file1.bak\n"));
    let output = work_dir.run_jj(["file", "untrack", "file1.bak"]);
    insta::assert_snapshot!(output, @"");
    let files_after = work_dir.run_jj(["file", "list"]).success();
    // The file is no longer tracked
    assert!(!files_after.stdout.raw().contains("file1.bak"));
    // Other files that match the ignore pattern are not untracked
    assert!(files_after.stdout.raw().contains("file2.bak"));
    // The files still exist on disk
    assert!(work_dir.root().join("file1.bak").exists());
    assert!(work_dir.root().join("file2.bak").exists());

    // Warns if path doesn't exist
    let output = work_dir.run_jj(["file", "untrack", "nonexistent"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    [EOF]
    ");

    // Errors out when multiple specified files are not ignored
    let output = work_dir.run_jj(["file", "untrack", "target"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Error: 'target/file2' and 1 other files are not ignored.
    Hint: Files that are not ignored will be added back by the next command.
    Make sure they're ignored, then try again.
    [EOF]
    [exit status: 1]
    ");

    // Can untrack after adding to ignore patterns
    work_dir.write_file(".gitignore", "*.bak\ntarget/\n");
    let output = work_dir.run_jj(["file", "untrack", "target"]);
    insta::assert_snapshot!(output, @"");
    let files_after = work_dir.run_jj(["file", "list"]).success();
    assert!(!files_after.stdout.raw().contains("target"));
}

#[test]
fn test_track_untrack_sparse() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");

    // When untracking a file that's not included in the sparse working copy, it
    // doesn't need to be ignored (because it won't be automatically added
    // back).
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    file2
    [EOF]
    ");
    work_dir
        .run_jj(["sparse", "set", "--clear", "--add", "file1"])
        .success();
    let output = work_dir.run_jj(["file", "untrack", "file2"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    // Trying to manually track a file that's not included in the sparse working has
    // no effect. TODO: At least a warning would be useful
    let output = work_dir.run_jj(["file", "track", "file2"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
}

#[test]
fn test_auto_track() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.auto-track = 'glob:*.rs'"#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1.rs", "initial");
    work_dir.write_file("file2.md", "initial");
    work_dir.write_file("file3.md", "initial");

    // Only configured paths get auto-tracked
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    [EOF]
    ");

    // Can manually track paths
    let output = work_dir.run_jj(["file", "track", "file3.md"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    file3.md
    [EOF]
    ");

    // Can manually untrack paths
    let output = work_dir.run_jj(["file", "untrack", "file3.md"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    [EOF]
    ");

    // CWD-relative paths in `snapshot.auto-track` are evaluated from the repo root
    let sub_dir = work_dir.create_dir("sub");
    sub_dir.write_file("file1.rs", "initial");
    let output = sub_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ../file1.rs
    [EOF]
    ");

    // But `jj file track` wants CWD-relative paths
    let output = sub_dir.run_jj(["file", "track", "file1.rs"]);
    insta::assert_snapshot!(output, @"");
    let output = sub_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ../file1.rs
    file1.rs
    [EOF]
    ");
}

#[test]
fn test_track_ignored() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.auto-track = 'none()'"#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.bak\n");
    work_dir.write_file("file1", "initial");
    work_dir.write_file("file1.bak", "initial");

    // Track an unignored path
    let output = work_dir.run_jj(["file", "track", "file1"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    // Track an ignored path without --include-ignored (should not work)
    let output = work_dir.run_jj(["file", "track", "file1.bak"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
}

#[test]
fn test_track_ignored_with_flag() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "*.ignored\n");
    work_dir.write_file("file1.txt", "content");
    work_dir.write_file("file2.ignored", "ignored content");

    // Test the setup
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    .gitignore
    file1.txt
    [EOF]
    ");

    // Track ignored file with --include-ignored
    let output = work_dir.run_jj(["file", "track", "--include-ignored", "file2.ignored"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    .gitignore
    file1.txt
    file2.ignored
    [EOF]
    ");
}

#[test]
fn test_track_large_file_with_flag() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.max-new-file-size = "10""#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("small.txt", "small");
    work_dir.write_file("large.txt", "a".repeat(20).as_str());

    // Test the setup
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    small.txt
    [EOF]
    ------- stderr -------
    Warning: Refused to snapshot some files:
      large.txt: 20.0B (20 bytes); the maximum size allowed is 10.0B (10 bytes)
    Hint: This is to prevent large files from being added by accident. You can fix this by:
      - Adding the file to `.gitignore`
      - Run `jj config set --repo snapshot.max-new-file-size 20`
        This will increase the maximum file size allowed for new files, in this repository only.
      - Run `jj --config snapshot.max-new-file-size=20 st`
        This will increase the maximum file size allowed for new files, for this command only.
    [EOF]
    ");

    // Track large file with --include-ignored
    let output = work_dir.run_jj(["file", "track", "--include-ignored", "large.txt"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Refused to snapshot some files:
      large.txt: 20.0B (20 bytes); the maximum size allowed is 10.0B (10 bytes)
    Hint: This is to prevent large files from being added by accident. You can fix this by:
      - Adding the file to `.gitignore`
      - Run `jj config set --repo snapshot.max-new-file-size 20`
        This will increase the maximum file size allowed for new files, in this repository only.
      - Run `jj --config snapshot.max-new-file-size=20 file track large.txt`
        This will increase the maximum file size allowed for new files, for this command only.
      - Run `jj file track --include-ignored large.txt`
        This will track the files even though they exceed the size limit.
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    large.txt
    small.txt
    [EOF]
    ");
}

#[test]
fn test_track_ignored_directory() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file(".gitignore", "ignored_dir/\n");
    let ignored_dir = work_dir.create_dir("ignored_dir");
    ignored_dir.write_file("file1.txt", "content1");
    ignored_dir.write_file("file2.txt", "content2");

    // Test the setup
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    .gitignore
    [EOF]
    ");

    // Track ignored directory with --include-ignored
    let output = work_dir.run_jj(["file", "track", "--include-ignored", "ignored_dir"]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    .gitignore
    ignored_dir/file1.txt
    ignored_dir/file2.txt
    [EOF]
    ");
}
