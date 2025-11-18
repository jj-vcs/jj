// Copyright 2023 The Jujutsu Authors
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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit;

#[test]
fn test_basics() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &[]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["a", "d"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [znk] e
    ├─╮
    │ ○  [vru] d
    │ ○  [roy] c
    │ │ ○  [zsu] b
    ├───╯
    ○ │  [rlv] a
    ├─╯
    ◆  [zzz]
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["abandon", "--retain-bookmarks", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      vruxwmqv 636920e4 d | d
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: znkkpsqq 38e96a1f e | e
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Parent commit (@-)      : royxmykx c12952d9 c d | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [znk] e
    ├─╮
    │ ○  [roy] c d
    │ │ ○  [zsu] b
    ├───╯
    ○ │  [rlv] a
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "--retain-bookmarks"]); // abandons `e`
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      znkkpsqq 03e0d4bf e | e
    Working copy  (@) now at: nkmrtpmo 179731fc (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a e?? | a
    Parent commit (@-)      : vruxwmqv 636920e4 d e?? | d
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [nkm]
    ├─╮
    │ ○  [vru] d e??
    │ ○  [roy] c
    │ │ ○  [zsu] b
    ├───╯
    ○ │  [rlv] a e??
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "descendants(d)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 2 commits:
      znkkpsqq 03e0d4bf e | e
      vruxwmqv 636920e4 d | d
    Deleted bookmarks: d, e
    Working copy  (@) now at: xtnwkqum 1c70f4d2 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Parent commit (@-)      : royxmykx c12952d9 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [xtn]
    ├─╮
    │ ○  [roy] c
    │ │ ○  [zsu] b
    ├───╯
    ○ │  [rlv] a
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    // Test abandoning the same commit twice directly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "-rb", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      zsuskuln 123b4d91 b | b
    Deleted bookmarks: b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [znk] e
    ├─╮
    │ ○  [vru] d
    │ ○  [roy] c
    ○ │  [rlv] a
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    // Test abandoning the same commit twice indirectly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "d::", "e"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 2 commits:
      znkkpsqq 03e0d4bf e | e
      vruxwmqv 636920e4 d | d
    Deleted bookmarks: d, e
    Working copy  (@) now at: xlzxqlsl 55d5c4c2 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Parent commit (@-)      : royxmykx c12952d9 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [xlz]
    ├─╮
    │ ○  [roy] c
    │ │ ○  [zsu] b
    ├───╯
    ○ │  [rlv] a
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    let output = work_dir.run_jj(["abandon", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No revisions to abandon.
    [EOF]
    ");
}

#[test]
fn test_abandon_many() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    for i in 0..10 {
        work_dir.run_jj(["new", &format!("-mcommit{i}")]).success();
    }

    // The list of commits should be elided.
    let output = work_dir.run_jj(["abandon", ".."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 11 commits:
      kpqxywon 6faec5d1 (empty) commit9
      znkkpsqq 80f67a53 (empty) commit8
      yostqsxw 8bd72fcc (empty) commit7
      vruxwmqv 06c46771 (empty) commit6
      yqosqzyt 168813e3 (empty) commit5
      royxmykx bd8c3571 (empty) commit4
      mzvwutvl 57958fa0 (empty) commit3
      zsuskuln 59515644 (empty) commit2
      kkmpptxz a969596d (empty) commit1
      rlvkpnrz 02fe38c9 (empty) commit0
      ...
    Working copy  (@) now at: kmkuslsw a36a913b (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

// This behavior illustrates https://github.com/jj-vcs/jj/issues/2600.
// See also the corresponding test in `test_rebase_command`
#[test]
fn test_bug_2600() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // We will not touch "nottherootcommit". See the
    // `test_bug_2600_rootcommit_special_case` for the one case where base being the
    // child of the root commit changes the expected behavior.
    create_commit(&work_dir, "nottherootcommit", &[]);
    create_commit(&work_dir, "base", &["nottherootcommit"]);
    create_commit(&work_dir, "a", &["base"]);
    create_commit(&work_dir, "b", &["base", "a"]);
    create_commit(&work_dir, "c", &["b"]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [znk] c
    ○    [vru] b
    ├─╮
    │ ○  [roy] a
    ├─╯
    ○  [zsu] base
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "base"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      zsuskuln 67c2f714 base | base
    Deleted bookmarks: base
    Rebased 3 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: znkkpsqq c1223866 c | c
    Parent commit (@-)      : vruxwmqv 1dfaa834 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commits "a" and "b" should both have "nottherootcommit" as parent, and "b"
    // should keep "a" as second parent.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [znk] c
    ○    [vru] b
    ├─╮
    │ ○  [roy] a
    ├─╯
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      royxmykx 183dbbca a | a
    Deleted bookmarks: a
    Rebased 2 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: znkkpsqq f863da3f c | c
    Parent commit (@-)      : vruxwmqv d7aed853 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commit "b" should have "base" as parent. It should not have two parent
    // pointers to that commit even though it was a merge commit before we abandoned
    // "a".
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [znk] c
    ○  [vru] b
    ○  [zsu] base
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      vruxwmqv cedee197 b | b
    Deleted bookmarks: b
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: znkkpsqq 4dc308fb c | c
    Parent commit (@-)      : zsuskuln 67c2f714 base | base
    Parent commit (@-)      : royxmykx 183dbbca a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commit "c" should inherit the parents from the abndoned commit "b".
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    [znk] c
    ├─╮
    │ ○  [roy] a
    ├─╯
    ○  [zsu] base
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // ========= Reminder of the setup ===========
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [znk] c
    ○    [vru] b
    ├─╮
    │ ○  [roy] a
    ├─╯
    ○  [zsu] base
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");
    let output = work_dir.run_jj(["abandon", "--retain-bookmarks", "a", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 2 commits:
      vruxwmqv cedee197 b | b
      royxmykx 183dbbca a | a
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: znkkpsqq b350f44b c | c
    Parent commit (@-)      : zsuskuln 67c2f714 a b base | base
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    // Commit "c" should have "base" as parent. As when we abandoned "a", it should
    // not have two parent pointers to the same commit.
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [znk] c
    ○  [zsu] a b base
    ○  [rlv] nottherootcommit
    ◆  [zzz]
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "list", "b"]);
    insta::assert_snapshot!(output, @r"
    b: zsuskuln 67c2f714 base
    [EOF]
    ");
}

#[test]
fn test_bug_2600_rootcommit_special_case() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up like `test_bug_2600`, but without the `nottherootcommit` commit.
    create_commit(&work_dir, "base", &[]);
    create_commit(&work_dir, "a", &["base"]);
    create_commit(&work_dir, "b", &["base", "a"]);
    create_commit(&work_dir, "c", &["b"]);

    // Setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  [vru] c
    ○    [roy] b
    ├─╮
    │ ○  [zsu] a
    ├─╯
    ○  [rlv] base
    ◆  [zzz]
    [EOF]
    ");

    // Now, the test
    let output = work_dir.run_jj(["abandon", "base"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The Git backend does not support creating merge commits with the root commit as one of the parents.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_double_abandon() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    // Test the setup
    insta::assert_snapshot!(work_dir.run_jj(["log", "--no-graph", "-r", "a"]), @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a 7d980be7
    a
    [EOF]
    ");

    let commit_id = work_dir
        .run_jj(["log", "--no-graph", "--color=never", "-T=commit_id", "-r=a"])
        .success()
        .stdout
        .into_raw();

    let output = work_dir.run_jj(["abandon", &commit_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      rlvkpnrz 7d980be7 a | a
    Deleted bookmarks: a
    Working copy  (@) now at: royxmykx 0cff017c (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = work_dir.run_jj(["abandon", &commit_id]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Skipping 1 revisions that are already hidden.
    No revisions to abandon.
    [EOF]
    ");
}

#[test]
fn test_abandon_restore_descendants() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "foo\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "baz\n");

    // Remove the commit containing "bar"
    let output = work_dir.run_jj(["abandon", "-r@-", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      rlvkpnrz b23f92c3 (no description set)
    Rebased 1 descendant commits (while preserving their content) onto parents of abandoned commits
    Working copy  (@) now at: kkmpptxz 2b575035 (no description set)
    Parent commit (@-)      : qpvuntsm d0c049cd (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output, @r"
    diff --git a/file b/file
    index 257cc5642c..76018072e0 100644
    --- a/file
    +++ b/file
    @@ -1,1 +1,1 @@
    -foo
    +baz
    [EOF]
    ");
}

#[test]
fn test_abandon_tracking_bookmarks() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "remote"]).success();
    let remote_dir = test_env.work_dir("remote");
    remote_dir
        .run_jj(["bookmark", "set", "-r@", "foo"])
        .success();
    remote_dir.run_jj(["git", "export"]).success();

    // Create colocated Git repo which may have @git tracking bookmarks
    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "clone",
                "--colocate",
                "--config=remotes.origin.auto-track-bookmarks='glob:*'",
                "remote/.jj/repo/store/git",
                "local",
            ],
        )
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["bookmark", "set", "-r@", "bar"])
        .success();
    insta::assert_snapshot!(get_log_output(&local_dir), @r"
    @  [zsu] bar
    │ ○  [qpv] foo
    ├─╯
    ◆  [zzz]
    [EOF]
    ");

    let output = local_dir.run_jj(["abandon", "foo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      qpvuntsm e8849ae1 foo | (empty) (no description set)
    Deleted bookmarks: foo
    Hint: Deleted bookmarks can be pushed by name or all at once with `jj git push --deleted`.
    [EOF]
    ");
    let output = local_dir.run_jj(["abandon", "bar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Abandoned 1 commits:
      zsuskuln c2934cfb bar | (empty) (no description set)
    Deleted bookmarks: bar
    Working copy  (@) now at: vruxwmqv b64f323d (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", "[" ++ change_id.short(3) ++ "]", bookmarks)"#;
    work_dir.run_jj(["log", "-T", template])
}
