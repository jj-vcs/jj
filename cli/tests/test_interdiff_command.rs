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
    -%%%%%%% diff from: qpvuntsm d0c049cd (from parent)
    -\\\\\\\        to:  (from context)
    --foo
    -+abc
    -+++++++ rlvkpnrz b23f92c3 (from revision)
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
fn test_interdiff_allows_gaps() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a linear chain A -> B -> C, each writing file
    // then overwriting it in the subsequent commit (old pattern).
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

    // A|C has a gap (B missing), but interdiff allows it — no error
    let output = work_dir.run_jj(["interdiff", "--from", "a|c", "--to", "c"]);
    output.success();
    let output = work_dir.run_jj(["interdiff", "--from", "c", "--to", "a|c"]);
    output.success();
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

    // main: create f0
    work_dir.run_jj(["desc", "-mfoo"]).success();
    work_dir.write_file("f0", "foo\n");
    work_dir.run_jj(["bookmark", "create", "main"]).success();

    // a: create f1
    work_dir.run_jj(["new", "-ma"]).success();
    work_dir.write_file("f1", "a\n");
    work_dir.run_jj(["bookmark", "create", "a"]).success();

    // b: add f2
    work_dir.run_jj(["new", "-mb"]).success();
    work_dir.write_file("f2", "b\n");
    work_dir.run_jj(["bookmark", "create", "b"]).success();

    // c: modify f1 (inherits f2 from b)
    work_dir.run_jj(["new", "-mc"]).success();
    work_dir.write_file("f1", "c\n");
    work_dir.run_jj(["bookmark", "create", "c"]).success();

    // duplicate b on top of main, move main to b'
    work_dir
        .run_jj(["duplicate", "-r", "b", "--onto", "main"])
        .success();
    work_dir
        .run_jj(["bookmark", "set", "main", "-r", "main+ ~ a"])
        .success();

    // create a new revision on top of main which adds content to f2, and move main
    // there
    work_dir.run_jj(["new", "main", "-md"]).success();
    work_dir.write_file("f2", "b\nd\n");
    work_dir
        .run_jj(["bookmark", "set", "main", "-r", "main+"])
        .success();

    // Duplicate a and c on top of main (skipping b)
    work_dir
        .run_jj(["duplicate", "-r", "a|c", "--onto", "main"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "a2", "-r", "main+"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "c2", "-r", "main++"])
        .success();

    let output = work_dir.run_jj(["log", "-T builtin_log_oneline"]);
    insta::assert_snapshot!(output, @"
    ○  rsllmpnm test.user 2001-02-03 08:05:20 c2 2bae6fbe c
    ○  lylxulpl test.user 2001-02-03 08:05:20 a2 c70c1a5c a
    @  kmkuslsw test.user 2001-02-03 08:05:19 main a141fe7b d
    ○  znkkpsqq test.user 2001-02-03 08:05:16 91d546a0 b
    │ ○  vruxwmqv test.user 2001-02-03 08:05:15 c f3c4ef5b c
    │ ○  royxmykx test.user 2001-02-03 08:05:13 b 0f303e9f b
    │ ○  zsuskuln test.user 2001-02-03 08:05:11 a d3a2d994 a
    ├─╯
    ○  qpvuntsm test.user 2001-02-03 08:05:09 85736a58 foo
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // comparing a|c to a2::c2 should show no diff
    let output = work_dir.run_jj(["interdiff", "--from", "a|c", "--to", "a2::c2"]);
    insta::assert_snapshot!(output, @"");

    // a::c includes b (adds f2=b). the content of b should show when comparing a::c
    // and a2::c2
    let output = work_dir.run_jj(["interdiff", "--from", "a::c", "--to", "a2::c2"]);
    insta::assert_snapshot!(output, @r#"
    Modified commit description:
       1    1: <<<<<<< conflict 1 of 1
       2     : %%%%%%% diff from: base #1
            2: %%%%%%% diff from: base
       3    3: \\\\\\\        to: side #1
       4    4: +c
       5     : %%%%%%% diff from: base #2
       6     : \\\\\\\        to: side #2
       7     : +b
       8     : +++++++ side #3
            5: +++++++ side #2
       9    6: a
      10    7: >>>>>>> conflict 1 of 1 ends
    Resolved conflict in f2:
       1     : <<<<<<< conflict 1 of 1
       2     : +++++++  (from context)
       3    1: b
       4    2: d
       5     : %%%%%%% diff from: zsuskuln d3a2d994 "a" (from parent)
       6     : \\\\\\\        to: vruxwmqv f3c4ef5b "c" (from revision)
       7     : +b
       8     : >>>>>>> conflict 1 of 1 ends
    [EOF]
    "#);
}

#[test]
fn test_interdiff_to_with_gap() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // main: create f0
    work_dir.run_jj(["desc", "-mfoo"]).success();
    work_dir.write_file("f0", "foo\n");
    work_dir.run_jj(["bookmark", "create", "main"]).success();

    // a: create f1
    work_dir.run_jj(["new", "-m", "revision a"]).success();
    work_dir.write_file("f1", "a\n");
    work_dir.run_jj(["bookmark", "create", "a"]).success();

    // b: add f2
    work_dir.run_jj(["new", "-m", "revision b"]).success();
    work_dir.write_file("f2", "b\n");
    work_dir.run_jj(["bookmark", "create", "b"]).success();

    // c: modify f1
    work_dir.run_jj(["new", "-m", "revision c"]).success();
    work_dir.write_file("f1", "c\n");
    work_dir.run_jj(["bookmark", "create", "c"]).success();

    // Duplicate a::c onto main, creating a2, b2, c2 as children
    work_dir
        .run_jj(["duplicate", "-r", "a::c", "--onto", "main"])
        .success();
    // Bookmark the duplicates
    // After duplicate, the structure is root() -> a2 -> b2 -> c2
    work_dir
        .run_jj(["bookmark", "create", "a2", "-r", "latest(main+)"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "b2", "-r", "children(a2)"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "c2", "-r", "children(b2)"])
        .success();

    // Insert d2 between b2 and c2 (d2 sets "d" to f3)
    work_dir
        .run_jj(["new", "-A", "b2", "-m", "revision d"])
        .success();
    work_dir.write_file("f3", "d\n");
    work_dir.run_jj(["bookmark", "create", "d2"]).success();

    // Rebase c2 on top of d2 so that d2 is now between b2 and c2
    work_dir
        .run_jj(["rebase", "-r", "c2", "-d", "d2"])
        .success();

    let output = work_dir.run_jj(["log", "-T builtin_log_oneline"]);
    insta::assert_snapshot!(output, @"
    ○  lpnsqqnl test.user 2001-02-03 08:05:21 c2 4331391a revision c
    @  lylxulpl test.user 2001-02-03 08:05:21 d2 fdf3a6a6 revision d
    ○  uuzqqzqu test.user 2001-02-03 08:05:16 b2 476b8d40 revision b
    ○  znkkpsqq test.user 2001-02-03 08:05:16 a2 86f06054 revision a
    │ ○  vruxwmqv test.user 2001-02-03 08:05:15 c cf0a2baf revision c
    │ ○  royxmykx test.user 2001-02-03 08:05:13 b 37b3bfe7 revision b
    │ ○  zsuskuln test.user 2001-02-03 08:05:11 a d1657f43 revision a
    ├─╯
    ○  qpvuntsm test.user 2001-02-03 08:05:09 main 85736a58 foo
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["interdiff", "--from", "a::c", "--to", "a2::c2 ~ d2"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["interdiff", "--from", "a::c", "--to", "a2::c2"]);
    insta::assert_snapshot!(output, @r"
    Modified commit description:
        ...
       4    4: +revision c
       5    5: %%%%%%% diff from: base #2
       6    6: \\\\\\\        to: side #2
            7: +revision d
            8: %%%%%%% diff from: base #3
            9: \\\\\\\        to: side #3
       7   10: +revision b
       8     : +++++++ side #3
           11: +++++++ side #4
       9   12: revision a
      10   13: >>>>>>> conflict 1 of 1 ends
    Added regular file f3:
            1: d
    [EOF]
    ");
}
