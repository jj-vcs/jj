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

use testutils::TestResult;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit;
use crate::common::fake_bisector_path;

#[test]
fn test_bisect_run_missing_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=.."]), @"
    ------- stderr -------
    Error: Command argument is required
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_bisect_run_empty_revset() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(&bisection_script, ["fail"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=none()", &bisector_path]), @"
    Search complete. To discard any revisions created during search, run:
      jj op restore 90267f31f904
    [EOF]
    ------- stderr -------
    Error: Could not find the first bad revision. Was the input range empty?
    [EOF]
    [exit status: 1]
    ");
    Ok(())
}

#[test]
fn test_bisect_run() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    std::fs::write(&bisection_script, ["fail"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 5 revisions left to test after this (roughly 3 steps)
    Now evaluating: ooyxmykx 26c624f4 c | c
    fake-bisector testing commit 26c624f4f2a7c2f9bc22924698172f94245aaa35
    The revision is bad.

    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore b4e8abf5e7b2
    The first bad revision is: ylvkpnrz a1afb583 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 750510fa (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  rsllmpnmslon 750510fa4b31 '' files:
    │ ○  wmkuslswpqwq dd4393d7d8e1 'f' files: f
    │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    │ ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ├─╯
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // Try with legacy command argument
    std::fs::write(&bisection_script, ["fail"].join("\0"))?;
    // Testing only stderr to avoid a variable op id in the stdout.
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--command", &bisector_path]).success().stderr, @"
    Warning: `--command` is deprecated; use positional arguments instead: `jj bisect run --range=... -- $FAKE_BISECTOR_PATH`
    Working copy  (@) now at: nkmrtpmo 959aed7c (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Added 2 files, modified 0 files, removed 0 files
    Working copy  (@) now at: ruktrxxu fbf4b3a0 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  ruktrxxusqqp fbf4b3a025e8 '' files:
    │ ○  wmkuslswpqwq dd4393d7d8e1 'f' files: f
    │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    │ ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ├─╯
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_bisect_run_find_first_good() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--find-good", &bisector_path]), @"
    Bisecting: 5 revisions left to test after this (roughly 3 steps)
    Now evaluating: ooyxmykx 26c624f4 c | c
    fake-bisector testing commit 26c624f4f2a7c2f9bc22924698172f94245aaa35
    The revision is good.

    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is good.

    Search complete. To discard any revisions created during search, run:
      jj op restore b4e8abf5e7b2
    The first good revision is: ylvkpnrz a1afb583 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 750510fa (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  rsllmpnmslon 750510fa4b31 '' files:
    │ ○  wmkuslswpqwq dd4393d7d8e1 'f' files: f
    │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    │ ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ├─╯
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
}

#[test]
fn test_bisect_run_missing_bisector() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    let output = work_dir.run_jj(["bisect", "run", "--range=..", "nonexistent"]);
    if cfg!(unix) {
        insta::assert_snapshot!(output, @r"
        Bisecting: 5 revisions left to test after this (roughly 3 steps)
        Now evaluating: ooyxmykx 26c624f4 c | c
        [EOF]
        ------- stderr -------
        Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
        Parent commit (@-)      : ooyxmykx 26c624f4 c | c
        Added 0 files, modified 0 files, removed 3 files
        Error: Failed to run evaluation command
        Caused by: No such file or directory (os error 2)
        [EOF]
        [exit status: 1]
        ");
    } else if cfg!(windows) {
        insta::assert_snapshot!(output, @"
        Bisecting: 5 revisions left to test after this (roughly 3 steps)
        Now evaluating: ooyxmykx 26c624f4 c | c
        [EOF]
        ------- stderr -------
        Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
        Parent commit (@-)      : ooyxmykx 26c624f4 c | c
        Added 0 files, modified 0 files, removed 3 files
        Error: Failed to run evaluation command
        Caused by: program not found
        [EOF]
        [exit status: 1]
        ");
    }
}

#[test]
fn test_bisect_run_with_args() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--find-good", "--", &bisector_path, "--require-file=c"]), @"
    Bisecting: 5 revisions left to test after this (roughly 3 steps)
    Now evaluating: ooyxmykx 26c624f4 c | c
    fake-bisector testing commit 26c624f4f2a7c2f9bc22924698172f94245aaa35
    The revision is good.

    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is bad.

    Bisecting: 1 revisions left to test after this (roughly 1 steps)
    Now evaluating: psuskuln dd148a1b b | b
    fake-bisector testing commit dd148a1be8f066ab36432210eec075e69aefef49
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore b4e8abf5e7b2
    The first good revision is: ooyxmykx 26c624f4 c | c
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 750510fa (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    Working copy  (@) now at: zqsquwqt d89179d3 (empty) (no description set)
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  zqsquwqtrvts d89179d31512 '' files:
    │ ○  wmkuslswpqwq dd4393d7d8e1 'f' files: f
    │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    ├─╯
    ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
}

#[test]
fn test_bisect_run_crash() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    // bisector crash is equivalent to a failure
    std::fs::write(&bisection_script, ["crash"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 5 revisions left to test after this (roughly 3 steps)
    Now evaluating: ooyxmykx 26c624f4 c | c
    fake-bisector testing commit 26c624f4f2a7c2f9bc22924698172f94245aaa35
    The revision is bad.

    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore b4e8abf5e7b2
    The first bad revision is: ylvkpnrz a1afb583 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 0120f5b7 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 26c624f4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 750510fa (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_bisect_run_abort() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);

    // stop immediately on failure
    std::fs::write(&bisection_script, ["abort"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    Evaluation command returned 127 (command not found) - aborting bisection.

    Search complete. To discard any revisions created during search, run:
      jj op restore ab395827f1de
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 680590b7 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    Error: Bisection aborted
    [EOF]
    [exit status: 1]
    ");
    Ok(())
}

#[test]
fn test_bisect_run_skip() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // head (b) is assumed to be bad, even though all revisions are skipped
    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);

    std::fs::write(&bisection_script, ["skip"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 1 revisions left to test after this (roughly 1 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    It could not be determined if the revision is good or bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore ba9728ecc7c5
    The first bad revision is: psuskuln dd148a1b b | b
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: royxmykx ad414a38 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_bisect_run_multiple_results() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // heads (d and b) are assumed to be bad
    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    create_commit(&work_dir, "d", &["c"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=a|b|c|d", &bisector_path]), @"
    Bisecting: 2 revisions left to test after this (roughly 2 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is good.

    Bisecting: 1 revisions left to test after this (roughly 1 steps)
    Now evaluating: ooyxmykx 45ee1acd c | c
    fake-bisector testing commit 45ee1acd6076b9fb29763ef077fd51adfb3eee6c
    The revision is good.

    Search complete. To discard any revisions created during search, run:
      jj op restore 1967458ead7b
    The first bad revisions are:
    truxwmqv 278414ea d | d
    psuskuln dd148a1b b | b
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: znkkpsqq 88ec4f47 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    Working copy  (@) now at: uuzqqzqu d3f77fc4 (empty) (no description set)
    Parent commit (@-)      : ooyxmykx 45ee1acd c | c
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
}

#[test]
fn test_bisect_run_write_file() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);

    std::fs::write(
        &bisection_script,
        ["write new-file\nsome contents", "fail"].join("\0"),
    )?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 4 revisions left to test after this (roughly 3 steps)
    Now evaluating: psuskuln dd148a1b b | b
    fake-bisector testing commit dd148a1be8f066ab36432210eec075e69aefef49
    The revision is bad.

    Bisecting: 1 revisions left to test after this (roughly 1 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore f4784f88004a
    The first bad revision is: ylvkpnrz a1afb583 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 1b61b030 (empty) (no description set)
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: msksykpx 27fab395 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  msksykpxotkr b96eaf357888 '' files: new-file
    │ ○  kmkuslswpqwq 8c70693e7499 '' files: new-file
    │ │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    │ ├─╯
    │ ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ├─╯
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // No concurrent operations
    let output = work_dir.run_jj(["op", "log", "-n=5", "-T=description"]);
    insta::assert_snapshot!(output, @"
    @  snapshot working copy
    ○  Updated to revision a1afb5834d8ee4dcb61b59db0f682c7a53f96f53 for bisection
    ○  snapshot working copy
    ○  Updated to revision dd148a1be8f066ab36432210eec075e69aefef49 for bisection
    ○  create bookmark e pointing to commit 0cd0d3a4354bd9ae486b7bf7bca0c04de119ba94
    [EOF]
    ");
    Ok(())
}

#[test]
fn test_bisect_run_jj_command() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);

    std::fs::write(&bisection_script, ["jj new -mtesting", "fail"].join("\0"))?;
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @"
    Bisecting: 4 revisions left to test after this (roughly 3 steps)
    Now evaluating: psuskuln dd148a1b b | b
    fake-bisector testing commit dd148a1be8f066ab36432210eec075e69aefef49
    The revision is bad.

    Bisecting: 1 revisions left to test after this (roughly 1 steps)
    Now evaluating: ylvkpnrz a1afb583 a | a
    fake-bisector testing commit a1afb5834d8ee4dcb61b59db0f682c7a53f96f53
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore f4784f88004a
    The first bad revision is: ylvkpnrz a1afb583 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 1b61b030 (empty) (no description set)
    Parent commit (@-)      : psuskuln dd148a1b b | b
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: wmkuslsw 9f916afb (empty) testing
    Parent commit (@-)      : kmkuslsw 1b61b030 (empty) (no description set)
    Working copy  (@) now at: msksykpx 27fab395 (empty) (no description set)
    Parent commit (@-)      : ylvkpnrz a1afb583 a | a
    Added 0 files, modified 0 files, removed 1 files
    Working copy  (@) now at: xmkuslsw 5acfc6c3 (empty) testing
    Parent commit (@-)      : msksykpx 27fab395 (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  xmkuslswpqwq 5acfc6c308c2 'testing' files:
    ○  msksykpxotkr 27fab395ffae '' files:
    │ ○  wmkuslswpqwq 9f916afb1c77 'testing' files:
    │ ○  kmkuslswpqwq 1b61b030fef0 '' files:
    │ │ ○  nnkkpsqqskkl 0cd0d3a4354b 'e' files: e
    │ │ ○  truxwmqvtpmx e87eb7e7ce86 'd' files: d
    │ │ ○  ooyxmykxtrkr 26c624f4f2a7 'c' files: c
    │ ├─╯
    │ ○  psuskulnrvyr dd148a1be8f0 'b' files: b
    ├─╯
    ○  ylvkpnrzqnoo a1afb5834d8e 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // No concurrent operations
    let output = work_dir.run_jj(["op", "log", "-n=5", "-T=description"]);
    insta::assert_snapshot!(output, @"
    @  new empty commit
    ○  Updated to revision a1afb5834d8ee4dcb61b59db0f682c7a53f96f53 for bisection
    ○  new empty commit
    ○  Updated to revision dd148a1be8f066ab36432210eec075e69aefef49 for bisection
    ○  create bookmark e pointing to commit 0cd0d3a4354bd9ae486b7bf7bca0c04de119ba94
    [EOF]
    ");
    Ok(())
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ",
    change_id.short(),
    commit_id.short(),
    "'" ++  description.first_line() ++ "'",
    "files: " ++ diff.files().map(|e| e.path())
)"#;
    work_dir.run_jj(["log", "-T", template])
}
