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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    387b928721d9   c
    ├─╮
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "all()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot duplicate the root commit
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["duplicate", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No revisions to duplicate.
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as kpqxywon 13eb8bd0 a
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    387b928721d9   c
    ├─╮
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
    ├─╯
    │ ○  13eb8bd0a547   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 9fdb56d75a27 (2001-02-03 08:05:13) create bookmark c pointing to commit 387b928721d9f2efff819ccce81868f32537d71f
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate" /* duplicates `c` */]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 387b928721d9 as lylxulpl 71c64df5 c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    387b928721d9   c
    ├─╮
    │ │ ○  71c64df584dc   c
    ╭─┬─╯
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    │ ○  991a7501d660   c
    ○ │  123b4d91f6e5   b
    ├─╯
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "b::"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 123b4d91f6e5 as wqnwkozp 10059c86 b
    Duplicated 0559be9bd4d0 as mouksmqu 0afe2f34 e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    ○ │  123b4d91f6e5   b
    │ │ ○  0afe2f348a93   e
    │ ╭─┤
    │ ○ │  a2dbb1aad514   d
    │ ○ │  991a7501d660   c
    ├─╯ │
    │   ○  10059c8651d7   b
    ├───╯
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    // Try specifying the same commit twice directly
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["duplicate", "b", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 123b4d91f6e5 as nkmrtpmo 1ccf2589 b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    │ ○  991a7501d660   c
    ○ │  123b4d91f6e5   b
    ├─╯
    │ ○  1ccf2589bfd1   b
    ├─╯
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    // Try specifying the same commit twice indirectly
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["duplicate", "b::", "d::"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 123b4d91f6e5 as xtnwkqum 1a94ffc6 b
    Duplicated a2dbb1aad514 as pqrnrkux 6a17a96d d
    Duplicated 0559be9bd4d0 as ztxkyksq b113bd5c e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    ○ │  123b4d91f6e5   b
    │ │ ○    b113bd5c550a   e
    │ │ ├─╮
    │ │ │ ○  6a17a96d77d2   d
    │ ├───╯
    │ ○ │  991a7501d660   c
    ├─╯ │
    │   ○  1a94ffc6e6aa   b
    ├───╯
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    // Reminder of the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    │ ○  991a7501d660   c
    ○ │  123b4d91f6e5   b
    ├─╯
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate", "d::", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as nlrtlrxv 117dd806 a
    Duplicated a2dbb1aad514 as plymsszl f2ec1b7f d
    Duplicated 0559be9bd4d0 as urrlptpw 9e54f34c e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    │ │ ○  9e54f34ca238   e
    ╭───┤
    │ │ ○  f2ec1b7f4e82   d
    │ ├─╯
    │ ○  991a7501d660   c
    ○ │  123b4d91f6e5   b
    ├─╯
    ○  7d980be7a1d4   a
    │ ○  117dd80623e6   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Check for BUG -- makes too many 'a'-s, etc.
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["duplicate", "a::"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as uuuvxpvw cb730319 a
    Duplicated 123b4d91f6e5 as nmpuuozl b00a23f6 b
    Duplicated 991a7501d660 as kzpokyyw 7c1b86d5 c
    Duplicated a2dbb1aad514 as yxrlprzz 2f5494bb d
    Duplicated 0559be9bd4d0 as mvkzkxrl 8a4c81fe e
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    0559be9bd4d0   e
    ├─╮
    │ ○  a2dbb1aad514   d
    │ ○  991a7501d660   c
    ○ │  123b4d91f6e5   b
    ├─╯
    ○  7d980be7a1d4   a
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit onto a single destination.
    let output = work_dir.run_jj(["duplicate", "a1", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as nkmrtpmo f7a7a3f6 a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  f7a7a3f627a2   a1
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit onto multiple destinations.
    let output = work_dir.run_jj(["duplicate", "a1", "-d", "c", "-d", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as xtnwkqum a515a8a7 a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    a515a8a7055d   a1
    ├─╮
    │ @  5248650c3314   d
    ○ │  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit onto its descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "-d", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as wvuyspvk d80473de (empty) a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  d80473de843a   a1
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    // Duplicate multiple commits without a direct ancestry relationship onto a
    // single destination.
    let output = work_dir.run_jj(["duplicate", "-r=a1", "-r=b", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as xlzxqlsl bfc29032 a1
    Duplicated c406dbab05ac as vnkwvqxw c57bb4be b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  c57bb4beac33   b
    │ │ ○  bfc29032c3ff   a1
    │ ├─╯
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship onto
    // multiple destinations.
    let output = work_dir.run_jj(["duplicate", "-r=a1", "b", "-d", "c", "-d", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as oupztwtk 83113cdf a1
    Duplicated c406dbab05ac as yxsqzptr 469ed79e b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    469ed79e4b00   b
    ├─╮
    │ │ ○  83113cdff475   a1
    ╭─┬─╯
    │ @  5248650c3314   d
    ○ │  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship onto a
    // single destination.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-d", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as wtszoswq 31bc8e32 a1
    Duplicated 5fb83d2b58d6 as qmykwtmu 2a318b78 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  2a318b7805df   a3
    │ ○  31bc8e32bcd5   a1
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship onto
    // multiple destinations.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-d", "c", "-d", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as rkoyqlrv d7d45f05 a1
    Duplicated 5fb83d2b58d6 as zxvrqtmq 44d5cb38 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  44d5cb3876a2   a3
    ○    d7d45f051a3c   a1
    ├─╮
    │ @  5248650c3314   d
    ○ │  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit after a single commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as pzsxstzt a5a114e3 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  3efc0755f6a0   b2
    │ ○  a5a114e3c1ff   a1
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after a single ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a3", "--after", "a1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as qmkrwlvp 4048f2fb a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  d6d86a8bdfda   a4
    │ ○  25c595527054   a3
    │ ○  4b55efbf3237   a2
    │ ○  4048f2fbcf5c   a3
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after a single descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as qwyusntz bf508d28 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  4231eba1accc   a4
    │ ○  bf508d280e4e   a1
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits with no direct
    // relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1", "--after", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as soqnvnyz 7e84ab7f a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  848cd66b5194   c2
    │ │ ○  d68f684471e0   b2
    │ ├─╯
    │ ○    7e84ab7f347e   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits including an ancestor.
    let output = work_dir.run_jj(["duplicate", "a3", "--after", "a2", "--after", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as nsrwusvy d4608971 a3
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  7f51acdb957f   a4
    │ ○  d9b98cc0291c   a3
    │ ○    d46089711a62   a3
    │ ├─╮
    │ │ ○  65b6f1fe6b41   b2
    │ │ ○  6a9343b8797a   b1
    ├───╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit after multiple commits including a descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--after", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as xpnwykqz 5a767b8b (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  1cc19988fa94   a4
    │ ○    5a767b8b010f   a1
    │ ├─╮
    │ │ ○  65b6f1fe6b41   b2
    │ │ ○  6a9343b8797a   b1
    ├───╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as sryyqqkq c5744841 a1
    Duplicated 6a9343b8797a as pxnqtknr 7ed6ac45 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    0c968c7bf23e   c2
    │ ├─╮
    │ │ ○  7ed6ac45ebcf   b1
    │ ○ │  c57448416766   a1
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit which is an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as pyoswmwk 5f4bd02b a3
    Duplicated 6a9343b8797a as yqnpwwmq d0cc3580 b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  d2ac58d2e44a   a4
    │ ○    88d3b0f2dde5   a3
    │ ├─╮
    │ │ ○  d0cc35804628   b1
    │ ○ │  5f4bd02b76d2   a3
    │ ├─╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after a
    // single commit which is a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as tpmlxquz 5ca04404 (empty) a1
    Duplicated 6a9343b8797a as uukzylyy d72c12f0 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○    cee03c20a724   a4
    │ ├─╮
    │ │ ○  d72c12f062ea   b1
    │ ○ │  5ca044045de1   a1
    │ ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--after", "d1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as knltnxnu b1a94bc3 a1
    Duplicated 6a9343b8797a as krtqozmx c43e8c46 b1
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 03190907 d2 | d2
    Parent commit (@-)      : knltnxnu b1a94bc3 a1
    Parent commit (@-)      : krtqozmx c43e8c46 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    03190907af36   d2
    ├─╮
    │ │ ○  44e3521f0e5a   c2
    ╭─┬─╯
    │ ○    c43e8c469e80   b1
    │ ├─╮
    ○ │ │  b1a94bc3e53c   a1
    ╰─┬─╮
      │ ○  ae61a031221a   d1
      ○ │  9b24b49f717e   c1
      ├─╯
    ○ │  65b6f1fe6b41   b2
    ○ │  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a1", "--after", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as wxzmtyol c560d597 a3
    Duplicated 6a9343b8797a as musouqkq 6fb77fdd b1
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    4afd614e7e5d   c2
    │ ├─╮
    │ │ │ ○  651826a78259   a4
    │ │ │ ○  1f1810d0c067   a3
    │ │ │ ○  06062ed2c764   a2
    │ ╭─┬─╯
    │ │ ○    6fb77fdd9246   b1
    │ │ ├─╮
    │ ○ │ │  c560d597e5ad   a3
    │ ╰─┬─╮
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  5d93a4b8f4bd   a1
    ├───╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship after
    // multiple commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as quyylypw 62721448 (empty) a1
    Duplicated 6a9343b8797a as prukwozq adc4a60a b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    1771e0dc1980   a4
    │ ├─╮
    │ │ ○    adc4a60abc7f   b1
    │ │ ├─╮
    │ ○ │ │  62721448a2d3   a1
    │ ╰─┬─╮
    │   │ ○  47a79ab4bbc6   c2
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  5fb83d2b58d6   a3
    │   ○  7bfd9fbe959c   a2
    │   ○  5d93a4b8f4bd   a1
    ├───╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as vvvtksvt 3b8c0610 a1
    Duplicated 5fb83d2b58d6 as yvrnrpnw 065f8c48 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  065f8c485faf   a3
    │ ○  3b8c061087d5   a1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a2", "a3", "--after", "a1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Warning: Duplicating commit 7bfd9fbe959c as an ancestor of itself
    Duplicated 7bfd9fbe959c as sukptuzs 8ea56d6a a2
    Duplicated 5fb83d2b58d6 as rxnrppxl 7dfd4afc a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  75ffab7951db   a4
    │ ○  6b0d57a3b912   a3
    │ ○  d0bf50e30335   a2
    │ ○  7dfd4afcc2b9   a3
    │ ○  8ea56d6ad70b   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after a single
    // descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7bfd9fbe959c as a descendant of itself
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as rwkyzntp 2e22df69 (empty) a1
    Duplicated 7bfd9fbe959c as nqtyztop 4a1cab80 (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  edfcd526983a   a4
    │ ○  4a1cab80c404   a2
    │ ○  2e22df6994f9   a1
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c2", "--after", "d2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as nwmqwkzz d4471259 a1
    Duplicated 5fb83d2b58d6 as uwrrnrtx 2d619dac a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  2d619dacc18a   a3
    ○    d447125943ed   a1
    ├─╮
    │ @  3e122d6a4b70   d2
    │ ○  ae61a031221a   d1
    ○ │  47a79ab4bbc6   c2
    ○ │  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a2", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit e9b68b6313be as an ancestor of itself
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as wunttkrp 2c3a430f a3
    Duplicated e9b68b6313be as puxpuzrm f1705363 a4
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  81dc0bb0c3b8   a4
    │ ○  c56d8a18caca   a3
    │ ○  f17053639e87   a4
    │ ○    2c3a430f6116   a3
    │ ├─╮
    │ │ ○  47a79ab4bbc6   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship after multiple
    // commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7bfd9fbe959c as a descendant of itself
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as zwvplpop 0679c0a3 (empty) a1
    Duplicated 7bfd9fbe959c as znsksvls 4d69ff37 (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  352c4687b8a8   a4
    │ ○  4d69ff370b1c   a2
    │ ○    0679c0a3fcd1   a1
    │ ├─╮
    │ │ ○  47a79ab4bbc6   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b1", "--after", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 65b6f1fe6b41 would be both an ancestor and a descendant of the duplicated commits
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit before a single commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as pzsxstzt a5a114e3 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  3efc0755f6a0   b2
    │ ○  a5a114e3c1ff   a1
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before a single ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as qmkrwlvp 12bdfd40 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  34b04bbb49e6   a4
    │ ○  5c5cd23039e8   a3
    │ ○  66ae7f9cd402   a2
    │ ○  c42e00e5d967   a1
    │ ○  12bdfd4031ac   a3
    ├─╯
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before a single descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as qwyusntz f9b156e7 (empty) a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  4586d15aa692   a4
    │ ○  11f942e98d1b   a3
    │ ○  f9b156e79e8b   a1
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits with no direct
    // relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as soqnvnyz 7e84ab7f a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  848cd66b5194   c2
    │ │ ○  d68f684471e0   b2
    │ ├─╯
    │ ○    7e84ab7f347e   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits including an ancestor.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--before", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as nsrwusvy 1fbb9ed2 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  19cfc3d27f27   b2
    │ │ ○  daf5b2e6bea1   a4
    │ │ ○  436e574b93c8   a3
    │ │ ○  aa581314212c   a2
    │ ├─╯
    │ ○    1fbb9ed2e53b   a3
    │ ├─╮
    │ │ ○  6a9343b8797a   b1
    ├───╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit before multiple commits including a descendant.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "a3", "--before", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as xpnwykqz f829aabe (empty) a1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  43e7fd47784c   b2
    │ │ ○  62300ba27133   a4
    │ │ ○  601308b69f16   a3
    │ ├─╯
    │ ○    f829aabe2f51   a1
    │ ├─╮
    │ │ ○  6a9343b8797a   b1
    ├───╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as sryyqqkq d3570b8d a1
    Duplicated 6a9343b8797a as pxnqtknr a298a26e b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  1c97956e2968   c2
    │ ○    56e6d6b25ef7   c1
    │ ├─╮
    │ │ ○  a298a26ec5c9   b1
    ├───╯
    │ ○  d3570b8d6109   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit which is an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--before", "a2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as pyoswmwk 6d382f0a a3
    Duplicated 6a9343b8797a as yqnpwwmq 7a8b8049 b1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  d73c86fdc91f   a4
    │ ○  06dafd59a24a   a3
    │ ○    536a010d2ac2   a2
    │ ├─╮
    │ │ ○  7a8b80497eb5   b1
    │ ○ │  6d382f0aecca   a3
    │ ├─╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before a
    // single commit which is a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as tpmlxquz 609ecdcd (empty) a1
    Duplicated 6a9343b8797a as uukzylyy b225b93f b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  efd540e1c353   a4
    │ ○    4433cae3e37f   a3
    │ ├─╮
    │ │ ○  b225b93fdf2a   b1
    │ ○ │  609ecdcd4537   a1
    │ ├─╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "c1", "--before", "d1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as knltnxnu 5ff608b7 a1
    Duplicated 6a9343b8797a as krtqozmx d90775d6 b1
    Rebased 4 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 90d9e046 d2 | d2
    Parent commit (@-)      : xznxytkn c97d9dd1 d1 | d1
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  90d9e046049a   d2
    ○    c97d9dd11c08   d1
    ├─╮
    │ │ ○  b26d4b0049cc   c2
    │ │ ○  f4ac74618f17   c1
    ╭─┬─╯
    │ ○  d90775d6f189   b1
    ○ │  5ff608b7ed1c   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--before", "a1", "--before", "c1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as wxzmtyol 589d5def a3
    Duplicated 6a9343b8797a as musouqkq bfc0bdd5 b1
    Rebased 6 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  46eaff09efae   c2
    │ ○    d76157ebc72f   c1
    │ ├─╮
    │ │ │ ○  96a7e7505923   a4
    │ │ │ ○  0e0b1ee222fd   a3
    │ │ │ ○  296a318721f2   a2
    │ │ │ ○  a3bd387e7dbf   a1
    │ ╭─┬─╯
    │ │ ○  bfc0bdd59039   b1
    ├───╯
    │ ○  589d5defb274   a3
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship before
    // multiple commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--before", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as quyylypw 67396c17 (empty) a1
    Duplicated 6a9343b8797a as prukwozq b43f94ae b1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    3805f705f18c   c2
    │ ├─╮
    │ │ │ ○  73ae5483e043   a4
    │ │ │ ○  06954bb8314f   a3
    │ ╭─┬─╯
    │ │ ○    b43f94ae704a   b1
    │ │ ├─╮
    │ ○ │ │  67396c171ca4   a1
    │ ╰─┬─╮
    │   │ ○  9b24b49f717e   c1
    ├─────╯
    │   ○  7bfd9fbe959c   a2
    │   ○  5d93a4b8f4bd   a1
    ├───╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // commit without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as vvvtksvt da397430 a1
    Duplicated 5fb83d2b58d6 as yvrnrpnw 39e64981 a3
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  cf5a6e4b5bde   c2
    │ ○  39e64981a9f3   a3
    │ ○  da397430d587   a1
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // ancestor commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "a1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Warning: Duplicating commit 5d93a4b8f4bd as an ancestor of itself
    Duplicated 5d93a4b8f4bd as sukptuzs c1e67cc4 a1
    Duplicated 5fb83d2b58d6 as rxnrppxl d224a34d a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  21bcc8fa5b2d   a4
    │ ○  3d16228126ce   a3
    │ ○  cbba38177a37   a2
    │ ○  ef99cb80d765   a1
    │ ○  d224a34d206e   a3
    │ ○  c1e67cc44e94   a1
    ├─╯
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before a single
    // descendant commit.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7bfd9fbe959c as a descendant of itself
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as rwkyzntp 8939e563 (empty) a1
    Duplicated 7bfd9fbe959c as nqtyztop 8f0c1f03 (empty) a2
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  29333bd03e54   a4
    │ ○  a3fb0a82f5eb   a3
    │ ○  8f0c1f038c57   a2
    │ ○  8939e563743c   a1
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--before", "c2", "--before", "d2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as nwmqwkzz 0c42d8f6 a1
    Duplicated 5fb83d2b58d6 as uwrrnrtx 5dc4c51b a3
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx dee40354 d2 | d2
    Parent commit (@-)      : uwrrnrtx 5dc4c51b a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  dee40354cc51   d2
    │ ○  84fe86982803   c2
    ├─╯
    ○  5dc4c51b1af5   a3
    ○    0c42d8f65b5f   a1
    ├─╮
    │ ○  ae61a031221a   d1
    ○ │  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits including an ancestor of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--before", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit e9b68b6313be as an ancestor of itself
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as wunttkrp 34d7d114 a3
    Duplicated e9b68b6313be as puxpuzrm f2f4e48c a4
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  90e363bf45c9   c2
    │ │ ○  27b30a1f951d   a4
    │ │ ○  c82aa4009567   a3
    │ │ ○  cb2359418fba   a2
    │ ├─╯
    │ ○  f2f4e48c7b5f   a4
    │ ○    34d7d114f747   a3
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship before multiple
    // commits including a descendant of one of the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7bfd9fbe959c as a descendant of itself
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as zwvplpop 8028cd9b (empty) a1
    Duplicated 7bfd9fbe959c as znsksvls 2c7805ae (empty) a2
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a3ddc184c2   c2
    │ │ ○  9514cf056d7f   a4
    │ │ ○  c3caf7b0f6aa   a3
    │ ├─╯
    │ ○  2c7805ae55e6   a2
    │ ○    8028cd9b91ae   a1
    │ ├─╮
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b1", "--before", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 6a9343b8797a would be both an ancestor and a descendant of the duplicated commits
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    // Duplicate a single commit in between commits with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--before", "b2", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as pzsxstzt d144734f a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    e0b0802624fc   b2
    │ ├─╮
    │ │ ○  d144734f7749   a1
    │ │ ○  47a79ab4bbc6   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between ancestor commits.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--after", "a1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as qmkrwlvp 4048f2fb a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  d6d86a8bdfda   a4
    │ ○  25c595527054   a3
    │ ○  4b55efbf3237   a2
    │ ○  4048f2fbcf5c   a3
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between an ancestor commit and a commit with no
    // direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "--before", "a2", "--after", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as qwyusntz 5bcb3c14 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  332ac36618b3   a4
    │ ○  5d1795632655   a3
    │ ○    9852ab50af53   a2
    │ ├─╮
    │ │ ○  5bcb3c14e204   a3
    │ │ ○  65b6f1fe6b41   b2
    │ │ ○  6a9343b8797a   b1
    ├───╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between descendant commits.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--before", "a4"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as soqnvnyz 339f4932 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e0b17484318d   a4
    │ ○  339f4932b49e   a1
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between a descendant commit and a commit with no
    // direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "a3", "--before", "b2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as nsrwusvy 26bc52e1 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○    f5c84c8e017a   b2
    │ ├─╮
    │ │ ○  26bc52e1184c   a1
    │ ○ │  6a9343b8797a   b1
    ├─╯ │
    │ ○ │  e9b68b6313be   a4
    │ ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate a single commit in between an ancestor commit and a descendant
    // commit.
    let output = work_dir.run_jj(["duplicate", "a2", "--after", "a1", "--before", "a4"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7bfd9fbe959c as xpnwykqz 26961c35 a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○    3bd6eedeb5d3   a4
    │ ├─╮
    │ │ ○  26961c351338   a2
    │ ○ │  5fb83d2b58d6   a3
    │ ○ │  7bfd9fbe959c   a2
    │ ├─╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between
    // commits without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as sryyqqkq c5744841 a1
    Duplicated 6a9343b8797a as pxnqtknr 7ed6ac45 b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx c786fa10 d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : sryyqqkq c5744841 a1
    Parent commit (@-)      : pxnqtknr 7ed6ac45 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      c786fa10d5b9   d2
    ├─┬─╮
    │ │ ○  7ed6ac45ebcf   b1
    │ ○ │  c57448416766   a1
    │ ├─╯
    ○ │  ae61a031221a   d1
    │ │ ○  47a79ab4bbc6   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between a
    // commit which is an ancestor of one of the duplicated commits and a commit
    // with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "b1", "--after", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5fb83d2b58d6 as pyoswmwk 5f4bd02b a3
    Duplicated 6a9343b8797a as yqnpwwmq d0cc3580 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○      2adae1b5ccbf   c2
    │ ├─┬─╮
    │ │ │ ○  d0cc35804628   b1
    │ │ ○ │  5f4bd02b76d2   a3
    │ │ ├─╯
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  65b6f1fe6b41   b2
    │ ○ │  6a9343b8797a   b1
    ├─╯ │
    │ ○ │  e9b68b6313be   a4
    │ ○ │  5fb83d2b58d6   a3
    │ ├─╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between a
    // commit which is a descendant of one of the duplicated commits and a
    // commit with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "a3", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as tpmlxquz 5ca04404 (empty) a1
    Duplicated 6a9343b8797a as uukzylyy d72c12f0 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○      05d64557cccd   c2
    │ ├─┬─╮
    │ │ │ ○  d72c12f062ea   b1
    │ │ ○ │  5ca044045de1   a1
    │ │ ├─╯
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  65b6f1fe6b41   b2
    │ ○ │  6a9343b8797a   b1
    ├─╯ │
    │ ○ │  e9b68b6313be   a4
    │ ├─╯
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits without a direct ancestry relationship between
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "b1", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as knltnxnu e6b0ec9f a1
    Duplicated 6a9343b8797a as krtqozmx 7acc5250 b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 1fdeca3e d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : knltnxnu e6b0ec9f a1
    Parent commit (@-)      : krtqozmx 7acc5250 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      1fdeca3ece59   d2
    ├─┬─╮
    │ │ ○  7acc52508f09   b1
    │ ○ │  e6b0ec9f7a4d   a1
    │ ├─╯
    ○ │  ae61a031221a   d1
    │ │ ○  47a79ab4bbc6   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between
    // commits without a direct relationship to the duplicated commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "--after", "c1", "--before", "d2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as wxzmtyol 09fb22cd a1
    Duplicated 5fb83d2b58d6 as musouqkq 12fcbea6 a3
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 2a7ef48b d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : musouqkq 12fcbea6 a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    2a7ef48b82d2   d2
    ├─╮
    │ ○  12fcbea66d19   a3
    │ ○  09fb22cdc78e   a1
    ○ │  ae61a031221a   d1
    │ │ ○  47a79ab4bbc6   c2
    │ ├─╯
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  e9b68b6313be   a4
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between a commit
    // which is an ancestor of one of the duplicated commits and a commit
    // without a direct relationship.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a2", "--before", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5fb83d2b58d6 as quyylypw 1e04ee7b a3
    Duplicated e9b68b6313be as prukwozq f86d4e2b a4
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    19896c7c8a53   c2
    │ ├─╮
    │ │ ○  f86d4e2b2759   a4
    │ │ ○  1e04ee7badf2   a3
    │ ○ │  9b24b49f717e   c1
    ├─╯ │
    │ ○ │  65b6f1fe6b41   b2
    │ ○ │  6a9343b8797a   b1
    ├─╯ │
    │ ○ │  e9b68b6313be   a4
    │ ○ │  5fb83d2b58d6   a3
    │ ├─╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between a commit
    // which is a a descendant of one of the duplicated commits and a commit
    // with no direct relationship.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--before", "a3", "--after", "c2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as vvvtksvt 3b8c0610 a1
    Duplicated 7bfd9fbe959c as yvrnrpnw ff33d0be a2
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  fbedc3581fd2   a4
    │ ○    724ff9ca6f25   a3
    │ ├─╮
    │ │ ○  ff33d0be8372   a2
    │ │ ○  3b8c061087d5   a1
    │ │ ○  47a79ab4bbc6   c2
    │ │ ○  9b24b49f717e   c1
    ├───╯
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between descendant
    // commits.
    let output = work_dir.run_jj(["duplicate", "a3", "a4", "--after", "a1", "--before", "a2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit e9b68b6313be as an ancestor of itself
    Warning: Duplicating commit 5fb83d2b58d6 as an ancestor of itself
    Duplicated 5fb83d2b58d6 as sukptuzs fcd8320b a3
    Duplicated e9b68b6313be as rxnrppxl 606e5abb a4
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  66a8bcd2e874   a4
    │ ○  19653d63dcb6   a3
    │ ○  675587f177c2   a2
    │ ○  606e5abbb07b   a4
    │ ○  fcd8320b69d9   a3
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between ancestor
    // commits.
    let output = work_dir.run_jj(["duplicate", "a1", "a2", "--after", "a3", "--before", "a4"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7bfd9fbe959c as a descendant of itself
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as rwkyzntp 2e22df69 (empty) a1
    Duplicated 7bfd9fbe959c as nqtyztop 4a1cab80 (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○  edfcd526983a   a4
    │ ○  4a1cab80c404   a2
    │ ○  2e22df6994f9   a1
    │ ○  5fb83d2b58d6   a3
    │ ○  7bfd9fbe959c   a2
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Duplicate multiple commits with an ancestry relationship between an ancestor
    // commit and a descendant commit.
    let output = work_dir.run_jj(["duplicate", "a2", "a3", "--after", "a1", "--before", "a4"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7bfd9fbe959c as nwmqwkzz 34a2b296 a2
    Duplicated 5fb83d2b58d6 as uwrrnrtx a9fed94a a3
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  65b6f1fe6b41   b2
    │ ○  6a9343b8797a   b1
    ├─╯
    │ ○    a7d82e5f8a74   a4
    │ ├─╮
    │ │ ○  a9fed94ac9fe   a3
    │ │ ○  34a2b296eced   a2
    │ ○ │  5fb83d2b58d6   a3
    │ ○ │  7bfd9fbe959c   a2
    │ ├─╯
    │ ○  5d93a4b8f4bd   a1
    ├─╯
    ◆  000000000000
    [EOF]
    ");
    work_dir.run_jj(["op", "restore", &setup_opid]).success();

    // Should error if a loop will be created.
    let output = work_dir.run_jj(["duplicate", "a1", "--after", "b2", "--before", "b1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to create a loop: commit 65b6f1fe6b41 would be both an ancestor and a descendant of the duplicated commits
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as mzvwutvl 346a7abe a
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7d980be7a1d4   a
    │ ○  346a7abed73c   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Restored to operation: 73a36404358e (2001-02-03 08:05:09) create bookmark a pointing to commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  7d980be7a1d4   a
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
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @r"
    @  dffaa0d4dacc   c @ 2001-02-03 04:05:13.000 +07:00
    ○  123b4d91f6e5   b @ 2001-02-03 04:05:11.000 +07:00
    ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated dffaa0d4dacc as yostqsxw fc2e8dc2 c
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated dffaa0d4dacc as znkkpsqq 14e2803a c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @r"
    @  dffaa0d4dacc   c @ 2001-02-03 04:05:13.000 +07:00
    │ ○  14e2803a4b0e   c @ 2001-02-03 04:05:16.000 +07:00
    ├─╯
    │ ○  fc2e8dc218ab   c @ 2001-02-03 04:05:15.000 +07:00
    ├─╯
    ○  123b4d91f6e5   b @ 2001-02-03 04:05:11.000 +07:00
    ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-s", "b", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 4 commits to destination
    Working copy  (@) now at: royxmykx fa60711d c | c
    Parent commit (@-)      : zsuskuln 594e9d32 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Some of the duplicate commits' timestamps were changed a little to make them
    // have distinct commit ids.
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @r"
    @  fa60711d6bd1   c @ 2001-02-03 04:05:18.000 +07:00
    │ ○  e320e3d23be0   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    │ ○  f9c10a3b2cfd   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    ○  594e9d322230   b @ 2001-02-03 04:05:18.000 +07:00
    │ ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
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
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  dffaa0d4dacc   c
    ○  123b4d91f6e5   b
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    // Test duplicate_commits()
    test_env.add_config(r#"templates.duplicate_description = "concat(description, '\n(cherry picked from commit ', commit_id, ')')""#);
    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as yostqsxw d6f0812f a
    [EOF]
    ");

    // Test duplicate_commits_onto_parents()
    let output = work_dir.run_jj(["duplicate", "a", "-B", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 7d980be7a1d4 as a descendant of itself
    Duplicated 7d980be7a1d4 as znkkpsqq fe35e4a3 (empty) a
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: royxmykx c414af3f c | c
    Parent commit (@-)      : zsuskuln 6960bbcf b | b
    [EOF]
    ");

    // Test empty template
    test_env.add_config("templates.duplicate_description = ''");
    let output = work_dir.run_jj(["duplicate", "b", "-d", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 6960bbcfd5b1 as kpqxywon 33044659 (no description set)
    [EOF]
    ");

    // Test `description` as an alias
    test_env.add_config("templates.duplicate_description = 'description'");
    let output = work_dir.run_jj([
        "duplicate",
        "c",
        "-d",
        "root()",
        // Use an argument here so we can actually see the log in the last test
        // (We don't have a way to remove a config in TestEnvironment)
        "--config",
        "template-aliases.description='\"alias\"'",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated c414af3f8d2f as kmkuslsw 3eb4e721 alias
    [EOF]
    ");

    let template = r#"commit_id.short() ++ "\n" ++ description"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  c414af3f8d2f
    │  c
    ○  6960bbcfd5b1
    │  b
    ○  fe35e4a3bf3a
    │  a
    │
    │  (cherry picked from commit 7d980be7a1d499e4d316ab4c01242885032f7eaf)
    ○  7d980be7a1d4
    │  a
    │ ○  3eb4e7210ce7
    ├─╯  alias
    │ ○  33044659b895
    ├─╯
    │ ○  d6f0812febab
    ├─╯  a
    │
    │    (cherry picked from commit 7d980be7a1d499e4d316ab4c01242885032f7eaf)
    ◆  000000000000
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
