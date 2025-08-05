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
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:13)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:13)
    │
    │      (no description set)
    │
    ○  Commit ID: 75591b1896b4990e7695701fd7cdbb32dba3ff50
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:11)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:11)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 11:00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 11:00:00)

           (no description set)

    [EOF]
    ");

    // Touch the commit (and its descendants)
    work_dir.run_jj(["touch", "kkmpptxzrspx"]).success();
    insta::assert_snapshot!(get_log(&work_dir), @r"
    @  Commit ID: b396a5373e525bd9b322cab64c65f5f67ece81e7
    │  Change ID: mzvwutvlkqwtuzoztpszkqxkqmqyqyxo
    │  Bookmarks: c
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:13)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:14)
    │
    │      (no description set)
    │
    ○  Commit ID: 53f5eea6f1d793859d38f1299ff10ebfb67d0a23
    │  Change ID: kkmpptxzrspxrzommnulwmwkkqwworpl
    │  Bookmarks: b
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:11)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:14)
    │
    │      (no description set)
    │
    ○  Commit ID: e6086990958c236d72030f0a2651806aa629f5dd
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Bookmarks: a
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)
    │
    │      (no description set)
    │
    ◆  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author   : (no name set) <(no email set)> (1970-01-01 11:00:00)
       Committer: (no name set) <(no email set)> (1970-01-01 11:00:00)

           (no description set)

    [EOF]
    ");
}

#[must_use]
fn get_log(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["log", "-T", "builtin_log_detailed"])
}
