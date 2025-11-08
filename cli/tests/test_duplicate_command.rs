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
    let setup_opid = work_dir.current_operation_id();

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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
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

    work_dir.run_jj(["op", "restore", &setup_opid]).success();
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
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
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
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as kxryzmor 08f0f980 a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  08f0f980b8ad   a1
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
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as xznxytkn 0bad88fb a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    0bad88fbab2e   a1
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
    let output = work_dir.run_jj(["duplicate", "a1", "-o", "a3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: Duplicating commit 5d93a4b8f4bd as a descendant of itself
    Duplicated 5d93a4b8f4bd as tlkvzzqu 8351ae70 (empty) a1
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  22370aa928dc   c
    ├─╯
    │ ○  c406dbab05ac   b
    ├─╯
    │ ○  8351ae7027e4   a1
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
    let output = work_dir.run_jj(["duplicate", "-r=a1", "-r=b", "-o", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as pzsxstzt 663f101c a1
    Duplicated c406dbab05ac as nxkxtmvy b8a86167 b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  b8a8616787a4   b
    │ │ ○  663f101c476a   a1
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
    let output = work_dir.run_jj(["duplicate", "-r=a1", "b", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as qmkrwlvp b0f531b0 a1
    Duplicated c406dbab05ac as pkqrwoqq 0df6b6b2 b
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○    0df6b6b21bfe   b
    ├─╮
    │ │ ○  b0f531b01b33   a1
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
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-o", "c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as qwyusntz 5f1c245f a1
    Duplicated 5fb83d2b58d6 as pwpvvyov 2a209b65 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  5248650c3314   d
    │ ○  2a209b658cd6   a3
    │ ○  5f1c245fb9ae   a1
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
    let output = work_dir.run_jj(["duplicate", "a1", "a3", "-o", "c", "-o", "d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Duplicated 5d93a4b8f4bd as soqnvnyz bc6678fd a1
    Duplicated 5fb83d2b58d6 as nmmmqslz a35b7e76 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  a35b7e76b4dd   a3
    ○    bc6678fdf2a5   a1
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
    Duplicated 5d93a4b8f4bd as nlrtlrxv 52959024 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  8124402d0ebe   b2
    │ ○  52959024d93a   a1
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
    Duplicated 5fb83d2b58d6 as uuuvxpvw f626655e a3
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
    │ ○  cfea8bb13adf   a4
    │ ○  3a595c648b6d   a3
    │ ○  307ab42af890   a2
    │ ○  f626655ef3dd   a3
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
    Duplicated 5d93a4b8f4bd as pkstwlsy 610e8ba1 (empty) a1
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
    │ ○  f8077bd38256   a4
    │ ○  610e8ba113f6   a1
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
    Duplicated 5d93a4b8f4bd as zowrlwsv a78c25cc a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  4150b5a5466e   c2
    │ │ ○  4a1f33498a39   b2
    │ ├─╯
    │ ○    a78c25cc7d58   a1
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
    Duplicated 5fb83d2b58d6 as wvmqtotl 0824a995 a3
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  a916f82ba67e   a4
    │ ○  3568f02a532c   a3
    │ ○    0824a99549cc   a3
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
    Duplicated 5d93a4b8f4bd as opwsxtwu 7548fb00 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  784c89a5ba72   a4
    │ ○    7548fb00a50a   a1
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
    Duplicated 5d93a4b8f4bd as ukwxllxp 0cedc1c7 a1
    Duplicated 6a9343b8797a as yrwmsomt 0d18a4ba b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    133e5ef54093   c2
    │ ├─╮
    │ │ ○  0d18a4ba8860   b1
    │ ○ │  0cedc1c7f5dc   a1
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
    Duplicated 5fb83d2b58d6 as szrrkvty 5ea1ddf1 a3
    Duplicated 6a9343b8797a as wvmrymqu 8081d164 b1
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
    │ ○  e5be4d6a351f   a4
    │ ○    d6fe9e37ad2e   a3
    │ ├─╮
    │ │ ○  8081d1648811   b1
    │ ○ │  5ea1ddf11f02   a3
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
    Duplicated 5d93a4b8f4bd as ztnvrxlv 896deede (empty) a1
    Duplicated 6a9343b8797a as upuzqpxs ad048f81 b1
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
    │ ○    87ec7e281ad9   a4
    │ ├─╮
    │ │ ○  ad048f810d3c   b1
    │ ○ │  896deede3c03   a1
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
    Duplicated 5d93a4b8f4bd as muymlknp 7db83f0f a1
    Duplicated 6a9343b8797a as snrzyvry d10bd4dd b1
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 57d2a947 d2 | d2
    Parent commit (@-)      : muymlknp 7db83f0f a1
    Parent commit (@-)      : snrzyvry d10bd4dd b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    57d2a947c305   d2
    ├─╮
    │ │ ○  aca0050dbbc4   c2
    ╭─┬─╯
    │ ○    d10bd4ddb880   b1
    │ ├─╮
    ○ │ │  7db83f0f554b   a1
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
    Duplicated 5fb83d2b58d6 as vnqwxmpr 749e7782 a3
    Duplicated 6a9343b8797a as pvqonzsn cdc6ab27 b1
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    77dbe4a4ab86   c2
    │ ├─╮
    │ │ │ ○  52a5fbe4cfe6   a4
    │ │ │ ○  87a6779455c5   a3
    │ │ │ ○  1df3435c91be   a2
    │ ╭─┬─╯
    │ │ ○    cdc6ab279cb7   b1
    │ │ ├─╮
    │ ○ │ │  749e778247d3   a3
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
    Duplicated 5d93a4b8f4bd as qtvkyytt e9ac46df (empty) a1
    Duplicated 6a9343b8797a as ouvslmur bbf796e8 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    ecaa1f7b16fb   a4
    │ ├─╮
    │ │ ○    bbf796e8f265   b1
    │ │ ├─╮
    │ ○ │ │  e9ac46df3691   a1
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
    Duplicated 5d93a4b8f4bd as qowqnpnw 0478473b a1
    Duplicated 5fb83d2b58d6 as mommxqln 6175d88f a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  6175d88f3848   a3
    │ ○  0478473bfff3   a1
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
    Duplicated 7bfd9fbe959c as qzusktlu abac3b29 a2
    Duplicated 5fb83d2b58d6 as zryotxso 0ac7bc72 a3
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
    │ ○  e6b5faccb8d6   a4
    │ ○  1b0489e81cdf   a3
    │ ○  91cc24eeda92   a2
    │ ○  0ac7bc72e563   a3
    │ ○  abac3b29eace   a2
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
    Duplicated 5d93a4b8f4bd as stzvpxow 607f49d4 (empty) a1
    Duplicated 7bfd9fbe959c as zrzsnomp 802bec72 (empty) a2
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
    │ ○  5968d4b950df   a4
    │ ○  802bec723045   a2
    │ ○  607f49d4e2dd   a1
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
    Duplicated 5d93a4b8f4bd as ysllonyo 24a4e9f8 a1
    Duplicated 5fb83d2b58d6 as kzxwzvzw 64d6f104 a3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  64d6f1045b10   a3
    ○    24a4e9f817c7   a1
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
    Duplicated 5fb83d2b58d6 as kvqpkqvl 59583d1b a3
    Duplicated e9b68b6313be as zqztuxrl b327e326 a4
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  c6d033e3dbae   a4
    │ ○  969593962a7c   a3
    │ ○  b327e326e151   a4
    │ ○    59583d1b591c   a3
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
    Duplicated 5d93a4b8f4bd as xsvtwpuq 579fa109 (empty) a1
    Duplicated 7bfd9fbe959c as tmzzmpyp 995ffc29 (empty) a2
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  a462f9946f62   a4
    │ ○  995ffc29e76b   a2
    │ ○    579fa1093ceb   a1
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
    Duplicated 5d93a4b8f4bd as nlrtlrxv 52959024 a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  8124402d0ebe   b2
    │ ○  52959024d93a   a1
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
    Duplicated 5fb83d2b58d6 as uuuvxpvw cbb38dd4 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  6d36ca1b1215   a4
    │ ○  ec479e2b5c1c   a3
    │ ○  54f79ca85067   a2
    │ ○  34db3229bbf9   a1
    │ ○  cbb38dd4f677   a3
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
    Duplicated 5d93a4b8f4bd as pkstwlsy e6dcd064 (empty) a1
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
    │ ○  88ed72c9e2cd   a4
    │ ○  bf1cebc7b328   a3
    │ ○  e6dcd064caba   a1
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
    Duplicated 5d93a4b8f4bd as zowrlwsv a78c25cc a1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  4150b5a5466e   c2
    │ │ ○  4a1f33498a39   b2
    │ ├─╯
    │ ○    a78c25cc7d58   a1
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
    Duplicated 5fb83d2b58d6 as wvmqtotl 388c8d9d a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  a7c33fa6bdd2   b2
    │ │ ○  4a4c76b85974   a4
    │ │ ○  6e519ad5a603   a3
    │ │ ○  40bb4cf3aecf   a2
    │ ├─╯
    │ ○    388c8d9d7b49   a3
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
    Duplicated 5d93a4b8f4bd as opwsxtwu 644afaf1 (empty) a1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  00f96f926fd4   b2
    │ │ ○  b0d1638032b2   a4
    │ │ ○  f2d014886a49   a3
    │ ├─╯
    │ ○    644afaf17a49   a1
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
    Duplicated 5d93a4b8f4bd as ukwxllxp 3323f9c3 a1
    Duplicated 6a9343b8797a as yrwmsomt a6ef0369 b1
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  a0d7984ef9ce   c2
    │ ○    2d5211a7d52f   c1
    │ ├─╮
    │ │ ○  a6ef03692b89   b1
    ├───╯
    │ ○  3323f9c396f8   a1
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
    Duplicated 5fb83d2b58d6 as szrrkvty fa34ba15 a3
    Duplicated 6a9343b8797a as wvmrymqu dd6d65a2 b1
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
    │ ○  0f4182479035   a4
    │ ○  2ce96f99ba9a   a3
    │ ○    18eda16173aa   a2
    │ ├─╮
    │ │ ○  dd6d65a2db44   b1
    │ ○ │  fa34ba15a5cc   a3
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
    Duplicated 5d93a4b8f4bd as ztnvrxlv 4131a4c1 (empty) a1
    Duplicated 6a9343b8797a as upuzqpxs 2a4ce4e3 b1
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
    │ ○  cf945b2bc8af   a4
    │ ○    6f86e945fda5   a3
    │ ├─╮
    │ │ ○  2a4ce4e3aebb   b1
    │ ○ │  4131a4c1003c   a1
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
    Duplicated 5d93a4b8f4bd as muymlknp 9add628e a1
    Duplicated 6a9343b8797a as snrzyvry b63fdd54 b1
    Rebased 4 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 1aec68e6 d2 | d2
    Parent commit (@-)      : xznxytkn 3b2ee7ee d1 | d1
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  1aec68e662c2   d2
    ○    3b2ee7eeb0c0   d1
    ├─╮
    │ │ ○  a2adf9d2849d   c2
    │ │ ○  fdc43aa17cf1   c1
    ╭─┬─╯
    │ ○  b63fdd54c3f9   b1
    ○ │  9add628e2f94   a1
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
    Duplicated 5fb83d2b58d6 as vnqwxmpr 73f3594f a3
    Duplicated 6a9343b8797a as pvqonzsn 67d4a940 b1
    Rebased 6 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  12da11313262   c2
    │ ○    dbb8e01766cc   c1
    │ ├─╮
    │ │ │ ○  622c81500218   a4
    │ │ │ ○  ae205a70138e   a3
    │ │ │ ○  9e97d48369cd   a2
    │ │ │ ○  22afb4fd6d02   a1
    │ ╭─┬─╯
    │ │ ○  67d4a9404b6b   b1
    ├───╯
    │ ○  73f3594f082f   a3
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
    Duplicated 5d93a4b8f4bd as qtvkyytt 2a37f838 (empty) a1
    Duplicated 6a9343b8797a as ouvslmur f2337ef6 b1
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    7562404a5fe2   c2
    │ ├─╮
    │ │ │ ○  116b92e839d4   a4
    │ │ │ ○  b814e7dd0b6d   a3
    │ ╭─┬─╯
    │ │ ○    f2337ef6d7af   b1
    │ │ ├─╮
    │ ○ │ │  2a37f83808d9   a1
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
    Duplicated 5d93a4b8f4bd as qowqnpnw 1cd05f49 a1
    Duplicated 5fb83d2b58d6 as mommxqln 51d00be6 a3
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  7d22145dee97   c2
    │ ○  51d00be616d6   a3
    │ ○  1cd05f491a63   a1
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
    Duplicated 5d93a4b8f4bd as qzusktlu d8aaed30 a1
    Duplicated 5fb83d2b58d6 as zryotxso 701cf123 a3
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  d44c70bd7777   a4
    │ ○  4898b1b61699   a3
    │ ○  c10b845b6926   a2
    │ ○  848ddba9889a   a1
    │ ○  701cf1238e40   a3
    │ ○  d8aaed30e073   a1
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
    Duplicated 5d93a4b8f4bd as stzvpxow ced08d1e (empty) a1
    Duplicated 7bfd9fbe959c as zrzsnomp fe7136d1 (empty) a2
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
    │ ○  f83372ee5f96   a4
    │ ○  aac435e6dffb   a3
    │ ○  fe7136d10a72   a2
    │ ○  ced08d1e6dd9   a1
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
    Duplicated 5d93a4b8f4bd as ysllonyo b9b23a94 a1
    Duplicated 5fb83d2b58d6 as kzxwzvzw 22d5f430 a3
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx b00d1c06 d2 | d2
    Parent commit (@-)      : kzxwzvzw 22d5f430 a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  b00d1c0624f9   d2
    │ ○  bbc756f1d839   c2
    ├─╯
    ○  22d5f4304b47   a3
    ○    b9b23a944c2a   a1
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
    Duplicated 5fb83d2b58d6 as kvqpkqvl 469d73d0 a3
    Duplicated e9b68b6313be as zqztuxrl 1146a57a a4
    Rebased 4 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  93291babd184   c2
    │ │ ○  83bce95a4e22   a4
    │ │ ○  26314519c960   a3
    │ │ ○  0d0144d0c427   a2
    │ ├─╯
    │ ○  1146a57afb7c   a4
    │ ○    469d73d08c1b   a3
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
    Duplicated 5d93a4b8f4bd as xsvtwpuq b166c219 (empty) a1
    Duplicated 7bfd9fbe959c as tmzzmpyp 111a9c6e (empty) a2
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  f1c1d4b44d61   c2
    │ │ ○  ffef7620a964   a4
    │ │ ○  9b06af9a8d57   a3
    │ ├─╯
    │ ○  111a9c6ec9da   a2
    │ ○    b166c219ae16   a1
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
    Duplicated 5d93a4b8f4bd as nlrtlrxv 16aac4ae a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    b0611efaa0d5   b2
    │ ├─╮
    │ │ ○  16aac4aec6de   a1
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
    Duplicated 5fb83d2b58d6 as uuuvxpvw f626655e a3
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
    │ ○  cfea8bb13adf   a4
    │ ○  3a595c648b6d   a3
    │ ○  307ab42af890   a2
    │ ○  f626655ef3dd   a3
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
    Duplicated 5fb83d2b58d6 as pkstwlsy 99b4ea10 a3
    Rebased 3 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○  8671cfa6437d   a4
    │ ○  b7e2ccc4ab64   a3
    │ ○    44b6be52de17   a2
    │ ├─╮
    │ │ ○  99b4ea109aff   a3
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
    Duplicated 5d93a4b8f4bd as zowrlwsv 5ba52649 (empty) a1
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
    │ ○  2ad2abd6ed40   a4
    │ ○  5ba52649f9f6   a1
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
    Duplicated 5d93a4b8f4bd as wvmqtotl ef1e2f46 (empty) a1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  47a79ab4bbc6   c2
    │ ○  9b24b49f717e   c1
    ├─╯
    │ ○    e9701b5153e5   b2
    │ ├─╮
    │ │ ○  ef1e2f461427   a1
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
    Duplicated 7bfd9fbe959c as opwsxtwu c9d7dee9 a2
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
    │ ○    5249308b5df2   a4
    │ ├─╮
    │ │ ○  c9d7dee9b730   a2
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
    Duplicated 5d93a4b8f4bd as ukwxllxp 0cedc1c7 a1
    Duplicated 6a9343b8797a as yrwmsomt 0d18a4ba b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 4aafd744 d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : ukwxllxp 0cedc1c7 a1
    Parent commit (@-)      : yrwmsomt 0d18a4ba b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      4aafd7445a66   d2
    ├─┬─╮
    │ │ ○  0d18a4ba8860   b1
    │ ○ │  0cedc1c7f5dc   a1
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
    Duplicated 5fb83d2b58d6 as szrrkvty 5ea1ddf1 a3
    Duplicated 6a9343b8797a as wvmrymqu 8081d164 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○      410b365b142e   c2
    │ ├─┬─╮
    │ │ │ ○  8081d1648811   b1
    │ │ ○ │  5ea1ddf11f02   a3
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
    Duplicated 5d93a4b8f4bd as ztnvrxlv 896deede (empty) a1
    Duplicated 6a9343b8797a as upuzqpxs ad048f81 b1
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○      e5e35bfebdf4   c2
    │ ├─┬─╮
    │ │ │ ○  ad048f810d3c   b1
    │ │ ○ │  896deede3c03   a1
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
    Duplicated 5d93a4b8f4bd as muymlknp e3890eb5 a1
    Duplicated 6a9343b8797a as snrzyvry d3066453 b1
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx 12e787a7 d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : muymlknp e3890eb5 a1
    Parent commit (@-)      : snrzyvry d3066453 b1
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      12e787a7acb9   d2
    ├─┬─╮
    │ │ ○  d30664539118   b1
    │ ○ │  e3890eb5520e   a1
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
    Duplicated 5d93a4b8f4bd as vnqwxmpr 0a6ab30c a1
    Duplicated 5fb83d2b58d6 as pvqonzsn eb5c8329 a3
    Rebased 1 commits onto duplicated commits
    Working copy  (@) now at: nmzmmopx e3fab709 d2 | d2
    Parent commit (@-)      : xznxytkn ae61a031 d1 | d1
    Parent commit (@-)      : pvqonzsn eb5c8329 a3
    Added 3 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    e3fab7093036   d2
    ├─╮
    │ ○  eb5c832980d2   a3
    │ ○  0a6ab30c6a03   a1
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
    Duplicated 5fb83d2b58d6 as qtvkyytt 92720226 a3
    Duplicated e9b68b6313be as ouvslmur c565fb9e a4
    Rebased 1 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○    45da82514fef   c2
    │ ├─╮
    │ │ ○  c565fb9eec0b   a4
    │ │ ○  927202268f69   a3
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
    Duplicated 5d93a4b8f4bd as qowqnpnw 0478473b a1
    Duplicated 7bfd9fbe959c as mommxqln fd58dece a2
    Rebased 2 commits onto duplicated commits
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  3e122d6a4b70   d2
    ○  ae61a031221a   d1
    │ ○  58ed94dc9684   a4
    │ ○    678cebe68e8a   a3
    │ ├─╮
    │ │ ○  fd58dece7c4f   a2
    │ │ ○  0478473bfff3   a1
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
    Duplicated 5fb83d2b58d6 as qzusktlu bfad21d4 a3
    Duplicated e9b68b6313be as zryotxso a83e9a5d a4
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
    │ ○  b88510005789   a4
    │ ○  55fd608caaf7   a3
    │ ○  d375a67f0929   a2
    │ ○  a83e9a5d4771   a4
    │ ○  bfad21d45a87   a3
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
    Duplicated 5d93a4b8f4bd as stzvpxow 607f49d4 (empty) a1
    Duplicated 7bfd9fbe959c as zrzsnomp 802bec72 (empty) a2
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
    │ ○  5968d4b950df   a4
    │ ○  802bec723045   a2
    │ ○  607f49d4e2dd   a1
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
    Duplicated 7bfd9fbe959c as ysllonyo b3b3e9e3 a2
    Duplicated 5fb83d2b58d6 as kzxwzvzw b5e1d0bf a3
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
    │ ○    90e48c697af8   a4
    │ ├─╮
    │ │ ○  b5e1d0bf2f7a   a3
    │ │ ○  b3b3e9e342b9   a2
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

    // exercise --quiet while here
    let output = work_dir.run_jj(["duplicate", "a", "--quiet"]);
    insta::assert_snapshot!(output, @"");
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

    let output = work_dir.run_jj(["rebase", "-s", "b", "-o", "root()"]);
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
    let output = work_dir.run_jj(["duplicate", "b", "-o", "root()"]);
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
        "-o",
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
