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

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_resolution_of_git_tracking_bookmarks() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "main", "-m", "old_message"])
        .success();

    // Create local-git tracking bookmark
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    // Move the local bookmark somewhere else
    work_dir
        .run_jj(["describe", "-r", "main", "-m", "new_message"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 384a1421 (empty) new_message
      @git (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden a7f9930b (empty) old_message
    [EOF]
    ");

    // Test that we can address both revisions
    let query = |expr| {
        let template = r#"commit_id ++ " " ++ description"#;
        work_dir.run_jj(["log", "-r", expr, "-T", template, "--no-graph"])
    };
    insta::assert_snapshot!(query("main"), @r"
    384a14213707d776d0517f65cdcf954d07d88c40 new_message
    [EOF]
    ");
    insta::assert_snapshot!(query("main@git"), @r"
    a7f9930bb6d54ba39e6c254135b9bfe32041fea4 old_message
    [EOF]
    ");
    insta::assert_snapshot!(query(r#"remote_bookmarks(exact:"main", exact:"git")"#), @r"
    a7f9930bb6d54ba39e6c254135b9bfe32041fea4 old_message
    [EOF]
    ");
}

#[test]
fn test_git_export_conflicting_git_refs() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main/sub"])
        .success();
    let output = work_dir.run_jj(["git", "export"]);
    insta::with_settings!({filters => vec![("Failed to set: .*", "Failed to set: ...")]}, {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Warning: Failed to export some bookmarks:
          main/sub@git: Failed to set: ...
        Hint: Git doesn't allow a branch/tag name that looks like a parent directory of
        another (e.g. `foo` and `foo/bar`). Try to rename the bookmarks/tags that failed
        to export or their "parent" bookmarks/tags.
        [EOF]
        "#);
    });
}

#[test]
fn test_git_export_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::open(work_dir.root().join(".jj/repo/store/git"));

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(work_dir.run_jj(["log", "-ra@git"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 a e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");

    // Exported refs won't be removed by undoing the export, but the git-tracking
    // bookmark is. This is the same as remote-tracking bookmarks.
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 503f3c779aff (2001-02-03 08:05:08) create bookmark a pointing to commit e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");
    insta::assert_debug_snapshot!(get_git_repo_refs(&git_repo), @r#"
    [
        (
            "refs/heads/a",
            CommitId(
                "e8849ae12c709f2321908879bc724fdb2ab8a781",
            ),
        ),
    ]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["log", "-ra@git"]), @r"
    ------- stderr -------
    Error: Revision `a@git` doesn't exist
    Hint: Did you mean `a`?
    [EOF]
    [exit status: 1]
    ");

    // This would re-export bookmark "a" and create git-tracking bookmark.
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(work_dir.run_jj(["log", "-ra@git"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 a e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");
}

#[test]
fn test_git_import_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::open(work_dir.root().join(".jj/repo/store/git"));

    // Create bookmark "a" in git repo
    let commit_id = work_dir
        .run_jj(&["log", "-Tcommit_id", "--no-graph", "-r@"])
        .success()
        .stdout
        .into_raw();
    let commit_id = gix::ObjectId::from_hex(commit_id.as_bytes()).unwrap();
    git_repo
        .reference(
            "refs/heads/a",
            commit_id,
            gix::refs::transaction::PreviousValue::Any,
            "",
        )
        .unwrap();

    // Initial state we will return to after `undo`. There are no bookmarks.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    let base_operation_id = work_dir.current_operation_id();

    let output = work_dir.run_jj(["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a@git [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // "git import" can be undone by default.
    let output = work_dir.run_jj(["op", "restore", &base_operation_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 8f47435a3990 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    // Try "git import" again, which should re-import the bookmark "a".
    let output = work_dir.run_jj(["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a@git [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_git_import_move_export_with_default_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::open(work_dir.root().join(".jj/repo/store/git"));

    // Create bookmark "a" in git repo
    let commit_id = work_dir
        .run_jj(&["log", "-Tcommit_id", "--no-graph", "-r@"])
        .success()
        .stdout
        .into_raw();
    let commit_id = gix::ObjectId::from_hex(commit_id.as_bytes()).unwrap();
    git_repo
        .reference(
            "refs/heads/a",
            commit_id,
            gix::refs::transaction::PreviousValue::Any,
            "",
        )
        .unwrap();

    // Initial state we will try to return to after `op restore`. There are no
    // bookmarks.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    let base_operation_id = work_dir.current_operation_id();

    let output = work_dir.run_jj(["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a@git [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: qpvuntsm e8849ae1 (empty) (no description set)
      @git: qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    // Move bookmark "a" and export to git repo
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "set", "a", "--to=@"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: royxmykx e7d0d5fd (empty) (no description set)
      @git (behind by 1 commits): qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["git", "export"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: royxmykx e7d0d5fd (empty) (no description set)
      @git: royxmykx e7d0d5fd (empty) (no description set)
    [EOF]
    ");

    // "git import" can be undone with the default `restore` behavior, as shown in
    // the previous test. However, "git export" can't: the bookmarks in the git
    // repo stay where they were.
    let output = work_dir.run_jj(["op", "restore", &base_operation_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 8f47435a3990 (2001-02-03 08:05:07) add workspace 'default'
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
    insta::assert_debug_snapshot!(get_git_repo_refs(&git_repo), @r#"
    [
        (
            "refs/heads/a",
            CommitId(
                "e7d0d5fdaf96051d0dacec1e74d9413d64a15822",
            ),
        ),
    ]
    "#);

    // The last bookmark "a" state is imported from git. No idea what's the most
    // intuitive result here.
    let output = work_dir.run_jj(["git", "import"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: a@git [new] tracked
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    a: royxmykx e7d0d5fd (empty) (no description set)
      @git: royxmykx e7d0d5fd (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_git_import_export_stats_color() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::open(work_dir.root().join(".jj/repo/store/git"));

    work_dir.run_jj(["bookmark", "set", "-r@", "foo"]).success();
    work_dir
        .run_jj(["bookmark", "set", "-r@", "'un:exportable'"])
        .success();
    work_dir.run_jj(["new", "--no-edit", "root()"]).success();
    let other_commit_id = work_dir
        .run_jj(&["log", "-Tcommit_id", "--no-graph", "-rvisible_heads() ~ @"])
        .success()
        .stdout
        .into_raw();

    let output = work_dir
        .run_jj(["git", "export", "--color=always"])
        .success();
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    [1m[38;5;3mWarning: [39mFailed to export some bookmarks:[0m
      [38;5;5m"un:exportable"@git[39m: Failed to set: The ref name or path is not a valid ref name: A reference must be a valid tag name as well: A ref must not contain invalid bytes or ascii control characters: ":"
    [1m[38;5;6mHint: [0m[39mGit doesn't allow a branch/tag name that looks like a parent directory of[39m
    [39manother (e.g. `foo` and `foo/bar`). Try to rename the bookmarks/tags that failed[39m
    [39mto export or their "parent" bookmarks/tags.[39m
    [EOF]
    "#);

    let other_commit_id = gix::ObjectId::from_hex(other_commit_id.as_bytes()).unwrap();
    for name in ["refs/heads/foo", "refs/heads/bar", "refs/tags/baz"] {
        git_repo
            .reference(
                name,
                other_commit_id,
                gix::refs::transaction::PreviousValue::Any,
                "",
            )
            .unwrap();
    }

    let output = work_dir
        .run_jj(["git", "import", "--color=always"])
        .success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: [38;5;5mbar@git[39m [new] tracked
    bookmark: [38;5;5mfoo@git[39m [updated] tracked
    tag: [38;5;5mbaz@git[39m [new] 
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["bookmark", "list", "--all-remotes"])
}

fn get_git_repo_refs(git_repo: &gix::Repository) -> Vec<(bstr::BString, CommitId)> {
    let mut refs: Vec<_> = git_repo
        .references()
        .unwrap()
        .all()
        .unwrap()
        .filter_ok(|git_ref| {
            matches!(
                git_ref.name().category(),
                Some(gix::reference::Category::Tag)
                    | Some(gix::reference::Category::LocalBranch)
                    | Some(gix::reference::Category::RemoteBranch),
            )
        })
        .filter_map_ok(|mut git_ref| {
            let full_name = git_ref.name().as_bstr().to_owned();
            let git_commit = git_ref.peel_to_commit().ok()?;
            let commit_id = CommitId::from_bytes(git_commit.id().as_bytes());
            Some((full_name, commit_id))
        })
        .try_collect()
        .unwrap();
    refs.sort();
    refs
}
