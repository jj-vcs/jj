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

use std::path;

use indoc::formatdoc;
use indoc::indoc;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::to_toml_value;

fn set_up_non_empty_git_repo(git_repo: &gix::Repository) {
    set_up_git_repo_with_file(git_repo, "file");
}

fn set_up_git_repo_with_file(git_repo: &gix::Repository, filename: &str) {
    git::add_commit(
        git_repo,
        "refs/heads/main",
        filename,
        b"content",
        "message",
        &[],
    );
    git::set_symbolic_reference(git_repo, "HEAD", "refs/heads/main");
}

#[test]
fn test_git_clone() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    // Clone an empty repo
    let output = root_dir.run_jj(["git", "clone", "source", "empty"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    [EOF]
    "#);

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: uuqppmxq 3711b3b5 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir = test_env.work_dir("clone");
    assert!(clone_dir.root().join("file").exists());

    // Subsequent fetch should just work even if the source path was relative
    let output = clone_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Failed clone should clean up the destination directory
    root_dir.create_dir("bad");
    let output = root_dir.run_jj(["git", "clone", "bad", "failed"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/failed"
    Error: Could not find repository at '$TEST_ENV/bad'
    [EOF]
    [exit status: 1]
    "#);
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    let failed_dir = root_dir.create_dir("failed");
    let output = root_dir.run_jj(["git", "clone", "bad", "failed"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/failed"
    Error: Could not find repository at '$TEST_ENV/bad'
    [EOF]
    [exit status: 1]
    "#);
    assert!(failed_dir.root().exists());
    assert!(!failed_dir.root().join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let output = root_dir.run_jj(["git", "clone", "bad", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    assert!(clone_dir.root().join(".jj").exists());

    // Try cloning into an existing workspace
    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Try cloning into an existing file
    root_dir.write_file("file", "contents");
    let output = root_dir.run_jj(["git", "clone", "source", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Try cloning into non-empty, non-workspace directory
    clone_dir.remove_dir_all(".jj");
    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Clone into a nested path
    let output = root_dir.run_jj(["git", "clone", "source", "nested/path/to/repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: uuzqqzqu c871b515 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
}

#[test]
fn test_git_clone_bad_source() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");

    let output = root_dir.run_jj(["git", "clone", "", "dest"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: local path "" does not specify a path to a repository
    [EOF]
    [exit status: 2]
    "#);

    // Invalid port number
    let output = root_dir.run_jj(["git", "clone", "https://example.net:bad-port/bar", "dest"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: URL "https://example.net:bad-port/bar" can not be parsed as valid URL
    Caused by: invalid port number
    [EOF]
    [exit status: 2]
    "#);
}

#[test]
fn test_git_clone_colocate() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    // Clone an empty repo
    let output = root_dir.run_jj(["git", "clone", "source", "empty", "--colocate"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    // git_target path should be relative to the store
    let empty_dir = test_env.work_dir("empty");
    let git_target_file_contents =
        String::from_utf8(empty_dir.read_file(".jj/repo/store/git_target").into()).unwrap();
    insta::assert_snapshot!(
        git_target_file_contents.replace(path::MAIN_SEPARATOR, "/"),
        @"../../../.git");

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let output = root_dir.run_jj(["git", "clone", "source", "clone", "--colocate"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: uuqppmxq 3711b3b5 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);
    let clone_dir = test_env.work_dir("clone");
    assert!(clone_dir.root().join("file").exists());
    assert!(clone_dir.root().join(".git").exists());

    eprintln!(
        "{:?}",
        git_repo.head().expect("Repo head should be set").name()
    );

    let jj_git_repo = git::open(clone_dir.root());
    assert_eq!(
        jj_git_repo
            .head_id()
            .expect("Clone Repo HEAD should be set.")
            .detach(),
        git_repo
            .head_id()
            .expect("Repo HEAD should be set.")
            .detach(),
    );
    // ".jj" directory should be ignored at Git side.
    let git_statuses = git::status(&jj_git_repo);
    insta::assert_debug_snapshot!(git_statuses, @r#"
    [
        GitStatus {
            path: ".jj/.gitignore",
            status: Worktree(
                Ignored,
            ),
        },
        GitStatus {
            path: ".jj/repo",
            status: Worktree(
                Ignored,
            ),
        },
        GitStatus {
            path: ".jj/working_copy",
            status: Worktree(
                Ignored,
            ),
        },
    ]
    "#);

    // The old default bookmark "master" shouldn't exist.
    insta::assert_snapshot!(get_bookmark_output(&clone_dir), @r"
    main: qomsplrm ebeb70d8 message
      @git: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // Subsequent fetch should just work even if the source path was relative
    let output = clone_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Failed clone should clean up the destination directory
    root_dir.create_dir("bad");
    let output = root_dir.run_jj(["git", "clone", "--colocate", "bad", "failed"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/failed"
    Error: Could not find repository at '$TEST_ENV/bad'
    [EOF]
    [exit status: 1]
    "#);
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    let failed_dir = root_dir.create_dir("failed");
    let output = root_dir.run_jj(["git", "clone", "--colocate", "bad", "failed"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/failed"
    Error: Could not find repository at '$TEST_ENV/bad'
    [EOF]
    [exit status: 1]
    "#);
    assert!(failed_dir.root().exists());
    assert!(!failed_dir.root().join(".git").exists());
    assert!(!failed_dir.root().join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let output = root_dir.run_jj(["git", "clone", "--colocate", "bad", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    assert!(clone_dir.root().join(".git").exists());
    assert!(clone_dir.root().join(".jj").exists());

    // Try cloning into an existing workspace
    let output = root_dir.run_jj(["git", "clone", "source", "clone", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Try cloning into an existing file
    root_dir.write_file("file", "contents");
    let output = root_dir.run_jj(["git", "clone", "source", "file", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Try cloning into non-empty, non-workspace directory
    clone_dir.remove_dir_all(".jj");
    let output = root_dir.run_jj(["git", "clone", "source", "clone", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");

    // Clone into a nested path
    let output = root_dir.run_jj([
        "git",
        "clone",
        "source",
        "nested/path/to/repo",
        "--colocate",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: vzqnnsmr fea36bca (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);
}

#[test]
fn test_git_clone_colocate_via_config() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    test_env.add_config("git.colocate = true");

    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);
    let clone_dir = test_env.work_dir("clone");
    assert!(clone_dir.root().join("file").exists());
    assert!(clone_dir.root().join(".git").exists());
}

#[test]
fn test_git_clone_no_colocate() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    test_env.add_config("git.colocate = true");

    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "--no-colocate", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir = test_env.work_dir("clone");
    assert!(clone_dir.root().join("file").exists());
    assert!(!clone_dir.root().join(".git").exists());
}

#[test]
fn test_git_clone_tags() {
    use gix::remote::fetch::Tags;

    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let source_git_repo = git::init(git_repo_path);

    git::add_commit(
        &source_git_repo,
        "refs/tags/v1.0",
        "foo",
        b"content",
        "message",
        &[],
    );

    let commit = git::add_commit(
        &source_git_repo,
        "refs/tags/v2.0",
        "bar",
        b"content",
        "message",
        &[],
    )
    .commit_id;

    git::add_commit(
        &source_git_repo,
        "refs/heads/main",
        "baz",
        b"content",
        "message",
        &[commit],
    );

    git::set_symbolic_reference(&source_git_repo, "HEAD", "refs/heads/main");

    let run_test = |name, args: &[_]| {
        // Clone an empty repo
        root_dir.run_jj(
            ["git", "clone", "source", name, "--colocate"]
                .iter()
                .chain(args),
        )
    };

    let get_remote_fetch_tags = |name| {
        git::open(test_env.env_root().join(name))
            .find_remote("origin")
            .unwrap()
            .fetch_tags()
    };

    insta::assert_snapshot!(run_test("default", &[]), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/default"
    bookmark: main@origin [new] tracked
    tag: v1.0@git [new] 
    tag: v2.0@git [new] 
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 88542a00 (empty) (no description set)
    Parent commit (@-)      : lnmyztun e93ca54d main | message
    Added 2 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    insta::assert_snapshot!(run_test("included", &["--fetch-tags", "included"]), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/included"
    bookmark: main@origin [new] tracked
    tag: v2.0@git [new] 
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: uuqppmxq 676b2fd8 (empty) (no description set)
    Parent commit (@-)      : lnmyztun e93ca54d main | message
    Added 2 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    insta::assert_snapshot!(run_test("all", &["--fetch-tags", "all"]), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/all"
    bookmark: main@origin [new] tracked
    tag: v1.0@git [new] 
    tag: v2.0@git [new] 
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: pmmvwywv cd5996a2 (empty) (no description set)
    Parent commit (@-)      : lnmyztun e93ca54d main | message
    Added 2 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    insta::assert_snapshot!(run_test("none", &["--fetch-tags", "none"]), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/none"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: rzvqmyuk 61c45a3c (empty) (no description set)
    Parent commit (@-)      : lnmyztun e93ca54d main | message
    Added 2 files, modified 0 files, removed 0 files
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    assert_eq!(Tags::Included, get_remote_fetch_tags("default"));
    assert_eq!(Tags::Included, get_remote_fetch_tags("included"));
    assert_eq!(Tags::All, get_remote_fetch_tags("all"));
    assert_eq!(Tags::None, get_remote_fetch_tags("none"));
}

#[test]
fn test_git_clone_remote_default_bookmark() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path.clone());

    set_up_non_empty_git_repo(&git_repo);

    // Create non-default bookmark in remote
    let head_id = git_repo.head_id().unwrap().detach();
    git_repo
        .reference(
            "refs/heads/feature1",
            head_id,
            gix::refs::transaction::PreviousValue::MustNotExist,
            "",
        )
        .unwrap();

    // All fetched bookmarks will be imported if auto-track-bookmarks = 'glob:*'
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let output = root_dir.run_jj(["git", "clone", "source", "clone1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone1"
    bookmark: feature1@origin [new] tracked
    bookmark: main@origin     [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir1 = test_env.work_dir("clone1");
    insta::assert_snapshot!(get_bookmark_output(&clone_dir1), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    main: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // "trunk()" alias should be set to default bookmark "main"
    let output = clone_dir1.run_jj(["config", "list", "--repo", "revset-aliases.'trunk()'"]);
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = "main@origin"
    [EOF]
    "#);

    // Only the default bookmark will be imported if auto-track-bookmarks = ''
    test_env.add_config("remotes.origin.auto-track-bookmarks = ''");
    let output = root_dir.run_jj(["git", "clone", "source", "clone2"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone2"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: rzvqmyuk 27e56779 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1@origin main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir2 = test_env.work_dir("clone2");
    insta::assert_snapshot!(get_bookmark_output(&clone_dir2), @r"
    feature1@origin: qomsplrm ebeb70d8 message
    main: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // Change the default bookmark in remote
    git::set_symbolic_reference(&git_repo, "HEAD", "refs/heads/feature1");
    let output = root_dir.run_jj(["git", "clone", "source", "clone3"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone3"
    bookmark: feature1@origin [new] tracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `feature1@origin`
    Working copy  (@) now at: nppvrztz b16020e9 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1 main@origin | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir3 = test_env.work_dir("clone3");
    insta::assert_snapshot!(get_bookmark_output(&clone_dir3), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    main@origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // "trunk()" alias should be set to new default bookmark "feature1"
    let output = clone_dir3.run_jj(["config", "list", "--repo", "revset-aliases.'trunk()'"]);
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = "feature1@origin"
    [EOF]
    "#);

    // No bookmarks should be imported if both auto-track-bookmarks and
    // track-default-bookmark-on-clone are turned off
    let output = root_dir.run_jj([
        "git",
        "clone",
        "--config=git.track-default-bookmark-on-clone=false",
        "source",
        "clone4",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone4"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `feature1@origin`
    Working copy  (@) now at: wmwvqwsz 5068d576 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1@origin main@origin | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir4 = test_env.work_dir("clone4");
    insta::assert_snapshot!(get_bookmark_output(&clone_dir4), @r"
    feature1@origin: qomsplrm ebeb70d8 message
    main@origin: qomsplrm ebeb70d8 message
    [EOF]
    ");

    // Show hint if track-default-bookmark-on-clone=false has no effect
    let output = root_dir.run_jj([
        "git",
        "clone",
        "--config=git.auto-local-bookmark=true",
        "--config=git.track-default-bookmark-on-clone=false",
        "source",
        "clone5",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Deprecated CLI-provided config: `git.auto-local-bookmark` is deprecated; use `remotes.<name>.auto-track-bookmarks` instead.
    Example: jj config set --user remotes.origin.auto-track-bookmarks 'glob:*'
    For details, see: https://docs.jj-vcs.dev/latest/config/#automatic-tracking-of-bookmarks
    Fetching into new repo in "$TEST_ENV/clone5"
    bookmark: feature1@origin [new] tracked
    bookmark: main@origin     [new] tracked
    Hint: `git.track-default-bookmark-on-clone=false` has no effect if `git.auto-local-bookmark` is enabled.
    Setting the revset alias `trunk()` to `feature1@origin`
    Working copy  (@) now at: vzqnnsmr fea36bca (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    let clone_dir5 = test_env.work_dir("clone5");
    insta::assert_snapshot!(get_bookmark_output(&clone_dir5), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    main: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");
}

// A branch with a strange name should get quoted in the config. Windows doesn't
// like the strange name, so we don't run the test there.
#[cfg(unix)]
#[test]
fn test_git_clone_remote_default_bookmark_with_escape() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    // Create a branch to something that needs to be escaped
    let commit_id = git::add_commit(
        &git_repo,
        "refs/heads/\"",
        "file",
        b"content",
        "message",
        &[],
    )
    .commit_id;
    git::set_head_to_id(&git_repo, commit_id);

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: "\""@origin [new] tracked
    Setting the revset alias `trunk()` to `"\""@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 "\"" | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // "trunk()" alias should be escaped and quoted
    let clone_dir = test_env.work_dir("clone");
    let output = clone_dir.run_jj(["config", "list", "--repo", "revset-aliases.'trunk()'"]);
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = '"\""@origin'
    [EOF]
    "#);
}

#[test]
fn test_git_clone_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // Should not update working-copy files
    let output = root_dir.run_jj(["git", "clone", "--ignore-working-copy", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    [EOF]
    "#);
    let clone_dir = test_env.work_dir("clone");

    let output = clone_dir.run_jj(["status", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-): qomsplrm ebeb70d8 main | message
    [EOF]
    ");

    // TODO: Correct, but might be better to check out the root commit?
    let output = clone_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation 8f47435a3990).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_clone_at_operation() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "--at-op=@-", "source", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_clone_with_remote_name() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path and a non-default remote name
    let output = root_dir.run_jj(["git", "clone", "source", "clone", "--remote", "upstream"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@upstream [new] tracked
    Setting the revset alias `trunk()` to `main@upstream`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
}

#[test]
fn test_git_clone_with_remote_named_git() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    git::init(git_repo_path);

    let output = root_dir.run_jj(["git", "clone", "--remote=git", "source", "dest"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_clone_with_remote_with_slashes() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    git::init(git_repo_path);

    let output = root_dir.run_jj(["git", "clone", "--remote=slash/origin", "source", "dest"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_clone_trunk_deleted() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);
    let clone_dir = test_env.work_dir("clone");

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    let output = clone_dir.run_jj(["bookmark", "forget", "--include-remotes", "main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    Warning: Failed to resolve `revset-aliases.trunk()`: Revision `main@origin` doesn't exist
    Hint: Use `jj config edit --repo` to adjust the `trunk()` alias.
    [EOF]
    ");

    let output = clone_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 1ca44815
    │  (empty) (no description set)
    ○  qomsplrm someone@example.org 1970-01-01 11:00:00 ebeb70d8
    │  message
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Warning: Failed to resolve `revset-aliases.trunk()`: Revision `main@origin` doesn't exist
    Hint: Use `jj config edit --repo` to adjust the `trunk()` alias.
    [EOF]
    ");
}

#[test]
fn test_git_clone_conditional_config() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let source_repo_path = test_env.env_root().join("source");
    let old_workspace_dir = test_env.work_dir("old");
    let new_workspace_dir = test_env.work_dir("new");
    let source_git_repo = git::init(source_repo_path);
    set_up_non_empty_git_repo(&source_git_repo);

    let run_jj = |work_dir: &TestWorkDir, args: &[&str]| {
        work_dir.run_jj_with(|cmd| {
            cmd.args(args)
                .env_remove("JJ_EMAIL")
                .env_remove("JJ_OP_HOSTNAME")
                .env_remove("JJ_OP_USERNAME")
        })
    };
    let log_template = r#"separate(' ', author.email(), description.first_line()) ++ "\n""#;
    let op_log_template = r#"separate(' ', user, description.first_line()) ++ "\n""#;

    // Override user.email and operation.username conditionally
    test_env.add_config(formatdoc! {"
        user.email = 'base@example.org'
        operation.hostname = 'base'
        operation.username = 'base'
        [[--scope]]
        --when.repositories = [{new_workspace_root}]
        user.email = 'new-repo@example.org'
        operation.username = 'new-repo'
        ",
        new_workspace_root = to_toml_value(new_workspace_dir.root().to_str().unwrap()),
    });

    // Override operation.hostname by repo config, which should be loaded into
    // the command settings, but shouldn't be copied to the new repo.
    run_jj(&root_dir, &["git", "init", "old"]).success();
    run_jj(
        &old_workspace_dir,
        &["config", "set", "--repo", "operation.hostname", "old-repo"],
    )
    .success();
    run_jj(&old_workspace_dir, &["new"]).success();
    let output = run_jj(&old_workspace_dir, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  base@old-repo new empty commit
    ○  base@base add workspace 'default'
    ○  @
    [EOF]
    ");

    // Clone repo at the old workspace directory.
    let output = run_jj(&old_workspace_dir, &["git", "clone", "../source", "../new"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/new"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: zxsnswpr 5479cd52 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    run_jj(&new_workspace_dir, &["new"]).success();
    let output = run_jj(&new_workspace_dir, &["log", "-T", log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆  someone@example.org message
    │
    ~
    [EOF]
    ");
    let output = run_jj(&new_workspace_dir, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base check out git remote's branch: main
    ○  new-repo@base fetch from git remote into empty repo
    ○  new-repo@base add git remote origin
    ○  new-repo@base add workspace 'default'
    ○  @
    [EOF]
    ");
}

#[test]
fn test_git_clone_with_depth() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let clone_dir = test_env.work_dir("clone");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "--depth", "1", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    let output = clone_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 1ca44815
    │  (empty) (no description set)
    ◆  qomsplrm someone@example.org 1970-01-01 11:00:00 main ebeb70d8
    │  message
    ~
    [EOF]
    ");
}

#[test]
fn test_git_clone_invalid_immutable_heads() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    test_env.add_config("revset-aliases.'immutable_heads()' = 'unknown'");
    // Suppress lengthy warnings in commit summary template
    test_env.add_config("revsets.short-prefixes = ''");

    // The error shouldn't be counted as an immutable working-copy commit. It
    // should be reported.
    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by: Revision `unknown` doesn't exist
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_clone_malformed() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    let clone_dir = test_env.work_dir("clone");
    // we can insert ".jj" entry to create a malformed clone
    set_up_git_repo_with_file(&git_repo, ".jj");

    // TODO: Perhaps, this should be a user error, not an internal error.
    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Internal error: Failed to check out commit 2f4286212884d472a0b2013a961b695a144ac65c
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    [EOF]
    [exit status: 255]
    "#);

    // The cloned workspace isn't usable.
    let output = clone_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation 353078ed182b).
    Hint: Run `jj workspace update-stale` to update it.
    See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");

    // The error can be somehow recovered.
    // TODO: add an update-stale flag to reset the working-copy?
    let output = clone_dir.run_jj(["workspace", "update-stale"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: Failed to check out commit 2f4286212884d472a0b2013a961b695a144ac65c
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    [EOF]
    [exit status: 255]
    ");
    let output = clone_dir.run_jj(["new", "root()", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @"");
    let output = clone_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : zsuskuln c2934cfb (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_git_clone_with_global_git_remote_config() {
    let mut test_env = TestEnvironment::default();
    test_env.work_dir("").write_file(
        "git-config",
        indoc! {r#"
            [remote "origin"]
                prune = true
        "#},
    );
    test_env.add_env_var("GIT_CONFIG_GLOBAL", test_env.env_root().join("git-config"));

    let root_dir = test_env.work_dir("");
    let git_repo_path = root_dir.root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
}

#[test]
fn test_git_clone_no_git_executable() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("git.executable-path = 'jj-test-missing-program'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    Error: Could not execute the git process, found in the OS path 'jj-test-missing-program'
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_git_clone_no_git_executable_with_path() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    let invalid_git_executable_path = test_env.env_root().join("invalid").join("path");
    test_env.add_config(format!(
        "git.executable-path = {}",
        to_toml_value(invalid_git_executable_path.to_str().unwrap())
    ));
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = root_dir.run_jj(["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    Error: Could not execute git process at specified path '$TEST_ENV/invalid/path'
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_git_clone_branch() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config("remotes.origin.auto-track-bookmarks = 'glob:*'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(&git_repo_path);
    set_up_non_empty_git_repo(&git_repo);
    let _ = git::add_commit(
        &git_repo,
        "refs/heads/feature1",
        "file",
        b"content",
        "message",
        &[],
    );

    // Clone the default branch by name
    let output = root_dir.run_jj(["git", "clone", "source", "clone", "--branch", "main"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy  (@) now at: sqpuoqvx 1ca44815 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Clone a branch that doesn't exist
    let output = root_dir.run_jj([
        "git",
        "clone",
        "source",
        "clone_no_such_branch",
        "--branch",
        "nonexistent_branch",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone_no_such_branch"
    Error: No such branch matching pattern: nonexistent_branch
    [EOF]
    [exit status: 1]
    "#);

    // Clone a specific, non-default branch
    let output = root_dir.run_jj([
        "git",
        "clone",
        "source",
        "clone_non_default",
        "--branch",
        "feature1",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone_non_default"
    bookmark: feature1@origin [new] tracked
    Working copy  (@) now at: pmmvwywv ea9c2659 (empty) (no description set)
    Parent commit (@-)      : qomsplrm ebeb70d8 feature1 | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    // Perform a fetch in that same repo
    let repo_dir = test_env.work_dir("clone_non_default");
    let output = repo_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_git_clone_auto_track_bookmarks() {
    let test_env = TestEnvironment::default();
    let root_dir = test_env.work_dir("");
    test_env.add_config(
        "
        [remotes.origin]
        auto-track-bookmarks = 'glob:mine/*'
        ",
    );

    root_dir
        .run_jj(["git", "init", "--colocate", "origin"])
        .success();
    let origin_dir = test_env.work_dir("origin");
    origin_dir.run_jj(["b", "c", "mine/foo"]).success();
    origin_dir.run_jj(["b", "c", "not-mine/foo"]).success();
    origin_dir.run_jj(["commit", "-mfoo"]).success();
    let output = origin_dir.run_jj(["show", "@-"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: d7828da83253475bf10c2ae6bd3f0f84bf4604c1
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Bookmarks: mine/foo not-mine/foo mine/foo@git not-mine/foo@git
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:10)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:10)

        foo

    [EOF]
    ");

    root_dir
        .run_jj(["git", "clone", "origin/.git", "repo"])
        .success();
    let repo_dir = test_env.work_dir("repo");

    let output = get_bookmark_output(&repo_dir);
    insta::assert_snapshot!(output, @r"
    mine/foo: qpvuntsm d7828da8 (empty) foo
      @origin: qpvuntsm d7828da8 (empty) foo
    not-mine/foo@origin: qpvuntsm d7828da8 (empty) foo
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["bookmark", "list", "--all-remotes"])
}
