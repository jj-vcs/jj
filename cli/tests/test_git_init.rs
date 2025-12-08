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
use std::path::PathBuf;

use indoc::formatdoc;
use test_case::test_case;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::to_toml_value;

fn init_git_repo(git_repo_path: &Path, bare: bool) -> gix::Repository {
    let git_repo = if bare {
        git::init_bare(git_repo_path)
    } else {
        git::init(git_repo_path)
    };

    let git::CommitResult { commit_id, .. } = git::add_commit(
        &git_repo,
        "refs/heads/my-bookmark",
        "some-file",
        b"some content",
        "My commit message",
        &[],
    );
    git::set_head_to_id(&git_repo, commit_id);
    git_repo
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["bookmark", "list", "--all-remotes"])
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
      commit_id.short(),
      bookmarks,
      description,
    )"#;
    work_dir.run_jj(["log", "-T", template, "-r=all()"])
}

#[must_use]
fn get_colocation_status(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj([
        "git",
        "colocation",
        "status",
        "--ignore-working-copy",
        "--quiet", // suppress hint
    ])
}

fn read_git_target(work_dir: &TestWorkDir) -> String {
    String::from_utf8(work_dir.read_file(".jj/repo/store/git_target").into()).unwrap()
}

#[test]
fn test_git_init_internal() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    [EOF]
    "#);

    let work_dir = test_env.work_dir("repo");
    let jj_path = work_dir.root().join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(work_dir.root().is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(store_path.join("git").is_dir());
    assert_eq!(read_git_target(&work_dir), "git");
}

#[test]
fn test_git_init_internal_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("").create_dir("repo");
    work_dir.write_file("file1", "");

    let output = work_dir.run_jj(["git", "init", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_internal_at_operation() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("").create_dir("repo");

    let output = work_dir.run_jj(["git", "init", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, bare);

    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Working copy  (@) now at: sqpuoqvx ed6b5138 (empty) (no description set)
    Parent commit (@-)      : nntyzxmz e80a42cc my-bookmark | My commit message
    Added 1 files, modified 0 files, removed 0 files
    Initialized repo in "repo"
    [EOF]
    "#);
    }

    let work_dir = test_env.work_dir("repo");
    let jj_path = work_dir.root().join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(work_dir.root().is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    let unix_git_target_file_contents = read_git_target(&work_dir).replace('\\', "/");
    if bare {
        assert!(unix_git_target_file_contents.ends_with("/git-repo"));
    } else {
        assert!(unix_git_target_file_contents.ends_with("/git-repo/.git"));
    }

    // Check that the Git repo's HEAD got checked out
    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&work_dir), @r"
        @  ed6b513890ae
        ○  e80a42cccd06 my-bookmark My commit message
        ◆  000000000000
        [EOF]
        ");
        insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
        Workspace is currently not colocated with Git.
        Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
        [EOF]
        ");
    }
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external_import_trunk(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_git_repo(&git_repo_path, bare);

    // Add remote bookmark "trunk" for remote "origin", and set it as "origin/HEAD"
    let oid = git_repo
        .find_reference("refs/heads/my-bookmark")
        .unwrap()
        .id();

    git_repo
        .reference(
            "refs/remotes/origin/trunk",
            oid.detach(),
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create remote ref",
        )
        .unwrap();

    git::set_symbolic_reference(
        &git_repo,
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    );

    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Setting the revset alias `trunk()` to `trunk@origin`
    Working copy  (@) now at: sqpuoqvx ed6b5138 (empty) (no description set)
    Parent commit (@-)      : nntyzxmz e80a42cc my-bookmark trunk@origin | My commit message
    Added 1 files, modified 0 files, removed 0 files
    Initialized repo in "repo"
    [EOF]
    "#);
    }

    // "trunk()" alias should be set to remote "origin"'s default bookmark "trunk"
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["config", "list", "--repo", "revset-aliases.\"trunk()\""]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r#"
        revset-aliases."trunk()" = "trunk@origin"
        [EOF]
        "#);
    }
}

#[test]
fn test_git_init_external_import_trunk_upstream_takes_precedence() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_git_repo(&git_repo_path, false);

    let oid = git_repo
        .find_reference("refs/heads/my-bookmark")
        .unwrap()
        .id();

    // Add both upstream and origin remotes with different default branches
    // upstream has "develop" as default
    git_repo
        .reference(
            "refs/remotes/upstream/develop",
            oid.detach(),
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create upstream remote ref",
        )
        .unwrap();

    git::set_symbolic_reference(
        &git_repo,
        "refs/remotes/upstream/HEAD",
        "refs/remotes/upstream/develop",
    );

    // origin has "trunk" as default
    git_repo
        .reference(
            "refs/remotes/origin/trunk",
            oid.detach(),
            gix::refs::transaction::PreviousValue::MustNotExist,
            "create origin remote ref",
        )
        .unwrap();

    git::set_symbolic_reference(
        &git_repo,
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    );

    let output = test_env.run_jj_in(
        ".",
        [
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Setting the revset alias `trunk()` to `develop@upstream`
    Working copy  (@) now at: sqpuoqvx ed6b5138 (empty) (no description set)
    Parent commit (@-)      : nntyzxmz e80a42cc develop@upstream my-bookmark trunk@origin | My commit message
    Added 1 files, modified 0 files, removed 0 files
    Initialized repo in "repo"
    [EOF]
    "#);
    }

    // "trunk()" alias should be set to "upstream"'s default, not "origin"'s
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["config", "list", "--repo", "revset-aliases.\"trunk()\""]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r#"
        revset-aliases."trunk()" = "develop@upstream"
        [EOF]
        "#);
    }
}

#[test]
fn test_git_init_external_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let work_dir = test_env.work_dir("").create_dir("repo");
    work_dir.write_file("file1", "");

    // No snapshot should be taken
    let output = work_dir.run_jj([
        "git",
        "init",
        "--ignore-working-copy",
        "--git-repo",
        git_repo_path.to_str().unwrap(),
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_at_operation() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let work_dir = test_env.work_dir("").create_dir("repo");

    let output = work_dir.run_jj([
        "git",
        "init",
        "--at-op=@-",
        "--git-repo",
        git_repo_path.to_str().unwrap(),
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_non_existent_directory() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "repo", "--git-repo", "non-existent"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to access the repository
    Caused by:
    1: Cannot access $TEST_ENV/non-existent
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_init_external_non_existent_git_directory() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let output = test_env.run_jj_in(".", ["git", "init", "repo", "--git-repo", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to access the repository
    Caused by:
    1: Failed to open git repository
    2: "$TEST_ENV/repo" does not appear to be a git repository
    3: Missing HEAD at '.git/HEAD'
    [EOF]
    [exit status: 1]
    "#);
    let jj_path = work_dir.root().join(".jj");
    assert!(!jj_path.exists());
}

#[test]
fn test_git_init_colocated_via_git_repo_path() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    init_git_repo(work_dir.root(), false);
    let output = work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);

    let jj_path = work_dir.root().join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(work_dir.root().is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(
        read_git_target(&work_dir)
            .replace('\\', "/")
            .ends_with("../../../.git")
    );

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_git_repo_path_gitlink() {
    let test_env = TestEnvironment::default();
    // <jj_work_dir>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_git_repo(&git_repo_path, false);
    let jj_work_dir = test_env.work_dir("").create_dir("repo");
    git::create_gitlink(jj_work_dir.root(), git_repo.path());

    assert!(jj_work_dir.root().join(".git").is_file());
    let output = jj_work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&jj_work_dir), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    jj_work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory() {
    let test_env = TestEnvironment::default();
    // <jj_work_dir>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let jj_work_dir = test_env.work_dir("").create_dir("repo");
    std::os::unix::fs::symlink(git_repo_path.join(".git"), jj_work_dir.root().join(".git"))
        .unwrap();
    let output = jj_work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&jj_work_dir), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    jj_work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory_without_bare_config() {
    let test_env = TestEnvironment::default();
    // <jj_work_dir>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo.git");
    let jj_work_dir = test_env.work_dir("repo");
    // Set up git repo without core.bare set (as the "repo" tool would do.)
    // The core.bare config is deduced from the directory name.
    let git_repo = init_git_repo(jj_work_dir.root(), false);
    git::remove_config_value(git_repo, "config", "bare");

    std::fs::rename(jj_work_dir.root().join(".git"), &git_repo_path).unwrap();
    std::os::unix::fs::symlink(&git_repo_path, jj_work_dir.root().join(".git")).unwrap();
    let output = jj_work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&jj_work_dir), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    jj_work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_gitlink() {
    let test_env = TestEnvironment::default();
    // <jj_work_dir>/.git -> <git_workdir_path>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_workdir_path = test_env.env_root().join("git-workdir");
    let git_repo = init_git_repo(&git_repo_path, false);
    std::fs::create_dir(&git_workdir_path).unwrap();
    git::create_gitlink(&git_workdir_path, git_repo.path());
    assert!(git_workdir_path.join(".git").is_file());
    let jj_work_dir = test_env.work_dir("").create_dir("repo");
    std::os::unix::fs::symlink(
        git_workdir_path.join(".git"),
        jj_work_dir.root().join(".git"),
    )
    .unwrap();
    let output = jj_work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(read_git_target(&jj_work_dir), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    jj_work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&jj_work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&jj_work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_git_repo_path_imported_refs() {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = '*'");

    // Set up remote refs
    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    remote_dir
        .run_jj(["bookmark", "create", "-r@", "local-remote", "remote-only"])
        .success();
    remote_dir.run_jj(["new"]).success();
    remote_dir.run_jj(["git", "export"]).success();

    let remote_git_path = remote_dir
        .root()
        .join(PathBuf::from_iter([".jj", "repo", "store", "git"]));
    let set_up_local_repo = |local_path: &Path| {
        let git_repo = git::clone(local_path, remote_git_path.to_str().unwrap(), None);
        let git_ref = git_repo
            .find_reference("refs/remotes/origin/local-remote")
            .unwrap();
        git_repo
            .reference(
                "refs/heads/local-remote",
                git_ref.target().id().to_owned(),
                gix::refs::transaction::PreviousValue::MustNotExist,
                "move local-remote bookmark",
            )
            .unwrap();
    };

    // With remotes.origin.auto-track-bookmarks = '*'
    let local_dir = test_env.work_dir("local1");
    set_up_local_repo(local_dir.root());
    let output = local_dir.run_jj(["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(get_bookmark_output(&local_dir), @r"
    local-remote: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
      @origin: qpvuntsm e8849ae1 (empty) (no description set)
    remote-only: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
      @origin: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // With remotes.origin.auto-track-bookmarks = '~*'
    test_env.add_config("remotes.origin.auto-track-bookmarks = '~*'");
    let local_dir = test_env.work_dir("local2");
    set_up_local_repo(local_dir.root());
    let output = local_dir.run_jj(["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Hint: The following remote bookmarks aren't associated with the existing local bookmarks:
      local-remote@origin
    Hint: Run the following command to keep local bookmarks updated on future pulls:
      jj bookmark track local-remote --remote=origin
    Initialized repo in "."
    [EOF]
    "#);
    insta::assert_snapshot!(get_bookmark_output(&local_dir), @r"
    local-remote: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    local-remote@origin: qpvuntsm e8849ae1 (empty) (no description set)
    remote-only@origin: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_dirty_working_copy() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = init_git_repo(work_dir.root(), false);

    let mut index_manager = git::IndexManager::new(&git_repo);

    index_manager.add_file("new-staged-file", b"new content");
    index_manager.add_file("some-file", b"new content");
    index_manager.sync_index();

    work_dir.write_file("unstaged-file", "new content");
    insta::assert_debug_snapshot!(git::status(&git_repo), @r#"
    [
        GitStatus {
            path: "new-staged-file",
            status: Index(
                Addition,
            ),
        },
        GitStatus {
            path: "some-file",
            status: Index(
                Modification,
            ),
        },
        GitStatus {
            path: "unstaged-file",
            status: Worktree(
                Added,
            ),
        },
    ]
    "#);

    let output = work_dir.run_jj(["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    [EOF]
    "#);

    // Working-copy changes should have been snapshotted.
    let output = work_dir.run_jj(["log", "-s", "--ignore-working-copy"]);
    insta::assert_snapshot!(output, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 6efc2a53
    │  (no description set)
    │  C {some-file => new-staged-file}
    │  M some-file
    │  C {some-file => unstaged-file}
    ○  nntyzxmz someone@example.org 1970-01-01 11:00:00 my-bookmark e80a42cc
    │  My commit message
    │  A some-file
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Git index should be consistent with the working copy parent. With the
    // current implementation, the index is unchanged. Since jj created new
    // working copy commit, it's also okay to update the index reflecting the
    // working copy commit or the working copy parent.
    insta::assert_debug_snapshot!(git::status(&git_repo), @r#"
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
        GitStatus {
            path: "new-staged-file",
            status: Index(
                Addition,
            ),
        },
        GitStatus {
            path: "some-file",
            status: Index(
                Modification,
            ),
        },
        GitStatus {
            path: "unstaged-file",
            status: Worktree(
                IntentToAdd,
            ),
        },
    ]
    "#);
}

#[test]
fn test_git_init_colocated_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    init_git_repo(work_dir.root(), false);
    work_dir.write_file("file1", "");

    let output = work_dir.run_jj(["git", "init", "--ignore-working-copy", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --ignore-working-copy is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_colocated_at_operation() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    init_git_repo(work_dir.root(), false);

    let output = work_dir.run_jj(["git", "init", "--at-op=@-", "--colocate"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --at-op is not respected
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_git_init_external_but_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let work_dir = test_env.work_dir("repo");
    git::init(&git_repo_path);
    init_git_repo(work_dir.root(), false);
    let output = work_dir.run_jj(["git", "init", "--git-repo", git_repo_path.to_str().unwrap()]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "."
    [EOF]
    "#);

    // The local ".git" repository is unrelated, so no commits should be imported
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e8849ae12c70
    ◆  000000000000
    [EOF]
    ");

    // Check that Git HEAD is not set because this isn't a colocated workspace
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  1c1c95df80e5
    ○  e8849ae12c70
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently not colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    init_git_repo(work_dir.root(), false);

    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "repo"
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[test]
fn test_git_init_colocated_via_config_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    init_git_repo(work_dir.root(), false);

    test_env.add_config("git.colocate = true");

    let output = test_env.run_jj_in(".", ["git", "init", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Done importing changes from the underlying Git repo.
    Initialized repo in "repo"
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e80a42cccd069007c7a2bb427ac7f1d10b408633
    [EOF]
    ");

    // Check that the Git repo's HEAD moves
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  bacc067e7740
    ○  f3fe58bc88cc
    ○  e80a42cccd06 my-bookmark My commit message
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: f3fe58bc88ccfb820b930a21297d8e48bf76ac2a
    [EOF]
    ");
}

#[test]
fn test_git_init_no_colocate() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config("git.colocate = true");

    let output = test_env.run_jj_in(".", ["git", "init", "--no-colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    [EOF]
    "#);

    assert!(!work_dir.root().join(".git").exists());
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_not_exists() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    Hint: Running `git clean -xdf` will remove `.jj/`!
    [EOF]
    "#);
    // No HEAD ref is available yet
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e8849ae12c70
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Create the default bookmark (create both in case we change the default)
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main", "master"])
        .success();

    // If .git/HEAD pointed to the default bookmark, new working-copy commit would
    // be created on top.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  e8849ae12c70 main master
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
}

#[test]
fn test_git_init_conditional_config() {
    let test_env = TestEnvironment::default();
    let old_workspace_dir = test_env.work_dir("old");
    let new_workspace_dir = test_env.work_dir("new");

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
    run_jj(&test_env.work_dir(""), &["git", "init", "old"]).success();
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

    // Create new repo at the old workspace directory.
    let output = run_jj(&old_workspace_dir, &["git", "init", "../new"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Initialized repo in "../new"
    [EOF]
    "#);
    run_jj(&new_workspace_dir, &["new"]).success();
    let output = run_jj(&new_workspace_dir, &["log", "-T", log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆
    [EOF]
    ");
    let output = run_jj(&new_workspace_dir, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(output, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base add workspace 'default'
    ○  @
    [EOF]
    ");
}

#[test]
fn test_git_init_bad_wc_path() {
    let test_env = TestEnvironment::default();
    std::fs::write(test_env.env_root().join("existing-file"), b"").unwrap();
    let output = test_env.run_jj_in(".", ["git", "init", "existing-file"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to create workspace
    [EOF]
    [exit status: 1]
    ");
}
