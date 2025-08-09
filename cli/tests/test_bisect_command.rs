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
use crate::common::create_commit;
use crate::common::fake_bisector_path;

#[test]
fn test_bisect_run() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=all()", "--command=false"]), @r"
    The first bad commit is: zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Now testing: zsuskuln 123b4d91 b | b
    Working copy  (@) now at: lylxulpl 0cb688b1 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 123b4d91 b | b
    Added 0 files, modified 0 files, removed 4 files
    The commit is bad.

    Now testing: zzzzzzzz 00000000 (empty) (no description set)
    Working copy  (@) now at: rsllmpnm 8afab1b9 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 2 files
    The commit is bad.

    [EOF]
    ");
}

#[test]
fn test_bisect_run_write_file() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c
    │   c
    ○  b
    │   b
    ○  a
    │   a
    ◆
    [EOF]
    ");

    std::fs::write(
        &bisection_script,
        ["write new-file\nsome contents", "fail\n"].join("\0"),
    )
    .unwrap();
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=all()", r"--command", &bisector_path]), @r"
    The first bad commit is: zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ------- stderr -------
    Now testing: rlvkpnrz 7d980be7 a | a
    Working copy  (@) now at: yostqsxw d9746ed4 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    The commit is bad.

    Now testing: zzzzzzzz 00000000 (empty) (no description set)
    Working copy  (@) now at: wmwvqwsz fc81b4ee (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 2 files
    The commit is bad.

    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  new-file
    │ ○  new-file
    │ │ ○  c
    │ │ │   c
    │ │ ○  b
    │ ├─╯   b
    │ ○  a
    ├─╯   a
    ◆
    [EOF]
    ");

    // No concurrent operations
    let output = work_dir.run_jj(["op", "log", "-n=5", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @  snapshot working copy
    ○  Checked out commit 0000000000000000000000000000000000000000 for bisection
    ○  snapshot working copy
    ○  Checked out commit 7d980be7a1d499e4d316ab4c01242885032f7eaf for bisection
    ○  create bookmark c pointing to commit dffaa0d4daccf6cee70bac3498fae3b3fd5d6b5b
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", description, diff.files().map(|e| e.path()))"#;
    work_dir.run_jj(["log", "-T", template])
}
