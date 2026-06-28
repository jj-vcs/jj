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
fn test_duplicate() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &["a", "b"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d9eeb5edcc43   c
    ├─╮
    │ ○  6819720393db   b
    ○ │  a1afb5834d8e   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "all()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot duplicate the root commit
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["duplicate", "none()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to duplicate.
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated a1afb5834d8e as kpqxywon 13eb8bd0 a
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d9eeb5edcc43   c
    ├─╮
    │ ○  6819720393db   b
    ○ │  a1afb5834d8e   a
    ├─╯
    │ ○  13eb8bd0a547   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: c4b5a9bbf559 (2001-02-03 08:05:17) duplicate 1 commit(s)
    Restored to operation: 1f185bd7a742 (2001-02-03 08:05:13) create bookmark c pointing to commit d9eeb5edcc43595ec2df4bbcbc539e863e0a550a
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate" /* duplicates `c` */]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated d9eeb5edcc43 as lylxulpl 39c49b11 c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    d9eeb5edcc43   c
    ├─╮
    │ │ ○  39c49b11cf6d   c
    ╭─┬─╯
    │ ○  6819720393db   b
    ○ │  a1afb5834d8e   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_duplicate_many() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["b", "d"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    │ ○  45ee1acd6076   c
    ○ │  dd148a1be8f0   b
    ├─╯
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["duplicate", "b::"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated dd148a1be8f0 as wqnwkozp d7f94df8 b
    Duplicated e7e415348f77 as mouksmqu c1babd61 e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    ○ │  dd148a1be8f0   b
    │ │ ○  c1babd613375   e
    │ ╭─┤
    │ ○ │  278414eace87   d
    │ ○ │  45ee1acd6076   c
    ├─╯ │
    │   ○  d7f94df81143   b
    ├───╯
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");

    // Try specifying the same commit twice directly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["duplicate", "b", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated dd148a1be8f0 as nkmrtpmo f8a8d54e b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    │ ○  45ee1acd6076   c
    ○ │  dd148a1be8f0   b
    ├─╯
    │ ○  f8a8d54e5bca   b
    ├─╯
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");

    // Try specifying the same commit twice indirectly
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["duplicate", "b::", "d::"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated dd148a1be8f0 as xtnwkqum a8d4c220 b
    Duplicated 278414eace87 as pqrnrkux 531366d3 d
    Duplicated e7e415348f77 as ztxkyksq c47b2bae e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    ○ │  dd148a1be8f0   b
    │ │ ○    c47b2bae5ce3   e
    │ │ ├─╮
    │ │ │ ○  531366d31b84   d
    │ ├───╯
    │ ○ │  45ee1acd6076   c
    ├─╯ │
    │   ○  a8d4c22086db   b
    ├───╯
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Reminder of the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    │ ○  45ee1acd6076   c
    ○ │  dd148a1be8f0   b
    ├─╯
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate", "d::", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated a1afb5834d8e as nlrtlrxv 117dd806 a
    Duplicated 278414eace87 as plymsszl f8ea2332 d
    Duplicated e7e415348f77 as urrlptpw 66f0d2b4 e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    │ │ ○  66f0d2b41a0a   e
    ╭───┤
    │ │ ○  f8ea2332db89   d
    │ ├─╯
    │ ○  45ee1acd6076   c
    ○ │  dd148a1be8f0   b
    ├─╯
    ○  a1afb5834d8e   a
    │ ○  117dd80623e6   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Check for BUG -- makes too many 'a'-s, etc.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["duplicate", "a::"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated a1afb5834d8e as uuuvxpvw cb730319 a
    Duplicated dd148a1be8f0 as nmpuuozl b00a23f6 b
    Duplicated 45ee1acd6076 as kzpokyyw 7c1b86d5 c
    Duplicated 278414eace87 as yxrlprzz 2f5494bb d
    Duplicated e7e415348f77 as mvkzkxrl 8a4c81fe e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    e7e415348f77   e
    ├─╮
    │ ○  278414eace87   d
    │ ○  45ee1acd6076   c
    ○ │  dd148a1be8f0   b
    ├─╯
    ○  a1afb5834d8e   a
    │ ○    8a4c81fee5f1   e
    │ ├─╮
    │ │ ○  2f5494bb9bce   d
    │ │ ○  7c1b86d551da   c
    │ ○ │  b00a23f660bf   b
    │ ├─╯
    │ ○  cb7303191ed7   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_duplicate_destination() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a1", &[]);
    create_commit(&work_dir, "a2", &["a1"]);
    create_commit(&work_dir, "a3", &["a2"]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &[]);
    create_commit(&work_dir, "d", &[]);
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37ccb2c26fd4   d
    │ ○  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit onto a single destination.
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as kxryzmor 3aa99cfa a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37ccb2c26fd4   d
    │ ○  3aa99cfaac35   a1
    │ ○  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit onto multiple destinations.
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as xznxytkn 47234b89 a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    47234b8989dd   a1
    ├─╮
    │ @  37ccb2c26fd4   d
    ○ │  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit onto its descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as tlkvzzqu e72119f1 (empty) a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37ccb2c26fd4   d
    │ ○  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  e72119f1aa17   a1
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Duplicate multiple commits without a direct ancestry relationship onto a
    // single destination.
    let output = work_dir.run_jj(["duplicate", "-r=a1", "-r=b", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as pzsxstzt baf4e167 a1
    Duplicated f7702717b8e8 as nxkxtmvy 0f23e3a3 b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37ccb2c26fd4   d
    │ ○  0f23e3a3cf38   b
    │ │ ○  baf4e167593f   a1
    │ ├─╯
    │ ○  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship onto
    // multiple destinations.
    let output = work_dir.run_jj(["duplicate", "-r=a1", "b", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as qmkrwlvp e622db81 a1
    Duplicated f7702717b8e8 as pkqrwoqq e6c7af33 b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○    e6c7af331f0c   b
    ├─╮
    │ │ ○  e622db8157e5   a1
    ╭─┬─╯
    │ @  37ccb2c26fd4   d
    ○ │  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship onto a
    // single destination.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-o", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as qwyusntz 46ae14e0 a1
    Duplicated fb5b814db965 as pwpvvyov d5b9939e a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  37ccb2c26fd4   d
    │ ○  d5b9939eaa52   a3
    │ ○  46ae14e05cf5   a1
    │ ○  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship onto
    // multiple destinations.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as soqnvnyz 4813e2a9 a1
    Duplicated fb5b814db965 as nmmmqslz 27e49f8c a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  27e49f8c36cb   a3
    ○    4813e2a9ac9e   a1
    ├─╮
    │ @  37ccb2c26fd4   d
    ○ │  5e749d71532c   c
    ├─╯
    │ ○  f7702717b8e8   b
    ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_duplicate_insert_after() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a1", &[]);
    create_commit(&work_dir, "a2", &["a1"]);
    create_commit(&work_dir, "a3", &["a2"]);
    create_commit(&work_dir, "a4", &["a3"]);
    create_commit(&work_dir, "b1", &[]);
    create_commit(&work_dir, "b2", &["b1"]);
    create_commit(&work_dir, "c1", &[]);
    create_commit(&work_dir, "c2", &["c1"]);
    create_commit(&work_dir, "d1", &[]);
    create_commit(&work_dir, "d2", &["d1"]);
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit after a single commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as nlrtlrxv 403a87a9 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  e03525446a4f   b2
    │ ○  403a87a97264   a1
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after a single ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a3", "--after", "a1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as uuuvxpvw 82ae0465 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  8767667a6595   a4
    │ ○  a6238b8a4ee7   a3
    │ ○  e50feabdd2c0   a2
    │ ○  82ae04656204   a3
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after a single descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as pkstwlsy 2ef408af (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  e315705b0c3b   a4
    │ ○  2ef408af2f15   a1
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits with no direct
    // relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1", "--after", "c1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as zowrlwsv 1c771e05 a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  71854ffefce7   c2
    │ │ ○  0f42648eec8e   b2
    │ ├─╯
    │ ○    1c771e05c0d3   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits including an ancestor.
    let output = work_dir.run_jj(["duplicate", "a3", "--after", "a2", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as wvmqtotl 91120b34 a3
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  56ff50de5055   a4
    │ ○  704308de44cc   a3
    │ ○    91120b348523   a3
    │ ├─╮
    │ │ ○  06712178d528   b2
    │ │ ○  e7241b6b0079   b1
    ├───╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits including a descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as opwsxtwu 0e7d347c (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  8f26be7aab41   a4
    │ ○    0e7d347c5b52   a1
    │ ├─╮
    │ │ ○  06712178d528   b2
    │ │ ○  e7241b6b0079   b1
    ├───╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as ukwxllxp 0cedc1c7 a1
    Duplicated e7241b6b0079 as yrwmsomt 0d18a4ba b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    78b9a5a33939   c2
    │ ├─╮
    │ │ ○  0d18a4ba8860   b1
    │ ○ │  0cedc1c7f5dc   a1
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit which is an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as szrrkvty d914610f a3
    Duplicated e7241b6b0079 as wvmrymqu e07aeee7 b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  4c8edef23e44   a4
    │ ○    69d7338e773e   a3
    │ ├─╮
    │ │ ○  e07aeee7dc3d   b1
    │ ○ │  d914610fbc0d   a3
    │ ├─╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit which is a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as ztnvrxlv 650404f6 (empty) a1
    Duplicated e7241b6b0079 as upuzqpxs a76583cc b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○    f0a61c7aee36   a4
    │ ├─╮
    │ │ ○  a76583cc72f4   b1
    │ ○ │  650404f6e8b5   a1
    │ ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--after", "d1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as muymlknp f0516baa a1
    Duplicated e7241b6b0079 as snrzyvry ca1f0f71 b1
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx 1a496e82 d2 | d2
    Parent commit (@-)      : muymlknp f0516baa a1
    Parent commit (@-)      : snrzyvry ca1f0f71 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    1a496e82b224   d2
    ├─╮
    │ │ ○  66899a31a50c   c2
    ╭─┬─╯
    │ ○    ca1f0f71408c   b1
    │ ├─╮
    ○ │ │  f0516baaccf2   a1
    ╰─┬─╮
      │ ○  8aae10eecc84   d1
      ○ │  9b24b49f717e   c1
      ├─╯
    ○ │  06712178d528   b2
    ○ │  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a1", "--after", "c1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as vnqwxmpr 616184ac a3
    Duplicated e7241b6b0079 as pvqonzsn b260f09e b1
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    0e5e70cb5c7d   c2
    │ ├─╮
    │ │ │ ○  d20e1531fe79   a4
    │ │ │ ○  c5c3cdb83d11   a3
    │ │ │ ○  fba9171322f3   a2
    │ ╭─┬─╯
    │ │ ○    b260f09e49cb   b1
    │ │ ├─╮
    │ ○ │ │  616184ac420c   a3
    │ ╰─┬─╮
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  94078b99a3ab   a1
    ├───╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as qtvkyytt c2d9e172 (empty) a1
    Duplicated e7241b6b0079 as ouvslmur d037ef3f b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    c56f22b44ec5   a4
    │ ├─╮
    │ │ ○    d037ef3fea72   b1
    │ │ ├─╮
    │ ○ │ │  c2d9e17233f9   a1
    │ ╰─┬─╮
    │   │ ○  10855af02fff   c2
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  fb5b814db965   a3
    │   ○  932fb7b4fc1e   a2
    │   ○  94078b99a3ab   a1
    ├───╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as qowqnpnw 661c603f a1
    Duplicated fb5b814db965 as mommxqln 526992a5 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  526992a57e7f   a3
    │ ○  661c603fdd94   a1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a2", "a3", "--after", "a1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Warning: Duplicating commit 932fb7b4fc1e as an ancestor of itself
    Duplicated 932fb7b4fc1e as qzusktlu 302e8c8e a2
    Duplicated fb5b814db965 as zryotxso 71b0f3f5 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  fcd4a84b877b   a4
    │ ○  d9de6aceb55b   a3
    │ ○  16282d473e5e   a2
    │ ○  71b0f3f51789   a3
    │ ○  302e8c8e1462   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 932fb7b4fc1e as a descendant of itself
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as stzvpxow 4bb169c3 (empty) a1
    Duplicated 932fb7b4fc1e as zrzsnomp 5d89a41f (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  4fa1d0d8f99b   a4
    │ ○  5d89a41f3267   a2
    │ ○  4bb169c3ab82   a1
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c2", "--after", "d2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as ysllonyo 697b40b2 a1
    Duplicated fb5b814db965 as kzxwzvzw 53ca8429 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    ○  53ca8429c1ba   a3
    ○    697b40b29246   a1
    ├─╮
    │ @  d7fd293d0ee4   d2
    │ ○  8aae10eecc84   d1
    ○ │  10855af02fff   c2
    ○ │  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a2", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 31f05d9b8a41 as an ancestor of itself
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as kvqpkqvl 782345d2 a3
    Duplicated 31f05d9b8a41 as zqztuxrl 8a9ab779 a4
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  22deba30c553   a4
    │ ○  e46513e020a7   a3
    │ ○  8a9ab779d9cc   a4
    │ ○    782345d22a6a   a3
    │ ├─╮
    │ │ ○  10855af02fff   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 932fb7b4fc1e as a descendant of itself
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as xsvtwpuq 03b391b5 (empty) a1
    Duplicated 932fb7b4fc1e as tmzzmpyp c040997b (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  a7b61baf26cd   a4
    │ ○  c040997b717e   a2
    │ ○    03b391b59022   a1
    │ ├─╮
    │ │ ○  10855af02fff   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit 06712178d528 would be both an ancestor and a descendant of the duplicated commits
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_duplicate_insert_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a1", &[]);
    create_commit(&work_dir, "a2", &["a1"]);
    create_commit(&work_dir, "a3", &["a2"]);
    create_commit(&work_dir, "a4", &["a3"]);
    create_commit(&work_dir, "b1", &[]);
    create_commit(&work_dir, "b2", &["b1"]);
    create_commit(&work_dir, "c1", &[]);
    create_commit(&work_dir, "c2", &["c1"]);
    create_commit(&work_dir, "d1", &[]);
    create_commit(&work_dir, "d2", &["d1"]);
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit before a single commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as nlrtlrxv 403a87a9 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  e03525446a4f   b2
    │ ○  403a87a97264   a1
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before a single ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as uuuvxpvw cbb38dd4 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  e85e551840d0   a4
    │ ○  6976425a5e4e   a3
    │ ○  01e4e43ca191   a2
    │ ○  87cf0c9c25fb   a1
    │ ○  cbb38dd4f677   a3
    ├─╯
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before a single descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as pkstwlsy 26ff671e (empty) a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  635ca6b60217   a4
    │ ○  9eb3fccbc0fb   a3
    │ ○  26ff671ef00d   a1
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits with no direct
    // relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as zowrlwsv 1c771e05 a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  71854ffefce7   c2
    │ │ ○  0f42648eec8e   b2
    │ ├─╯
    │ ○    1c771e05c0d3   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits including an ancestor.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as wvmqtotl 3aa4ac96 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  388fd4668840   b2
    │ │ ○  c822624aed5d   a4
    │ │ ○  feaa49d43b8e   a3
    │ │ ○  87f815dbcc15   a2
    │ ├─╯
    │ ○    3aa4ac96c873   a3
    │ ├─╮
    │ │ ○  e7241b6b0079   b1
    ├───╯
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits including a descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "a3", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as opwsxtwu e2dcaa9c (empty) a1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  55c7d5e620d1   b2
    │ │ ○  e1478dae0b59   a4
    │ │ ○  a1e7be94b5c9   a3
    │ ├─╯
    │ ○    e2dcaa9c31d5   a1
    │ ├─╮
    │ │ ○  e7241b6b0079   b1
    ├───╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "c1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as ukwxllxp 3323f9c3 a1
    Duplicated e7241b6b0079 as yrwmsomt a6ef0369 b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  5529d29f53bf   c2
    │ ○    2d5211a7d52f   c1
    │ ├─╮
    │ │ ○  a6ef03692b89   b1
    ├───╯
    │ ○  3323f9c396f8   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit which is an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--before", "a2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as szrrkvty 872c3f20 a3
    Duplicated e7241b6b0079 as wvmrymqu 340f6bb8 b1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  59c0b3ba82a7   a4
    │ ○  954c28d0b37b   a3
    │ ○    386c48637ff1   a2
    │ ├─╮
    │ │ ○  340f6bb8eba8   b1
    │ ○ │  872c3f209e23   a3
    │ ├─╯
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit which is a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as ztnvrxlv fa5a74fa (empty) a1
    Duplicated e7241b6b0079 as upuzqpxs b3d7a96b b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  052d0b0c2edf   a4
    │ ○    ffe51e55e530   a3
    │ ├─╮
    │ │ ○  b3d7a96b290f   b1
    │ ○ │  fa5a74fac661   a1
    │ ├─╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "c1", "--before", "d1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as muymlknp 9add628e a1
    Duplicated e7241b6b0079 as snrzyvry b63fdd54 b1
    Rebased 4 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx dec31ad0 d2 | d2
    Parent commit (@-)      : mznxytkn 70b5dbc6 d1 | d1
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  dec31ad0c0d5   d2
    ○    70b5dbc66dc7   d1
    ├─╮
    │ │ ○  8432e9329ed5   c2
    │ │ ○  fdc43aa17cf1   c1
    ╭─┬─╯
    │ ○  b63fdd54c3f9   b1
    ○ │  9add628e2f94   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--before", "a1", "--before", "c1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as vnqwxmpr 73f3594f a3
    Duplicated e7241b6b0079 as pvqonzsn 67d4a940 b1
    Rebased 6 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  9fdad5c6c18c   c2
    │ ○    dbb8e01766cc   c1
    │ ├─╮
    │ │ │ ○  a8f5997056f0   a4
    │ │ │ ○  2c140b15105f   a3
    │ │ │ ○  efcfa3a910d7   a2
    │ │ │ ○  559edec70334   a1
    │ ╭─┬─╯
    │ │ ○  67d4a9404b6b   b1
    ├───╯
    │ ○  73f3594f082f   a3
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as qtvkyytt d6b68ab1 (empty) a1
    Duplicated e7241b6b0079 as ouvslmur f9d630e5 b1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    3e00ce9e0b75   c2
    │ ├─╮
    │ │ │ ○  fc357110d6e0   a4
    │ │ │ ○  d37ac4734b62   a3
    │ ╭─┬─╯
    │ │ ○    f9d630e59922   b1
    │ │ ├─╮
    │ ○ │ │  d6b68ab125ee   a1
    │ ╰─┬─╮
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  932fb7b4fc1e   a2
    │   ○  94078b99a3ab   a1
    ├───╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as qowqnpnw 1cd05f49 a1
    Duplicated fb5b814db965 as mommxqln 51d00be6 a3
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  97207ca0668e   c2
    │ ○  51d00be616d6   a3
    │ ○  1cd05f491a63   a1
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "a1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Warning: Duplicating commit 94078b99a3ab as an ancestor of itself
    Duplicated 94078b99a3ab as qzusktlu d8aaed30 a1
    Duplicated fb5b814db965 as zryotxso 701cf123 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  049b474c3237   a4
    │ ○  55db825c3612   a3
    │ ○  4676026aa09a   a2
    │ ○  3333c388c9bb   a1
    │ ○  701cf1238e40   a3
    │ ○  d8aaed30e073   a1
    ├─╯
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 932fb7b4fc1e as a descendant of itself
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as stzvpxow b30be90c (empty) a1
    Duplicated 932fb7b4fc1e as zrzsnomp 9b00bd13 (empty) a2
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  4e3254a554d2   a4
    │ ○  0c2c875c730c   a3
    │ ○  9b00bd13027c   a2
    │ ○  b30be90c574e   a1
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "c2", "--before", "d2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as ysllonyo 0744dd96 a1
    Duplicated fb5b814db965 as kzxwzvzw a5eb90b5 a3
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx a395b8c4 d2 | d2
    Parent commit (@-)      : kzxwzvzw a5eb90b5 a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a395b8c4f9b8   d2
    │ ○  2c33d86fcf48   c2
    ├─╯
    ○  a5eb90b55b73   a3
    ○    0744dd965980   a1
    ├─╮
    │ ○  8aae10eecc84   d1
    ○ │  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--before", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 31f05d9b8a41 as an ancestor of itself
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as kvqpkqvl 2af19233 a3
    Duplicated 31f05d9b8a41 as zqztuxrl 0f8b723e a4
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  547066cdd064   c2
    │ │ ○  d259a3385807   a4
    │ │ ○  88e9b5424cdf   a3
    │ │ ○  540df4eecd9a   a2
    │ ├─╯
    │ ○  0f8b723eabf8   a4
    │ ○    2af192339fec   a3
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  94078b99a3ab   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 932fb7b4fc1e as a descendant of itself
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as xsvtwpuq 25e380ee (empty) a1
    Duplicated 932fb7b4fc1e as tmzzmpyp 304eb3d9 (empty) a2
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  34fad5173b7f   c2
    │ │ ○  ce1cca82ac7c   a4
    │ │ ○  e69839ebb5db   a3
    │ ├─╯
    │ ○  304eb3d9d0c2   a2
    │ ○    25e380ee496d   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b1", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit e7241b6b0079 would be both an ancestor and a descendant of the duplicated commits
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_duplicate_insert_after_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a1", &[]);
    create_commit(&work_dir, "a2", &["a1"]);
    create_commit(&work_dir, "a3", &["a2"]);
    create_commit(&work_dir, "a4", &["a3"]);
    create_commit(&work_dir, "b1", &[]);
    create_commit(&work_dir, "b2", &["b1"]);
    create_commit(&work_dir, "c1", &[]);
    create_commit(&work_dir, "c2", &["c1"]);
    create_commit(&work_dir, "d1", &[]);
    create_commit(&work_dir, "d2", &["d1"]);
    let setup_opid = work_dir.current_operation_id();

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit in between commits with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as nlrtlrxv 9d768b2f a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    d181500bbf1e   b2
    │ ├─╮
    │ │ ○  9d768b2f99bf   a1
    │ │ ○  10855af02fff   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between ancestor commits.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--after", "a1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as uuuvxpvw 82ae0465 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  8767667a6595   a4
    │ ○  a6238b8a4ee7   a3
    │ ○  e50feabdd2c0   a2
    │ ○  82ae04656204   a3
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between an ancestor commit and a commit with no
    // direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--after", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as pkstwlsy ce36a444 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  ee6e2d96b7b7   a4
    │ ○  9ad0ab3dfd07   a3
    │ ○    afe639bdac35   a2
    │ ├─╮
    │ │ ○  ce36a4449e02   a3
    │ │ ○  06712178d528   b2
    │ │ ○  e7241b6b0079   b1
    ├───╯
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between descendant commits.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--before", "a4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as zowrlwsv b3121b8b (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  8a60dca460aa   a4
    │ ○  b3121b8b5955   a1
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between a descendant commit and a commit with no
    // direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--before", "b2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as wvmqtotl 907f4e68 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○    53db1227a0ba   b2
    │ ├─╮
    │ │ ○  907f4e687807   a1
    │ ○ │  e7241b6b0079   b1
    ├─╯ │
    │ ○ │  31f05d9b8a41   a4
    │ ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between an ancestor commit and a descendant
    // commit.
    let output = work_dir.run_jj(["duplicate", "a2", "--after", "a1", "--before", "a4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 932fb7b4fc1e as opwsxtwu fb042846 a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○    38f9a19a55b2   a4
    │ ├─╮
    │ │ ○  fb042846b520   a2
    │ ○ │  fb5b814db965   a3
    │ ○ │  932fb7b4fc1e   a2
    │ ├─╯
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between
    // commits without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as ukwxllxp 0cedc1c7 a1
    Duplicated e7241b6b0079 as yrwmsomt 0d18a4ba b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx 0f64dfec d2 | d2
    Parent commit (@-)      : mznxytkn 8aae10ee d1 | d1
    Parent commit (@-)      : ukwxllxp 0cedc1c7 a1
    Parent commit (@-)      : yrwmsomt 0d18a4ba b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      0f64dfec719e   d2
    ├─┬─╮
    │ │ ○  0d18a4ba8860   b1
    │ ○ │  0cedc1c7f5dc   a1
    │ ├─╯
    ○ │  8aae10eecc84   d1
    │ │ ○  10855af02fff   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between a
    // commit which is an ancestor of one of the duplicated commits and a commit
    // with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated fb5b814db965 as szrrkvty d914610f a3
    Duplicated e7241b6b0079 as wvmrymqu e07aeee7 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○      9cc20a631b4d   c2
    │ ├─┬─╮
    │ │ │ ○  e07aeee7dc3d   b1
    │ │ ○ │  d914610fbc0d   a3
    │ │ ├─╯
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  06712178d528   b2
    │ ○ │  e7241b6b0079   b1
    ├─╯ │
    │ ○ │  31f05d9b8a41   a4
    │ ○ │  fb5b814db965   a3
    │ ├─╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between a
    // commit which is a descendant of one of the duplicated commits and a
    // commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as ztnvrxlv 650404f6 (empty) a1
    Duplicated e7241b6b0079 as upuzqpxs a76583cc b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○      e7c3f5420501   c2
    │ ├─┬─╮
    │ │ │ ○  a76583cc72f4   b1
    │ │ ○ │  650404f6e8b5   a1
    │ │ ├─╯
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  06712178d528   b2
    │ ○ │  e7241b6b0079   b1
    ├─╯ │
    │ ○ │  31f05d9b8a41   a4
    │ ├─╯
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as muymlknp e3890eb5 a1
    Duplicated e7241b6b0079 as snrzyvry d3066453 b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx 292325b5 d2 | d2
    Parent commit (@-)      : mznxytkn 8aae10ee d1 | d1
    Parent commit (@-)      : muymlknp e3890eb5 a1
    Parent commit (@-)      : snrzyvry d3066453 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @      292325b5e23e   d2
    ├─┬─╮
    │ │ ○  d30664539118   b1
    │ ○ │  e3890eb5520e   a1
    │ ├─╯
    ○ │  8aae10eecc84   d1
    │ │ ○  10855af02fff   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as vnqwxmpr 0a6ab30c a1
    Duplicated fb5b814db965 as pvqonzsn eb5c8329 a3
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: rmzmmopx f667599d d2 | d2
    Parent commit (@-)      : mznxytkn 8aae10ee d1 | d1
    Parent commit (@-)      : pvqonzsn eb5c8329 a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    f667599dc50c   d2
    ├─╮
    │ ○  eb5c832980d2   a3
    │ ○  0a6ab30c6a03   a1
    ○ │  8aae10eecc84   d1
    │ │ ○  10855af02fff   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  31f05d9b8a41   a4
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between a commit
    // which is an ancestor of one of the duplicated commits and a commit
    // without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated fb5b814db965 as qtvkyytt 63062e2d a3
    Duplicated 31f05d9b8a41 as ouvslmur 08ed9574 a4
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○    4657b371268e   c2
    │ ├─╮
    │ │ ○  08ed957425b3   a4
    │ │ ○  63062e2d8b3f   a3
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  06712178d528   b2
    │ ○ │  e7241b6b0079   b1
    ├─╯ │
    │ ○ │  31f05d9b8a41   a4
    │ ○ │  fb5b814db965   a3
    │ ├─╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between a commit
    // which is a a descendant of one of the duplicated commits and a commit
    // with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 94078b99a3ab as qowqnpnw 661c603f a1
    Duplicated 932fb7b4fc1e as mommxqln 0f27a04e a2
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  e5c598b73191   a4
    │ ○    bbba043083dd   a3
    │ ├─╮
    │ │ ○  0f27a04e3aeb   a2
    │ │ ○  661c603fdd94   a1
    │ │ ○  10855af02fff   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between descendant
    // commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a1", "--before", "a2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 31f05d9b8a41 as an ancestor of itself
    Warning: Duplicating commit fb5b814db965 as an ancestor of itself
    Duplicated fb5b814db965 as qzusktlu 27c7398b a3
    Duplicated 31f05d9b8a41 as zryotxso 48becebd a4
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  2049bb0c1213   a4
    │ ○  3b847034bd03   a3
    │ ○  37c590c83d22   a2
    │ ○  48becebd34bf   a4
    │ ○  27c7398b920d   a3
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between ancestor
    // commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3", "--before", "a4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 932fb7b4fc1e as a descendant of itself
    Warning: Duplicating commit 94078b99a3ab as a descendant of itself
    Duplicated 94078b99a3ab as stzvpxow 4bb169c3 (empty) a1
    Duplicated 932fb7b4fc1e as zrzsnomp 5d89a41f (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○  4fa1d0d8f99b   a4
    │ ○  5d89a41f3267   a2
    │ ○  4bb169c3ab82   a1
    │ ○  fb5b814db965   a3
    │ ○  932fb7b4fc1e   a2
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between an ancestor
    // commit and a descendant commit.
    let output = work_dir.run_jj(["duplicate", "a2", "a3", "--after", "a1", "--before", "a4"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 932fb7b4fc1e as ysllonyo 646ba07b a2
    Duplicated fb5b814db965 as kzxwzvzw b761c9da a3
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  d7fd293d0ee4   d2
    ○  8aae10eecc84   d1
    │ ○  10855af02fff   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  06712178d528   b2
    │ ○  e7241b6b0079   b1
    ├─╯
    │ ○    c8ea4f3f2dde   a4
    │ ├─╮
    │ │ ○  b761c9dacf4d   a3
    │ │ ○  646ba07be1da   a2
    │ ○ │  fb5b814db965   a3
    │ ○ │  932fb7b4fc1e   a2
    │ ├─╯
    │ ○  94078b99a3ab   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b2", "--before", "b1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to create a loop: commit 06712178d528 would be both an ancestor and a descendant of the duplicated commits
    [EOF]
    [exit status: 1]
    ");
}

// https://github.com/jj-vcs/jj/issues/1050
#[test]
fn test_undo_after_duplicate() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");

    // exercise --quiet while here
    let output = work_dir.run_jj(["duplicate", "a", "--quiet"]);
    insta::assert_snapshot!(output, @"");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a1afb5834d8e   a
    │ ○  346a7abed73c   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 7508dc2c713d (2001-02-03 08:05:11) duplicate 1 commit(s)
    Restored to operation: 5852f7bcda39 (2001-02-03 08:05:09) create bookmark a pointing to commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");
}

// https://github.com/jj-vcs/jj/issues/694
#[test]
fn test_rebase_duplicates() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  26c624f4f2a7   c @ 2001-02-03 04:05:13.000 +07:00
    ○  dd148a1be8f0   b @ 2001-02-03 04:05:11.000 +07:00
    ○  a1afb5834d8e   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 26c624f4f2a7 as yostqsxw 851ae923 c
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 26c624f4f2a7 as znkkpsqq 75642066 c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  26c624f4f2a7   c @ 2001-02-03 04:05:13.000 +07:00
    │ ○  75642066eb4e   c @ 2001-02-03 04:05:16.000 +07:00
    ├─╯
    │ ○  851ae9234e09   c @ 2001-02-03 04:05:15.000 +07:00
    ├─╯
    ○  dd148a1be8f0   b @ 2001-02-03 04:05:11.000 +07:00
    ○  a1afb5834d8e   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-s", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Working copy  (@) now at: ooyxmykx 84f86707 c | c
    Parent commit (@-)      : psuskuln ab040f78 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Some of the duplicate commits' timestamps were changed a little to make them
    // have distinct commit ids.
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  84f867077804   c @ 2001-02-03 04:05:18.000 +07:00
    │ ○  c5e49011b9aa   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    │ ○  78b763f2ae89   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    ○  ab040f786d0b   b @ 2001-02-03 04:05:18.000 +07:00
    │ ○  a1afb5834d8e   a @ 2001-02-03 04:05:09.000 +07:00
    ├─╯
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");
}

#[test]
fn test_duplicate_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  26c624f4f2a7   c
    ○  dd148a1be8f0   b
    ○  a1afb5834d8e   a
    ◆  000000000000
    [EOF]
    ");

    // Test duplicate_commits()
    test_env.add_config(r#"templates.duplicate_description = "concat(description, '\n(cherry picked from commit ', commit_id, ')')""#);
    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated a1afb5834d8e as yostqsxw 937cc4bf a
    [EOF]
    ");

    // Test duplicate_commits_onto_parents()
    let output = work_dir.run_jj(["duplicate", "a", "-B", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit a1afb5834d8e as a descendant of itself
    Duplicated a1afb5834d8e as znkkpsqq 319a3bbb (empty) a
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: ooyxmykx c1a62298 c | c
    Parent commit (@-)      : psuskuln 0847a3bb b | b
    [EOF]
    ");

    // Test empty template
    test_env.add_config("templates.duplicate_description = ''");
    let output = work_dir.run_jj(["duplicate", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 0847a3bb3e92 as kpqxywon 33044659 (no description set)
    [EOF]
    ");

    // Test `description` as an alias
    test_env.add_config("templates.duplicate_description = 'description'");
    let output = work_dir.run_jj([
        "duplicate",
        "c",
        "-o",
        "root()",
        // Use an argument here so we can actually see the log in the last test
        // (We don't have a way to remove a config in TestEnvironment)
        "--config",
        "template-aliases.description='\"alias\"'",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated c1a622989f46 as kmkuslsw e36bebd2 alias
    [EOF]
    ");

    let template = r#"commit_id.short() ++ "\n" ++ description ++ "[END]\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @"
    @  c1a622989f46
    │  c
    │  [END]
    ○  0847a3bb3e92
    │  b
    │  [END]
    ○  319a3bbbc606
    │  a
    │
    │  (cherry picked from commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53)
    │  [END]
    ○  a1afb5834d8e
    │  a
    │  [END]
    │ ○  e36bebd28ab6
    ├─╯  alias
    │    [END]
    │ ○  33044659b895
    ├─╯  [END]
    │ ○  937cc4bf468a
    ├─╯  a
    │
    │    (cherry picked from commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53)
    │    [END]
    ◆  000000000000
       [END]
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ "   " ++ description.first_line()"#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_log_output_with_ts(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    commit_id.short() ++ "   " ++ description.first_line() ++ " @ " ++ committer.timestamp()
    "#;
    work_dir.run_jj(["log", "-T", template])
}
