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
fn test_print_new_tracked_files() {
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
    ------- stderr -------
    Auto-tracking 1 new file:
    A file1
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

    // Tracking 20 files
    for i in 12..32 {
        work_dir.write_file(format!("file{i}"), "\n");
    }
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    A file10
    A file11
    A file12
    A file13
    A file14
    A file15
    A file16
    A file17
    A file18
    A file19
    A file2
    A file20
    A file21
    A file22
    A file23
    A file24
    A file25
    A file26
    A file27
    A file28
    A file29
    A file3
    A file30
    A file31
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    Working copy  (@) : qpvuntsm 89bf5dc3 (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Tracking file12, file13, file14, file15, file16, file17, file18, file19 and 12 other files
    [EOF]
    ");

    // Nothing should be newly tracked
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    Working copy changes:
    A file1
    A file10
    A file11
    A file12
    A file13
    A file14
    A file15
    A file16
    A file17
    A file18
    A file19
    A file2
    A file20
    A file21
    A file22
    A file23
    A file24
    A file25
    A file26
    A file27
    A file28
    A file29
    A file3
    A file30
    A file31
    A file4
    A file5
    A file6
    A file7
    A file8
    A file9
    Working copy  (@) : qpvuntsm 89bf5dc3 (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}
