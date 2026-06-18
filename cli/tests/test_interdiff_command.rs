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

#[test]
fn test_interdiff_basic() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["new", "-madd file2 left"]).success();
    work_dir.write_file("file2", "foo\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file3", "foo\n");
    work_dir.run_jj(["new", "-madd file2 right"]).success();
    work_dir.write_file("file2", "foo\nbar\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();
    let setup_opid = work_dir.current_operation_id();

    // implicit --to
    let output = work_dir.run_jj(["interdiff", "--from", "left"]);
    insta::assert_snapshot!(output, @"
    Modified commit description:
       1     : add file2 left
            1: add file2 right
    Modified regular file file2:
       1    1: foo
            2: bar
    [EOF]
    ");

    // explicit --to
    work_dir.run_jj(["new", "@-"]).success();
    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "right"]);
    insta::assert_snapshot!(output, @"
    Modified commit description:
       1     : add file2 left
            1: add file2 right
    Modified regular file file2:
       1    1: foo
            2: bar
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // formats specifiers
    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "right", "-s"]);
    insta::assert_snapshot!(output, @"
    M file2
    [EOF]
    ");

    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "right", "--git"]);
    insta::assert_snapshot!(output, @"
    diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
    --- JJ-COMMIT-DESCRIPTION
    +++ JJ-COMMIT-DESCRIPTION
    @@ -1,1 +1,1 @@
    -add file2 left
    +add file2 right
    diff --git a/file2 b/file2
    index 257cc5642c..3bd1f0e297 100644
    --- a/file2
    +++ b/file2
    @@ -1,1 +1,2 @@
     foo
    +bar
    [EOF]
    ");
}

#[test]
fn test_interdiff_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "foo\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "bar\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "foo\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "baz\n");
    work_dir.write_file("file2", "baz\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();

    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "right", "file1"]);
    insta::assert_snapshot!(output, @"
    Modified regular file file1:
       1     : bar
            1: baz
    [EOF]
    ");

    let output = work_dir.run_jj([
        "interdiff",
        "--from",
        "left",
        "--to",
        "right",
        "file1",
        "file2",
        "nonexistent",
    ]);
    insta::assert_snapshot!(output, @"
    Modified regular file file1:
       1     : bar
            1: baz
    Modified regular file file2:
       1     : bar
            1: baz
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    [EOF]
    ");

    // Running interdiff on commits with deleted files should not show a warning.
    work_dir.run_jj(["edit", "right"]).success();
    work_dir.remove_file("file1");
    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj([
        "interdiff",
        "--from",
        "left",
        "--to",
        "right",
        "file1",
        "file2",
    ]);
    insta::assert_snapshot!(output, @"
    Removed regular file file1:
       1     : bar
    Modified regular file file2:
       1     : bar
            1: baz
    [EOF]
    ");
}

#[test]
fn test_interdiff_revset_ranges() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // An original range of two commits on top of "base".
    work_dir.run_jj(["describe", "-mbase"]).success();
    work_dir.write_file("context", "old\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();
    work_dir.run_jj(["new", "-madd foo"]).success();
    work_dir.write_file("foo", "1\n2\n");
    work_dir.run_jj(["new", "-madd bar"]).success();
    work_dir.write_file("bar", "x\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "old"])
        .success();

    // The same changes, rebased onto "base2" and modified.
    work_dir.run_jj(["new", "root()", "-mbase2"]).success();
    work_dir.write_file("context", "new\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base2"])
        .success();
    work_dir.run_jj(["new", "-madd foo"]).success();
    work_dir.write_file("foo", "1\n2\n3\n");
    work_dir.run_jj(["new", "-madd bar"]).success();
    work_dir.write_file("bar", "x\ny\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "new"])
        .success();

    // Both ranges are treated as if they were squashed into a single
    // revision. The differing "context" file is not part of either range, so
    // it shouldn't show up in the diff.
    let output = work_dir.run_jj(["interdiff", "--from", "base..old", "--to", "base2..new"]);
    insta::assert_snapshot!(output, @r"
    Modified regular file bar:
       1    1: x
            2: y
    Modified regular file foo:
       1    1: 1
       2    2: 2
            3: 3
    [EOF]
    ");

    let output = work_dir.run_jj([
        "interdiff",
        "--from",
        "base..old",
        "--to",
        "base2..new",
        "--git",
    ]);
    insta::assert_snapshot!(output, @r"
    diff --git a/bar b/bar
    index 587be6b4c3..b77b4eb1d9 100644
    --- a/bar
    +++ b/bar
    @@ -1,1 +1,2 @@
     x
    +y
    diff --git a/foo b/foo
    index 1191247b6d..01e79c32a8 100644
    --- a/foo
    +++ b/foo
    @@ -1,2 +1,3 @@
     1
     2
    +3
    [EOF]
    ");

    // A single revision can be compared to a range.
    let output = work_dir.run_jj(["interdiff", "--from", "old", "--to", "base2..new", "bar"]);
    insta::assert_snapshot!(output, @r"
    Modified commit description:
       1     : add bar
            1: <<<<<<< conflict 1 of 1
            2: %%%%%%% diff from: base
            3: \\\\\\\        to: side #1
            4: +add bar
            5: +++++++ side #2
            6: add foo
            7: >>>>>>> conflict 1 of 1 ends
    Modified regular file bar:
       1    1: x
            2: y
    [EOF]
    ");
}

#[test]
fn test_interdiff_revset_range_with_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // A range containing a merge commit: base -> (side a, side b) -> merge.
    work_dir.run_jj(["describe", "-mbase"]).success();
    work_dir.write_file("context", "old\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();
    work_dir.run_jj(["new", "-mside a"]).success();
    work_dir.write_file("a", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "side-a"])
        .success();
    work_dir.run_jj(["new", "base", "-mside b"]).success();
    work_dir.write_file("b", "b\n");
    work_dir.run_jj(["new", "side-a", "@", "-mmerge"]).success();
    work_dir.write_file("m", "m\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "old"])
        .success();

    // The same changes squashed into a single commit on top of "base2", with
    // file "b" modified.
    work_dir.run_jj(["new", "root()", "-mbase2"]).success();
    work_dir.write_file("context", "new\n");
    work_dir.run_jj(["new", "-meverything"]).success();
    work_dir.write_file("a", "a\n");
    work_dir.write_file("b", "B\n");
    work_dir.write_file("m", "m\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "new"])
        .success();

    // The merge commit in the range shouldn't cause spurious diffs or
    // conflicts; only the modification to "b" is shown.
    let output = work_dir.run_jj(["interdiff", "--from", "base..old", "--to", "new", "-s"]);
    insta::assert_snapshot!(output, @r"
    M b
    [EOF]
    ");

    let output = work_dir.run_jj([
        "interdiff",
        "--from",
        "base..old",
        "--to",
        "new",
        "--git",
        "b",
    ]);
    insta::assert_snapshot!(output, @r"
    diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
    --- JJ-COMMIT-DESCRIPTION
    +++ JJ-COMMIT-DESCRIPTION
    @@ -1,10 +1,1 @@
    -<<<<<<< conflict 1 of 1
    -%%%%%%% diff from: base #1
    -\\\\\\\        to: side #1
    -+merge
    -%%%%%%% diff from: base #2
    -\\\\\\\        to: side #2
    -+side b
    -+++++++ side #3
    -side a
    ->>>>>>> conflict 1 of 1 ends
    +everything
    diff --git a/b b/b
    index 6178079822..223b7836fb 100644
    --- a/b
    +++ b/b
    @@ -1,1 +1,1 @@
    -b
    +B
    [EOF]
    ");
}

#[test]
fn test_interdiff_revset_with_gap() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-mfirst"]).success();
    work_dir.write_file("file", "1\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "first"])
        .success();
    work_dir.run_jj(["new", "-msecond"]).success();
    work_dir.write_file("file", "1\n2\n");
    work_dir.run_jj(["new", "-mthird"]).success();
    work_dir.write_file("file", "1\n2\n3\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "third"])
        .success();

    let output = work_dir.run_jj(["interdiff", "--from", "first | third", "--to", "third"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot diff revsets with gaps in.
    Hint: Revision de8260255ca6 would need to be in the set.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_interdiff_empty_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "1\n");

    let output = work_dir.run_jj(["interdiff", "--from", "none()", "--to", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `none()` didn't resolve to any revisions
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["interdiff", "--from", "@", "--to", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Revset `none()` didn't resolve to any revisions
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_interdiff_conflicting() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "foo\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "bar\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file", "abc\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "def\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();

    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "right", "--git"]);
    insta::assert_snapshot!(output, @r"
    diff --git a/file b/file
    index 0000000000..24c5735c3e 100644
    --- a/file
    +++ b/file
    @@ -1,8 +1,1 @@
    -<<<<<<< conflict 1 of 1
    -%%%%%%% diff from: qpvuntsm d0c049cd (original parents)
    -\\\\\\\        to: zsuskuln 0b2c304e (new parents)
    --foo
    -+abc
    -+++++++ rlvkpnrz b23f92c3 (original revision)
    -bar
    ->>>>>>> conflict 1 of 1 ends
    +def
    [EOF]
    ");

    let output = work_dir.run_jj([
        "interdiff",
        "--config=diff.color-words.conflict=pair",
        "--color=always",
        "--from=left",
        "--to=right",
    ]);
    insta::assert_snapshot!(output, @"
    [38;5;3mResolved conflict in file:[39m
    [38;5;6m<<<<<<< Resolved conflict[39m
    [38;5;6m+++++++ left side #1 to right side #1[39m
    [38;5;1m   1[39m [38;5;2m   1[39m: [4m[38;5;1mabc[38;5;2mdef[24m[39m
    [38;5;6m------- left base #1 to right side #1[39m
    [38;5;2m   1[39m [38;5;1m   1[39m: [4m[38;5;2mfoo[38;5;1mdef[24m[39m
    [38;5;6m+++++++ left side #2 to right side #1[39m
    [38;5;1m   1[39m [38;5;2m   1[39m: [4m[38;5;1mbar[38;5;2mdef[24m[39m
    [38;5;6m>>>>>>> Conflict ends[39m
    [EOF]
    ");
}
