// Copyright 2025 The Jujutsu Authors
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
fn test_with_status_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Tracking only one new file
    work_dir.write_file("file1", "\n");

    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    Working copy  (@) : qpvuntsm e12a776d (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Tracking 10 new files,
    for i in 2..12 {
        work_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    A file10
    A file11
    A file2
    A file3
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    Working copy  (@) : qpvuntsm a89fcde9 (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Tracking 20 files in a directory.
    let target_dir = work_dir.create_dir("target");
    for i in 12..32 {
        target_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["status"]).normalize_backslash();
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    A file10
    A file11
    A file2
    A file3
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    A target/file12
    A target/file13
    A target/file14
    A target/file15
    A target/file16
    A target/file17
    A target/file18
    A target/file19
    A target/file20
    A target/file21
    A target/file22
    A target/file23
    A target/file24
    A target/file25
    A target/file26
    A target/file27
    A target/file28
    A target/file29
    A target/file30
    A target/file31
    Working copy  (@) : qpvuntsm 54c9b3fe (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Track 10 more files in target directory.
    for i in 32..42 {
        target_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["status"]).normalize_backslash();
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    A file10
    A file11
    A file2
    A file3
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    A target/file12
    A target/file13
    A target/file14
    A target/file15
    A target/file16
    A target/file17
    A target/file18
    A target/file19
    A target/file20
    A target/file21
    A target/file22
    A target/file23
    A target/file24
    A target/file25
    A target/file26
    A target/file27
    A target/file28
    A target/file29
    A target/file30
    A target/file31
    A target/file32
    A target/file33
    A target/file34
    A target/file35
    A target/file36
    A target/file37
    A target/file38
    A target/file39
    A target/file40
    A target/file41
    Working copy  (@) : qpvuntsm 68770366 (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");

    // Nothing should be newly tracked
    let ref_stdout = output.stdout;
    let output = work_dir.run_jj(["status"]);
    assert_eq!(output.stdout, ref_stdout);
    insta::assert_snapshot!(output.stderr, @r"");

    // Assure that nothing get printed if configured.
    let output = work_dir.run_jj(["config", "set", "--repo", "ui.show-newly-tracked", "false"]);
    insta::assert_snapshot!(output, @r"");
    let output = work_dir.run_jj(["status"]);
    assert_eq!(output.stdout, ref_stdout);
    insta::assert_snapshot!(output.stderr, @r"");
}

#[test]
fn test_with_log_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Tracking only one new file
    work_dir.write_file("file1", "\n");

    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 e12a776d
    │  (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Auto-tracking 1 new file:
    A file1
    [EOF]
    ");

    // Tracking 10 new files,
    for i in 2..12 {
        work_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:09 a89fcde9
    │  (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Auto-tracking 10 new files:
    A file10
    A file11
    A file2
    A file3
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    [EOF]
    ");

    // Tracking 20 files in a directory.
    let target_dir = work_dir.create_dir("target");
    for i in 12..32 {
        target_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["log"]).normalize_backslash();
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:10 54c9b3fe
    │  (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Auto-tracking 20 new files:
    A target/ (20 files)
    [EOF]
    ");

    // Track 10 more files in target directory.
    for i in 32..42 {
        target_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["log"]).normalize_backslash();
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:11 68770366
    │  (no description set)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Auto-tracking 10 new files:
    A target/file32
    A target/file33
    A target/file34
    A target/file35
    A target/file36
    A target/file37
    A target/file38
    A target/file39
    A target/file40
    A target/file41
    [EOF]
    ");

    // Nothing should be newly tracked
    let ref_stdout = output.stdout;
    let output = work_dir.run_jj(["log"]);
    assert_eq!(output.stdout, ref_stdout);
    insta::assert_snapshot!(output.stderr, @r"");

    // Assure that nothing get printed if configured.
    let output = work_dir.run_jj(["config", "set", "--repo", "ui.show-newly-tracked", "false"]);
    insta::assert_snapshot!(output, @r"");
    let output = work_dir.run_jj(["log"]);
    assert_eq!(output.stdout, ref_stdout);
    insta::assert_snapshot!(output.stderr, @r"");
}
