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
use crate::common::create_commit_with_files;

#[test]
fn test_status_copies() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("copy-source", "copy1\ncopy2\ncopy3\n");
    work_dir.write_file("rename-source", "rename");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("copy-source", "copy1\ncopy2\ncopy3\nsource\n");
    work_dir.write_file("copy-target", "copy1\ncopy2\ncopy3\ntarget\n");
    work_dir.remove_file("rename-source");
    work_dir.write_file("rename-target", "rename");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M copy-source
    C {copy-source => copy-target}
    R {rename-source => rename-target}
    Working copy  (@) : rlvkpnrz c2fce842 (no description set)
    Parent commit (@-): qpvuntsm ebf799bc (no description set)
    [EOF]
    ");
}

#[test]
fn test_status_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "base");
    work_dir.run_jj(["new", "-m=left"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();
    work_dir.run_jj(["new", "@-", "-m=right"]).success();
    work_dir.write_file("file", "right");
    work_dir.run_jj(["new", "left", "@"]).success();

    // The output should mention each parent, and the diff should be empty (compared
    // to the auto-merged parents)
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : mzvwutvl f62dad77 (empty) (no description set)
    Parent commit (@-): rlvkpnrz a007d87b left | (empty) left
    Parent commit (@-): zsuskuln e6ad1952 right
    [EOF]
    ");
}

// See https://github.com/jj-vcs/jj/issues/2051.
#[test]
fn test_status_ignored_gitignore() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let untracked_dir = work_dir.create_dir("untracked");
    untracked_dir.write_file("inside_untracked", "test");
    untracked_dir.write_file(".gitignore", "!inside_untracked\n");
    work_dir.write_file(".gitignore", "untracked/\n!dummy\n");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A .gitignore
    Working copy  (@) : qpvuntsm 32bad97e (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_status_filtered() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file_1", "file_1");
    work_dir.write_file("file_2", "file_2");

    // The output filtered to file_1 should not list the addition of file_2.
    let output = work_dir.run_jj(["status", "file_1"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file_1
    Working copy  (@) : qpvuntsm 2f169edb (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // The output filtered to a non-existent file should display a warning.
    let output = work_dir.run_jj(["status", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    Working copy  (@) : qpvuntsm 2f169edb (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    [EOF]
    ");
}

#[test]
fn test_status_conflicted_bookmarks() {
    // create conflicted local bookmark
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["bookmark", "create", "local_bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m=a"]).success();
    work_dir
        .run_jj(["describe", "-m=b", "--at-op=@-"])
        .success();

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm/1 99025a24 local_bookmark?? | (divergent) (empty) a
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    Warning: These bookmarks have conflicts:
      local_bookmark
    Hint: Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to resolve.
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    // create remote
    test_env
        .run_jj_in(".", ["git", "init", "origin", "--colocate"])
        .success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir.root().join(".git");
    origin_dir
        .run_jj(["bookmark", "create", "remote_bookmark"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

    // fetch remote bookmark with empty changes
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "origin",
            origin_git_repo_path.to_str().unwrap(),
        ])
        .success();
    work_dir.run_jj(["git", "fetch"]).success();

    // update remote
    origin_dir.write_file("file.txt", "");
    origin_dir.run_jj(["git", "export"]).success();

    // create conflicted remote bookmark
    work_dir.run_jj(["git", "fetch", "--at-op", "@-"]).success();
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : qpvuntsm/1 99025a24 local_bookmark?? | (divergent) (empty) a
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    Warning: These bookmarks have conflicts:
      local_bookmark
    Hint: Use `jj bookmark list` to see details. Use `jj bookmark set <name> -r <rev>` to resolve.
    Warning: These remote bookmarks have conflicts:
      remote_bookmark@origin
    Hint: Use `jj bookmark list` to see details. Use `jj git fetch` to resolve.
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

// See <https://github.com/jj-vcs/jj/issues/3108>
// See <https://github.com/jj-vcs/jj/issues/4147>
#[test]
fn test_status_display_relevant_working_commit_conflict_hints() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();

    let work_dir = test_env.work_dir("repo");
    // PARENT: Write the initial file
    work_dir.write_file("conflicted1.txt", "initial contents");
    work_dir.write_file("conflicted2.txt", "initial contents");
    work_dir
        .run_jj(["describe", "--message", "Initial contents"])
        .success();

    // CHILD1: New commit on top of <PARENT>
    work_dir
        .run_jj(["new", "--message", "First part of conflicting change"])
        .success();
    work_dir.write_file("conflicted1.txt", "Child 1");
    work_dir.write_file("conflicted2.txt", "Child 1");

    // CHILD2: New commit also on top of <PARENT>
    work_dir
        .run_jj([
            "new",
            "--message",
            "Second part of conflicting change",
            "@-",
        ])
        .success();
    work_dir.write_file("conflicted1.txt", "Child 2");
    work_dir.write_file("conflicted2.txt", "Child 2");

    // CONFLICT: New commit that is conflicted by merging <CHILD1> and <CHILD2>
    work_dir
        .run_jj(["new", "--message", "boom", "(@-)+"])
        .success();
    // Adding more descendants to ensure we correctly find the root ancestors with
    // conflicts, not just the parents.
    work_dir.run_jj(["new", "--message", "boom-cont"]).success();
    work_dir
        .run_jj(["new", "--message", "boom-cont-2"])
        .success();

    let output = work_dir.run_jj(["log", "-r", "::"]);

    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:13 d31638b7 (conflict)
    │  (empty) boom-cont-2
    ×  royxmykx test.user@example.com 2001-02-03 08:05:12 20d4acb3 (conflict)
    │  (empty) boom-cont
    ×    mzvwutvl test.user@example.com 2001-02-03 08:05:11 553ce839 (conflict)
    ├─╮  (empty) boom
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 7bb1724e
    │ │  First part of conflicting change
    ○ │  zsuskuln test.user@example.com 2001-02-03 08:05:11 263e2eb6
    ├─╯  Second part of conflicting change
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8b2bca65
    │  Initial contents
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : yqosqzyt d31638b7 (conflict) (empty) boom-cont-2
    Parent commit (@-): royxmykx 20d4acb3 (conflict) (empty) boom-cont
    Warning: There are unresolved conflicts at these paths:
    conflicted1.txt    2-sided conflict
    conflicted2.txt    2-sided conflict
    Hint: To resolve the conflicts, start by creating a commit on top of
    the first conflicted commit:
      jj new mzvwutvl
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    // Can filter by paths for conflicts.
    let output = work_dir.run_jj(["status", "conflicted1.txt"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : yqosqzyt d31638b7 (conflict) (empty) boom-cont-2
    Parent commit (@-): royxmykx 20d4acb3 (conflict) (empty) boom-cont
    Warning: There are unresolved conflicts at these paths:
    conflicted1.txt    2-sided conflict
    Hint: To resolve the conflicts, start by creating a commit on top of
    the first conflicted commit:
      jj new mzvwutvl
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    let output = work_dir.run_jj(["status", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : [1m[38;5;13my[38;5;8mqosqzyt[39m [38;5;12md[38;5;8m31638b7[39m [38;5;9m(conflict)[39m [38;5;10m(empty)[39m boom-cont-2[0m
    Parent commit (@-): [1m[38;5;5mr[0m[38;5;8moyxmykx[39m [1m[38;5;4m20[0m[38;5;8md4acb3[39m [38;5;1m(conflict)[39m [38;5;2m(empty)[39m boom-cont
    [1m[38;5;3mWarning: [39mThere are unresolved conflicts at these paths:[0m
    conflicted1.txt    [38;5;3m2-sided conflict[39m
    conflicted2.txt    [38;5;3m2-sided conflict[39m
    [1m[38;5;6mHint: [0m[39mTo resolve the conflicts, start by creating a commit on top of[39m
    [39mthe first conflicted commit:[39m
    [39m  jj new [1m[38;5;5mm[0m[38;5;8mzvwutvl[39m[39m
    [39mThen use `jj resolve`, or edit the conflict markers in the file directly.[39m
    [39mOnce the conflicts are resolved, you can inspect the result with `jj diff`.[39m
    [39mThen run `jj squash` to move the resolution into the conflicted commit.[39m
    [EOF]
    ");

    let output = work_dir.run_jj(["status", "--config=hints.resolving-conflicts=false"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : yqosqzyt d31638b7 (conflict) (empty) boom-cont-2
    Parent commit (@-): royxmykx 20d4acb3 (conflict) (empty) boom-cont
    Warning: There are unresolved conflicts at these paths:
    conflicted1.txt    2-sided conflict
    conflicted2.txt    2-sided conflict
    [EOF]
    ");

    // Resolve conflict
    work_dir.run_jj(["new", "--message", "fixed 1"]).success();
    work_dir.write_file("conflicted1.txt", "first commit to fix conflict");
    work_dir.write_file("conflicted2.txt", "first commit to fix conflict");

    // Add one more commit atop the commit that resolves the conflict.
    work_dir.run_jj(["new", "--message", "fixed 2"]).success();
    work_dir.write_file("conflicted1.txt", "edit not conflict");
    work_dir.write_file("conflicted2.txt", "edit not conflict");

    // wc is now conflict free, parent is also conflict free
    let output = work_dir.run_jj(["log", "-r", "::"]);

    insta::assert_snapshot!(output, @r"
    @  lylxulpl test.user@example.com 2001-02-03 08:05:21 db84565c
    │  fixed 2
    ○  wqnwkozp test.user@example.com 2001-02-03 08:05:20 b64eded1
    │  fixed 1
    ×  yqosqzyt test.user@example.com 2001-02-03 08:05:13 d31638b7 (conflict)
    │  (empty) boom-cont-2
    ×  royxmykx test.user@example.com 2001-02-03 08:05:12 20d4acb3 (conflict)
    │  (empty) boom-cont
    ×    mzvwutvl test.user@example.com 2001-02-03 08:05:11 553ce839 (conflict)
    ├─╮  (empty) boom
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 7bb1724e
    │ │  First part of conflicting change
    ○ │  zsuskuln test.user@example.com 2001-02-03 08:05:11 263e2eb6
    ├─╯  Second part of conflicting change
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8b2bca65
    │  Initial contents
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["status"]);

    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M conflicted1.txt
    M conflicted2.txt
    Working copy  (@) : lylxulpl db84565c fixed 2
    Parent commit (@-): wqnwkozp b64eded1 fixed 1
    [EOF]
    ");

    // Step back one.
    // wc is still conflict free, parent has a conflict.
    work_dir.run_jj(["edit", "@-"]).success();
    let output = work_dir.run_jj(["log", "-r", "::"]);

    insta::assert_snapshot!(output, @r"
    ○  lylxulpl test.user@example.com 2001-02-03 08:05:21 db84565c
    │  fixed 2
    @  wqnwkozp test.user@example.com 2001-02-03 08:05:20 b64eded1
    │  fixed 1
    ×  yqosqzyt test.user@example.com 2001-02-03 08:05:13 d31638b7 (conflict)
    │  (empty) boom-cont-2
    ×  royxmykx test.user@example.com 2001-02-03 08:05:12 20d4acb3 (conflict)
    │  (empty) boom-cont
    ×    mzvwutvl test.user@example.com 2001-02-03 08:05:11 553ce839 (conflict)
    ├─╮  (empty) boom
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 7bb1724e
    │ │  First part of conflicting change
    ○ │  zsuskuln test.user@example.com 2001-02-03 08:05:11 263e2eb6
    ├─╯  Second part of conflicting change
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8b2bca65
    │  Initial contents
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["status"]);

    insta::assert_snapshot!(output, @r"
    Working copy changes:
    M conflicted1.txt
    M conflicted2.txt
    Working copy  (@) : wqnwkozp b64eded1 fixed 1
    Parent commit (@-): yqosqzyt d31638b7 (conflict) (empty) boom-cont-2
    Hint: Conflict in parent commit has been resolved in working copy
    [EOF]
    ");

    // Step back to all the way to `root()+` so that wc has no conflict, even though
    // there is a conflict later in the tree. So that we can confirm
    // our hinting logic doesn't get confused.
    work_dir.run_jj(["edit", "root()+"]).success();
    let output = work_dir.run_jj(["log", "-r", "::"]);

    insta::assert_snapshot!(output, @r"
    ○  lylxulpl test.user@example.com 2001-02-03 08:05:21 db84565c
    │  fixed 2
    ○  wqnwkozp test.user@example.com 2001-02-03 08:05:20 b64eded1
    │  fixed 1
    ×  yqosqzyt test.user@example.com 2001-02-03 08:05:13 d31638b7 (conflict)
    │  (empty) boom-cont-2
    ×  royxmykx test.user@example.com 2001-02-03 08:05:12 20d4acb3 (conflict)
    │  (empty) boom-cont
    ×    mzvwutvl test.user@example.com 2001-02-03 08:05:11 553ce839 (conflict)
    ├─╮  (empty) boom
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 7bb1724e
    │ │  First part of conflicting change
    ○ │  zsuskuln test.user@example.com 2001-02-03 08:05:11 263e2eb6
    ├─╯  Second part of conflicting change
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 8b2bca65
    │  Initial contents
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["status"]);

    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A conflicted1.txt
    A conflicted2.txt
    Working copy  (@) : qpvuntsm 8b2bca65 Initial contents
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_status_simplify_conflict_sides() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Creates a 4-sided conflict, with fileA and fileB having different conflicts:
    // fileA: A - B + C - B + B - B + B
    // fileB: A - A + A - A + B - C + D
    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[("fileA", "base\n"), ("fileB", "base\n")],
    );
    create_commit_with_files(&work_dir, "a1", &["base"], &[("fileA", "1\n")]);
    create_commit_with_files(&work_dir, "a2", &["base"], &[("fileA", "2\n")]);
    create_commit_with_files(&work_dir, "b1", &["base"], &[("fileB", "1\n")]);
    create_commit_with_files(&work_dir, "b2", &["base"], &[("fileB", "2\n")]);
    create_commit_with_files(&work_dir, "conflictA", &["a1", "a2"], &[]);
    create_commit_with_files(&work_dir, "conflictB", &["b1", "b2"], &[]);
    create_commit_with_files(&work_dir, "conflict", &["conflictA", "conflictB"], &[]);

    insta::assert_snapshot!(work_dir.run_jj(["status"]),
    @r"
    The working copy has no changes.
    Working copy  (@) : nkmrtpmo 88a1af9c conflict | (conflict) (empty) conflict
    Parent commit (@-): kmkuslsw 0fb57c44 conflictA | (conflict) (empty) conflictA
    Parent commit (@-): lylxulpl 4ec7381a conflictB | (conflict) (empty) conflictB
    Warning: There are unresolved conflicts at these paths:
    fileA    2-sided conflict
    fileB    2-sided conflict
    Hint: To resolve the conflicts, start by creating a commit on top of
    one of the first conflicted commits:
      jj new lylxulpl
      jj new kmkuslsw
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
}

#[test]
fn test_status_untracked_files() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.auto-track = "none()""#);

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("always-untracked-file", "...");
    work_dir.write_file("initially-untracked-file", "...");
    let sub_dir = work_dir.create_dir("sub");
    sub_dir.write_file("always-untracked", "...");
    sub_dir.write_file("initially-untracked", "...");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Untracked paths:
    ? always-untracked-file
    ? initially-untracked-file
    ? sub/
    Working copy  (@) : qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    work_dir
        .run_jj([
            "file",
            "track",
            "initially-untracked-file",
            "sub/initially-untracked",
        ])
        .success();

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Working copy changes:
    A initially-untracked-file
    A sub/initially-untracked
    Untracked paths:
    ? always-untracked-file
    ? sub/always-untracked
    Working copy  (@) : qpvuntsm b8c1286d (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Untracked paths:
    ? always-untracked-file
    ? sub/always-untracked
    Working copy  (@) : mzvwutvl daa133b8 (empty) (no description set)
    Parent commit (@-): qpvuntsm b8c1286d (no description set)
    [EOF]
    ");

    work_dir
        .run_jj([
            "file",
            "untrack",
            "initially-untracked-file",
            "sub/initially-untracked",
        ])
        .success();
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Working copy changes:
    D initially-untracked-file
    D sub/initially-untracked
    Untracked paths:
    ? always-untracked-file
    ? initially-untracked-file
    ? sub/
    Working copy  (@) : mzvwutvl 240f261a (no description set)
    Parent commit (@-): qpvuntsm b8c1286d (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Untracked paths:
    ? always-untracked-file
    ? initially-untracked-file
    ? sub/
    Working copy  (@) : yostqsxw 50beac0d (empty) (no description set)
    Parent commit (@-): mzvwutvl 240f261a (no description set)
    [EOF]
    ");

    let output = work_dir.dir("sub").run_jj(["status"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Untracked paths:
    ? ../always-untracked-file
    ? ../initially-untracked-file
    ? ./
    Working copy  (@) : yostqsxw 50beac0d (empty) (no description set)
    Parent commit (@-): mzvwutvl 240f261a (no description set)
    [EOF]
    ");
}

#[test]
fn test_status_no_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["workspace", "forget"]).success();

    insta::assert_snapshot!(work_dir.run_jj(["status"]), @r"
    No working copy
    [EOF]
    ");
}
