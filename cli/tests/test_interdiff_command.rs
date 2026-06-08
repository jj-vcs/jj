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

#[test]
fn test_interdiff_gap_detection() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a linear chain A -> B -> C
    work_dir.write_file("file", "a\n");
    work_dir.run_jj(["new", "-mcommit-a"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.write_file("file", "b\n");
    work_dir.run_jj(["new", "-mcommit-b"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir.write_file("file", "c\n");
    work_dir.run_jj(["new", "-mcommit-c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    // Gap in --from (A|C where B is between them)
    let output = work_dir.run_jj(["interdiff", "--from", "a|c", "--to", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot diff revsets with gaps in --from.
    Hint: Revision 7772739fe4c7 would need to be in the set.
    [EOF]
    [exit status: 1]
    ");

    // Gap in --to (A|C where B is between them)
    let output = work_dir.run_jj(["interdiff", "--from", "c", "--to", "a|c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot diff revsets with gaps in --to.
    Hint: Revision 7772739fe4c7 would need to be in the set.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_interdiff_multi_rev() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create two independent siblings from root
    work_dir.write_file("file1", "base\n");
    work_dir.write_file("file2", "base\n");

    // left: modifies file1
    work_dir.run_jj(["new", "-mleft"]).success();
    work_dir.write_file("file1", "left\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "left"])
        .success();

    // right: modifies file2 (sibling of left, both children of initial)
    work_dir.run_jj(["new", "@-", "-mright"]).success();
    work_dir.write_file("file2", "right\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "right"])
        .success();

    // Multi-rev --from (left|right) single --to (right): right cancels out,
    // only left's changes shown
    let output = work_dir.run_jj(["interdiff", "--from", "left|right", "--to", "right"]);
    insta::assert_snapshot!(output, @r"
    Modified commit description:
       1     : <<<<<<< conflict 1 of 1
       2     : +++++++ side #1
       3    1: right
       4     : %%%%%%% diff from: base
       5     : \\\\\\\        to: side #2
       6     : +left
       7     : >>>>>>> conflict 1 of 1 ends
    Modified regular file file1:
       1     : left
            1: base
    [EOF]
    ");

    // Multi-rev --to: single --from (left) vs multi --to (left|right):
    // left cancels out, only right's changes shown
    let output = work_dir.run_jj(["interdiff", "--from", "left", "--to", "left|right"]);
    insta::assert_snapshot!(output, @r"
    Modified commit description:
       1     : left
            1: <<<<<<< conflict 1 of 1
            2: +++++++ side #1
            3: right
            4: %%%%%%% diff from: base
            5: \\\\\\\        to: side #2
            6: +left
            7: >>>>>>> conflict 1 of 1 ends
    Modified regular file file2:
       1     : base
            1: right
    [EOF]
    ");
}

#[test]
fn test_interdiff_range_duplicate() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up: a -> b -> c (chain modifying f1)
    //         a -> d -> b2 -> c2 (d adds f2, then b2/c2 duplicate b/c on d)
    // b::c and b2::c2 should have same changes => empty interdiff

    // a: create f1 with "base"
    work_dir.run_jj(["new", "-ma"]).success();
    work_dir.write_file("f1", "base\n");
    work_dir.run_jj(["bookmark", "create", "a"]).success();

    // b: modify f1 to add "b"
    work_dir.run_jj(["new", "-mb"]).success();
    work_dir.write_file("f1", "base\nb\n");
    work_dir.run_jj(["bookmark", "create", "b"]).success();

    // c: modify f1 to add "c"
    work_dir.run_jj(["new", "-mc"]).success();
    work_dir.write_file("f1", "base\nb\nc\n");
    work_dir.run_jj(["bookmark", "create", "c"]).success();

    // d: add f2 from a (create separate branch from a)
    work_dir.run_jj(["new", "a", "-md"]).success();
    work_dir.write_file("f2", "d\n");
    work_dir.run_jj(["bookmark", "create", "d"]).success();

    // Duplicate b::c on top of d, creating b2 and c2
    work_dir
        .run_jj(["duplicate", "-r", "b::c", "--onto", "d"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "b2", "-r", "d+"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "c2", "-r", "d++"])
        .success();

    // interdiff between b::c and b2::c2: identical changes => empty
    let output = work_dir.run_jj(["interdiff", "--from", "b::c", "--to", "b2::c2"]);
    insta::assert_snapshot!(output, @"");
}
