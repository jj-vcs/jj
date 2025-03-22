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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_absorb_simple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m0"]).success();
    work_dir.write_file("file1", "");

    work_dir.run_jj(["new", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n");

    work_dir.run_jj(["new", "-m2"]).success();
    work_dir.write_file("file1", "1a\n1b\n2a\n2b\n");

    // Empty commit
    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Insert first and last lines
    work_dir.write_file("file1", "1X\n1a\n1b\n2a\n2b\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 2 revisions:
      zsuskuln 3027ca7a 2
      kkmpptxz d0f1e8dd 1
    Working copy now at: yqosqzyt 277bed24 (empty) (no description set)
    Parent commit      : zsuskuln 3027ca7a 2
    [EOF]
    ");

    // Modify middle line in hunk
    work_dir.write_file("file1", "1X\n1A\n1b\n2a\n2b\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      kkmpptxz d366d92c 1
    Rebased 1 descendant commits.
    Working copy now at: vruxwmqv 32eb72fe (empty) (no description set)
    Parent commit      : zsuskuln 5bf0bc06 2
    [EOF]
    ");

    // Remove middle line from hunk
    work_dir.write_file("file1", "1X\n1A\n1b\n2a\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      zsuskuln 6e2c4777 2
    Working copy now at: yostqsxw 4a48490c (empty) (no description set)
    Parent commit      : zsuskuln 6e2c4777 2
    [EOF]
    ");

    // Insert ambiguous line in between
    work_dir.write_file("file1", "1X\n1A\n1b\nY\n2a\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  yostqsxw 80965bcc (no description set)
    │  diff --git a/file1 b/file1
    │  index 8653ca354d..88eb438902 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,5 +1,6 @@
    │   1X
    │   1A
    │   1b
    │  +Y
    │   2a
    │   2Z
    ○  zsuskuln 6e2c4777 2
    │  diff --git a/file1 b/file1
    │  index ed237b5112..8653ca354d 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,3 +1,5 @@
    │   1X
    │   1A
    │   1b
    │  +2a
    │  +2Z
    ○  kkmpptxz d366d92c 1
    │  diff --git a/file1 b/file1
    │  index e69de29bb2..ed237b5112 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -0,0 +1,3 @@
    │  +1X
    │  +1A
    │  +1b
    ○  qpvuntsm 1a4edb91 0
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..e69de29bb2
    [EOF]
    ");
    insta::assert_snapshot!(get_evolog(&work_dir, "description(1)"), @r"
    ○    kkmpptxz d366d92c 1
    ├─╮
    │ ○  yqosqzyt hidden c506fbc7 (no description set)
    │ ○  yqosqzyt hidden 277bed24 (empty) (no description set)
    ○    kkmpptxz hidden d0f1e8dd 1
    ├─╮
    │ ○  mzvwutvl hidden 8935ee61 (no description set)
    │ ○  mzvwutvl hidden 2bc3d2ce (empty) (no description set)
    ○  kkmpptxz hidden ee76d790 1
    ○  kkmpptxz hidden 677e62d5 (empty) 1
    [EOF]
    ");
    insta::assert_snapshot!(get_evolog(&work_dir, "description(2)"), @r"
    ○    zsuskuln 6e2c4777 2
    ├─╮
    │ ○  vruxwmqv hidden 7b1da5cd (no description set)
    │ ○  vruxwmqv hidden 32eb72fe (empty) (no description set)
    ○  zsuskuln hidden 5bf0bc06 2
    ○    zsuskuln hidden 3027ca7a 2
    ├─╮
    │ ○  mzvwutvl hidden 8935ee61 (no description set)
    │ ○  mzvwutvl hidden 2bc3d2ce (empty) (no description set)
    ○  zsuskuln hidden cca09b4d 2
    ○  zsuskuln hidden 7b092471 (empty) 2
    [EOF]
    ");
}

#[test]
fn test_absorb_replace_single_line_hunk() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n");

    work_dir.run_jj(["new", "-m2"]).success();
    work_dir.write_file("file1", "2a\n1a\n2b\n");

    // Replace single-line hunk, which produces a conflict right now. If our
    // merge logic were based on interleaved delta, the hunk would be applied
    // cleanly.
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "2a\n1A\n2b\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      qpvuntsm 7e885236 (conflict) 1
    Rebased 1 descendant commits.
    Working copy now at: mzvwutvl e9c3b95b (empty) (no description set)
    Parent commit      : kkmpptxz 7c36845c 2
    New conflicts appeared in 1 commits:
      qpvuntsm 7e885236 (conflict) 1
    Hint: To resolve the conflicts, start by updating to it:
      jj new qpvuntsm
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  mzvwutvl e9c3b95b (empty) (no description set)
    ○  kkmpptxz 7c36845c 2
    │  diff --git a/file1 b/file1
    │  index 0000000000..2f87e8e465 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,10 +1,3 @@
    │  -<<<<<<< Conflict 1 of 1
    │  -%%%%%%% Changes from base to side #1
    │  --2a
    │  - 1a
    │  --2b
    │  -+++++++ Contents of side #2
    │   2a
    │   1A
    │   2b
    │  ->>>>>>> Conflict 1 of 1 ends
    ×  qpvuntsm 7e885236 (conflict) 1
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..0000000000
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,10 @@
       +<<<<<<< Conflict 1 of 1
       +%%%%%%% Changes from base to side #1
       +-2a
       + 1a
       +-2b
       ++++++++ Contents of side #2
       +2a
       +1A
       +2b
       +>>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}

#[test]
fn test_absorb_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m0"]).success();
    work_dir.write_file("file1", "0a\n");

    work_dir.run_jj(["new", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n0a\n");

    work_dir.run_jj(["new", "-m2", "description(0)"]).success();
    work_dir.write_file("file1", "0a\n2a\n2b\n");

    let output = work_dir.run_jj(["new", "-m3", "description(1)", "description(2)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: mzvwutvl 08898161 (empty) 3
    Parent commit      : kkmpptxz 7e9df299 1
    Parent commit      : zsuskuln baf056cf 2
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // Modify first and last lines, absorb from merge
    work_dir.write_file("file1", "1A\n1b\n0a\n2a\n2B\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 2 revisions:
      zsuskuln 71d1ee56 2
      kkmpptxz 4d379399 1
    Rebased 1 descendant commits.
    Working copy now at: mzvwutvl 9db19b54 (empty) 3
    Parent commit      : kkmpptxz 4d379399 1
    Parent commit      : zsuskuln 71d1ee56 2
    [EOF]
    ");

    // Add hunk to merge revision
    work_dir.write_file("file2", "3a\n");

    // Absorb into merge
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2", "3A\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      mzvwutvl e93c0210 3
    Working copy now at: vruxwmqv 1b10dfa4 (empty) (no description set)
    Parent commit      : mzvwutvl e93c0210 3
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  vruxwmqv 1b10dfa4 (empty) (no description set)
    ○    mzvwutvl e93c0210 3
    ├─╮  diff --git a/file2 b/file2
    │ │  new file mode 100644
    │ │  index 0000000000..44442d2d7b
    │ │  --- /dev/null
    │ │  +++ b/file2
    │ │  @@ -0,0 +1,1 @@
    │ │  +3A
    │ ○  zsuskuln 71d1ee56 2
    │ │  diff --git a/file1 b/file1
    │ │  index eb6e8821f1..4907935b9f 100644
    │ │  --- a/file1
    │ │  +++ b/file1
    │ │  @@ -1,1 +1,3 @@
    │ │   0a
    │ │  +2a
    │ │  +2B
    ○ │  kkmpptxz 4d379399 1
    ├─╯  diff --git a/file1 b/file1
    │    index eb6e8821f1..902dd8ef13 100644
    │    --- a/file1
    │    +++ b/file1
    │    @@ -1,1 +1,3 @@
    │    +1A
    │    +1b
    │     0a
    ○  qpvuntsm 3777b700 0
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..eb6e8821f1
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,1 @@
       +0a
    [EOF]
    ");
}

#[test]
fn test_absorb_discardable_merge_with_descendant() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m0"]).success();
    work_dir.write_file("file1", "0a\n");

    work_dir.run_jj(["new", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n0a\n");

    work_dir.run_jj(["new", "-m2", "description(0)"]).success();
    work_dir.write_file("file1", "0a\n2a\n2b\n");

    let output = work_dir.run_jj(["new", "description(1)", "description(2)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: mzvwutvl f59b2364 (empty) (no description set)
    Parent commit      : kkmpptxz 7e9df299 1
    Parent commit      : zsuskuln baf056cf 2
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // Modify first and last lines in the merge commit
    work_dir.write_file("file1", "1A\n1b\n0a\n2a\n2B\n");
    // Add new commit on top
    work_dir.run_jj(["new", "-m3"]).success();
    work_dir.write_file("file2", "3a\n");
    // Then absorb the merge commit
    let output = work_dir.run_jj(["absorb", "--from=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 2 revisions:
      zsuskuln 02668cf6 2
      kkmpptxz fcabe394 1
    Rebased 1 descendant commits.
    Working copy now at: royxmykx f04f1247 3
    Parent commit      : kkmpptxz fcabe394 1
    Parent commit      : zsuskuln 02668cf6 2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @    royxmykx f04f1247 3
    ├─╮  diff --git a/file2 b/file2
    │ │  new file mode 100644
    │ │  index 0000000000..31cd755d20
    │ │  --- /dev/null
    │ │  +++ b/file2
    │ │  @@ -0,0 +1,1 @@
    │ │  +3a
    │ ○  zsuskuln 02668cf6 2
    │ │  diff --git a/file1 b/file1
    │ │  index eb6e8821f1..4907935b9f 100644
    │ │  --- a/file1
    │ │  +++ b/file1
    │ │  @@ -1,1 +1,3 @@
    │ │   0a
    │ │  +2a
    │ │  +2B
    ○ │  kkmpptxz fcabe394 1
    ├─╯  diff --git a/file1 b/file1
    │    index eb6e8821f1..902dd8ef13 100644
    │    --- a/file1
    │    +++ b/file1
    │    @@ -1,1 +1,3 @@
    │    +1A
    │    +1b
    │     0a
    ○  qpvuntsm 3777b700 0
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..eb6e8821f1
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,1 @@
       +0a
    [EOF]
    ");
}

#[test]
fn test_absorb_conflict() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n");

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file1", "2a\n2b\n");
    let output = work_dir.run_jj(["rebase", "-r@", "-ddescription(1)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits onto destination
    Working copy now at: kkmpptxz 74405a07 (conflict) (no description set)
    Parent commit      : qpvuntsm 3619e4e5 1
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file1    2-sided conflict
    New conflicts appeared in 1 commits:
      kkmpptxz 74405a07 (conflict) (no description set)
    Hint: To resolve the conflicts, start by updating to it:
      jj new kkmpptxz
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    let conflict_content = work_dir.read_file("file1");
    insta::assert_snapshot!(conflict_content, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    +1a
    +1b
    +++++++ Contents of side #2
    2a
    2b
    >>>>>>> Conflict 1 of 1 ends
    ");

    // Cannot absorb from conflict
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Skipping file1: Is a conflict
    Nothing changed.
    [EOF]
    ");

    // Cannot absorb from resolved conflict
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "1A\n1b\n2a\n2B\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Skipping file1: Is a conflict
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_absorb_deleted_file() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n");

    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");

    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Skipping file1: Deleted file
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_absorb_file_mode() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n");
    work_dir.run_jj(["file", "chmod", "x", "file1"]).success();

    // Modify content and mode
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "1A\n");
    work_dir.run_jj(["file", "chmod", "n", "file1"]).success();

    // Mode change shouldn't be absorbed
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      qpvuntsm 991365da 1
    Rebased 1 descendant commits.
    Working copy now at: zsuskuln 77de368e (no description set)
    Parent commit      : qpvuntsm 991365da 1
    Remaining changes:
    M file1
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  zsuskuln 77de368e (no description set)
    │  diff --git a/file1 b/file1
    │  old mode 100755
    │  new mode 100644
    ○  qpvuntsm 991365da 1
    │  diff --git a/file1 b/file1
    ~  new file mode 100755
       index 0000000000..268de3f3ec
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,1 @@
       +1A
    [EOF]
    ");
}

#[test]
fn test_absorb_from_into() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n1c\n");

    work_dir.run_jj(["new", "-m2"]).success();
    work_dir.write_file("file1", "1a\n2a\n1b\n1c\n2b\n");

    // Line "X" and "Z" have unambiguous adjacent line within the destinations
    // range. Line "Y" doesn't have such line.
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "1a\nX\n2a\n1b\nY\n1c\n2b\nZ\n");
    let output = work_dir.run_jj(["absorb", "--into=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      kkmpptxz 91df4543 2
    Rebased 1 descendant commits.
    Working copy now at: zsuskuln d5424357 (no description set)
    Parent commit      : kkmpptxz 91df4543 2
    Remaining changes:
    M file1
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "@-::"), @r"
    @  zsuskuln d5424357 (no description set)
    │  diff --git a/file1 b/file1
    │  index faf62af049..c2d0b12547 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -2,6 +2,7 @@
    │   X
    │   2a
    │   1b
    │  +Y
    │   1c
    │   2b
    │   Z
    ○  kkmpptxz 91df4543 2
    │  diff --git a/file1 b/file1
    ~  index 352e9b3794..faf62af049 100644
       --- a/file1
       +++ b/file1
       @@ -1,3 +1,7 @@
        1a
       +X
       +2a
        1b
        1c
       +2b
       +Z
    [EOF]
    ");

    // Absorb all lines from the working-copy parent. An empty commit won't be
    // discarded because "absorb" isn't a command to squash commit descriptions.
    let output = work_dir.run_jj(["absorb", "--from=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      rlvkpnrz 3a5fd02e 1
    Rebased 2 descendant commits.
    Working copy now at: zsuskuln 53ce490b (no description set)
    Parent commit      : kkmpptxz c94cd773 (empty) 2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  zsuskuln 53ce490b (no description set)
    │  diff --git a/file1 b/file1
    │  index faf62af049..c2d0b12547 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -2,6 +2,7 @@
    │   X
    │   2a
    │   1b
    │  +Y
    │   1c
    │   2b
    │   Z
    ○  kkmpptxz c94cd773 (empty) 2
    ○  rlvkpnrz 3a5fd02e 1
    │  diff --git a/file1 b/file1
    │  new file mode 100644
    │  index 0000000000..faf62af049
    │  --- /dev/null
    │  +++ b/file1
    │  @@ -0,0 +1,7 @@
    │  +1a
    │  +X
    │  +2a
    │  +1b
    │  +1c
    │  +2b
    │  +Z
    ○  qpvuntsm 230dd059 (empty) (no description set)
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_absorb_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n");
    work_dir.write_file("file2", "1a\n");

    // Modify both files
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "1A\n");
    work_dir.write_file("file2", "1A\n");

    let output = work_dir.run_jj(["absorb", "unknown"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    let output = work_dir.run_jj(["absorb", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      qpvuntsm ae044adb 1
    Rebased 1 descendant commits.
    Working copy now at: kkmpptxz c6f31836 (no description set)
    Parent commit      : qpvuntsm ae044adb 1
    Remaining changes:
    M file2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  kkmpptxz c6f31836 (no description set)
    │  diff --git a/file2 b/file2
    │  index a8994dc188..268de3f3ec 100644
    │  --- a/file2
    │  +++ b/file2
    │  @@ -1,1 +1,1 @@
    │  -1a
    │  +1A
    ○  qpvuntsm ae044adb 1
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..268de3f3ec
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,1 @@
       +1A
       diff --git a/file2 b/file2
       new file mode 100644
       index 0000000000..a8994dc188
       --- /dev/null
       +++ b/file2
       @@ -0,0 +1,1 @@
       +1a
    [EOF]
    ");
}

#[test]
fn test_absorb_immutable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config("revset-aliases.'immutable_heads()' = 'present(main)'");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n");

    work_dir.run_jj(["new", "-m2"]).success();
    work_dir
        .run_jj(["bookmark", "set", "-r@-", "main"])
        .success();
    work_dir.write_file("file1", "1a\n1b\n2a\n2b\n");

    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "1A\n1b\n2a\n2B\n");

    // Immutable revisions are excluded by default
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      kkmpptxz d80e3c2a 2
    Rebased 1 descendant commits.
    Working copy now at: mzvwutvl 3021153d (no description set)
    Parent commit      : kkmpptxz d80e3c2a 2
    Remaining changes:
    M file1
    [EOF]
    ");

    // Immutable revisions shouldn't be rewritten
    let output = work_dir.run_jj(["absorb", "--into=all()"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit 3619e4e52fce is immutable
    Hint: Could not modify commit: qpvuntsm 3619e4e5 main | 1
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);

    insta::assert_snapshot!(get_diffs(&work_dir, ".."), @r"
    @  mzvwutvl 3021153d (no description set)
    │  diff --git a/file1 b/file1
    │  index 75e4047831..428796ca20 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,4 +1,4 @@
    │  -1a
    │  +1A
    │   1b
    │   2a
    │   2B
    ○  kkmpptxz d80e3c2a 2
    │  diff --git a/file1 b/file1
    │  index 8c5268f893..75e4047831 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,2 +1,4 @@
    │   1a
    │   1b
    │  +2a
    │  +2B
    ◆  qpvuntsm 3619e4e5 1
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..8c5268f893
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,2 @@
       +1a
       +1b
    [EOF]
    ");
}

#[must_use]
fn get_diffs(work_dir: &TestWorkDir, revision: &str) -> CommandOutput {
    let template = r#"format_commit_summary_with_refs(self, "") ++ "\n""#;
    work_dir.run_jj(["log", "-r", revision, "-T", template, "--git"])
}

#[must_use]
fn get_evolog(work_dir: &TestWorkDir, revision: &str) -> CommandOutput {
    let template = r#"format_commit_summary_with_refs(self, "") ++ "\n""#;
    work_dir.run_jj(["evolog", "-r", revision, "-T", template])
}
