// Copyright 2020 The Jujutsu Authors
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

use std::io::Write as _;

use jj_lib::file_util::check_symlink_support;
use jj_lib::file_util::symlink_file;
use testutils::TestResult;
use testutils::git;

use crate::common::TestEnvironment;

#[test]
fn test_gitignores() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();

    // Say in core.excludesFiles that we don't want file1, file2, or file3
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(work_dir.root().join(".git").join("config"))?;
    // Put the file in "~/my-ignores" so we also test that "~" expands to "$HOME"
    file.write_all(b"[core]\nexcludesFile=~/my-ignores\n")?;
    drop(file);
    std::fs::write(
        test_env.home_dir().join("my-ignores"),
        "file1\nfile2\nfile3",
    )?;

    // Say in .git/info/exclude that we actually do want file2 and file3
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(work_dir.root().join(".git").join("info").join("exclude"))?;
    file.write_all(b"!file2\n!file3")?;
    drop(file);

    // Say in .gitignore (in the working copy) that we actually do not want file2
    // (again)
    work_dir.write_file(".gitignore", "file2");

    // Writes some files to the working copy
    work_dir.write_file("file0", "contents");
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");

    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @"
    A .gitignore
    A file0
    A file3
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_gitignores_relative_excludes_file_path() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(work_dir.root().join(".git").join("config"))?;
    file.write_all(b"[core]\nexcludesFile=../my-ignores\n")?;
    drop(file);
    std::fs::write(test_env.env_root().join("my-ignores"), "ignored\n")?;

    work_dir.write_file("ignored", "");
    work_dir.write_file("not-ignored", "");

    // core.excludesFile should be resolved relative to the workspace root, not
    // to the cwd.
    let sub_dir = work_dir.create_dir("sub");
    let output = sub_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    A ../not-ignored
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["-Rrepo", "diff", "-s"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    A repo/not-ignored
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_gitignores_symlinked_ignore_files() -> TestResult {
    if !check_symlink_support()? {
        eprintln!("Skipping test because symlink isn't supported");
        return Ok(());
    }

    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--colocate"]).success();

    // Point core.excludesFile at a symlink to a file that ignores file1. Git
    // follows symlinks here, so file1 should be ignored.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(work_dir.root().join(".git").join("config"))?;
    file.write_all(b"[core]\nexcludesFile=~/my-ignores\n")?;
    drop(file);
    let ignores_target_path = test_env.home_dir().join("my-ignores-target");
    std::fs::write(&ignores_target_path, "file1\n")?;
    symlink_file(ignores_target_path, test_env.home_dir().join("my-ignores"))?;

    // Make .git/info/exclude a symlink to a file that ignores file2. Git
    // follows symlinks here too, so file2 should be ignored.
    let exclude_path = work_dir.root().join(".git").join("info").join("exclude");
    let exclude_target_path = test_env.env_root().join("exclude-target");
    std::fs::write(&exclude_target_path, "file2\n")?;
    std::fs::remove_file(&exclude_path)?;
    symlink_file(exclude_target_path, &exclude_path)?;

    // Make the in-tree .gitignore a symlink to a file that would ignore file3,
    // and add a dangling symlink in a subdirectory. Git does not follow
    // symlinks for in-tree .gitignore, so file3 should *not* be ignored.
    work_dir.write_file("gitignore-target", "file3\n");
    symlink_file("gitignore-target", work_dir.root().join(".gitignore"))?;
    let sub_dir = work_dir.create_dir("sub");
    symlink_file("non-existent", sub_dir.root().join(".gitignore"))?;

    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");
    sub_dir.write_file("file4", "contents");

    // file1/file2 are ignored via the followed symlinks, file3 is not because
    // the in-tree .gitignore symlink isn't followed. The dangling symlink is
    // skipped without an error.
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    A .gitignore
    A file3
    A gitignore-target
    A sub/.gitignore
    A sub/file4
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_gitignores_ignored_file_in_target_commit() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();

    // Create a commit with file "ignored" in it
    work_dir.write_file("ignored", "committed contents\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "with-file"])
        .success();
    let target_commit_id = work_dir
        .run_jj(["log", "--no-graph", "-T=commit_id", "-r=@"])
        .success()
        .stdout
        .into_raw();

    // Create another commit where we ignore that path
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("ignored", "contents in working copy\n");
    work_dir.write_file(".gitignore", ".gitignore\nignored\n");

    // Update to the commit with the "ignored" file
    let output = work_dir.run_jj(["edit", "with-file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm 3cf51c1a with-file | (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 1 files, modified 0 files, removed 0 files
    Warning: 1 of those updates were skipped because there were conflicting changes in the working copy.
    Hint: Inspect the changes compared to the intended target with `jj diff --from 3cf51c1acf24`.
    Discard the conflicting changes with `jj restore --from 3cf51c1acf24`.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git", "--from", &target_commit_id]);
    insta::assert_snapshot!(output, @"
    diff --git a/ignored b/ignored
    index 8a69467466..4d9be5127b 100644
    --- a/ignored
    +++ b/ignored
    @@ -1,1 +1,1 @@
    -committed contents
    +contents in working copy
    [EOF]
    ");
}
