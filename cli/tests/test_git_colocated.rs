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
use std::process::Command;

use assert_cmd::assert::OutputAssertExt as _;
use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_git_colocated() {
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
        git_repo.head_id().unwrap().to_string(),
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
        git_repo.head_id().unwrap().to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
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
        git_repo.head_id().unwrap().to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 97358f54806c7cd005ed5ade68a779595efbae7e
    [EOF]
    ");

    // Create a new change from jj and check that it's reflected in Git
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  4ddddef596e9d68f729f1be9e1b2cdaaf45bef08
    ○  9dfe8c7005c8dff6078ecdfd953c6bfddc633c90
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"9dfe8c7005c8dff6078ecdfd953c6bfddc633c90"
    );
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 9dfe8c7005c8dff6078ecdfd953c6bfddc633c90
    [EOF]
    ");
}

#[test]
fn test_git_colocated_intent_to_add() {
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
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file2.txt
    ");

    let op_id_new_file = work_dir.current_operation_id();

    // After creating a new commit, it should not longer be marked as intent-to-add
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2.txt", "contents");
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    ");

    // If we edit an existing commit, new files are marked as intent-to-add
    work_dir.run_jj(["edit", "@-"]).success();
    work_dir.run_jj(["status"]).success();
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
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
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 .gitignore
    Unconflicted Mode(FILE) 0839b2e9412b ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    ");
}

#[test]
fn test_git_colocated_unborn_bookmark() {
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
    assert!(git_repo.head().unwrap().is_unborn());
    assert_eq!(
        git_repo.head_name().unwrap().unwrap().as_bstr(),
        b"refs/heads/master"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Stage some change, and check out root. This shouldn't clobber the HEAD.
    add_file_to_index("file0", "");
    let output = work_dir.run_jj(["new", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: zsuskuln c2934cfb (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_unborn());
    assert_eq!(
        git_repo.head_name().unwrap().unwrap().as_bstr(),
        b"refs/heads/master"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  c2934cfbfb196d2c473959667beffcc19e71e5e8
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @r"
    The working copy has no changes.
    Working copy  (@) : zsuskuln c2934cfb (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Stage some change, and create new HEAD. This shouldn't move the default
    // bookmark.
    add_file_to_index("file1", "");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 2d7a8abb (empty) (no description set)
    Parent commit (@-)      : zsuskuln ff536684 (no description set)
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"ff5366846b039b25c6c4998fa74dca821c246243"
    );
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  2d7a8abb601ebf559df4037279e9f2e851a75e63
    ○  ff5366846b039b25c6c4998fa74dca821c246243
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: ff5366846b039b25c6c4998fa74dca821c246243
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @r"
    The working copy has no changes.
    Working copy  (@) : vruxwmqv 2d7a8abb (empty) (no description set)
    Parent commit (@-): zsuskuln ff536684 (no description set)
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: wqnwkozp 88e8407a (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_unborn());
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  88e8407a4f0a5e6f40a7c6c494106764adc00fed
    │ ○  2dd7385602e703388fd266b939bba6f57a1439d3
    │ ○  ff5366846b039b25c6c4998fa74dca821c246243 master
    ├─╯
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");
    // Staged change shouldn't persist.
    checkout_index();
    insta::assert_snapshot!(work_dir.run_jj(["status"]), @r"
    The working copy has no changes.
    Working copy  (@) : wqnwkozp 88e8407a (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // New snapshot and commit can be created after the HEAD got unset.
    work_dir.write_file("file3", "");
    let output = work_dir.run_jj(["new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: uyznsvlq 2fb16499 (empty) (no description set)
    Parent commit (@-)      : wqnwkozp bb21bc2d (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  2fb16499a987e632407402e38976ed250c939c42
    ○  bb21bc2dce2af92973fdd6d42686d77bd16bc466
    │ ○  2dd7385602e703388fd266b939bba6f57a1439d3
    │ ○  ff5366846b039b25c6c4998fa74dca821c246243 master
    ├─╯
    │ ○  e6669bb3438ef218fa618e1047a1911d2b3410dd
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: bb21bc2dce2af92973fdd6d42686d77bd16bc466
    [EOF]
    ");
}

#[test]
fn test_git_colocated_export_bookmarks_on_snapshot() {
    // Checks that we export bookmarks that were changed only because the working
    // copy was snapshotted

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
        .find_reference("refs/heads/foo")
        .unwrap()
        .id()
        .to_string(), @"00fc09f48ccf5c8b025a0f93b0ec3b0e4294a598");
}

#[test]
fn test_git_colocated_rebase_on_import() {
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
        .find_reference("refs/heads/master")
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .parent_ids()
        .next()
        .unwrap()
        .detach();
    git_repo
        .reference(
            "refs/heads/master",
            parent_commit,
            gix::refs::transaction::PreviousValue::Any,
            "update ref",
        )
        .unwrap();
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
}

#[test]
fn test_git_colocated_bookmarks() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    work_dir.run_jj(["new", "-m", "foo"]).success();
    work_dir.run_jj(["new", "@-", "-m", "bar"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  95e79774f8e7c785fc36da2b798ecfe0dc864e02 bar
    │ ○  b51ab2e2c88fe2d38bd7ca6946c4d87f281ce7e2 foo
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
        git_repo.find_reference("refs/heads/master").unwrap().target().id().to_string(),
        @"95e79774f8e7c785fc36da2b798ecfe0dc864e02"
    );
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781"
    );

    // Update the bookmark in Git
    let target_id = work_dir
        .run_jj(["log", "--no-graph", "-T=commit_id", "-r=subject(foo)"])
        .success()
        .stdout
        .into_raw();
    git_repo
        .reference(
            "refs/heads/master",
            gix::ObjectId::from_hex(target_id.as_bytes()).unwrap(),
            gix::refs::transaction::PreviousValue::Any,
            "test",
        )
        .unwrap();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  507c0edcfc028f714f3c7a3027cb141f6610e867
    │ ○  b51ab2e2c88fe2d38bd7ca6946c4d87f281ce7e2 master foo
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
}

#[test]
fn test_git_colocated_bookmark_forget() {
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
    @  43444d88b0096888ebfd664c0cf792c9d15e3f14 foo
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    foo: rlvkpnrz 43444d88 (empty) (no description set)
      @git: rlvkpnrz 43444d88 (empty) (no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "forget", "--include-remotes", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Forgot 1 local bookmarks.
    Forgot 1 remote bookmarks.
    [EOF]
    ");
    // A forgotten bookmark is deleted in the git repo. For a detailed demo
    // explaining this, see `test_bookmark_forget_export` in
    // `test_bookmark_command.rs`.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @"");
}

#[test]
fn test_git_colocated_bookmark_at_root() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["bookmark", "create", "foo", "-r=root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Created 1 bookmarks pointing to zzzzzzzz 00000000 foo | (empty) (no description set)
    Warning: Failed to export some bookmarks:
      foo@git: Ref cannot point to the root commit in Git
    [EOF]
    ");

    let output = work_dir.run_jj(["bookmark", "move", "foo", "--to=@"]);
    insta::assert_snapshot!(output, @r"
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Target revision is empty.
    Moved 1 bookmarks to zzzzzzzz 00000000 foo* | (empty) (no description set)
    Warning: Failed to export some bookmarks:
      foo@git: Ref cannot point to the root commit in Git
    [EOF]
    ");
}

#[test]
fn test_git_colocated_conflicting_git_refs() {
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
}

#[test]
fn test_git_colocated_checkout_non_empty_working_copy() {
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
        git_repo.head_id().unwrap().to_string(),
        @"97358f54806c7cd005ed5ade68a779595efbae7e"
    );

    work_dir.write_file("two", "y");

    work_dir.run_jj(["describe", "-m", "two"]).success();
    work_dir.run_jj(["new", "@-"]).success();
    let output = work_dir.run_jj(["describe", "-m", "new"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 986aa548 (empty) new
    Parent commit (@-)      : slsumksp 97358f54 master | initial
    [EOF]
    ");

    assert_eq!(
        git_repo.head_name().unwrap().unwrap().as_bstr(),
        b"refs/heads/master"
    );

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  986aa548466ed43b48c059854720e70d8ec2bf71 new
    │ ○  6b0f7d59e0749d3a6ff2ecf686d5fa48023b7b93 two
    ├─╯
    ○  97358f54806c7cd005ed5ade68a779595efbae7e master initial
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 97358f54806c7cd005ed5ade68a779595efbae7e
    [EOF]
    ");
}

#[test]
fn test_git_colocated_fetch_deleted_or_moved_bookmark() {
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
    @  0060713e4c7c46c4ce0d69a43ac16451582eda79
    │ ○  dd905babf5b4ad4689f2da1350fd4f0ac5568209 C_to_move original C
    ├─╯
    │ ○  b2ea51c027e11c0f2871cce2a52e648e194df771 B_to_delete B_to_delete
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    bookmark: B_to_delete@origin [deleted] untracked
    bookmark: C_to_move@origin   [updated] tracked
    Abandoned 2 commits that are no longer reachable:
      royxmykx/1 dd905bab C_to_move@git | (divergent) (empty) original C
      zsuskuln b2ea51c0 B_to_delete@git | (empty) B_to_delete
    [EOF]
    ");
    // "original C" and "B_to_delete" are abandoned, as the corresponding bookmarks
    // were deleted or moved on the remote (#864)
    insta::assert_snapshot!(get_log_output(&clone_dir), @"
    @  0060713e4c7c46c4ce0d69a43ac16451582eda79
    │ ○  fb297975e4ef98dc057f65b761aed2cdb0386598 C_to_move moved C
    ├─╯
    ◆  8777db25171cace71ad014598663d5ffc4fae6b1 A A
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_git_colocated_rebase_dirty_working_copy() {
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
    git_repo
        .find_reference("refs/heads/feature")
        .unwrap()
        .delete()
        .unwrap();

    // Because the working copy is dirty, the new working-copy commit will be
    // diverged. Therefore, the feature bookmark has change-delete conflict.
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M file
    Working copy  (@) : rlvkpnrz e23559e3 feature?? | (no description set)
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
    @  e23559e3bc6f22a5562297696fc357e2c581df77 feature??
    ○  f99015d7d9b82a5912ec4d96a18d2a4afbd8dd49
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // The working-copy content shouldn't be lost.
    insta::assert_snapshot!(work_dir.read_file("file"), @"new");
}

#[test]
fn test_git_colocated_external_checkout() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    let git_check_out_ref = |name| {
        let target = git_repo
            .find_reference(name)
            .unwrap()
            .into_fully_peeled_id()
            .unwrap()
            .detach();
        git::set_head_to_id(&git_repo, target);
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
    @  6f8612f0e7f6d52efd8a72615796df06f8d64cdc
    ○  319eaafc8fd04c763a0683a000bba5452082feb3 B
    │ ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Check out another bookmark by external command
    git_check_out_ref("refs/heads/master");

    // The old working-copy commit gets abandoned, but the whole bookmark should not
    // be abandoned. (#1042)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  7ceeaaae54c8ac99ad34eeed7fe1e896f535be99
    ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    │ ○  319eaafc8fd04c763a0683a000bba5452082feb3 B
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
    ○  823204bc895aad19d46b895bc510fb3e9d0c97c7 C
    @  c6abf242550b7c4116d3821b69c79326889aeba0
    ○  319eaafc8fd04c763a0683a000bba5452082feb3 B
    │ ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Check out another bookmark by external command
    git_check_out_ref("refs/heads/master");

    // The old working-copy commit shouldn't be abandoned. (#3747)
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  277b693c61dcdea59ac26d6982370f78751f6ef5
    ○  8777db25171cace71ad014598663d5ffc4fae6b1 master A
    │ ○  823204bc895aad19d46b895bc510fb3e9d0c97c7 C
    │ ○  c6abf242550b7c4116d3821b69c79326889aeba0
    │ ○  319eaafc8fd04c763a0683a000bba5452082feb3 B
    ├─╯
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ------- stderr -------
    Reset the working copy parent to the new Git HEAD.
    [EOF]
    ");
}

#[test]
#[cfg_attr(windows, ignore = "uses POSIX sh")]
fn test_git_colocated_concurrent_checkout() {
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
    Caused by: The reference "HEAD" should have content dc0b92dfa0af129b2929fa1789fc896b075782b2, actual content was 091e39feb0aba632ab9a9503ceb1dddeac4dd496
    Working copy  (@) now at: mzvwutvl cf0ddbb4 (empty) (no description set)
    Parent commit (@-)      : zsuskuln b6786455 (empty) commit3
    [EOF]
    "#);

    // git_head() isn't updated because the export failed
    insta::assert_snapshot!(work_dir.run_jj(["log", "--summary", "--ignore-working-copy"]), @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:11 cf0ddbb4
    │  (empty) (no description set)
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:11 b6786455
    │  (empty) commit3
    ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 dc0b92df
    │  commit2
    │  A file2
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 091e39fe
    │  commit1
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // The current Git HEAD is imported on the next jj invocation
    insta::assert_snapshot!(work_dir.run_jj(["log", "--summary"]), @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:13 9529e8f5
    │  (empty) (no description set)
    │ ○  zsuskuln test.user@example.com 2001-02-03 08:05:11 b6786455
    │ │  (empty) commit3
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 dc0b92df
    ├─╯  commit2
    │    A file2
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 091e39fe
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
}

#[test]
fn test_git_colocated_squash_undo() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();
    work_dir.run_jj(["ci", "-m=A"]).success();
    // Test the setup
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @r"
    @  rlvkpnrzqnoo 682c866b0a2f
    ○  qpvuntsmwlqt 8777db25171c A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");

    work_dir.run_jj(["squash"]).success();
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @r"
    @  zsuskulnrvyr e1c3034f23b9
    ○  qpvuntsmwlqt ba304e200f4f A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");
    work_dir.run_jj(["undo"]).success();
    // There should be no divergence here (#922)
    insta::assert_snapshot!(get_log_output_divergence(&work_dir), @r"
    @  rlvkpnrzqnoo 682c866b0a2f
    ○  qpvuntsmwlqt 8777db25171c A
    ◆  zzzzzzzzzzzz 000000000000
    [EOF]
    ");
}

#[test]
fn test_git_colocated_undo_head_move() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let git_repo = git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    // Create new HEAD
    work_dir.run_jj(["new"]).success();
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  43444d88b0096888ebfd664c0cf792c9d15e3f14
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");

    // HEAD should be unset
    work_dir.run_jj(["undo"]).success();
    assert!(git_repo.head().unwrap().is_unborn());
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: (none)
    [EOF]
    ");

    // Create commit on non-root commit
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  5e37f1b8313299eb1b62221eefcf32881b0dc4c6
    ○  23e6e06a7471634da3567ef975fadf883082658f
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: 23e6e06a7471634da3567ef975fadf883082658f
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"23e6e06a7471634da3567ef975fadf883082658f");

    // HEAD should be moved back
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Restored to operation: 64c74033fba2 (2001-02-03 08:05:14) new empty commit
    Working copy  (@) now at: vruxwmqv 23e6e06a (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");
    assert!(git_repo.head().unwrap().is_detached());
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
        @"e8849ae12c709f2321908879bc724fdb2ab8a781");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  23e6e06a7471634da3567ef975fadf883082658f
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: e8849ae12c709f2321908879bc724fdb2ab8a781
    [EOF]
    ");
}

#[test]
fn test_git_colocated_update_index_preserves_timestamps() {
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
    @  a1886a45815f0dcca5cefcc334d11ffb908a1eb8
    ○  8b0c962ef1fea901fb16f8a484e692a1f0dcbc59 commit2
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=0:0 mtime=0:0 size=0 flags=0 file1.txt
    Unconflicted Mode(FILE) 2e0996000b7e ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) d5f7fc3f74f7 ctime=0:0 mtime=0:0 size=0 flags=0 file4.txt
    ");

    // Update index with stats for all files. We may want to do this automatically
    // in the future after we update the index in `git::reset_head` (#3786), but for
    // now, we at least want to preserve existing stat information when possible.
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 2e0996000b7e ctime=[nonzero] mtime=[nonzero] size=9 flags=0 file2.txt
    Unconflicted Mode(FILE) d5f7fc3f74f7 ctime=[nonzero] mtime=[nonzero] size=6 flags=0 file4.txt
    ");

    // Edit parent commit, causing the changes to be removed from the index without
    // touching the working copy
    work_dir.run_jj(["edit", "commit2"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  8b0c962ef1fea901fb16f8a484e692a1f0dcbc59 commit2
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain stat for unchanged file still.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 28d2718c947b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) 528557ab3a42 ctime=0:0 mtime=0:0 size=0 flags=0 file3.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 file4.txt
    ");

    // Create sibling commit, causing working copy to match index
    work_dir.run_jj(["new", "commit1"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d9c7f1932e1135856d5905f1a0fc194ce2657065
    │ ○  8b0c962ef1fea901fb16f8a484e692a1f0dcbc59 commit2
    ├─╯
    ○  d37eac5eea00fa74a41c1512839711f42aca2c35 commit1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain stat for unchanged file still.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) ed48318d9bf4 ctime=[nonzero] mtime=[nonzero] size=18 flags=0 file1.txt
    Unconflicted Mode(FILE) 28d2718c947b ctime=0:0 mtime=0:0 size=0 flags=0 file2.txt
    Unconflicted Mode(FILE) 528557ab3a42 ctime=0:0 mtime=0:0 size=0 flags=0 file3.txt
    ");
}

#[test]
fn test_git_colocated_update_index_merge_conflict() {
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

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 right.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 right.txt
    ");

    // Create merge conflict
    work_dir.run_jj(["new", "left", "right"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    67305afbbe0f166da93da08aa67412bb870c1747
    ├─╮
    │ ○  620e15db9fcd05fff912c52d2cafd36c9e01523c right
    ○ │  d0f55ffafa1e0e72980202c349af23d093f825be left
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Conflict should be added in index with correct blob IDs. The stat for
    // base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  040579f58a38cfe5fba2a67c78c512c4798cf47c
    ×    67305afbbe0f166da93da08aa67412bb870c1747
    ├─╮
    │ ○  620e15db9fcd05fff912c52d2cafd36c9e01523c right
    ○ │  d0f55ffafa1e0e72980202c349af23d093f825be left
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should be the same after `jj new`.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");
}

#[test]
fn test_git_colocated_update_index_rebase_conflict() {
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
    @  d0f55ffafa1e0e72980202c349af23d093f825be left
    │ ○  620e15db9fcd05fff912c52d2cafd36c9e01523c right
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    ");

    // Create rebase conflict
    work_dir
        .run_jj(["rebase", "-r", "left", "-o", "right"])
        .success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  b3e1abf04842a7756ba95d0a72d72f5f40243362 left
    ○  620e15db9fcd05fff912c52d2cafd36c9e01523c right
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should contain files from parent commit, so there should be no conflict
    // in conflict.txt yet. The stat for base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  4f18f4ac428d3cd0e2ce9169e60222628fe04f4f
    ×  b3e1abf04842a7756ba95d0a72d72f5f40243362 left
    ○  620e15db9fcd05fff912c52d2cafd36c9e01523c right
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Now the working copy commit's parent is conflicted, so the index should have
    // a conflict with correct blob IDs.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Base         Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=1000 conflict.txt
    Ours         Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=2000 conflict.txt
    Theirs       Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=3000 conflict.txt
    Unconflicted Mode(FILE) 45cf141ba67d ctime=0:0 mtime=0:0 size=0 flags=0 left.txt
    Unconflicted Mode(FILE) c376d892e8b1 ctime=0:0 mtime=0:0 size=0 flags=0 right.txt
    ");
}

#[test]
fn test_git_colocated_update_index_3_sided_conflict() {
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

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 side-3.txt
    ");

    // Update index with stat for base.txt
    update_git_index(work_dir.root());

    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) df967b96a579 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) e69de29bb2d1 ctime=0:0 mtime=0:0 size=0 flags=20004000 side-3.txt
    ");

    // Create 3-sided merge conflict
    work_dir
        .run_jj(["new", "side-1", "side-2", "side-3"])
        .success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      fc4cf6fcac1da27c71e4cd136bf87f6c33d25710
    ├─┬─╮
    │ │ ○  5008c8807feaa955d02e96cb1b0dcf51536fefb8 side-3
    │ ○ │  da6e0a03f8b72f6868a9ea33836123fe965c0cb4 side-2
    │ ├─╯
    ○ │  ad7eaf61b769dce99884d2ceb0ddf48fc4eac463 side-1
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // We can't add conflicts with more than 2 sides to the index, so we add a dummy
    // conflict instead. The stat for base.txt should not change.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Ours         Mode(FILE) eb8299123d2a ctime=0:0 mtime=0:0 size=0 flags=2000 .jj-do-not-resolve-this-conflict
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 side-1.txt
    Unconflicted Mode(FILE) 7b44e11df720 ctime=0:0 mtime=0:0 size=0 flags=0 side-2.txt
    Unconflicted Mode(FILE) 42f37a71bf20 ctime=0:0 mtime=0:0 size=0 flags=0 side-3.txt
    ");

    work_dir.run_jj(["new"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  2527e09e99326385dfbe14bb3f23f1cc12b9090b
    ×      fc4cf6fcac1da27c71e4cd136bf87f6c33d25710
    ├─┬─╮
    │ │ ○  5008c8807feaa955d02e96cb1b0dcf51536fefb8 side-3
    │ ○ │  da6e0a03f8b72f6868a9ea33836123fe965c0cb4 side-2
    │ ├─╯
    ○ │  ad7eaf61b769dce99884d2ceb0ddf48fc4eac463 side-1
    ├─╯
    ○  1861378a9167e6561bf8ce4a6fef2d7c0897dd87 base
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Index should be the same after `jj new`.
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
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
    insta::assert_snapshot!(get_index_state(work_dir.root()), @r"
    Unconflicted Mode(FILE) f73f3093ff86 ctime=0:0 mtime=0:0 size=0 flags=0 .jj-do-not-resolve-this-conflict
    Unconflicted Mode(FILE) df967b96a579 ctime=[nonzero] mtime=[nonzero] size=5 flags=0 base.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 conflict.txt
    Unconflicted Mode(FILE) dd8f930010b3 ctime=0:0 mtime=0:0 size=0 flags=0 side-1.txt
    Unconflicted Mode(FILE) 7b44e11df720 ctime=0:0 mtime=0:0 size=0 flags=0 side-2.txt
    Unconflicted Mode(FILE) 42f37a71bf20 ctime=0:0 mtime=0:0 size=0 flags=0 side-3.txt
    ");
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

/// Skips the test if git command is not available.
/// Returns true if the test should be skipped.
fn skip_if_git_unavailable() -> bool {
    if Command::new("git").arg("--version").status().is_err() {
        eprintln!("Skipping because git command might fail to run");
        return true;
    }
    false
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
fn test_git_colocated_unreachable_commits() {
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
        git_repo.head_id().unwrap().to_string(),
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
    git_repo
        .find_reference("refs/heads/dummy")
        .unwrap()
        .delete()
        .unwrap();
    insta::assert_snapshot!(
        git_repo.head_id().unwrap().to_string(),
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
        git_repo.head_id().unwrap().to_string(),
        @"cd740e230992f334de13a0bd0b35709b3f7a89af"
    );

    // Check that trying to look up the second commit fails gracefully
    let output = work_dir.run_jj(["show", &commit2.to_string()]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revision `b23bb53bdce25f0e03ff9e484eadb77626256041` doesn't exist
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_git_colocated_operation_cleanup() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Initialized repo in "repo"
    Hint: Running `git clean -xdf` will remove `.jj/`!
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
    @  40638ce20b8b74e94460e95709cb077f4307ad7c
    ○  a50e55141dcd5f8f8d549acd2232ce4839eaa798 feature 3
    │ ○  cf3bb116ded416d9b202e71303f260e504c2eeb9 main 2
    ├─╯
    ○  87f64775047d7ce62b7ee81412b8e4cc07aea40a 1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // Start a rebase in Git and expect a merge conflict.
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["rebase", "main"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    // Check that we’re in the middle of a conflicted rebase.
    assert!(std::fs::exists(work_dir.root().join(".git").join("rebase-merge")).unwrap());
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["status", "--porcelain=v1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout).unwrap(), @"UU file");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  588c505e689d116180684778b29c540fe7180268
    ○  cf3bb116ded416d9b202e71303f260e504c2eeb9 main 2
    │ ○  a50e55141dcd5f8f8d549acd2232ce4839eaa798 feature 3
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
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw aa14563c (empty) (no description set)
    Parent commit (@-)      : kkmpptxz cf3bb116 main | 2
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  aa14563cf5d892238f1e60260c5c284627d76e7c
    │ ○  588c505e689d116180684778b29c540fe7180268
    ├─╯
    ○  cf3bb116ded416d9b202e71303f260e504c2eeb9 main 2
    │ ○  a50e55141dcd5f8f8d549acd2232ce4839eaa798 feature 3
    ├─╯
    ○  87f64775047d7ce62b7ee81412b8e4cc07aea40a 1
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
    insta::assert_snapshot!(get_colocation_status(&work_dir), @r"
    Workspace is currently colocated with Git.
    Last imported/exported Git HEAD: cf3bb116ded416d9b202e71303f260e504c2eeb9
    [EOF]
    ");

    // Check that the operation was correctly aborted.
    assert!(!std::fs::exists(work_dir.root().join(".git").join("rebase-merge")).unwrap());
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["status", "--porcelain=v1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout).unwrap(), @"");
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

/// Tests that creating a non-colocated workspace from a colocated repo
/// works correctly - the secondary workspace should not try to update git HEAD.
///
/// Verifies that:
/// 1. Creating a non-colocated workspace doesn't move the primary's git HEAD
/// 2. Operations in the non-colocated workspace succeed
/// 3. Operations in the non-colocated workspace don't affect the primary's git
///    HEAD
#[test]
fn test_git_colocated_create_workspace_not_moving_head() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let _second_work_dir = test_env.work_dir("second");

    work_dir
        .run_jj(["commit", "-m", "second_wc_parent"])
        .success();
    let output = work_dir
        .run_jj(["log", "-Tcommit_id", "-r@-", "--no-graph"])
        .success();
    let second_wc_parent = output.stdout.normalized();

    work_dir
        .run_jj(["commit", "-m", "should be git head"])
        .success();
    work_dir
        .run_jj(["workspace", "add", "../second", "-r", second_wc_parent])
        .success();

    // Verify git_head wasn't moved during workspace creation.
    // The second workspace is non-colocated (no .git), so creating it
    // should not affect git HEAD.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  dff72cf4427ad90c331d81a97d68dbdd6b1b9894
    ○  b24869b3336626eca0f69ba14929c1be0a38e0e7 should be git head
    │ ○  fa5c7df5654c7f8f4dd49ea881a4bb70a06c389e
    ├─╯
    ○  410296fbafc1655b3335548eff3b26753c6888c2 second_wc_parent
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_colocated_workspace_git_symlink_to_wrong_repo() {
    let test_env = TestEnvironment::default();

    // Create a non-colocated jj repo
    test_env
        .run_jj_in(
            test_env.env_root(),
            ["git", "init", "--no-colocate", "repo"],
        )
        .success();
    let work_dir = test_env.work_dir("repo");
    let workspace_root = work_dir.root();

    // Create another git repo that we'll symlink to
    let other_git_repo = test_env.env_root().join("other-git-repo");
    git::init(&other_git_repo);

    // Create a .git symlink pointing to the other repo
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other_git_repo, workspace_root.join(".git")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&other_git_repo, workspace_root.join(".git")).unwrap();

    // Run a jj command and verify the warning is shown
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.stderr, @"
    Warning: Workspace has a .git symlink (pointing to $TEST_ENV/other-git-repo), that isn't pointing to jj's git repo
    Hint: To remove this symlink, run `rm .git`
    [EOF]
    ");
}

/// Substitute for `jj workspace add --colocate` or similar using git CLI,
/// please replace with the real thing when it lands.
fn stopgap_workspace_colocate(
    test_env: &TestEnvironment,
    repo_path: &Path,
    original_colocated: bool,
    dst: &str,
    initial_head: &str,
) {
    // Can't use gix/git2, as neither can repair the broken worktree we're about to
    // create.
    let repo_relative_path = if original_colocated {
        dst.to_owned()
    } else {
        format!("../../../../{dst}")
    };
    Command::new("git")
        .args(["worktree", "add", &repo_relative_path])
        .arg(initial_head)
        .current_dir(if original_colocated {
            repo_path.to_path_buf()
        } else {
            repo_path.join(".jj/repo/store/git")
        })
        // NOTE: Ensure the output is in English.
        .env("LANG", "C")
        .assert()
        .success()
        .stderr(format!(
            "Preparing worktree (detached HEAD {})
",
            &initial_head[..7]
        ));
    let dst_path = repo_path.join(dst);
    let tmp_path = test_env.env_root().join("__tmp_worktree__");
    if tmp_path.exists() {
        std::fs::remove_dir_all(&tmp_path).unwrap();
    }
    std::fs::rename(&dst_path, &tmp_path).unwrap();
    // Use --no-colocate since this helper manually manages the git worktree
    test_env
        .work_dir("repo")
        .run_jj(["workspace", "add", "--no-colocate", dst])
        .success();
    std::fs::rename(tmp_path.join(".git"), dst_path.join(".git")).unwrap();
    std::fs::write(
        dst_path.join(".jj/.gitignore"),
        "*
",
    )
    .unwrap();
    Command::new("git")
        .args(["worktree", "repair"])
        .current_dir(&dst_path)
        .assert()
        .success();
    Command::new("git")
        .arg("checkout")
        .arg(initial_head)
        .current_dir(&dst_path)
        .assert()
        .success();
}

#[test]
fn test_colocated_workspace_in_bare_repo() {
    // TODO: Remove when this stops requiring git (stopgap_workspace_colocate)
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let default_work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");
    //
    // git init without --colocate creates a bare repo
    default_work_dir.create_dir_all("");
    default_work_dir.run_jj(["git", "init"]).success();
    default_work_dir.write_file("file", b"contents");
    default_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    let initial_commit = default_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        default_work_dir.root(),
        false,
        "../second",
        &initial_commit,
    );

    insta::assert_snapshot!(get_log_output(&second_work_dir), @"
    @  514e1b3adab7336794cf569e7b9c60c1dbcad1b4
    │ ○  64393b1a826a63bba44c4c5cec90d7a9040063b9
    ├─╯
    ○  dda9521046c4649797052c184beab33a9cf9754b initial commit
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    second_work_dir
        .run_jj(["commit", "-m", "commit in second workspace"])
        .success();
    insta::assert_snapshot!(get_log_output(&second_work_dir), @"
    @  104cab0c0a2b384e247b0efe73b0dc5cc2d2df45
    ○  a45eccddae45d27fdc5008294fa2ff5461d2c25b commit in second workspace
    │ ○  64393b1a826a63bba44c4c5cec90d7a9040063b9
    ├─╯
    ○  dda9521046c4649797052c184beab33a9cf9754b initial commit
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // FIXME: There should still be no git HEAD in the default workspace, which
    // is not colocated. However, git_head() is a property of the view. And
    // currently, all colocated workspaces read and write from the same
    // entry of the common view.
    //
    // let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--no-graph",
    // "-r", "git_head()"]); insta::assert_snapshot!(stdout, @r#""#);

    let output = second_work_dir
        .run_jj(["op", "log", "-Tself.description().first_line()"])
        .success();
    insta::assert_snapshot!(output, @"
    @  commit 514e1b3adab7336794cf569e7b9c60c1dbcad1b4
    ○  import git head
    ○  create initial working-copy commit in workspace second
    ○  add workspace 'second'
    ○  commit 006bd1130b84e90ab082adeabd7409270d5a86da
    ○  snapshot working copy
    ○  add workspace 'default'
    ○
    [EOF]
    ");
}

#[test]
fn test_colocated_workspace_moved_original_on_disk() {
    if Command::new("git").arg("--version").status().is_err() {
        eprintln!("Skipping because git command might fail to run");
        return;
    }

    let test_env = TestEnvironment::default();
    let default_work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");
    let new_repo_path = test_env.env_root().join("repo-moved");

    default_work_dir.create_dir_all("");
    default_work_dir
        .run_jj(["git", "init", "--colocate"])
        .success();
    default_work_dir.write_file("file", b"contents");
    default_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    let initial_commit = default_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        default_work_dir.root(),
        true,
        "../second",
        &initial_commit,
    );

    // Break our worktree by moving the original repo on disk
    std::fs::rename(default_work_dir.root(), &new_repo_path).unwrap();
    // imagine JJ were able to do this
    std::fs::write(
        second_work_dir.root().join(".jj/repo"),
        new_repo_path
            .join(".jj/repo")
            .as_os_str()
            .as_encoded_bytes(),
    )
    .unwrap();

    // REVIEW: Is this the best way to do that?
    let output = second_work_dir.run_jj(["status"]);
    // hack for windows paths
    let gitfile_contents = std::fs::read_to_string(second_work_dir.root().join(".git"))
        .unwrap()
        .strip_prefix("gitdir: ")
        .unwrap()
        .trim()
        .to_owned();
    let stderr = output
        .stderr
        .normalized()
        .replace(&gitfile_contents, "$TEST_ENV/repo/.git/worktrees/second");
    insta::assert_snapshot!(stderr, @r"
    Warning: Workspace is a broken Git worktree
    The .git file points at: $TEST_ENV/repo/.git/worktrees/second
    Hint: If this is meant to be a colocated jj workspace, you may like to try `git -C $TEST_ENV/repo-moved worktree repair`
    ");

    Command::new("git")
        .args(["worktree", "repair"])
        .current_dir(&new_repo_path)
        .assert()
        .success();
    insta::assert_snapshot!(get_log_output(&second_work_dir), @"
    @  514e1b3adab7336794cf569e7b9c60c1dbcad1b4
    │ ○  64393b1a826a63bba44c4c5cec90d7a9040063b9
    ├─╯
    ○  dda9521046c4649797052c184beab33a9cf9754b initial commit
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_colocated_workspace_wrong_gitdir() {
    // TODO: Remove when this stops requiring git (stopgap_workspace_colocate)
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let default_work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");
    let other_work_dir = test_env.work_dir("other");
    let other_second_work_dir = test_env.work_dir("other_second");

    default_work_dir.create_dir_all("");
    default_work_dir
        .run_jj(["git", "init", "--colocate"])
        .success();
    default_work_dir.write_file("file", b"contents");
    default_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    let initial_commit = default_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        default_work_dir.root(),
        true,
        "../second",
        &initial_commit,
    );

    other_work_dir.create_dir_all("");
    other_work_dir
        .run_jj(["git", "init", "--colocate"])
        .success();
    other_work_dir.write_file("file", b"contents2");
    other_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    // REVIEW: Is this the best way to do that?
    let other_initial_commit = other_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        other_work_dir.root(),
        true,
        "../other_second",
        &other_initial_commit,
    );

    // Break one of our worktrees
    std::fs::copy(
        other_second_work_dir.root().join(".git"),
        second_work_dir.root().join(".git"),
    )
    .unwrap();

    let output = second_work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.stderr, @"
    Warning: Workspace is also a Git worktree that is not managed by jj
    Hint: To remove this worktree, run `git worktree remove .` from the parent Git repo
    [EOF]
    ");
}

#[test]
fn test_colocated_workspace_invalid_gitdir() {
    // TODO: Remove when this stops requiring git (stopgap_workspace_colocate)
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let default_work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");

    default_work_dir.create_dir_all("");
    default_work_dir
        .run_jj(["git", "init", "--colocate"])
        .success();
    default_work_dir.write_file("file", b"contents");
    default_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    // REVIEW: Is this the best way to do that?
    let initial_commit = default_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        default_work_dir.root(),
        true,
        "../second",
        &initial_commit,
    );

    // Break one of our worktrees
    std::fs::write(second_work_dir.root().join(".git"), "invalid").unwrap();

    let output = second_work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.stderr, @r"
    Warning: Workspace is a broken Git worktree
    The .git file points at: invalid
    Hint: If this is meant to be a colocated jj workspace, you may like to try `git -C $TEST_ENV/repo worktree repair`
    [EOF]
    ");
}

#[test]
fn test_colocated_workspace_independent_heads() {
    // TODO: Remove when this stops requiring git (stopgap_workspace_colocate)
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let default_work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");

    default_work_dir.create_dir_all("");
    default_work_dir
        .run_jj(["git", "init", "--colocate"])
        .success();
    // create a commit so that git can have a HEAD
    default_work_dir.write_file("file", b"contents");
    default_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    let initial_commit = default_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    // TODO: replace with workspace add, when it can create worktrees
    stopgap_workspace_colocate(
        &test_env,
        default_work_dir.root(),
        true,
        "../second",
        &initial_commit,
    );

    {
        let first_git = git::open(default_work_dir.root());
        let mut first_head = first_git.head().unwrap();
        assert!(first_head.is_detached());

        let commit = first_head.peel_to_commit().unwrap().id();
        assert_eq!(commit.to_string(), initial_commit);

        let second_git = git::open(second_work_dir.root());
        let mut second_head = second_git.head().unwrap();
        assert!(second_head.is_detached());

        let commit = second_head.peel_to_commit().unwrap().id();
        assert_eq!(commit.to_string(), initial_commit);
    }

    // now commit again in the second worktree, and make sure the original
    // repo's head does not move.
    //
    // This tests that we are writing HEAD to the corresponding worktree,
    // rather than unconditionally to the default workspace.
    default_work_dir.write_file("file", b"contents");
    second_work_dir
        .run_jj(["commit", "-m", "followup commit"])
        .success();
    let followup_commit = second_work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();

    {
        // git HEAD should not move in the default workspace
        let first_git = git::open(default_work_dir.root());
        let mut first_head = first_git.head().unwrap();
        assert!(first_head.is_detached());
        // still initial
        assert_eq!(
            first_head.peel_to_commit().unwrap().id().to_string(),
            initial_commit,
            "default workspace's git HEAD should not have moved from {initial_commit}"
        );

        let second_git = git::open(second_work_dir.root());
        let mut second_head = second_git.head().unwrap();
        assert!(second_head.is_detached());
        assert_eq!(
            second_head.peel_to_commit().unwrap().id().to_string(),
            followup_commit,
            "second workspace's git HEAD should have advanced to {followup_commit}"
        );
    }

    // Finally, test imports. Test that a commit written to HEAD in one workspace
    // does not get imported by the other workspace.

    // Write in default, expect second not to import it
    let new_commit = test_independent_import(&default_work_dir, &second_work_dir, &followup_commit);
    // Write in second, expect default not to import it
    test_independent_import(&second_work_dir, &default_work_dir, &new_commit);

    fn test_independent_import(
        commit_in: &TestWorkDir,
        no_import_in_workspace: &TestWorkDir,
        workspace_at: &str,
    ) -> String {
        // Commit in one workspace
        let mut repo = gix::open(commit_in.root()).unwrap();
        {
            use gix::config::tree::*;
            let mut config = repo.config_snapshot_mut();
            let (name, email) = ("JJ test", "jj@example.com");
            config.set_value(&Author::NAME, name).unwrap();
            config.set_value(&Author::EMAIL, email).unwrap();
            config.set_value(&Committer::NAME, name).unwrap();
            config.set_value(&Committer::EMAIL, email).unwrap();
        }
        let tree = repo.head_tree_id().unwrap();
        let current = repo.head_commit().unwrap().id;
        let new_commit = repo
            .commit(
                "HEAD",
                format!("empty commit in {}", commit_in.root().display()),
                tree,
                [current],
            )
            .unwrap()
            .to_string();

        let output = no_import_in_workspace.run_jj([
            "log",
            "--no-graph",
            "-r",
            "git_head()",
            "-T",
            "commit_id",
        ]);
        // Asserting no import message in stderr => no import occurred
        // (note: git_head() revset is deprecated and produces a warning)
        assert!(
            !output.stderr.normalized().contains("imported"),
            "Should not have imported HEAD in workspace {}",
            no_import_in_workspace.root().display()
        );
        // And the commit_id should be pointing to what it was before
        assert_eq!(
            output.stdout.normalized(),
            workspace_at,
            "should still be at {workspace_at} in workspace {}",
            no_import_in_workspace.root().display()
        );

        // Now we import the new HEAD in the commit_in workspace, so it's up to date.
        let output = commit_in.run_jj(["log", "--no-graph", "-r", "git_head()", "-T", "commit_id"]);
        // (note: git_head() revset is deprecated and produces a warning, so check
        // contains)
        assert!(
            output
                .stderr
                .normalized()
                .contains("Reset the working copy parent to the new Git HEAD."),
            "should have imported HEAD in workspace {}",
            commit_in.root().display()
        );
        assert_eq!(
            output.stdout.normalized(),
            new_commit,
            "should have advanced to {new_commit} in workspace {}",
            commit_in.root().display()
        );
        new_commit
    }
}

// =============================================================================
// Tests for `jj workspace add --colocate`
// =============================================================================

#[test]
fn test_workspace_add_colocate_basic() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let second_dir = test_env.env_root().join("second");

    // Create a colocated repo
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // Add a colocated workspace
    let output = work_dir.run_jj(["workspace", "add", "--colocate", "../second"]);
    output.success();

    // Verify the workspace is listed
    let output = work_dir.run_jj(["workspace", "list"]).success();
    let stdout = output.stdout.normalized();
    assert!(stdout.contains("default:"), "Should list default workspace");
    assert!(stdout.contains("second:"), "Should list second workspace");

    // Verify: Secondary workspace directory exists and has .git file (not
    // directory)
    assert!(
        second_dir.exists(),
        "Secondary workspace directory should exist"
    );
    let git_path = second_dir.join(".git");
    assert!(git_path.exists(), "Secondary workspace should have .git");
    assert!(
        git_path.is_file(),
        ".git should be a file (worktree), not directory"
    );

    // Verify: Files from the repo are accessible in secondary workspace
    assert!(
        second_dir.join("file").exists(),
        "Files should be accessible in secondary workspace"
    );
}

#[test]
fn test_workspace_add_colocate_non_colocated_error() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");

    // Create a non-colocated repo (default is non-colocated)
    work_dir.create_dir_all("");
    work_dir.run_jj(["git", "init"]).success();

    // Try to add a colocated workspace - should fail
    let output = work_dir.run_jj(["workspace", "add", "--colocate", "../second"]);
    assert!(!output.status.success(), "Command should fail");
    insta::assert_snapshot!(output.stderr, @r"
    Error: Cannot use --colocate: repository is not colocated with Git
    [EOF]
    ");
}

#[test]
fn test_workspace_add_colocate_creates_git_worktree() {
    // This test requires git command
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let second_work_dir = test_env.work_dir("second");

    // Create a colocated repo with two commits
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    work_dir.write_file("file", "contents");
    work_dir.run_jj(["commit", "-m", "first commit"]).success();
    // Get the first commit ID for later
    let first_commit = work_dir
        .run_jj(["log", "--no-graph", "-T", "commit_id", "-r", "@-"])
        .success()
        .stdout
        .into_raw();
    work_dir.write_file("file2", "more contents");
    work_dir.run_jj(["commit", "-m", "second commit"]).success();

    // Add a colocated workspace at the FIRST commit (not HEAD)
    // This tests the reload: git worktree add creates HEAD at primary's HEAD,
    // but jj needs to update it to the first commit.
    work_dir
        .run_jj([
            "workspace",
            "add",
            "--colocate",
            "../second",
            "-r",
            &first_commit,
        ])
        .success();

    // Verify: ../second/.git file exists (not directory)
    let git_path = second_work_dir.root().join(".git");
    assert!(
        git_path.exists(),
        ".git should exist in secondary workspace"
    );
    assert!(
        git_path.is_file(),
        ".git should be a file (Git worktree), not a directory"
    );

    // Verify: File starts with "gitdir:" (Git worktree marker)
    let git_contents = second_work_dir.read_file(".git");
    assert!(
        git_contents.starts_with(b"gitdir:"),
        ".git file should start with 'gitdir:' but was: {git_contents:?}"
    );

    // Verify: jj commands work in secondary workspace (is colocated)
    let output = second_work_dir.run_jj(["status"]);
    assert!(output.status.success(), "jj status should succeed");
    // The workspace should be colocated - no warning about non-colocated
    assert!(
        !output.stderr.normalized().contains("not colocated"),
        "Secondary workspace should be colocated"
    );

    // Verify the reload behavior: check that the secondary workspace's git HEAD
    // is set correctly DURING workspace creation. Without the reload that passes
    // workspace_root to RepoLoader, the secondary workspace would have
    // colocated=false and its initial working copy commit wouldn't update git HEAD.
    //
    // The git worktree was created at primary's HEAD (second commit), but jj
    // should have updated it to the first commit (our -r argument).
    let secondary_head = std::process::Command::new("git")
        .arg("-C")
        .arg(second_work_dir.root())
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .expect("git command failed");
    let secondary_head = String::from_utf8_lossy(&secondary_head.stdout)
        .trim()
        .to_string();

    assert_eq!(
        secondary_head, first_commit,
        "Secondary workspace's git HEAD should be updated to first commit during creation"
    );
}

#[test]
fn test_workspace_add_colocate_git_failure() {
    // This test requires git command
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");

    // Create a colocated repo
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    work_dir.write_file("file", "contents");
    work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // Pre-create a conflicting file at the destination
    let second_dir = test_env.env_root().join("second");
    std::fs::create_dir_all(&second_dir).unwrap();
    std::fs::write(second_dir.join(".git"), "conflicting content").unwrap();

    // Try to add a colocated workspace - should fail gracefully
    let output = work_dir.run_jj(["workspace", "add", "--colocate", "../second"]);
    assert!(!output.status.success(), "Command should fail");
    // The error should mention git worktree creation failure
    assert!(
        output.stderr.normalized().contains("git worktree")
            || output.stderr.normalized().contains("Git worktree")
            || output.stderr.normalized().contains("fatal:"),
        "Error should mention git worktree failure, got: {}",
        output.stderr.normalized()
    );
}

#[test]
fn test_workspace_add_colocate_empty_repo() {
    // This test verifies that workspace add --colocate works on an empty repo.
    // With --orphan, git worktree add works even without any commits.
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    let second_dir = test_env.env_root().join("second");

    // 1. Create colocated repo WITHOUT any commits
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();

    // 2. Add colocated workspace - should succeed even in empty repo
    work_dir
        .run_jj(["workspace", "add", "--colocate", "../second"])
        .success();

    // 3. Verify the workspace was created
    assert!(second_dir.exists(), "Secondary workspace should exist");
    let git_path = second_dir.join(".git");
    assert!(git_path.exists(), "Secondary workspace should have .git");
    assert!(
        git_path.is_file(),
        ".git should be a file (worktree), not directory"
    );
}

// =============================================================================
// Tests for import checking all worktrees' HEAD
// =============================================================================

#[test]
fn test_import_detects_secondary_worktree_head_change() {
    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_work_dir = test_env.work_dir("secondary");

    // 1. Create colocated repo in primary/
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // 2. Create secondary workspace with --colocate
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../secondary"])
        .success();

    // 3. In secondary, make a git commit bypassing jj
    // Use testutils::git with proper config for isolated git operations
    let secondary_git_repo = git::open(secondary_work_dir.root());
    let head_id = secondary_git_repo.head_id().unwrap().detach();
    let head_tree = secondary_git_repo
        .find_commit(head_id)
        .unwrap()
        .tree_id()
        .unwrap()
        .detach();
    git::write_commit(
        &secondary_git_repo,
        "HEAD",
        head_tree,
        "git commit in secondary worktree",
        &[head_id],
    );

    // 4. In primary, run any jj command (triggers import)
    let output = primary_work_dir
        .run_jj(["log", "--no-graph", "-T", "description"])
        .success();

    // 5. Verify: The git commit appears in jj log
    assert!(
        output
            .stdout
            .normalized()
            .contains("git commit in secondary worktree"),
        "Git commit from secondary worktree should be imported, got: {}",
        output.stdout.normalized()
    );

    // 6. Verify: The imported commit is usable (can be rebased)
    // This is a critical test - if the commit structure is broken, rebase will fail
    let output = primary_work_dir.run_jj([
        "rebase",
        "-r",
        "description('git commit in secondary worktree')",
        "-d",
        "root()",
        "--skip-emptied",
    ]);
    assert!(
        output.status.success(),
        "Should be able to rebase imported commit: {}",
        output.stderr.normalized()
    );

    // 7. Verify: Primary workspace data is preserved and functional
    assert!(
        primary_work_dir.root().join("file").exists(),
        "Primary workspace file should be preserved"
    );
    let output = primary_work_dir.run_jj(["status"]);
    assert!(
        output.status.success(),
        "jj status should work after import"
    );
}

#[test]
fn test_import_all_worktrees_heads() {
    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_work_dir = test_env.work_dir("secondary");
    let tertiary_work_dir = test_env.work_dir("tertiary");

    // 1. Create colocated repo + 2 secondary workspaces
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../secondary"])
        .success();
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../tertiary"])
        .success();

    // 2. Make git commits in both secondary workspaces
    // Use testutils::git with proper config for isolated git operations
    for (work_dir, msg) in [
        (&secondary_work_dir, "commit from secondary"),
        (&tertiary_work_dir, "commit from tertiary"),
    ] {
        let worktree_repo = git::open(work_dir.root());
        let head_id = worktree_repo.head_id().unwrap().detach();
        let head_tree = worktree_repo
            .find_commit(head_id)
            .unwrap()
            .tree_id()
            .unwrap()
            .detach();
        git::write_commit(&worktree_repo, "HEAD", head_tree, msg, &[head_id]);
    }

    // 3. Run jj in primary
    let output = primary_work_dir
        .run_jj(["log", "--no-graph", "-T", "description"])
        .success();

    // 4. Verify: Both commits imported
    let stdout = output.stdout.normalized();
    assert!(
        stdout.contains("commit from secondary"),
        "Commit from secondary should be imported, got: {stdout}"
    );
    assert!(
        stdout.contains("commit from tertiary"),
        "Commit from tertiary should be imported, got: {stdout}"
    );

    // 5. Verify: Both commits are usable (can be rebased)
    // This is a critical test - if the commit structure is broken, rebase will fail
    for commit_desc in ["commit from secondary", "commit from tertiary"] {
        let output = primary_work_dir.run_jj([
            "rebase",
            "-r",
            &format!("description('{commit_desc}')"),
            "-d",
            "root()",
            "--skip-emptied",
        ]);
        assert!(
            output.status.success(),
            "Should be able to rebase '{}': {}",
            commit_desc,
            output.stderr.normalized()
        );
    }

    // 6. Verify: Primary workspace data is preserved and functional
    assert!(
        primary_work_dir.root().join("file").exists(),
        "Primary workspace file should be preserved"
    );
    let output = primary_work_dir.run_jj(["status"]);
    assert!(
        output.status.success(),
        "jj status should work after import"
    );
}

// =============================================================================
// Tests for `jj workspace forget` with git worktree cleanup
// =============================================================================

#[test]
fn test_workspace_forget_removes_git_worktree() {
    // This test requires git command
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");

    // 1. Create colocated repo
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // 2. Add colocated workspace
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../second"])
        .success();

    // 3. Verify: git worktree list shows two worktrees
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(primary_work_dir.root())
        .arg("worktree")
        .arg("list")
        .output()
        .expect("git command failed");
    let worktree_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        worktree_list.contains("primary") && worktree_list.contains("second"),
        "Should have two worktrees listed: {worktree_list}"
    );

    // 4. Forget the workspace with --cleanup --force (--force needed since
    // jj-checked-out files are untracked from git's perspective)
    primary_work_dir
        .run_jj(["workspace", "forget", "--cleanup", "--force", "second"])
        .success();

    // 5. Verify: git worktree list shows only one worktree
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(primary_work_dir.root())
        .arg("worktree")
        .arg("list")
        .output()
        .expect("git command failed");
    let worktree_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        !worktree_list.contains("second"),
        "Second worktree should be removed: {worktree_list}"
    );
}

#[test]
fn test_workspace_forget_with_custom_name_removes_git_worktree() {
    // This test requires git command
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");

    // 1. Create colocated repo
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // 2. Add colocated workspace with a CUSTOM NAME that differs from directory
    // The directory will be "seconddir" but workspace name will be "myworkspace"
    primary_work_dir
        .run_jj([
            "workspace",
            "add",
            "--colocate",
            "--name",
            "myworkspace",
            "../seconddir",
        ])
        .success();

    // Verify workspace is created with the custom name
    let output = primary_work_dir.run_jj(["workspace", "list"]).success();
    assert!(
        output.stdout.normalized().contains("myworkspace"),
        "Workspace should be named myworkspace"
    );

    // 3. Verify: git worktree list shows the worktree (named after directory)
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(primary_work_dir.root())
        .arg("worktree")
        .arg("list")
        .output()
        .expect("git command failed");
    let worktree_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        worktree_list.contains("seconddir"),
        "Should have seconddir worktree: {worktree_list}"
    );

    // 4. Forget the workspace by its jj name (myworkspace, not seconddir)
    // Use --cleanup --force since jj-checked-out files are untracked from git's
    // perspective
    primary_work_dir
        .run_jj(["workspace", "forget", "--cleanup", "--force", "myworkspace"])
        .success();

    // 5. Verify: git worktree should be removed even though name differs
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(primary_work_dir.root())
        .arg("worktree")
        .arg("list")
        .output()
        .expect("git command failed");
    let worktree_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        !worktree_list.contains("seconddir"),
        "seconddir worktree should be removed: {worktree_list}"
    );
}

#[test]
fn test_workspace_forget_handles_missing_worktree() {
    // This test requires git command
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_dir = test_env.env_root().join("second");

    // 1. Create colocated repo + secondary workspace
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../second"])
        .success();

    // 2. Manually delete the secondary workspace directory
    std::fs::remove_dir_all(&secondary_dir).unwrap();

    // 3. Forget the workspace - should succeed without error
    let output = primary_work_dir.run_jj(["workspace", "forget", "second"]);
    // Should not fail (though may warn)
    assert!(
        output.status.success(),
        "Forgetting workspace with missing directory should succeed: {}",
        output.stderr.normalized()
    );
}

#[test]
fn test_workspace_forget_non_colocated_no_git_cleanup() {
    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_dir = test_env.env_root().join("second");

    // 1. Create non-colocated repo
    primary_work_dir.create_dir_all("");
    primary_work_dir.run_jj(["git", "init"]).success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // 2. Add regular workspace (no --colocate)
    primary_work_dir
        .run_jj(["workspace", "add", "../second"])
        .success();

    // Verify the secondary workspace exists
    assert!(secondary_dir.exists(), "Secondary workspace should exist");

    // 3. Forget the workspace
    primary_work_dir
        .run_jj(["workspace", "forget", "second"])
        .success();

    // 4. Verify: Directory still exists (current behavior - not touched on disk)
    assert!(
        secondary_dir.exists(),
        "Secondary workspace directory should still exist (not a git worktree)"
    );
}

#[test]
fn test_workspace_forget_dirty_worktree_warns() {
    // This test verifies that forgetting a colocated workspace with uncommitted
    // changes shows a warning and preserves the data (unless --force is used).
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_dir = test_env.env_root().join("second");

    // 1. Create colocated repo + secondary workspace
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../second"])
        .success();

    // 2. Create uncommitted file in secondary workspace
    std::fs::write(secondary_dir.join("important.txt"), "user data").unwrap();

    // 3. Forget with --cleanup but without --force - should warn about dirty
    //    worktree
    let output = primary_work_dir.run_jj(["workspace", "forget", "--cleanup", "second"]);
    // The command succeeds (workspace is forgotten from jj) but warns about git
    // worktree
    assert!(output.status.success());
    insta::assert_snapshot!(output.stderr.normalized(), @r"
    Warning: Git worktree for workspace second has uncommitted changes and was not removed.
    Hint: Use --cleanup --force to remove it anyway, or manually clean up with `git worktree remove --force $TEST_ENV/second`
    ");

    // 4. Verify: The uncommitted file should still exist
    assert!(
        secondary_dir.join("important.txt").exists(),
        "Uncommitted file should be preserved when not using --force"
    );

    // 5. Verify: The workspace directory still exists (git worktree not removed)
    assert!(
        secondary_dir.exists(),
        "Workspace directory should still exist"
    );
}

#[test]
fn test_workspace_forget_force_removes_dirty_worktree() {
    // This test verifies that --force removes the git worktree even with
    // uncommitted changes.
    if skip_if_git_unavailable() {
        return;
    }

    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");
    let secondary_dir = test_env.env_root().join("second");

    // 1. Create colocated repo + secondary workspace
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../second"])
        .success();

    // 2. Create uncommitted file in secondary workspace
    std::fs::write(secondary_dir.join("important.txt"), "user data").unwrap();

    // 3. Forget WITH --cleanup --force - should remove worktree despite dirty state
    let output = primary_work_dir.run_jj(["workspace", "forget", "--cleanup", "--force", "second"]);
    assert!(output.status.success());
    // No warning when --force is used
    let stderr = output.stderr.normalized();
    assert!(
        !stderr.contains("uncommitted changes"),
        "Should not warn about uncommitted changes with --force, got: {stderr}"
    );

    // 4. Verify: The workspace directory should be removed
    assert!(
        !secondary_dir.exists(),
        "Workspace directory should be removed with --force"
    );
}

#[test]
fn test_workspace_switch_no_spurious_commits() {
    // Regression test: Switching between colocated workspaces should not create
    // spurious commits. Previously, jj stored a single global git_head, but Git
    // maintains separate HEAD files per worktree. When switching workspaces, jj
    // would misinterpret the workspace switch as an "external Git change" and
    // create a new checkout.
    let test_env = TestEnvironment::default();
    let primary_work_dir = test_env.work_dir("primary");

    // Create colocated repo
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "primary"])
        .success();
    primary_work_dir.write_file("file", "contents");
    primary_work_dir
        .run_jj(["commit", "-m", "initial commit"])
        .success();

    // Create second colocated workspace
    primary_work_dir
        .run_jj(["workspace", "add", "--colocate", "../secondary"])
        .success();
    let secondary_work_dir = test_env.work_dir("secondary");

    // Record secondary workspace's initial state (change_id of @)
    let initial_output = secondary_work_dir
        .run_jj(["log", "-r", "@", "-T", "change_id"])
        .success();
    let initial_change_id = initial_output.stdout.raw().trim().to_string();

    // Switch to primary workspace (run some jj command)
    primary_work_dir.run_jj(["log"]).success();

    // Switch back to secondary workspace - should NOT create new commit
    let final_output = secondary_work_dir
        .run_jj(["log", "-r", "@", "-T", "change_id"])
        .success();
    let final_change_id = final_output.stdout.raw().trim().to_string();

    assert_eq!(
        initial_change_id, final_change_id,
        "Workspace switching created spurious commit! Initial change_id: {initial_change_id}, \
         Final change_id: {final_change_id}"
    );
}
