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

    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--command=false"]), @r"
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
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--command", &bisector_path]), @r"
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
    insta::assert_snapshot!(work_dir.run_jj(["bisect", "run", "--range=..", "--command", &bisector_path]), @r"
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
    Working copy  (@) now at: kmkuslsw?? 55b3b4a8 (empty) testing
    Parent commit (@-)      : kmkuslsw?? 17e2a972 (empty) (no description set)
    Working copy  (@) now at: msksykpx 2f6e298d (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 7d980be7 a | a
    Added 0 files, modified 0 files, removed 1 files
    Working copy  (@) now at: kmkuslsw?? 2f80658c (empty) testing
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
