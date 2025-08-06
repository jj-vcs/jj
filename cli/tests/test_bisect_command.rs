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
fn test_bisect_run_missing_command() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=.."]), @r"
    ------- stderr -------
    Error: Command argument is required
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_bisect_run_empty_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=none()", "false"]), @r"
    Search complete. To discard any revisions created during search, run:
      jj op restore 8f47435a3990
    [EOF]
    ------- stderr -------
    Error: Could not find the first bad revision. Was the input range empty?
    [EOF]
    [exit status: 1]
    ");
}

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

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "false"]), @r"
    Now evaluating: royxmykx dffaa0d4 c | c
    The revision is bad.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 9152b6b19cce
    The first bad revision is: rlvkpnrz 7d980be7 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 68b3a16f (empty) (no description set)
    Parent commit (@-)      : royxmykx dffaa0d4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 5f328bc5 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  rsllmpnmslon 5f328bc5fde0 '' files:
    │ ○  kmkuslswpqwq 8b67af288466 'f' files: f
    │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    │ ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ├─╯
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // Try with legacy command argument
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--command", "false"]), @r"
    Now evaluating: royxmykx dffaa0d4 c | c
    The revision is bad.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 5473934e3b2f
    The first bad revision is: rlvkpnrz 7d980be7 a | a
    [EOF]
    ------- stderr -------
    Warning: `--command` is deprecated; use positional arguments instead: `jj bisect run --range=... -- false
    Working copy  (@) now at: nkmrtpmo 1601f7b4 (empty) (no description set)
    Parent commit (@-)      : royxmykx dffaa0d4 c | c
    Added 2 files, modified 0 files, removed 0 files
    Working copy  (@) now at: ruktrxxu fb9e625c (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  ruktrxxusqqp fb9e625c1023 '' files:
    │ ○  kmkuslswpqwq 8b67af288466 'f' files: f
    │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    │ ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ├─╯
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
}

#[test]
fn test_bisect_run_find_first_good() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--find-good", "true"]), @r"
    Now evaluating: royxmykx dffaa0d4 c | c
    The revision is good.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    The revision is good.

    Search complete. To discard any revisions created during search, run:
      jj op restore 9152b6b19cce
    The first good revision is: rlvkpnrz 7d980be7 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 68b3a16f (empty) (no description set)
    Parent commit (@-)      : royxmykx dffaa0d4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 5f328bc5 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  rsllmpnmslon 5f328bc5fde0 '' files:
    │ ○  kmkuslswpqwq 8b67af288466 'f' files: f
    │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    │ ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ├─╯
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
}

#[test]
fn test_bisect_run_with_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["e"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--find-good", "--", "test", "-f", "c"]), @r"
    Now evaluating: royxmykx dffaa0d4 c | c
    The revision is good.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    The revision is bad.

    Now evaluating: zsuskuln 123b4d91 b | b
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 9152b6b19cce
    The first good revision is: royxmykx dffaa0d4 c | c
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: lylxulpl 68b3a16f (empty) (no description set)
    Parent commit (@-)      : royxmykx dffaa0d4 c | c
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: rsllmpnm 5f328bc5 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    Working copy  (@) now at: zqsquwqt 042badd2 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 123b4d91 b | b
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zqsquwqtrvts 042badd28c1d '' files:
    │ ○  kmkuslswpqwq 8b67af288466 'f' files: f
    │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    ├─╯
    ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");
}

#[test]
fn test_bisect_run_abort() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);

    // stop immediately on failure
    std::fs::write(&bisection_script, ["abort"].join("\0")).unwrap();
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @r"
    Now evaluating: rlvkpnrz 7d980be7 a | a
    fake-bisector testing commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 538d9e7f (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    Error: Evaluation command returned 127 (command not found) - aborting bisection.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_bisect_run_skip() {
    let mut test_env = TestEnvironment::default();
    let bisector_path = fake_bisector_path();
    let bisection_script = test_env.set_up_fake_bisector();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // head (b) is assumed to be bad, even though all revisions are skipped
    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);

    std::fs::write(&bisection_script, ["skip"].join("\0")).unwrap();
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @r"
    Now evaluating: rlvkpnrz 7d980be7 a | a
    fake-bisector testing commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    It could not be determined if the revision is good or bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 9cc40e5398a9
    The first bad revision is: zsuskuln 123b4d91 b | b
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: royxmykx 2144134b (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
}

#[test]
fn test_bisect_run_multiple_results() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // heads (d and b) are assumed to be bad
    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    create_commit(&work_dir, "d", &["c"]);

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=a|b|c|d", "true"]), @r"
    Now evaluating: rlvkpnrz 7d980be7 a | a
    The revision is good.

    Now evaluating: royxmykx 991a7501 c | c
    The revision is good.

    Search complete. To discard any revisions created during search, run:
      jj op restore d750de12e02a
    The first bad revisions are:
    vruxwmqv a2dbb1aa d | d
    zsuskuln 123b4d91 b | b
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: znkkpsqq 1b117fe7 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    Working copy  (@) now at: uuzqqzqu 6bf5f5e7 (empty) (no description set)
    Parent commit (@-)      : royxmykx 991a7501 c | c
    Added 1 files, modified 0 files, removed 0 files
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
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);

    std::fs::write(
        &bisection_script,
        ["write new-file\nsome contents", "fail"].join("\0"),
    )
    .unwrap();
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @r"
    Now evaluating: zsuskuln 123b4d91 b | b
    fake-bisector testing commit 123b4d91f6e5e39bfed39bae3bacf9380dc79078
    The revision is bad.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    fake-bisector testing commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 156d8a1abcb8
    The first bad revision is: rlvkpnrz 7d980be7 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 17e2a972 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 123b4d91 b | b
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: msksykpx 2f6e298d (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  msksykpxotkr 891aeb03b623 '' files: new-file
    │ ○  kmkuslswpqwq 2bae881dc1bc '' files: new-file
    │ │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    │ ├─╯
    │ ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ├─╯
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // No concurrent operations
    let output = work_dir.run_jj(["op", "log", "-n=5", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @  snapshot working copy
    ○  Updated to revision 7d980be7a1d499e4d316ab4c01242885032f7eaf for bisection
    ○  snapshot working copy
    ○  Updated to revision 123b4d91f6e5e39bfed39bae3bacf9380dc79078 for bisection
    ○  create bookmark e pointing to commit 62d30ded0e8fdf8cf87012e6223898b97977fc8e
    [EOF]
    ");
}

#[test]
fn test_bisect_run_jj_command() {
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

    std::fs::write(&bisection_script, ["jj new -mtesting", "fail"].join("\0")).unwrap();
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", &bisector_path]), @r"
    Now evaluating: zsuskuln 123b4d91 b | b
    fake-bisector testing commit 123b4d91f6e5e39bfed39bae3bacf9380dc79078
    The revision is bad.

    Now evaluating: rlvkpnrz 7d980be7 a | a
    fake-bisector testing commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    The revision is bad.

    Search complete. To discard any revisions created during search, run:
      jj op restore 156d8a1abcb8
    The first bad revision is: rlvkpnrz 7d980be7 a | a
    [EOF]
    ------- stderr -------
    Working copy  (@) now at: kmkuslsw 17e2a972 (empty) (no description set)
    Parent commit (@-)      : zsuskuln 123b4d91 b | b
    Added 0 files, modified 0 files, removed 3 files
    Working copy  (@) now at: kmkuslsw/0 55b3b4a8 (empty) testing
    Parent commit (@-)      : kmkuslsw/1 17e2a972 (empty) (no description set)
    Working copy  (@) now at: msksykpx 2f6e298d (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 1 files
    Working copy  (@) now at: kmkuslsw/0 2f80658c (empty) testing
    Parent commit (@-)      : msksykpx 2f6e298d (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kmkuslswpqwq 2f80658c4d26 'testing' files:
    ○  msksykpxotkr 2f6e298d59bd '' files:
    │ ○  kmkuslswpqwq 55b3b4a8b253 'testing' files:
    │ ○  kmkuslswpqwq 17e2a9721f61 '' files:
    │ │ ○  znkkpsqqskkl 62d30ded0e8f 'e' files: e
    │ │ ○  vruxwmqvtpmx 86be7a223919 'd' files: d
    │ │ ○  royxmykxtrkr dffaa0d4dacc 'c' files: c
    │ ├─╯
    │ ○  zsuskulnrvyr 123b4d91f6e5 'b' files: b
    ├─╯
    ○  rlvkpnrzqnoo 7d980be7a1d4 'a' files: a
    ◆  zzzzzzzzzzzz 000000000000 '' files:
    [EOF]
    ");

    // No concurrent operations
    let output = work_dir.run_jj(["op", "log", "-n=5", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @  new empty commit
    ○  Updated to revision 7d980be7a1d499e4d316ab4c01242885032f7eaf for bisection
    ○  new empty commit
    ○  Updated to revision 123b4d91f6e5e39bfed39bae3bacf9380dc79078 for bisection
    ○  create bookmark e pointing to commit 62d30ded0e8fdf8cf87012e6223898b97977fc8e
    [EOF]
    ");
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
