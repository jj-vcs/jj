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

use std::fmt::Write as _;
use std::path::Path;

use testutils::TestResult;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_git_colocated() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());

    // Create an initial commit in Git
    let tree_id = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file",
        b"contents",
        "initial",
        &[],
    )
    .tree_id;
    git::checkout_tree_index(&git_repo, tree_id);
    assert_eq!(work_dir.read_file("file"), b"contents");
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );

    // Import the repo
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  524826059adc6f74de30f6be8f8eb86715d75b62
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 97358f54806c7cd005ed5ade68a779595efbae7e
    [EOF]
    ");

    // Modify the working copy. The working-copy commit should changed, but the Git
    // HEAD commit should not
    work_dir.write_file("file", "modified");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  9dfe8c7005c8dff6078ecdfd953c6bfddc633c90
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 97358f54806c7cd005ed5ade68a779595efbae7e
    [EOF]
    ");

    // Create a new change from jj and check that it's reflected in Git
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  83f7a30bd92ed65a71d8394e9a617a10a7c205c2
    ○  9dfe8c7005c8dff6078ecdfd953c6bfddc633c90
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"9dfe8c7005c8dff6078ecdfd953c6bfddc633c90"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 9dfe8c7005c8dff6078ecdfd953c6bfddc633c90
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_intent_to_add() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // A file added directly on top of the root commit should be marked as
    // intent-to-add
    work_dir.write_file("file1.txt", "contents");
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file1.txt");

    // Previously, this would fail due to the empty blob not being written to the
    // store when marking files as intent-to-add.
    work_dir.run_jj(["util", "gc"]).success();

    // Another new file should be marked as intent-to-add
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2.txt", "contents");
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file2.txt
    ");

    let op_id_new_file = work_dir.current_operation_id();

    // After creating a new commit, it should not longer be marked as intent-to-add
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2.txt", "contents");
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    ");

    // If we edit an existing commit, new files are marked as intent-to-add
    work_dir.run_jj(["edit", "@-"]).success();
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file2.txt
    ");

    // If we remove the added file, it's removed from the index
    work_dir.remove_file("file2.txt");
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt");

    // If we untrack the file, it's removed from the index
    work_dir
        .run_jj(["op", "restore", op_id_new_file.as_str()])
        .success();
    work_dir.write_file(".gitignore", "file2.txt");
    work_dir.run_jj(["file", "untrack", "file2.txt"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 .gitignore
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    ");

    Ok(())
}

#[test]
fn test_git_colocated_unborn_bookmark() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());

    // add a file to an (in memory) index
    let add_file_to_index = |name: &str, data: &str| {
        let mut index_manager = git::IndexManager::new(&git_repo);
        index_manager.add_file(name, data.as_bytes());
        index_manager.sync_index();
    };

    // checkout index (i.e., drop the in-memory changes)
    let checkout_index = || {
        let mut index = git_repo.open_index().unwrap();
        let objects = git_repo.objects.clone();
        gix::worktree::state::checkout(
            &mut index,
            git_repo.workdir().unwrap(),
            objects,
            &gix::progress::Discard,
            &gix::progress::Discard,
            &gix::interrupt::IS_INTERRUPTED,
            gix::worktree::state::checkout::Options::default(),
        )
        .unwrap();
    };

    // Initially, HEAD isn't set.
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    assert!(git_repo.head()?.is_unborn());
    assert_eq!(
        git_repo.head_name()?.unwrap().as_bstr(),
        b"refs/heads/master"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Stage some change, and check out root. This shouldn't clobber the HEAD.
    add_file_to_index("file0", "");
    let output = work_dir.run_jj(["new", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: psuskuln 78fad9fc (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    assert!(git_repo.head()?.is_unborn());
    assert_eq!(
        git_repo.head_name()?.unwrap().as_bstr(),
        b"refs/heads/master"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  78fad9fc172edf4d55cf6d090ffb45c9d477a421
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @"
    The working copy has no changes.
    Working copy  (@) : psuskuln 78fad9fc (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Stage some change, and create new HEAD. This shouldn't move the default
    // bookmark.
    add_file_to_index("file1", "");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: uruxwmqv 404c464d (empty) (no description set)
    Parent commit (@-)      : psuskuln 13f22618 (no description set)
    [EOF]
    ");
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"13f22618dd6530ad1909b1081c63b6974b206c0f"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  404c464d074a10abfbdf0173e5f8aa010727f7f4
    ○  13f22618dd6530ad1909b1081c63b6974b206c0f
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 13f22618dd6530ad1909b1081c63b6974b206c0f
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @"
    The working copy has no changes.
    Working copy  (@) : uruxwmqv 404c464d (empty) (no description set)
    Parent commit (@-): psuskuln 13f22618 (no description set)
    [EOF]
    ");

    // Assign the default bookmark. The bookmark is no longer "unborn".
    work_dir
        .run_jj(["bookmark", "create", "-r@-", "master"])
        .success();

    // Stage some change, and check out root again. This should unset the HEAD.
    // https://github.com/jj-vcs/jj/issues/1495
    add_file_to_index("file2", "");
    let output = work_dir.run_jj(["new", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: wqnwkozp 88e8407a (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    assert!(git_repo.head()?.is_unborn());
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  88e8407a4f0a5e6f40a7c6c494106764adc00fed
    │ ○  94a99b37620f5bb3c2960dbffbf65a6e5cfa1214
    │ ○  13f22618dd6530ad1909b1081c63b6974b206c0f master
    ├─╯
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @"
    The working copy has no changes.
    Working copy  (@) : wqnwkozp 88e8407a (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // New snapshot and commit can be created after the HEAD got unset.
    work_dir.write_file("file3", "");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: vyznsvlq 73cf66f6 (empty) (no description set)
    Parent commit (@-)      : wqnwkozp bb21bc2d (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  73cf66f69f26cbe791026d59a1867e6b47275b7d
    ○  bb21bc2dce2af92973fdd6d42686d77bd16bc466
    │ ○  94a99b37620f5bb3c2960dbffbf65a6e5cfa1214
    │ ○  13f22618dd6530ad1909b1081c63b6974b206c0f master
    ├─╯
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: bb21bc2dce2af92973fdd6d42686d77bd16bc466
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_export_bookmarks_on_snapshot() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();

    // Create bookmark pointing to the initial commit
    work_dir.write_file("file", "initial");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  82a10a4d9ef783fd68b661f40ce10dd80d599d9e foo
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // The bookmark gets updated when we modify the working copy, and it should get
    // exported to Git without requiring any other changes
    work_dir.write_file("file", "modified");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  00fc09f48ccf5c8b025a0f93b0ec3b0e4294a598 foo
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(git_repo
        .find_reference("refs/heads/foo")?
        .id()
        .to_string(), @"00fc09f48ccf5c8b025a0f93b0ec3b0e4294a598");

    Ok(())
}

#[test]
fn test_git_colocated_rebase_on_import() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();

    // Make some changes in jj and check that they're reflected in git
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["commit", "-m", "add a file"]).success();
    work_dir.write_file("file", "modified");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "master"])
        .success();
    work_dir.run_jj(["commit", "-m", "modify a file"]).success();
    // TODO: We shouldn't need this command here to trigger an import of the
    // refs/heads/master we just exported
    work_dir.run_jj(["st"]).success();

    // Move `master` backwards, which should result in commit2 getting hidden,
    // and the working-copy commit rebased.
    let parent_commit = git_repo
        .find_reference("refs/heads/master")?
        .peel_to_commit()?
        .parent_ids()
        .next()
        .unwrap()
        .detach();
    git_repo.reference(
        "refs/heads/master",
        parent_commit,
        gix::refs::transaction::PreviousValue::Any,
        "update ref",
    )?;
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d46583362b91d0e172aec469ea1689995540de81
    ○  cbd6c887108743a4abb0919305646a6a914a665e master add a file
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Abandoned 1 commits that are no longer reachable.
    Rebased 1 descendant commits off of commits rewritten from git
    Working copy  (@) now at: zsuskuln d4658336 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm cbd6c887 master | add a file
    Added 0 files, modified 1 files, removed 0 files
    Done importing changes from the underlying Git repo.
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_bookmarks() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    work_dir.run_jj(["new", "-m", "foo"]).success();
    work_dir.run_jj(["new", "@-", "-m", "bar"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  029c37b5d34cb575cd6b3e33ace2335e0cc66610 bar
    │ ○  74f128eafed036ef5507e302b24ccd5131c09665 foo
    ├─╯
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Create a bookmark in jj. It should be exported to Git even though it points
    // to the working-copy commit.
    work_dir
        .run_jj(["bookmark", "create", "-r@", "master"])
        .success();
    insta::assert_snapshot!(
        git_repo.find_reference("refs/heads/master")?.target().id().to_string(),
        @"029c37b5d34cb575cd6b3e33ace2335e0cc66610"
    );
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781"
    );

    // Update the bookmark in Git
    let target_id = work_dir
        .run_jj(["log", "--no-graph", "-T=commit_id", "-r=subject(foo)"])
        .success()
        .stdout
        .into_raw();
    git_repo.reference(
        "refs/heads/master",
        gix::ObjectId::from_hex(target_id.as_bytes())?,
        gix::refs::transaction::PreviousValue::Any,
        "test",
    )?;
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  507c0edcfc028f714f3c7a3027cb141f6610e867
    │ ○  74f128eafed036ef5507e302b24ccd5131c09665 master foo
    ├─╯
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Abandoned 1 commits that are no longer reachable.
    Working copy  (@) now at: yqosqzyt 507c0edc (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Done importing changes from the underlying Git repo.
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_bookmark_forget() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "foo"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e521ae80c3d70ec4b6dab193c8915c8de7cb7311 foo
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"
    foo: ylvkpnrz e521ae80 (empty) (no description set)
      @git: ylvkpnrz e521ae80 (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "forget", "--include-remotes", "foo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");
    // A forgotten bookmark is deleted in the git repo. For a detailed demo
    // explaining this, see `test_bookmark_forget_export` in
    // `test_bookmark_command.rs`.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");

    Ok(())
}

#[test]
fn test_git_colocated_bookmark_at_root() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["bookmark", "create", "foo", "-r=root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to zzzzzzzz 00000000 foo | (empty) (no description set)
    Warning: Failed to export some bookmarks:
      foo@git: Ref cannot point to the root commit in Git
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "move", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to qpvuntsm e8849ae1 foo | (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj([
        "bookmark",
        "move",
        "foo",
        "--allow-backwards",
        "--to=root()",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to zzzzzzzz 00000000 foo* | (empty) (no description set)
    Warning: Failed to export some bookmarks:
      foo@git: Ref cannot point to the root commit in Git
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_conflicting_git_refs() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    let output = work_dir.run_jj(["bookmark", "create", "-r@", "main/sub"]);
    insta::with_settings!({filters => vec![("Failed to set: .*", "Failed to set: ...")]}, {
        insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Warning: Target revision is empty.
        Created 1 bookmarks pointing to qpvuntsm e8849ae1 main main/sub | (empty) (no description set)
        Warning: Failed to export some bookmarks:
          main/sub@git: Failed to set: ...
        Hint: Git doesn't allow a branch/tag name that looks like a parent directory of
        another (e.g. `foo` and `foo/bar`). Try to rename the bookmarks/tags that failed
        to export or their "parent" bookmarks/tags.
        [EOF]
        "#);
    });

    Ok(())
}

#[test]
fn test_git_colocated_checkout_non_empty_working_copy() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();

    // Create an initial commit in Git
    // We use this to set HEAD to master
    let tree_id = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "file",
        b"contents",
        "initial",
        &[],
    )
    .tree_id;
    git::checkout_tree_index(&git_repo, tree_id);
    assert_eq!(work_dir.read_file("file"), b"contents");
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );

    work_dir.write_file("two", "y");

    work_dir.run_jj(["describe", "-m", "two"]).success();
    work_dir.run_jj(["new", "@-"]).success();
    let output = work_dir.run_jj(["describe", "-m", "new"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: nkmpptxz a2527ce8 (empty) new
    Parent commit (@-)      : slsumksp 97358f54 master | initial
    [EOF]
    ");

    assert_eq!(
        git_repo.head_name()?.unwrap().as_bstr(),
        b"refs/heads/master"
    );

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a2527ce855aecd5dd1e8af71d39873edb6e04212 new
    │ ○  6b0f7d59e0749d3a6ff2ecf686d5fa48023b7b93 two
    ├─╯
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 97358f54806c7cd005ed5ade68a779595efbae7e
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_fetch_deleted_or_moved_bookmark() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env.add_config("remotes.origin.auto-track-bookmarks = '*'");
    let origin_dir = test_env.work_dir("origin");
    git::init(origin_dir.root());
    origin_dir.run_jj(["git", "init", "--git-repo=."]).success();
    origin_dir.run_jj(["describe", "-m=A"]).success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "A"])
        .success();
    origin_dir.run_jj(["new", "-m=B_to_delete"]).success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "B_to_delete"])
        .success();
    origin_dir.run_jj(["new", "-m=original C", "@-"]).success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "C_to_move"])
        .success();

    let clone_dir = test_env.work_dir("clone");
    git::clone(clone_dir.root(), origin_dir.root().to_str().unwrap(), None);
    clone_dir.run_jj(["git", "init", "--git-repo=."]).success();
    clone_dir.run_jj(["new", "A"]).success();
    insta::assert_snapshot!(get_log_output(&clone_dir), @"
    @  b982f42f97d7191ab84657935e2fe1f24916e6f3
    │ ○  082bd179562bd973fa0355ee827b555bad397d88 C_to_move original C
    ├─╯
    │ ○  6def29fe5d0be7c497e6e6c757365376d961ff9d B_to_delete B_to_delete
    ├─╯
    ◆  8777db25171cace71ad014598663d5ffc4fae6b1 A A
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    origin_dir
        .run_jj(["bookmark", "delete", "B_to_delete"])
        .success();
    // Move bookmark C sideways
    origin_dir
        .run_jj(["describe", "C_to_move", "-m", "moved C"])
        .success();
    let output = clone_dir.run_jj(["git", "fetch"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    bookmark: B_to_delete@origin [deleted] untracked
    bookmark: C_to_move@origin   [updated] tracked
    Abandoned 1 commits that are no longer reachable:
      psuskuln 6def29fe B_to_delete@git | (empty) B_to_delete
    Updated 1 rewritten commits.
    [EOF]
    ");
    // "original C" and "B_to_delete" are abandoned, as the corresponding bookmarks
    // were deleted or moved on the remote (#864)
    insta::assert_snapshot!(get_log_output(&clone_dir), @"
    @  b982f42f97d7191ab84657935e2fe1f24916e6f3
    │ ○  f121e99940c996eb2f117e214d44937a963e34d3 C_to_move moved C
    ├─╯
    ◆  8777db25171cace71ad014598663d5ffc4fae6b1 A A
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_rebase_dirty_working_copy() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    work_dir.write_file("file", "base");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "old");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "feature"])
        .success();

    // Make the working-copy dirty, delete the checked out bookmark.
    work_dir.write_file("file", "new");
    git_repo.find_reference("refs/heads/feature")?.delete()?;

    // Because the working copy is dirty, the new working-copy commit will be
    // diverged. Therefore, the feature bookmark has change-delete conflict.
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @"
    Working copy changes:
    M file
    Working copy  (@) : ylvkpnrz 6282fedc feature?? | (no description set)
    Parent commit (@-): qpvuntsm f99015d7 (no description set)
    Warning: These bookmarks have conflicts:
      feature
    Hint: Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to resolve.
    [EOF]
    ------- stderr -------
    Warning: Failed to export some bookmarks:
      feature@git: Modified ref had been deleted in Git
    Done importing changes from the underlying Git repo.
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  6282fedc66bd0409f6e0b372890d9c013b9882ea feature??
    ○  f99015d7d9b82a5912ec4d96a18d2a4afbd8dd49
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // The working-copy content shouldn't be lost.
    insta::assert_snapshot!(work_dir.read_file("file"), @"new");

    Ok(())
}

#[test]
fn test_git_colocated_external_checkout() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    let git_check_out_ref = |name| -> TestResult {
        let target = git_repo
            .find_reference(name)?
            .into_fully_peeled_id()?
            .detach();
        git::set_head_to_id(&git_repo, target);
        Ok(())
    };

    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    work_dir.run_jj(["ci", "-m=A"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@-", "master"])
        .success();
    work_dir.run_jj(["new", "-m=B", "root()"]).success();
    work_dir.run_jj(["new"]).success();

    // Checked out anonymous bookmark
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e9d967013a147358e51843d0f8872dbb5a2de3b3
    ○  5febc7be568fde865c28f77dc4052b72c62b816e B
    │ ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Check out another bookmark by external command
    git_check_out_ref("refs/heads/master")?;

    // The old working-copy commit gets abandoned, but the whole bookmark should not
    // be abandoned. (#1042)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  7ceeaaae54c8ac99ad34eeed7fe1e896f535be99
    ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    │ ○  5febc7be568fde865c28f77dc4052b72c62b816e B
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    [EOF]
    ");

    // Edit non-head commit
    work_dir.run_jj(["new", "subject(B)"]).success();
    work_dir.run_jj(["new", "-m=C", "--no-edit"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  0890c7d45b6b6323fc99dca7a43af28c2c0e2347 C
    @  5be1f497ea0b178008cc945971568f4b631a62ce
    ○  5febc7be568fde865c28f77dc4052b72c62b816e B
    │ ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Check out another bookmark by external command
    git_check_out_ref("refs/heads/master")?;

    // The old working-copy commit shouldn't be abandoned. (#3747)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  277b693c61dcdea59ac26d6982370f78751f6ef5
    ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    │ ○  0890c7d45b6b6323fc99dca7a43af28c2c0e2347 C
    │ ○  5be1f497ea0b178008cc945971568f4b631a62ce
    │ ○  5febc7be568fde865c28f77dc4052b72c62b816e B
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    [EOF]
    ");

    // With --no-integrate-operation, the reset operation shouldn't persist
    work_dir.run_jj(["new", "subject(C)"]).success();
    git_check_out_ref("refs/heads/master")?;
    let output = work_dir.run_jj(["status", "--no-integrate-operation"]);
    insta::assert_snapshot!(output, @"
    The working copy has no changes.
    Working copy  (@) : wqnwkozp 8a57e340 (empty) (no description set)
    Parent commit (@-): qpvuntsm 8777db25 master | (empty) A
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    Operation left uncommitted because --no-integrate-operation was requested: 4e66050b9947
    [EOF]
    ");
    let output = work_dir.run_jj(["status", "--no-integrate-operation"]);
    insta::assert_snapshot!(output, @"
    The working copy has no changes.
    Working copy  (@) : lylxulpl 5c2c75fa (empty) (no description set)
    Parent commit (@-): qpvuntsm 8777db25 master | (empty) A
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    Operation left uncommitted because --no-integrate-operation was requested: e74d2bcb6913
    [EOF]
    ");

    Ok(())
}

#[test]
#[cfg_attr(windows, ignore = "uses POSIX sh")]
fn test_git_colocated_concurrent_checkout() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-mcommit1"]).success();
    work_dir.write_file("file1", "");
    work_dir.run_jj(["new", "-mcommit2"]).success();
    work_dir.write_file("file2", "");
    work_dir.run_jj(["new", "-mcommit3"]).success();

    // Run "jj commit" and "git checkout" concurrently
    let output = work_dir.run_jj([
        "commit",
        "--config=ui.editor=['sh', '-c', 'git checkout -q HEAD^']",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Failed to update Git HEAD ref
    Caused by: The reference "HEAD" should have content 646030fc00319f9b3a778b8c8d60c8ff02413713, actual content was 211c1ba69e5581709c911f9810112b8e5b0ee497
    Working copy  (@) now at: mzvwutvl c41c6649 (empty) (no description set)
    Parent commit (@-)      : psuskuln e6d5793c (empty) commit3
    [EOF]
    "#);

    // git_head() isn't updated because the export failed
    insta::assert_snapshot!(work_dir.run_jj(["log", "--summary", "--ignore-working-copy"]), @"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:11 c41c6649
    │  (empty) (no description set)
    ○  psuskuln test.user@example.com 2001-02-03 08:05:11 e6d5793c
    │  (empty) commit3
    ○  nkmpptxz test.user@example.com 2001-02-03 08:05:10 646030fc
    │  commit2
    │  A file2
    ○  ylvkpnrz test.user@example.com 2001-02-03 08:05:09 211c1ba6
    │  commit1
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // The current Git HEAD is imported on the next jj invocation
    insta::assert_snapshot!(work_dir.run_jj(["log", "--summary"]), @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:13 105eea93
    │  (empty) (no description set)
    │ ○  psuskuln test.user@example.com 2001-02-03 08:05:11 e6d5793c
    │ │  (empty) commit3
    │ ○  nkmpptxz test.user@example.com 2001-02-03 08:05:10 646030fc
    ├─╯  commit2
    │    A file2
    ○  ylvkpnrz test.user@example.com 2001-02-03 08:05:09 211c1ba6
    │  commit1
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_squash_undo() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    work_dir.run_jj(["ci", "-m=A"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @"
    @  rlvkpnrzqnoo 682c866b0a2f
    ○  qpvuntsmwlqt 8777db25171c A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");

    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @"
    @  zsuskulnrvyr e1c3034f23b9
    ○  qpvuntsmwlqt ba304e200f4f A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");
    work_dir.run_jj(["undo"]).success();
    // There should be no divergence here (#922)
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @"
    @  rlvkpnrzqnoo 682c866b0a2f
    ○  qpvuntsmwlqt 8777db25171c A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_undo_head_move() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    // Create new HEAD
    work_dir.run_jj(["new"]).success();
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e521ae80c3d70ec4b6dab193c8915c8de7cb7311
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");

    // HEAD should be unset
    work_dir.run_jj(["undo"]).success();
    assert!(git_repo.head()?.is_unborn());
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Create commit on non-root commit
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  3f0378f662f827e0e4f4ee78c505dda5ffca22a2
    ○  fa8ebb809983154839420ad3fc22b504f3b7972e
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: fa8ebb809983154839420ad3fc22b504f3b7972e
    [EOF]
    ");
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"fa8ebb809983154839420ad3fc22b504f3b7972e");

    // HEAD should be moved back
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 02eb4e1d35d7 (2001-02-03 08:05:15) new empty commit
    Restored to operation: e93edf58e767 (2001-02-03 08:05:14) new empty commit
    Working copy  (@) now at: truxwmqv fa8ebb80 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    assert!(git_repo.head()?.is_detached());
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  fa8ebb809983154839420ad3fc22b504f3b7972e
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_update_index_preserves_timestamps() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Create a commit with some files
    work_dir.write_file("file1.txt", "will be unchanged\n");
    work_dir.write_file("file2.txt", "will be modified\n");
    work_dir.write_file("file3.txt", "will be deleted\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "commit1"])
        .success();
    work_dir.run_jj(["new"]).success();

    // Create a commit with some changes to the files
    work_dir.write_file("file2.txt", "modified\n");
    work_dir.remove_file("file3.txt");
    work_dir.write_file("file4.txt", "added\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "commit2"])
        .success();
    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  cef9c4d8a5a7fbab004b7abe4b4f8f215d38bb3e
    ○  b4dd73903ef9ac426dd35a019692c0eb7d2eedef commit2
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) 2e0996000b7e ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) d5f7fc3f74f7 ctime=0:0 mtime=0:0 size=0 flags=0 file4.txt
    ");

    // Update index with stats for all files. We may want to do this automatically
    // in the future after we update the index in `git::reset_head` (#3786), but for
    // now, we at least want to preserve existing stat information when possible.
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 2e0996000b7e ctime=[nonzero] mtime=[nonzero] size=9 flags=0 file2.txt
    Unconflicted Mode(FILE) d5f7fc3f74f7 ctime=[nonzero] mtime=[nonzero] size=6 flags=0 file4.txt
    ");

    // Edit parent commit, causing the changes to be removed from the index without
    // touching the working copy
    work_dir.run_jj(["edit", "commit2"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  b4dd73903ef9ac426dd35a019692c0eb7d2eedef commit2
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain stat for unchanged file still.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 28d2718c947b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) 528557ab3a42 ctime=0:0 mtime=0:0 size=0 flags=0 file3.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file4.txt
    ");

    // Create sibling commit, causing working copy to match index
    work_dir.run_jj(["new", "commit1"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  2690256ebae7884d564a55640f07fbe889289f9a
    │ ○  b4dd73903ef9ac426dd35a019692c0eb7d2eedef commit2
    ├─╯
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain stat for unchanged file still.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 28d2718c947b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) 528557ab3a42 ctime=0:0 mtime=0:0 size=0 flags=0 file3.txt
    ");

    Ok(())
}

#[test]
fn test_git_colocated_update_index_merge_conflict() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Set up conflict files
    work_dir.write_file("conflict.txt", "base\n");
    work_dir.write_file("base.txt", "base\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "left\n");
    work_dir.write_file("left.txt", "left\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "right\n");
    work_dir.write_file("right.txt", "right\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 right.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 right.txt
    ");

    // Create merge conflict
    work_dir.run_jj(["new", "left", "right"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    27ab0d254a9693167a9940cc5424231dae69c96e
    ├─╮
    │ ○  35530dae260329267e22c23a97897dd43005bd7c right
    ○ │  1f13710f6a46af7b2949b90e52654dd96f559704 left
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Conflict should be added in index with correct blob IDs. The stat for
    // base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  7457e29b8deab4fcaafd35ad138a96dad9375518
    ×    27ab0d254a9693167a9940cc5424231dae69c96e
    ├─╮
    │ ○  35530dae260329267e22c23a97897dd43005bd7c right
    ○ │  1f13710f6a46af7b2949b90e52654dd96f559704 left
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should be the same after `jj new`.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    Ok(())
}

#[test]
fn test_git_colocated_update_index_rebase_conflict() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Set up conflict files
    work_dir.write_file("conflict.txt", "base\n");
    work_dir.write_file("base.txt", "base\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "left\n");
    work_dir.write_file("left.txt", "left\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "right\n");
    work_dir.write_file("right.txt", "right\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();

    work_dir.run_jj(["edit", "left"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  1f13710f6a46af7b2949b90e52654dd96f559704 left
    │ ○  35530dae260329267e22c23a97897dd43005bd7c right
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    ");

    // Create rebase conflict
    work_dir
        .run_jj(["rebase", "-r", "left", "-o", "right"])
        .success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  b0e2e0c4fdc8111581d218509bf0e9d64550e7fb left
    ○  35530dae260329267e22c23a97897dd43005bd7c right
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain files from parent commit, so there should be no conflict
    // in conflict.txt yet. The stat for base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  fd0ec12b8148f2851efaf23d2ca7b471eeaab4f7
    ×  b0e2e0c4fdc8111581d218509bf0e9d64550e7fb left
    ○  35530dae260329267e22c23a97897dd43005bd7c right
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Now the working copy commit's parent is conflicted, so the index should have
    // a conflict with correct blob IDs.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    Ok(())
}

#[test]
fn test_git_colocated_update_index_3_sided_conflict() -> TestResult {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Set up conflict files
    work_dir.write_file("conflict.txt", "base\n");
    work_dir.write_file("base.txt", "base\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "side-1\n");
    work_dir.write_file("side-1.txt", "side-1\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "side-1"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "side-2\n");
    work_dir.write_file("side-2.txt", "side-2\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "side-2"])
        .success();

    work_dir.run_jj(["new", "base"]).success();
    work_dir.write_file("conflict.txt", "side-3\n");
    work_dir.write_file("side-3.txt", "side-3\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "side-3"])
        .success();

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 side-3.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 side-3.txt
    ");

    // Create 3-sided merge conflict
    work_dir
        .run_jj(["new", "side-1", "side-2", "side-3"])
        .success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      be1155a2aee8f49a6132d0cb5d4dcb8e71281420
    ├─┬─╮
    │ │ ○  8e557c4fa56ee8713b44769e7a20b413d922931e side-3
    │ ○ │  13a224f2ddfb75582340c5a0d58c195d4314b798 side-2
    │ ├─╯
    ○ │  5b2fc6e007973fa56221fed78fa33aab06b4f386 side-1
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // We can't add conflicts with more than 2 sides to the index, so we add a dummy
    // conflict instead. The stat for base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Ours         Mode(FILE) eb8299123d2a ctime=0:0 mtime=0:0 size=0 flags=2000 .jj-do-not-resolve-this-conflict
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 side-1.txt
    Unconflicted Mode(FILE) 7b44e11df720 ctime=0:0 mtime=0:0 size=0 flags=0 side-2.txt
    Unconflicted Mode(FILE) 42f37a71bf20 ctime=0:0 mtime=0:0 size=0 flags=0 side-3.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  78e326a70f68a8909dc18978620796005ec9bf31
    ×      be1155a2aee8f49a6132d0cb5d4dcb8e71281420
    ├─┬─╮
    │ │ ○  8e557c4fa56ee8713b44769e7a20b413d922931e side-3
    │ ○ │  13a224f2ddfb75582340c5a0d58c195d4314b798 side-2
    │ ├─╯
    ○ │  5b2fc6e007973fa56221fed78fa33aab06b4f386 side-1
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should be the same after `jj new`.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Ours         Mode(FILE) eb8299123d2a ctime=0:0 mtime=0:0 size=0 flags=2000 .jj-do-not-resolve-this-conflict
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 side-1.txt
    Unconflicted Mode(FILE) 7b44e11df720 ctime=0:0 mtime=0:0 size=0 flags=0 side-2.txt
    Unconflicted Mode(FILE) 42f37a71bf20 ctime=0:0 mtime=0:0 size=0 flags=0 side-3.txt
    ");

    // If we add a file named ".jj-do-not-resolve-this-conflict", it should take
    // precedence over the dummy conflict.
    work_dir.write_file(".jj-do-not-resolve-this-conflict", "file\n");
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @"
    Unconflicted Mode(FILE) f73f3093ff86 ctime=0:0 mtime=0:0 size=0 flags=0 .jj-do-not-resolve-this-conflict
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 side-1.txt
    Unconflicted Mode(FILE) 7b44e11df720 ctime=0:0 mtime=0:0 size=0 flags=0 side-2.txt
    Unconflicted Mode(FILE) 42f37a71bf20 ctime=0:0 mtime=0:0 size=0 flags=0 side-3.txt
    ");

    Ok(())
}

#[must_use]
fn get_log_output_divergence(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
      change_id.short(),
      commit_id.short(),
      description.first_line(),
      bookmarks,
      if(divergent, "!divergence!"),
    )
    "#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    separate(" ",
      commit_id,
      bookmarks,
      description,
    )
    "#;
    work_dir.run_jj(["log", "-T", template, "-r=all()"])
}

fn update_git_index(repo_path: &Path) {
    let mut iter = git::open(repo_path)
        .status(gix::progress::Discard)
        .unwrap()
        .into_index_worktree_iter(None)
        .unwrap();

    // need to explicitly iterate over the changes to recreate the index

    for item in iter.by_ref() {
        item.unwrap();
    }

    iter.outcome_mut()
        .unwrap()
        .write_changes()
        .unwrap()
        .unwrap();
}

fn get_index_state(repo_path: &Path) -> String {
    let git_repo = gix::open(repo_path).expect("git repo should exist");
    let mut buffer = String::new();
    // We can't use the real time from disk, since it would change each time the
    // tests are run. Instead, we just show whether it's zero or nonzero.
    let format_time = |time: gix::index::entry::stat::Time| {
        if time.secs == 0 && time.nsecs == 0 {
            "0:0"
        } else {
            "[nonzero]"
        }
    };
    let index = git_repo.index_or_empty().unwrap();
    for entry in index.entries() {
        writeln!(
            &mut buffer,
            "{:12} {:?} {} ctime={} mtime={} size={} flags={:x} {}",
            format!("{:?}", entry.stage()),
            entry.mode,
            entry.id.to_hex_with_len(12),
            format_time(entry.stat.ctime),
            format_time(entry.stat.mtime),
            entry.stat.size,
            entry.flags.bits(),
            entry.path_in(index.path_backing()),
        )
        .unwrap();
    }
    buffer
}

#[test]
fn test_git_colocated_unreachable_commits() -> TestResult {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());

    // Create an initial commit in Git
    let commit1 = git::add_commit(
        &git_repo,
        "refs/heads/master",
        "some-file",
        b"some content",
        "initial",
        &[],
    )
    .commit_id;
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"cd740e230992f334de13a0bd0b35709b3f7a89af"
    );

    // Add a second commit in Git
    let commit2 = git::add_commit(
        &git_repo,
        "refs/heads/dummy",
        "next-file",
        b"more content",
        "next",
        &[commit1],
    )
    .commit_id;
    git_repo.find_reference("refs/heads/dummy")?.delete()?;
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"cd740e230992f334de13a0bd0b35709b3f7a89af"
    );

    // Import the repo while there is no path to the second commit
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  f3677b3e3b95a34e7017655ab612e1d11b59c713
    ○  cd740e230992f334de13a0bd0b35709b3f7a89af master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        git_repo.head_id()?.to_string(),
        @"cd740e230992f334de13a0bd0b35709b3f7a89af"
    );

    // Check that trying to look up the second commit fails gracefully
    let output = work_dir.run_jj(["show", &commit2.to_string()]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Revision `b23bb53bdce25f0e03ff9e484eadb77626256041` doesn't exist
    [EOF]
    [exit status: 1]
    ");

    Ok(())
}

#[test]
fn test_git_colocated_operation_cleanup() -> TestResult {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    [EOF]
    "#);

    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "1");
    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.run_jj(["new"]).success();

    work_dir.write_file("file", "2");
    work_dir.run_jj(["describe", "-m2"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["new", "root()+"]).success();

    work_dir.write_file("file", "3");
    work_dir.run_jj(["describe", "-m3"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "feature"])
        .success();
    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  91b09a48325e050be83835a3c817485385d60538
    ○  b774c9f6118536f3855a2970b465dd2d34377cdd feature 3
    │ ○  60476fc631a3c4602eb8b9a64b6b3d0951ed2837 main 2
    ├─╯
    ○  87f64775047d7ce62b7ee81412b8e4cc07aea40a 1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Start a rebase in Git and expect a merge conflict.
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["rebase", "main"])
        .output()?;
    assert!(!output.status.success());

    // Check that we’re in the middle of a conflicted rebase.
    assert!(std::fs::exists(
        work_dir.root().join(".git").join("rebase-merge")
    )?);
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["status", "--porcelain=v1"])
        .output()?;
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout)?, @"UU file");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  9b52e6dfe5200f47ca82cf6cf522f0186005c4eb
    ○  60476fc631a3c4602eb8b9a64b6b3d0951ed2837 main 2
    │ ○  b774c9f6118536f3855a2970b465dd2d34377cdd feature 3
    ├─╯
    ○  87f64775047d7ce62b7ee81412b8e4cc07aea40a 1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    [EOF]
    ");

    // Reset the Git HEAD with Jujutsu.
    let output = work_dir.run_jj(["new", "main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: xmkuslsw b25e19a9 (empty) (no description set)
    Parent commit (@-)      : nkmpptxz 60476fc6 main | 2
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  b25e19a9f57508b66d1c0137e3d0a08ae4eb4031
    │ ○  9b52e6dfe5200f47ca82cf6cf522f0186005c4eb
    ├─╯
    ○  60476fc631a3c4602eb8b9a64b6b3d0951ed2837 main 2
    │ ○  b774c9f6118536f3855a2970b465dd2d34377cdd feature 3
    ├─╯
    ○  87f64775047d7ce62b7ee81412b8e4cc07aea40a 1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 60476fc631a3c4602eb8b9a64b6b3d0951ed2837
    [EOF]
    ");

    // Check that the operation was correctly aborted.
    assert!(!std::fs::exists(
        work_dir.root().join(".git").join("rebase-merge")
    )?);
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["status", "--porcelain=v1"])
        .output()?;
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout)?, @"");

    Ok(())
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
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
