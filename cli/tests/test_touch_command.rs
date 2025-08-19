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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_touch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");
    // Test the setup
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: 22be6c4e01da7039a1a8c3adb91b8841252bb354
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 75591b1896b4990e7695701fd7cdbb32dba3ff50
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:11.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Touch the commit (and its descendants)
    work_dir.run_jj(["touch", "kkmpptxzrspx"]).success();
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: b396a5373e525bd9b322cab64c65f5f67ece81e7
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:14.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 53f5eea6f1d793859d38f1299ff10ebfb67d0a23
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:14.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");

    // Update author
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "touch",
            "--config=user.name=Ove Ridder",
            "--config=user.email=ove.ridder@example.com",
            "--update-author",
            "kkmpptxzrspx",
        ])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: 6f31b2555777ac2261dd17008b6fdc42619ebe1f
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:17.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 590c8b6945666401d01269190c1b82cd3311a0cd
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:17.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");

    // Update author timestamp
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj(["touch", "--update-author-timestamp", "kkmpptxzrspx"])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: b23f6a3f160d122f8d8dacd8d2acff2d29d5ba84
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:20.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: f121a0fb72e1790e4116b2e3b6989c795ac7f74b
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:20.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:20.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");

    // Set author
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "touch",
            "--author",
            "Alice <alice@example.com>",
            "kkmpptxzrspx",
        ])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: 74007c679b9e4f13d1e3d553ef8397586b033421
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:23.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: d070c8adbc590813c81e296591d6b2cac8f3bb41
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Alice <alice@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:23.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");
}

#[test]
fn test_new_change_id() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();
    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();
    work_dir.write_file("file1", "c\n");

    let output = work_dir.run_jj(["touch", "--update-change-id", "kkmpptxzrspx"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Touched 1 commits:
      yqosqzyt 01d6741e b | (no description set)
    Rebased 1 descendant commits
    Working copy  (@) now at: mzvwutvl 0c3fe2d8 c | (no description set)
    Parent commit (@-)      : yqosqzyt 01d6741e b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: 0c3fe2d854b2b492a053156a505d6c40fe783138
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 01d6741ed708318bcd5911320237066db4b63b53
    │  Change ID: yqosqzytrlswkspswpqrmlplxylrzsnz
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:09.000 +07:00)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r", "yqosqzytrlswkspswpqrmlplxylrzsnz"]), @r"
    ○  yqosqzyt test.user@example.com 2001-02-03 08:05:13 b 01d6741e
    │  (no description set)
    │  -- operation c1ee90a05107 touch commit 75591b1896b4990e7695701fd7cdbb32dba3ff50
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:11 75591b18
    │  (no description set)
    │  -- operation 4b33c26502f8 snapshot working copy
    ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 acebf2bd
       (empty) (no description set)
       -- operation 686c6e44c08d new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r", "mzvwut"]), @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:13 c 0c3fe2d8
    │  (no description set)
    │  -- operation c1ee90a05107 touch commit 75591b1896b4990e7695701fd7cdbb32dba3ff50
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:13 22be6c4e
    │  (no description set)
    │  -- operation 6cd93c8c2f48 snapshot working copy
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 b9f5490a
       (empty) (no description set)
       -- operation e3fbc5040416 new empty commit
    [EOF]
    ");
}

#[test]
fn test_squash_option_mutual_exclusion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["describe", "-m=b"]).success();
    insta::assert_snapshot!(work_dir.run_jj([
        "touch",
        "--author=Alice <alice@example.com>",
        "--update-author",
    ]), @r"
    ------- stderr -------
    error: the argument '--author <AUTHOR>' cannot be used with '--update-author'

    Usage: jj touch --author <AUTHOR> [REVSETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[must_use]
fn get_log(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj([
        "--config",
        "template-aliases.'format_timestamp(t)'='t'",
        "log",
        "-T",
        "builtin_log_detailed",
    ])
}
