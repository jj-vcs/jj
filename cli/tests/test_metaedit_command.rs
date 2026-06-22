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
fn test_metaedit() {
    let mut test_env = TestEnvironment::default();
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
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 1cc10709fbc078f11bacb8640a3dc11a41ec8221
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 099a44efcab4d977ed5b622bf030b3de2b59fcef
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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

    // Without arguments, the commits are not rewritten.
    // TODO: Require an argument?
    let output = work_dir.run_jj(["metaedit", "nkmpptxzrspx"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // When resetting the author has no effect, the commits are not rewritten.
    let output = work_dir.run_jj([
        "metaedit",
        "--config=user.name=Test User",
        "--update-author",
        "nkmpptxzrspx",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Update author, ensure the commit can be specified with -r too
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "metaedit",
            "--config=user.name=Ove Ridder",
            "--config=user.email=ove.ridder@example.com",
            "--update-author",
            "-r",
            "nkmpptxzrspx",
        ])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: ae4f674f90aa5e6cb5e35dbba89f07fbadda7212
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:17.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 4030125719ab8eab6bf3512f2e94ef1086f6f719
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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
        .run_jj(["metaedit", "--update-author-timestamp", "nkmpptxzrspx"])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 2ffc2fd0145bb13872d5824292192464269a7c74
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:20.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 636156987ae8c5e2659afeee34f167448d2cf80c
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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
            "metaedit",
            "--author",
            "Alice <alice@example.com>",
            "nkmpptxzrspx",
        ])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 36bf20bf6335c7cd930d34609a71ebd221d0b9ab
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:23.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 0e1a59998553e269e4fc5a72e500abc4d65cbc99
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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

    // new author date
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "metaedit",
            "--author-timestamp",
            "1995-12-19T16:39:57-08:00",
        ])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 50da9de3c56ed519d452f6070e9b6f4b45cc83c5
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (1995-12-19 16:39:57.000 -08:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:26.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 099a44efcab4d977ed5b622bf030b3de2b59fcef
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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

    // invalid date gives an error
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["metaedit", "--author-timestamp", "aaaaaa"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: invalid value 'aaaaaa' for '--author-timestamp <AUTHOR_TIMESTAMP>': premature end of input

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Update committer timestamp
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj(["metaedit", "--force-rewrite", "nkmpptxzrspx"])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: ebc5d975db02477c3203e21ef442454dd31682f5
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:31.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 6df7d0b4a9a9ce270e201fb2d906ef75bb2c1c62
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:31.000 +07:00)
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

    // change test author config for changing committer
    test_env.add_env_var("JJ_USER", "Test Committer");
    test_env.add_env_var("JJ_EMAIL", "test.committer@example.com");
    let work_dir = test_env.work_dir("repo");

    // update existing commit with restored test author config
    insta::assert_snapshot!(work_dir.run_jj(["metaedit", "--force-rewrite"]), @"
    ------- stderr -------
    Modified 1 commits:
      pzvwutvl 82611617 c | (no description set)
    Working copy  (@) now at: pzvwutvl 82611617 c | (no description set)
    Parent commit (@-)      : nkmpptxz 6df7d0b4 b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["show"]), @"
    Commit ID: 82611617f1bc287e1ee773eb65f11c946a77fc75
    Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    Bookmarks: c
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:13)
    Committer: Test Committer <test.committer@example.com> (2001-02-03 08:05:33)

        (no description set)

    Modified regular file file1:
       1     : b
            1: c
    [EOF]
    ");

    // When resetting the description has no effect, the commits are not rewritten.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["metaedit", "--message", "", "nkmpptxzrspx"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    // Update description
    work_dir
        .run_jj(["metaedit", "--message", "d\ne\nf"])
        .success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: b1d3b74bbc7edf70c6bcd708b125ba02bf39ea73
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test Committer <test.committer@example.com> (2001-02-03 04:05:37.000 +07:00)
    │
    │      d
    │      e
    │      f
    │
    ○  Commit ID: 099a44efcab4d977ed5b622bf030b3de2b59fcef
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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

    // Set empty description
    work_dir.run_jj(["metaedit", "--message", ""]).success();
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: a2e143c4646126b61caa10c275e886a5c92f1113
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test Committer <test.committer@example.com> (2001-02-03 04:05:39.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 099a44efcab4d977ed5b622bf030b3de2b59fcef
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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
}

#[test]
fn test_metaedit_no_matching_revisions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["metaedit", "--update-change-id", "none()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to modify.
    [EOF]
    ");
}

#[test]
fn test_metaedit_multiple_revisions() {
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
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 1cc10709fbc078f11bacb8640a3dc11a41ec8221
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:13.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 099a44efcab4d977ed5b622bf030b3de2b59fcef
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
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

    // Update multiple revisions
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj([
        "metaedit",
        "--config=user.name=Ove Ridder",
        "--config=user.email=ove.ridder@example.com",
        "--update-author",
        "nkmpptxz::pzvwutvl",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Modified 2 commits:
      nkmpptxz c2cf6241 b | (no description set)
      pzvwutvl 819e7624 c | (no description set)
    Rebased 1 descendant commits
    Working copy  (@) now at: sostqsxw d2aae73d (empty) (no description set)
    Parent commit (@-)      : pzvwutvl 819e7624 c | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: d2aae73d443bec2e01e223987d27c002e854f622
    │  Change ID: sostqsxwqrltovqlrlzszywzslusmuup
    │  Author   : Test User <test.user@example.com> (2001-02-03 04:05:15.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:16.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: 819e762406dd4f214ea8ac63b7aff1e2ae1465fc
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:13.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:16.000 +07:00)
    │
    │      (no description set)
    │
    ○  Commit ID: c2cf62419f1c0bd5022bd4c33fa6d055d5c75384
    │  Change ID: nkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:11.000 +07:00)
    │  Committer: Ove Ridder <ove.ridder@example.com> (2001-02-03 04:05:16.000 +07:00)
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

    let output = work_dir.run_jj(["metaedit", "--update-change-id", "nkmpptxzrspx"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Modified 1 commits:
      yqosqzyt 01d6741e b | (no description set)
    Rebased 1 descendant commits
    Working copy  (@) now at: pzvwutvl 97c0b8a6 c | (no description set)
    Parent commit (@-)      : yqosqzyt 01d6741e b | (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log(&work_dir), @"
    @  Commit ID: 97c0b8a6c66784b8ee76df8f7636f34336b5ef1a
    │  Change ID: pzvwutvlkqwtuzoztpszkqxkqmqyqyxo
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
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r", "yqosqzytrlswkspswpqrmlplxylrzsnz"]), @"
    ○  yqosqzyt test.user@example.com 2001-02-03 08:05:13 b 01d6741e
    │  (no description set)
    │  -- operation 5b02e577448e edit commit metadata for commit 099a44efcab4d977ed5b622bf030b3de2b59fcef
    ○  nkmpptxz/0 test.user@example.com 2001-02-03 08:05:11 099a44ef (hidden)
    │  (no description set)
    │  -- operation 7b1b5edae383 snapshot working copy
    ○  nkmpptxz/1 test.user@example.com 2001-02-03 08:05:09 11ba6b0f (hidden)
       (empty) (no description set)
       -- operation 130349b44890 new empty commit
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "-r", "pzvwut"]), @"
    @  pzvwutvl test.user@example.com 2001-02-03 08:05:13 c 97c0b8a6
    │  (no description set)
    │  -- operation 5b02e577448e edit commit metadata for commit 099a44efcab4d977ed5b622bf030b3de2b59fcef
    ○  pzvwutvl/1 test.user@example.com 2001-02-03 08:05:13 1cc10709 (hidden)
    │  (no description set)
    │  -- operation 543f77a89e53 snapshot working copy
    ○  pzvwutvl/2 test.user@example.com 2001-02-03 08:05:11 e0a459cb (hidden)
       (empty) (no description set)
       -- operation 2a59b6403d9d new empty commit
    [EOF]
    ");
}

#[test]
fn test_metaedit_option_mutual_exclusion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["commit", "-m=a"]).success();
    work_dir.run_jj(["describe", "-m=b"]).success();
    insta::assert_snapshot!(work_dir.run_jj([
        "metaedit",
        "--author=Alice <alice@example.com>",
        "--update-author",
    ]), @"
    ------- stderr -------
    error: the argument '--author <AUTHOR>' cannot be used with '--update-author'

    Usage: jj metaedit --author <AUTHOR> [REVSETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_update_empty_author_or_email() {
    let mut test_env = TestEnvironment::default();

    // get rid of test author config
    test_env.add_env_var("JJ_USER", "");
    test_env.add_env_var("JJ_EMAIL", "");

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();

    // show that commit has no author set
    insta::assert_snapshot!(test_env.work_dir("repo").run_jj(["show"]), @"
    Commit ID: 42c91a3e183efb4499038d0d9aa3d14b5deafde0
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Author   : (no name set) <(no email set)> (2001-02-03 08:05:07)
    Committer: (no name set) <(no email set)> (2001-02-03 08:05:07)

        (no description set)

    [EOF]
    ");

    // restore test author config, exercise --quiet
    test_env.add_env_var("JJ_USER", "Test User");
    test_env.add_env_var("JJ_EMAIL", "test.user@example.com");
    let work_dir = test_env.work_dir("repo");

    // update existing commit with restored test author config
    insta::assert_snapshot!(work_dir.run_jj(["metaedit", "--update-author", "--quiet"]), @"");
    insta::assert_snapshot!(work_dir.run_jj(["show"]), @"
    Commit ID: 0f13b5f2ea7fad147c133c81b87d31e7b1b8c564
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    [EOF]
    ");

    // confirm user / email can be cleared/restored separately
    // leave verbose to see no-email warning + hint
    test_env.add_env_var("JJ_EMAIL", "");
    let work_dir = test_env.work_dir("repo");
    insta::assert_snapshot!(work_dir.run_jj(["metaedit", "--update-author"]), @r#"
    ------- stderr -------
    Modified 1 commits:
      qpvuntsm 234908d4 (empty) (no description set)
    Working copy  (@) now at: qpvuntsm 234908d4 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: Email not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.email "someone@example.com"
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["show"]), @"
    Commit ID: 234908d4748ff3224a87888d8b52a4923e1a89a5
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Author   : Test User <(no email set)> (2001-02-03 08:05:09)
    Committer: Test User <(no email set)> (2001-02-03 08:05:11)

        (no description set)

    [EOF]
    ");

    // confirm no-name warning + hint
    test_env.add_env_var("JJ_USER", "");
    test_env.add_env_var("JJ_EMAIL", "test.user@example.com");
    let work_dir = test_env.work_dir("repo");
    insta::assert_snapshot!(work_dir.run_jj(["metaedit", "--update-author"]), @r#"
    ------- stderr -------
    Modified 1 commits:
      qpvuntsm ac5048cf (empty) (no description set)
    Working copy  (@) now at: qpvuntsm ac5048cf (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: Name not configured. Until configured, your commits will be created with the empty identity, and can't be pushed to remotes.
    Hint: To configure, run:
      jj config set --user user.name "Some One"
    [EOF]
    "#);
    insta::assert_snapshot!(work_dir.run_jj(["show"]), @"
    Commit ID: ac5048cf35372ddc30e2590271781a3eab0bcaf8
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Author   : (no name set) <test.user@example.com> (2001-02-03 08:05:09)
    Committer: (no name set) <test.user@example.com> (2001-02-03 08:05:13)

        (no description set)

    [EOF]
    ");
}

#[test]
/// Test that setting the same timestamp twice does nothing (issue #7602)
fn test_metaedit_set_same_timestamp_twice() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set the author-timestamp to the same value twice
    // and check that the second time it does nothing
    let output = work_dir.run_jj([
        "metaedit",
        "--author-timestamp",
        "2001-02-03 04:05:14.000+07:00",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Modified 1 commits:
      qpvuntsm 51b97b23 (empty) (no description set)
    Working copy  (@) now at: qpvuntsm 51b97b23 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Running it again with the same date has no effect
    let output = work_dir.run_jj([
        "metaedit",
        "--author-timestamp",
        "2001-02-03 04:05:14.000+07:00",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
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
