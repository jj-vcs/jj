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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [nnkkp] e
    ├─╮
    │ ○  [truxw] d
    │ ○  [ooyxm] c
    │ │ ○  [psusk] b
    ├───╯
    ○ │  [ylvkp] a
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["abandon", "--retain-bookmarks", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      truxwmqv 295a5aee d | d
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: nnkkpsqq 0f3014db e | e
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : ooyxmykx 8d67ed49 c d | c
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [nnkkp] e
    ├─╮
    │ ○  [ooyxm] c d
    │ │ ○  [psusk] b
    ├───╯
    ○ │  [ylvkp] a
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "--retain-bookmarks"]); // abandons `e`
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      nnkkpsqq 5ba56987 e | e
    Working copy  (@) now at: nkmrtpmo 57904f85 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a e?? | a
    Parent commit (@-)      : truxwmqv 295a5aee d e?? | d
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [nkmrt]
    ├─╮
    │ ○  [truxw] d e??
    │ ○  [ooyxm] c
    │ │ ○  [psusk] b
    ├───╯
    ○ │  [ylvkp] a e??
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "descendants(d)"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 2 commits:
      nnkkpsqq 5ba56987 e | e
      truxwmqv 295a5aee d | d
    Deleted bookmarks: d, e
    Working copy  (@) now at: xtnwkqum a36f99b7 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : ooyxmykx 8d67ed49 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [xtnwk]
    ├─╮
    │ ○  [ooyxm] c
    │ │ ○  [psusk] b
    ├───╯
    ○ │  [ylvkp] a
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    // Test abandoning the same commit twice directly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "-rb", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      psuskuln dd148a1b b | b
    Deleted bookmarks: b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [nnkkp] e
    ├─╮
    │ ○  [truxw] d
    │ ○  [ooyxm] c
    ○ │  [ylvkp] a
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    // Test abandoning the same commit twice indirectly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "d::", "e"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 2 commits:
      nnkkpsqq 5ba56987 e | e
      truxwmqv 295a5aee d | d
    Deleted bookmarks: d, e
    Working copy  (@) now at: xlzxqlsl 48c7fefa (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Parent commit (@-)      : ooyxmykx 8d67ed49 c | c
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [xlzxq]
    ├─╮
    │ ○  [ooyxm] c
    │ │ ○  [psusk] b
    ├───╯
    ○ │  [ylvkp] a
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    let output = work_dir.run_jj(["abandon", "none()"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 11 commits:
      kpqxywon 2903c8b6 (empty) commit9
      snkkpsqq f825fefa (empty) commit8
      tostqsxw 552f0060 (empty) commit7
      vruxwmqv ab1c7c55 (empty) commit6
      mqosqzyt 43b92eab (empty) commit5
      ooyxmykx 56237244 (empty) commit4
      rzvwutvl 24d10c5f (empty) commit3
      psuskuln 0541396e (empty) commit2
      nkmpptxz 81825895 (empty) commit1
      ylvkpnrz 22772eda (empty) commit0
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [nnkkp] c
    ○    [truxw] b
    ├─╮
    │ ○  [ooyxm] a
    ├─╯
    ○  [psusk] base
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "base"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      psuskuln 2c0a0f28 base | base
    Deleted bookmarks: base
    Rebased 3 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: nnkkpsqq e4e03c6b c | c
    Parent commit (@-)      : truxwmqv 0b196c6f b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commits "a" and "b" should both have "nottherootcommit" as parent, and "b"
    // should keep "a" as second parent.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [nnkkp] c
    ○    [truxw] b
    ├─╮
    │ ○  [ooyxm] a
    ├─╯
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      ooyxmykx 290e5e8d a | a
    Deleted bookmarks: a
    Rebased 2 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: nnkkpsqq 01ebc121 c | c
    Parent commit (@-)      : truxwmqv 16219805 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commit "b" should have "base" as parent. It should not have two parent
    // pointers to that commit even though it was a merge commit before we abandoned
    // "a".
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [nnkkp] c
    ○  [truxw] b
    ○  [psusk] base
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["abandon", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      truxwmqv 41d5097e b | b
    Deleted bookmarks: b
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: nnkkpsqq 058d8551 c | c
    Parent commit (@-)      : psuskuln 2c0a0f28 base | base
    Parent commit (@-)      : ooyxmykx 290e5e8d a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Commit "c" should inherit the parents from the abndoned commit "b".
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    [nnkkp] c
    ├─╮
    │ ○  [ooyxm] a
    ├─╯
    ○  [psusk] base
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // ========= Reminder of the setup ===========
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [nnkkp] c
    ○    [truxw] b
    ├─╮
    │ ○  [ooyxm] a
    ├─╯
    ○  [psusk] base
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");
    let output = work_dir.run_jj(["abandon", "--retain-bookmarks", "a", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 2 commits:
      truxwmqv 41d5097e b | b
      ooyxmykx 290e5e8d a | a
    Rebased 1 descendant commits onto parents of abandoned commits
    Working copy  (@) now at: nnkkpsqq 2289a28e c | c
    Parent commit (@-)      : psuskuln 2c0a0f28 a b base | base
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    // Commit "c" should have "base" as parent. As when we abandoned "a", it should
    // not have two parent pointers to the same commit.
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [nnkkp] c
    ○  [psusk] a b base
    ○  [ylvkp] nottherootcommit
    ◆  [zzzzz]
    [EOF]
    ");
    let output = work_dir.run_jj(["bookmark", "list", "b"]);
    insta::assert_snapshot!(output, @"
    b: psuskuln 2c0a0f28 base
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
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  [truxw] c
    ○    [ooyxm] b
    ├─╮
    │ ○  [psusk] a
    ├─╯
    ○  [ylvkp] base
    ◆  [zzzzz]
    [EOF]
    ");

    // Now, the test
    let output = work_dir.run_jj(["abandon", "base"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(work_dir.run_jj(["log", "--no-graph", "-r", "a"]), @"
    ylvkpnrz test.user@example.com 2001-02-03 08:05:09 a a1afb583
    a
    [EOF]
    ");

    let commit_id = work_dir
        .run_jj(["log", "--no-graph", "--color=never", "-T=commit_id", "-r=a"])
        .success()
        .stdout
        .into_raw();

    let output = work_dir.run_jj(["abandon", &commit_id]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      ylvkpnrz a1afb583 a | a
    Deleted bookmarks: a
    Working copy  (@) now at: royxmykx 0cff017c (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = work_dir.run_jj(["abandon", &commit_id]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      ylvkpnrz ab2bba2b (no description set)
    Rebased 1 descendant commits (while preserving their content) onto parents of abandoned commits
    Working copy  (@) now at: nkmpptxz 2d2eb500 (no description set)
    Parent commit (@-)      : qpvuntsm d0c049cd (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output, @"
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
                "--config=remotes.origin.auto-track-bookmarks='*'",
                "remote/.jj/repo/store/git",
                "local",
            ],
        )
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir
        .run_jj(["bookmark", "set", "-r@", "bar"])
        .success();
    insta::assert_snapshot!(get_log_output(&local_dir), @"
    @  [zsusk] bar
    │ ○  [qpvun] foo
    ├─╯
    ◆  [zzzzz]
    [EOF]
    ");

    let output = local_dir.run_jj(["abandon", "foo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Abandoned 1 commits:
      qpvuntsm e8849ae1 foo | (empty) (no description set)
    Deleted bookmarks: foo
    Hint: Deleted bookmarks can be pushed by name or all at once with `jj git push --deleted`.
    [EOF]
    ");
    let output = local_dir.run_jj(["abandon", "bar"]);
    insta::assert_snapshot!(output, @"
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
    let template = r#"separate(" ", "[" ++ change_id.short(5) ++ "]", bookmarks)"#;
    work_dir.run_jj(["log", "-T", template])
}
