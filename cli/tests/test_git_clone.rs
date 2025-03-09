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
use std::path::Path;
use std::path::PathBuf;

use indoc::formatdoc;
use test_case::test_case;
use testutils::git;

use crate::common::to_toml_value;
use crate::common::CommandOutput;
use crate::common::TestEnvironment;

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

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    // Clone an empty repo
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "empty"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    [EOF]
    "#);
    }

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuqppmxq f78d2645 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
    assert!(test_env.env_root().join("clone").join("file").exists());

    // Subsequent fetch should just work even if the source path was relative
    let output = test_env.run_jj_in(&test_env.env_root().join("clone"), ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }

    // Failed clone should clean up the destination directory
    std::fs::create_dir(test_env.env_root().join("bad")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "bad", "failed"]);
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        [EOF]
        [exit status: 1]
        "#);
    } else {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        [EOF]
        [exit status: 1]
        "#);
    }
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    std::fs::create_dir(test_env.env_root().join("failed")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "bad", "failed"]);
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        [EOF]
        [exit status: 1]
        "#);
    } else {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        [EOF]
        [exit status: 1]
        "#);
    }
    assert!(test_env.env_root().join("failed").exists());
    assert!(!test_env.env_root().join("failed").join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let output = test_env.run_jj_in(".", ["git", "clone", "bad", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }
    assert!(test_env.env_root().join("clone").join(".jj").exists());

    // Try cloning into an existing workspace
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Try cloning into an existing file
    std::fs::write(test_env.env_root().join("file"), "contents").unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "file"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Try cloning into non-empty, non-workspace directory
    std::fs::remove_dir_all(test_env.env_root().join("clone").join(".jj")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Clone into a nested path
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "nested/path/to/repo"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuzqqzqu cf5d593e (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_bad_source(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }

    let output = test_env.run_jj_in(".", ["git", "clone", "", "dest"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: local path "" does not specify a path to a repository
    [EOF]
    [exit status: 2]
    "#);
    }

    // Invalid port number
    let output = test_env.run_jj_in(
        ".",
        ["git", "clone", "https://example.net:bad-port/bar", "dest"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: URL "https://example.net:bad-port/bar" can not be parsed as valid URL
    Caused by: invalid port number
    [EOF]
    [exit status: 2]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_colocate(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);

    // Clone an empty repo
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "empty", "--colocate"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    [EOF]
    "#);
    }

    // git_target path should be relative to the store
    let store_path = test_env
        .env_root()
        .join(PathBuf::from_iter(["empty", ".jj", "repo", "store"]));
    let git_target_file_contents = std::fs::read_to_string(store_path.join("git_target")).unwrap();
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        git_target_file_contents.replace(path::MAIN_SEPARATOR, "/"),
        @"../../../.git");
    }

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone", "--colocate"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuqppmxq f78d2645 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
    assert!(test_env.env_root().join("clone").join("file").exists());
    assert!(test_env.env_root().join("clone").join(".git").exists());

    eprintln!(
        "{:?}",
        git_repo.head().expect("Repo head should be set").name()
    );

    let jj_git_repo = git::open(test_env.env_root().join("clone"));
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
    insta::allow_duplicates! {
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
    }

    // The old default bookmark "master" shouldn't exist.
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone")), @r"
    main: qomsplrm ebeb70d8 message
      @git: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");
    }

    // Subsequent fetch should just work even if the source path was relative
    let output = test_env.run_jj_in(&test_env.env_root().join("clone"), ["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    }

    // Failed clone should clean up the destination directory
    std::fs::create_dir(test_env.env_root().join("bad")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "--colocate", "bad", "failed"]);
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        [EOF]
        [exit status: 1]
        "#);
    } else {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        [EOF]
        [exit status: 1]
        "#);
    }
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    std::fs::create_dir(test_env.env_root().join("failed")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "--colocate", "bad", "failed"]);
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        [EOF]
        [exit status: 1]
        "#);
    } else {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        [EOF]
        [exit status: 1]
        "#);
    }
    assert!(test_env.env_root().join("failed").exists());
    assert!(!test_env.env_root().join("failed").join(".git").exists());
    assert!(!test_env.env_root().join("failed").join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let output = test_env.run_jj_in(".", ["git", "clone", "--colocate", "bad", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }
    assert!(test_env.env_root().join("clone").join(".git").exists());
    assert!(test_env.env_root().join("clone").join(".jj").exists());

    // Try cloning into an existing workspace
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone", "--colocate"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Try cloning into an existing file
    std::fs::write(test_env.env_root().join("file"), "contents").unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "file", "--colocate"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Try cloning into non-empty, non-workspace directory
    std::fs::remove_dir_all(test_env.env_root().join("clone").join(".jj")).unwrap();
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone", "--colocate"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Destination path exists and is not an empty directory
    [EOF]
    [exit status: 1]
    ");
    }

    // Clone into a nested path
    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "clone",
            "source",
            "nested/path/to/repo",
            "--colocate",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: vzqnnsmr 589d0921 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_remote_default_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
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

    // All fetched bookmarks will be imported if auto-local-bookmark is on
    test_env.add_config("git.auto-local-bookmark = true");
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone1"
    bookmark: feature1@origin [new] tracked
    bookmark: main@origin     [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 feature1 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone1")), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    main: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");
    }

    // "trunk()" alias should be set to default bookmark "main"
    let output = test_env.run_jj_in(
        &test_env.env_root().join("clone1"),
        ["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = "main@origin"
    [EOF]
    "#);
    }

    // Only the default bookmark will be imported if auto-local-bookmark is off
    test_env.add_config("git.auto-local-bookmark = false");
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone2"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: rzvqmyuk 018092c2 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 feature1@origin main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone2")), @r"
    feature1@origin: qomsplrm ebeb70d8 message
    main: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    [EOF]
    ");
    }

    // Change the default bookmark in remote
    git::set_symbolic_reference(&git_repo, "HEAD", "refs/heads/feature1");
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone3"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone3"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `feature1@origin`
    Working copy now at: nppvrztz 5fd587f4 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 feature1 main@origin | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone3")), @r"
    feature1: qomsplrm ebeb70d8 message
      @origin: qomsplrm ebeb70d8 message
    main@origin: qomsplrm ebeb70d8 message
    [EOF]
    ");
    }

    // "trunk()" alias should be set to new default bookmark "feature1"
    let output = test_env.run_jj_in(
        &test_env.env_root().join("clone3"),
        ["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = "feature1@origin"
    [EOF]
    "#);
    }
}

// A branch with a strange name should get quoted in the config. Windows doesn't
// like the strange name, so we don't run the test there.
#[cfg(unix)]
#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_remote_default_bookmark_with_escape(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
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

    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: "\""@origin [new] untracked
    Setting the revset alias `trunk()` to `"\""@origin`
    Working copy now at: sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 " | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }

    // "trunk()" alias should be escaped and quoted
    let output = test_env.run_jj_in(
        &test_env.env_root().join("clone"),
        ["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    revset-aliases.'trunk()' = '"\""@origin'
    [EOF]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_ignore_working_copy(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // Should not update working-copy files
    let output = test_env.run_jj_in(
        ".",
        ["git", "clone", "--ignore-working-copy", "source", "clone"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    [EOF]
    "#);
    }
    let clone_path = test_env.env_root().join("clone");

    let output = test_env.run_jj_in(&clone_path, ["status", "--ignore-working-copy"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy : sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit: qomsplrm ebeb70d8 main | message
    [EOF]
    ");
    }

    // TODO: Correct, but might be better to check out the root commit?
    let output = test_env.run_jj_in(&clone_path, ["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation eac759b9ab75).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_at_operation(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = test_env.run_jj_in(".", ["git", "clone", "--at-op=@-", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_with_remote_name(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path and a non-default remote name
    let output = test_env.run_jj_in(
        ".",
        ["git", "clone", "source", "clone", "--remote", "upstream"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@upstream [new] tracked
    Setting the revset alias `trunk()` to `main@upstream`
    Working copy now at: sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_with_remote_named_git(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    git::init(git_repo_path);

    let output = test_env.run_jj_in(".", ["git", "clone", "--remote=git", "source", "dest"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remote named 'git' is reserved for local Git repository
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_with_remote_with_slashes(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    git::init(git_repo_path);

    let output = test_env.run_jj_in(
        ".",
        ["git", "clone", "--remote=slash/origin", "source", "dest"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Git remotes with slashes are incompatible with jj: slash/origin
    [EOF]
    [exit status: 1]
    ");
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_trunk_deleted(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);
    let clone_path = test_env.env_root().join("clone");

    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    }

    let output = test_env.run_jj_in(
        &clone_path,
        ["bookmark", "forget", "--include-remotes", "main"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    Warning: Failed to resolve `revset-aliases.trunk()`: Revision `main@origin` doesn't exist
    Hint: Use `jj config edit --repo` to adjust the `trunk()` alias.
    [EOF]
    ");
    }

    let output = test_env.run_jj_in(&clone_path, ["log"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 2ca1c979
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
}

#[test]
fn test_git_clone_conditional_config() {
    let test_env = TestEnvironment::default();
    let source_repo_path = test_env.env_root().join("source");
    let old_workspace_root = test_env.env_root().join("old");
    let new_workspace_root = test_env.env_root().join("new");
    let source_git_repo = git::init(source_repo_path);
    set_up_non_empty_git_repo(&source_git_repo);

    let run_jj_in = |current_dir: &Path, args: &[&str]| {
        test_env.run_jj_with(|cmd| {
            cmd.current_dir(current_dir)
                .args(args)
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
        new_workspace_root = to_toml_value(new_workspace_root.to_str().unwrap()),
    });

    // Override operation.hostname by repo config, which should be loaded into
    // the command settings, but shouldn't be copied to the new repo.
    run_jj_in(test_env.env_root(), &["git", "init", "old"]).success();
    run_jj_in(
        &old_workspace_root,
        &["config", "set", "--repo", "operation.hostname", "old-repo"],
    )
    .success();
    run_jj_in(&old_workspace_root, &["new"]).success();
    let output = run_jj_in(&old_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  base@old-repo new empty commit
    ○  base@base add workspace 'default'
    ○  @
    [EOF]
    ");

    // Clone repo at the old workspace directory.
    let output = run_jj_in(
        &old_workspace_root,
        &["git", "clone", "../source", "../new"],
    );
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/new"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: zxsnswpr 9ffb42e2 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);
    run_jj_in(&new_workspace_root, &["new"]).success();
    let output = run_jj_in(&new_workspace_root, &["log", "-T", log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆  someone@example.org message
    │
    ~
    [EOF]
    ");
    let output = run_jj_in(&new_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base check out git remote's default branch
    ○  new-repo@base fetch from git remote into empty repo
    ○  new-repo@base add workspace 'default'
    ○  @
    [EOF]
    ");
}

#[cfg(feature = "git2")]
#[test]
fn test_git_clone_with_depth_git2() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config("git.subprocess = false");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // git does support shallow clones on the local transport, so it will work
    // (we cannot replicate git2's erroneous behaviour wrt git)
    // local transport does not support shallow clones so we just test that the
    // depth arg is passed on here
    let output = test_env.run_jj_in(".", ["git", "clone", "--depth", "1", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    Error: shallow fetch is not supported by the local transport; class=Net (12)
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_git_clone_with_depth_subprocess() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    let clone_path = test_env.env_root().join("clone");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    // git does support shallow clones on the local transport, so it will work
    // (we cannot replicate git2's erroneous behaviour wrt git)
    let output = test_env.run_jj_in(".", ["git", "clone", "--depth", "1", "source", "clone"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx 2ca1c979 (empty) (no description set)
    Parent commit      : qomsplrm ebeb70d8 main | message
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    "#);

    let output = test_env.run_jj_in(&clone_path, ["log"]);
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 2ca1c979
    │  (empty) (no description set)
    ◆  qomsplrm someone@example.org 1970-01-01 11:00:00 main ebeb70d8
    │  message
    ~
    [EOF]
    ");
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_invalid_immutable_heads(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    test_env.add_config("revset-aliases.'immutable_heads()' = 'unknown'");
    // Suppress lengthy warnings in commit summary template
    test_env.add_config("revsets.short-prefixes = ''");

    // The error shouldn't be counted as an immutable working-copy commit. It
    // should be reported.
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by: Revision `unknown` doesn't exist
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    "#);
    }
}

#[cfg_attr(feature = "git2", test_case(false; "use git2 for remote calls"))]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_malformed(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if !subprocess {
        test_env.add_config("git.subprocess = false");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    let clone_path = test_env.env_root().join("clone");
    // we can insert ".jj" entry to create a malformed clone
    set_up_git_repo_with_file(&git_repo, ".jj");

    // TODO: Perhaps, this should be a user error, not an internal error.
    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Internal error: Failed to check out commit 0a09cb41583450703459a2310d63da61456364ce
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    [EOF]
    [exit status: 255]
    "#);
    }

    // The cloned workspace isn't usable.
    let output = test_env.run_jj_in(&clone_path, ["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The working copy is stale (not updated since operation 57e024eb3edf).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    [EOF]
    [exit status: 1]
    ");
    }

    // The error can be somehow recovered.
    // TODO: add an update-stale flag to reset the working-copy?
    let output = test_env.run_jj_in(&clone_path, ["workspace", "update-stale"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Internal error: Failed to check out commit 0a09cb41583450703459a2310d63da61456364ce
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    [EOF]
    [exit status: 255]
    ");
    }
    let output = test_env.run_jj_in(&clone_path, ["new", "root()", "--ignore-working-copy"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @"");
    }
    let output = test_env.run_jj_in(&clone_path, ["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy : zsuskuln f652c321 (empty) (no description set)
    Parent commit: zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    }
}

#[test]
fn test_git_clone_no_git_executable() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.executable-path = 'jj-test-missing-program'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
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
    let invalid_git_executable_path = test_env.env_root().join("invalid").join("path");
    test_env.add_config(format!(
        "git.executable-path = {}",
        to_toml_value(invalid_git_executable_path.to_str().unwrap())
    ));
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git::init(git_repo_path);
    set_up_non_empty_git_repo(&git_repo);

    let output = test_env.run_jj_in(".", ["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r#"
    ------- stderr -------
    Fetching into new repo in "$TEST_ENV/clone"
    Error: Could not execute git process at specified path '$TEST_ENV/invalid/path'
    [EOF]
    [exit status: 1]
    "#);
}

#[must_use]
fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> CommandOutput {
    test_env.run_jj_in(repo_path, ["bookmark", "list", "--all-remotes"])
}
