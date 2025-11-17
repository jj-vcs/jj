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
      zsuskuln 95568809 2
      kkmpptxz bd7d4016 1
    Working copy  (@) now at: yqosqzyt 977269ac (empty) (no description set)
    Parent commit (@-)      : zsuskuln 95568809 2
    [EOF]
    ");

    // Modify middle line in hunk
    work_dir.write_file("file1", "1X\n1A\n1b\n2a\n2b\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      kkmpptxz 5810eb0f 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: vruxwmqv 48c7d8fa (empty) (no description set)
    Parent commit (@-)      : zsuskuln 8edd60a2 2
    [EOF]
    ");

    // Remove middle line from hunk
    work_dir.write_file("file1", "1X\n1A\n1b\n2a\n2Z\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      zsuskuln dd109863 2
    Working copy  (@) now at: yostqsxw 7482f74b (empty) (no description set)
    Parent commit (@-)      : zsuskuln dd109863 2
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
    @  yostqsxw bde51bc9 (no description set)
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
    ○  zsuskuln dd109863 2
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
    ○  kkmpptxz 5810eb0f 1
    │  diff --git a/file1 b/file1
    │  index e69de29bb2..ed237b5112 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -0,0 +1,3 @@
    │  +1X
    │  +1A
    │  +1b
    ○  qpvuntsm 6a446874 0
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..e69de29bb2
    [EOF]
    ");
    insta::assert_snapshot!(get_evolog(&work_dir, "subject(1)"), @r"
    ○    kkmpptxz 5810eb0f 1
    ├─╮
    │ ○  yqosqzyt hidden 39b42898 (no description set)
    │ ○  yqosqzyt hidden 977269ac (empty) (no description set)
    ○    kkmpptxz hidden bd7d4016 1
    ├─╮
    │ ○  mzvwutvl hidden 0b307741 (no description set)
    │ ○  mzvwutvl hidden f2709b4e (empty) (no description set)
    ○  kkmpptxz hidden 1553c5e8 1
    ○  kkmpptxz hidden eb943711 (empty) 1
    [EOF]
    ");
    insta::assert_snapshot!(get_evolog(&work_dir, "subject(2)"), @r"
    ○    zsuskuln dd109863 2
    ├─╮
    │ ○  vruxwmqv hidden 761492a8 (no description set)
    │ ○  vruxwmqv hidden 48c7d8fa (empty) (no description set)
    ○  zsuskuln hidden 8edd60a2 2
    ○    zsuskuln hidden 95568809 2
    ├─╮
    │ ○  mzvwutvl hidden 0b307741 (no description set)
    │ ○  mzvwutvl hidden f2709b4e (empty) (no description set)
    ○  zsuskuln hidden 36fad385 2
    ○  zsuskuln hidden 561fbce9 (empty) 2
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
      qpvuntsm 19034586 (conflict) 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl f9c426f2 (empty) (no description set)
    Parent commit (@-)      : kkmpptxz a5f84679 2
    New conflicts appeared in 1 commits:
      qpvuntsm 19034586 (conflict) 1
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new qpvuntsm
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  mzvwutvl f9c426f2 (empty) (no description set)
    ○  kkmpptxz a5f84679 2
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
    ×  qpvuntsm 19034586 (conflict) 1
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

    work_dir.run_jj(["new", "-m2", "subject(0)"]).success();
    work_dir.write_file("file1", "0a\n2a\n2b\n");

    let output = work_dir.run_jj(["new", "-m3", "subject(1)", "subject(2)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 42875bf7 (empty) 3
    Parent commit (@-)      : kkmpptxz 9c66f62f 1
    Parent commit (@-)      : zsuskuln 6a3dcbcf 2
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");

    // Modify first and last lines, absorb from merge
    work_dir.write_file("file1", "1A\n1b\n0a\n2a\n2B\n");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 2 revisions:
      zsuskuln a6fde7ea 2
      kkmpptxz 00ecc958 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl 30499858 (empty) 3
    Parent commit (@-)      : kkmpptxz 00ecc958 1
    Parent commit (@-)      : zsuskuln a6fde7ea 2
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
      mzvwutvl faf778a4 3
    Working copy  (@) now at: vruxwmqv cec519a1 (empty) (no description set)
    Parent commit (@-)      : mzvwutvl faf778a4 3
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  vruxwmqv cec519a1 (empty) (no description set)
    ○    mzvwutvl faf778a4 3
    ├─╮  diff --git a/file2 b/file2
    │ │  new file mode 100644
    │ │  index 0000000000..44442d2d7b
    │ │  --- /dev/null
    │ │  +++ b/file2
    │ │  @@ -0,0 +1,1 @@
    │ │  +3A
    │ ○  zsuskuln a6fde7ea 2
    │ │  diff --git a/file1 b/file1
    │ │  index eb6e8821f1..4907935b9f 100644
    │ │  --- a/file1
    │ │  +++ b/file1
    │ │  @@ -1,1 +1,3 @@
    │ │   0a
    │ │  +2a
    │ │  +2B
    ○ │  kkmpptxz 00ecc958 1
    ├─╯  diff --git a/file1 b/file1
    │    index eb6e8821f1..902dd8ef13 100644
    │    --- a/file1
    │    +++ b/file1
    │    @@ -1,1 +1,3 @@
    │    +1A
    │    +1b
    │     0a
    ○  qpvuntsm d4f07be5 0
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

    work_dir.run_jj(["new", "-m2", "subject(0)"]).success();
    work_dir.write_file("file1", "0a\n2a\n2b\n");

    let output = work_dir.run_jj(["new", "subject(1)", "subject(2)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl ad00b91a (empty) (no description set)
    Parent commit (@-)      : kkmpptxz 9c66f62f 1
    Parent commit (@-)      : zsuskuln 6a3dcbcf 2
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
      zsuskuln a6cd8e87 2
      kkmpptxz 98b7d214 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: royxmykx df946e9b 3
    Parent commit (@-)      : kkmpptxz 98b7d214 1
    Parent commit (@-)      : zsuskuln a6cd8e87 2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @    royxmykx df946e9b 3
    ├─╮  diff --git a/file2 b/file2
    │ │  new file mode 100644
    │ │  index 0000000000..31cd755d20
    │ │  --- /dev/null
    │ │  +++ b/file2
    │ │  @@ -0,0 +1,1 @@
    │ │  +3a
    │ ○  zsuskuln a6cd8e87 2
    │ │  diff --git a/file1 b/file1
    │ │  index eb6e8821f1..4907935b9f 100644
    │ │  --- a/file1
    │ │  +++ b/file1
    │ │  @@ -1,1 +1,3 @@
    │ │   0a
    │ │  +2a
    │ │  +2B
    ○ │  kkmpptxz 98b7d214 1
    ├─╯  diff --git a/file1 b/file1
    │    index eb6e8821f1..902dd8ef13 100644
    │    --- a/file1
    │    +++ b/file1
    │    @@ -1,1 +1,3 @@
    │    +1A
    │    +1b
    │     0a
    ○  qpvuntsm d4f07be5 0
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
    let output = work_dir.run_jj(["rebase", "-r@", "-dsubject(1)"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 commits to destination
    Working copy  (@) now at: kkmpptxz 01e6cd99 (conflict) (no description set)
    Parent commit (@-)      : qpvuntsm e35bcaff 1
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file1    2-sided conflict
    New conflicts appeared in 1 commits:
      kkmpptxz 01e6cd99 (conflict) (no description set)
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new kkmpptxz
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
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
    work_dir.write_file("file2", "1a\n");
    work_dir.write_file("file3", "");

    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", ""); // emptied
    work_dir.remove_file("file3"); // no content change

    // Since the destinations are chosen based on content diffs, file3 cannot be
    // absorbed.
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      qpvuntsm 38af7fd3 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: kkmpptxz efd883f6 (no description set)
    Parent commit (@-)      : qpvuntsm 38af7fd3 1
    Remaining changes:
    D file3
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  kkmpptxz efd883f6 (no description set)
    │  diff --git a/file3 b/file3
    │  deleted file mode 100644
    │  index e69de29bb2..0000000000
    ○  qpvuntsm 38af7fd3 1
    │  diff --git a/file2 b/file2
    ~  new file mode 100644
       index 0000000000..e69de29bb2
       diff --git a/file3 b/file3
       new file mode 100644
       index 0000000000..e69de29bb2
    [EOF]
    ");
}

#[test]
fn test_absorb_deleted_file_with_multiple_hunks() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m1"]).success();
    work_dir.write_file("file1", "1a\n1b\n");
    work_dir.write_file("file2", "1a\n");

    work_dir.run_jj(["new", "-m2"]).success();
    work_dir.write_file("file1", "1a\n");
    work_dir.write_file("file2", "1a\n1b\n");

    // These changes produce conflicts because
    // - for file1, "1a\n" is deleted from the commit 1,
    // - for file2, two consecutive hunks are deleted.
    //
    // Since file2 change is split to two separate hunks, the file deletion
    // cannot be propagated. If we implement merging based on interleaved delta,
    // the file2 change will apply cleanly. The file1 change might be split into
    // "1a\n" deletion at the commit 1 and file deletion at the commit 2, but
    // I'm not sure if that's intuitive.
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.remove_file("file2");
    let output = work_dir.run_jj(["absorb"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 2 revisions:
      kkmpptxz 9210e16d (conflict) 2
      qpvuntsm a52f61f7 (conflict) 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln f8744d38 (no description set)
    Parent commit (@-)      : kkmpptxz 9210e16d (conflict) 2
    New conflicts appeared in 2 commits:
      kkmpptxz 9210e16d (conflict) 2
      qpvuntsm a52f61f7 (conflict) 1
    Hint: To resolve the conflicts, start by creating a commit on top of
    the first conflicted commit:
      jj new qpvuntsm
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    Remaining changes:
    D file2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  zsuskuln f8744d38 (no description set)
    │  diff --git a/file2 b/file2
    │  deleted file mode 100644
    │  index 0000000000..0000000000
    │  --- a/file2
    │  +++ /dev/null
    │  @@ -1,7 +0,0 @@
    │  -<<<<<<< Conflict 1 of 1
    │  -%%%%%%% Changes from base to side #1
    │  --1a
    │  - 1b
    │  -+++++++ Contents of side #2
    │  -1a
    │  ->>>>>>> Conflict 1 of 1 ends
    ×  kkmpptxz 9210e16d (conflict) 2
    │  diff --git a/file1 b/file1
    │  deleted file mode 100644
    │  index 0000000000..0000000000
    │  --- a/file1
    │  +++ /dev/null
    │  @@ -1,6 +0,0 @@
    │  -<<<<<<< Conflict 1 of 1
    │  -%%%%%%% Changes from base to side #1
    │  - 1a
    │  -+1b
    │  -+++++++ Contents of side #2
    │  ->>>>>>> Conflict 1 of 1 ends
    │  diff --git a/file2 b/file2
    │  --- a/file2
    │  +++ b/file2
    │  @@ -1,7 +1,7 @@
    │   <<<<<<< Conflict 1 of 1
    │   %%%%%%% Changes from base to side #1
    │  - 1a
    │  --1b
    │  +-1a
    │  + 1b
    │   +++++++ Contents of side #2
    │  -1b
    │  +1a
    │   >>>>>>> Conflict 1 of 1 ends
    ×  qpvuntsm a52f61f7 (conflict) 1
    │  diff --git a/file1 b/file1
    ~  new file mode 100644
       index 0000000000..0000000000
       --- /dev/null
       +++ b/file1
       @@ -0,0 +1,6 @@
       +<<<<<<< Conflict 1 of 1
       +%%%%%%% Changes from base to side #1
       + 1a
       ++1b
       ++++++++ Contents of side #2
       +>>>>>>> Conflict 1 of 1 ends
       diff --git a/file2 b/file2
       new file mode 100644
       index 0000000000..0000000000
       --- /dev/null
       +++ b/file2
       @@ -0,0 +1,7 @@
       +<<<<<<< Conflict 1 of 1
       +%%%%%%% Changes from base to side #1
       + 1a
       +-1b
       ++++++++ Contents of side #2
       +1b
       +>>>>>>> Conflict 1 of 1 ends
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
      qpvuntsm 2a0c7f1d 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln 8ca9761d (no description set)
    Parent commit (@-)      : qpvuntsm 2a0c7f1d 1
    Remaining changes:
    M file1
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  zsuskuln 8ca9761d (no description set)
    │  diff --git a/file1 b/file1
    │  old mode 100755
    │  new mode 100644
    ○  qpvuntsm 2a0c7f1d 1
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
      kkmpptxz cae507ef 2
    Rebased 1 descendant commits.
    Working copy  (@) now at: zsuskuln f02fd9ea (no description set)
    Parent commit (@-)      : kkmpptxz cae507ef 2
    Remaining changes:
    M file1
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "@-::"), @r"
    @  zsuskuln f02fd9ea (no description set)
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
    ○  kkmpptxz cae507ef 2
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
      rlvkpnrz ddaed33d 1
    Rebased 2 descendant commits.
    Working copy  (@) now at: zsuskuln 3652e5e5 (no description set)
    Parent commit (@-)      : kkmpptxz 7f4339e7 (empty) 2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  zsuskuln 3652e5e5 (no description set)
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
    ○  kkmpptxz 7f4339e7 (empty) 2
    ○  rlvkpnrz ddaed33d 1
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
    ○  qpvuntsm e8849ae1 (empty) (no description set)
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

    let output = work_dir.run_jj(["absorb", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Nothing changed.
    [EOF]
    ");

    let output = work_dir.run_jj(["absorb", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Absorbed changes into 1 revisions:
      qpvuntsm ca07fabe 1
    Rebased 1 descendant commits.
    Working copy  (@) now at: kkmpptxz 4d80ada8 (no description set)
    Parent commit (@-)      : qpvuntsm ca07fabe 1
    Remaining changes:
    M file2
    [EOF]
    ");

    insta::assert_snapshot!(get_diffs(&work_dir, "mutable()"), @r"
    @  kkmpptxz 4d80ada8 (no description set)
    │  diff --git a/file2 b/file2
    │  index a8994dc188..268de3f3ec 100644
    │  --- a/file2
    │  +++ b/file2
    │  @@ -1,1 +1,1 @@
    │  -1a
    │  +1A
    ○  qpvuntsm ca07fabe 1
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
      kkmpptxz e68cc3e2 2
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl 88443af7 (no description set)
    Parent commit (@-)      : kkmpptxz e68cc3e2 2
    Remaining changes:
    M file1
    [EOF]
    ");

    // Immutable revisions shouldn't be rewritten
    let output = work_dir.run_jj(["absorb", "--into=all()"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Commit e35bcaffcb55 is immutable
    Hint: Could not modify commit: qpvuntsm e35bcaff main | 1
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://docs.jj-vcs.dev/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "#);

    insta::assert_snapshot!(get_diffs(&work_dir, ".."), @r"
    @  mzvwutvl 88443af7 (no description set)
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
    ○  kkmpptxz e68cc3e2 2
    │  diff --git a/file1 b/file1
    │  index 8c5268f893..75e4047831 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,2 +1,4 @@
    │   1a
    │   1b
    │  +2a
    │  +2B
    ◆  qpvuntsm e35bcaff 1
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
    let template = r#"format_commit_summary_with_refs(commit, "") ++ "\n""#;
    work_dir.run_jj(["evolog", "-r", revision, "-T", template])
}
