// Copyright 2026 The Jujutsu Authors
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
use crate::common::create_commit_with_files;
use crate::common::force_interactive;

#[test]
fn test_converge_no_divergence() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "a")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("file2", "b")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "c")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  c  royxmykx  78dcec21:  a
    │ ○  b  zsuskuln  056564da:  a
    ├─╯
    ○  a  rlvkpnrz  3b93fc14
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergent changes found.
    [EOF]
    ");
}

#[test]
fn test_converge_simple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  d  znkkpsqq  ecbe1d2f:  b1
    ○  b1  zsuskuln  48bf33ab:  a
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 3f194323 (divergent) b2
    ○  zsuskuln/1 48bf33ab (divergent) b2
    ○  zsuskuln/2 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    Successfully converged change: created commit ff9156feaf8f.
    Rebased 1 descendants
    Working copy  (@) now at: znkkpsqq 9e0d0773 d | d
    Parent commit (@-)      : zsuskuln ff9156fe b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  9e0d0773:  b1 b2
    ○  b1 b2  zsuskuln  ff9156fe:  c
    ○  c  royxmykx  0fdb9e5a:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln ff9156fe b2
    ├─╮
    ○ │  zsuskuln/1 3f194323 (hidden) b2
    ├─╯
    ○  zsuskuln/2 48bf33ab (hidden) b2
    ○  zsuskuln/3 fd685708 (hidden) (empty) b2
    [EOF]
    ");
}

#[test]
fn test_converge_two_divergent_changes_in_non_interactive_mode() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);

    // Set up commit graph with two divergent changes
    // First divergent change:
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Second divergent change:
    create_commit_with_files(&work_dir, "e2", &["a"], &[("file1", "initial\ne\n")]);
    create_commit_with_files(&work_dir, "f", &["a"], &[("file2", "f\n")]);
    work_dir.run_jj(["rebase", "-r", "e2", "-o", "f"]).success();
    work_dir
        .run_jj(["bookmark", "create", "e1", "-r", "at_operation(@-, e2)"])
        .success();
    create_commit_with_files(&work_dir, "g", &["e1"], &[("file3", "g\n")]);

    // Test the setup (commit B is duplicated and commit E is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  ca4326e1:  e1
    ○  e1  kmkuslsw  58b94e24:  a
    │ ○  e2  kmkuslsw  67edcfef:  f
    │ ○  f  lylxulpl  f2174e65:  a
    ├─╯
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    │ ○  d  znkkpsqq  ecbe1d2f:  b1
    │ ○  b1  zsuskuln  48bf33ab:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 3f194323 (divergent) b2
    ○  zsuskuln/1 48bf33ab (divergent) b2
    ○  zsuskuln/2 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "e2"), @r"
    ○  kmkuslsw/0 67edcfef (divergent) e2
    ○  kmkuslsw/1 58b94e24 (divergent) e2
    ○  kmkuslsw/2 c3466dc2 (hidden) (empty) e2
    [EOF]
    ");

    // Pass --non-interactive to jj converge command.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge", "--no-interactive"]));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 67edcfef e2 | (divergent) e2
        kmkuslsw/1 58b94e24 e1 | (divergent) e2

    Error: No change selected
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_converge_two_divergent_changes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);

    // Set up commit graph with two divergent changes
    // First divergent change:
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Second divergent change:
    create_commit_with_files(&work_dir, "e2", &["a"], &[("file1", "initial\ne\n")]);
    create_commit_with_files(&work_dir, "f", &["a"], &[("file2", "f\n")]);
    work_dir.run_jj(["rebase", "-r", "e2", "-o", "f"]).success();
    work_dir
        .run_jj(["bookmark", "create", "e1", "-r", "at_operation(@-, e2)"])
        .success();
    create_commit_with_files(&work_dir, "g", &["e1"], &[("file3", "g\n")]);

    // Test the setup (commit B is duplicated and commit E is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  ca4326e1:  e1
    ○  e1  kmkuslsw  58b94e24:  a
    │ ○  e2  kmkuslsw  67edcfef:  f
    │ ○  f  lylxulpl  f2174e65:  a
    ├─╯
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    │ ○  d  znkkpsqq  ecbe1d2f:  b1
    │ ○  b1  zsuskuln  48bf33ab:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 3f194323 (divergent) b2
    ○  zsuskuln/1 48bf33ab (divergent) b2
    ○  zsuskuln/2 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "e2"), @r"
    ○  kmkuslsw/0 67edcfef (divergent) e2
    ○  kmkuslsw/1 58b94e24 (divergent) e2
    ○  kmkuslsw/2 c3466dc2 (hidden) (empty) e2
    [EOF]
    ");

    // When running in quiet mode the command fails because it cannot prompt the
    // user.
    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 67edcfef e2 | (divergent) e2
        kmkuslsw/1 58b94e24 e1 | (divergent) e2

    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Error: Cannot prompt for input since the output is not connected to a terminal
    [EOF]
    [exit status: 1]
    ");

    // Now run force interactive execution; user chooses to quit when presented with
    // the choices.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("q\n"));

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 67edcfef e2 | (divergent) e2
        kmkuslsw/1 58b94e24 e1 | (divergent) e2

    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: 

    Error: No change selected
    [EOF]
    [exit status: 1]
    ");

    // Now run force interactive execution and choose the first divergent change.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("1\n"));

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 67edcfef e2 | (divergent) e2
        kmkuslsw/1 58b94e24 e1 | (divergent) e2

    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: 

    Successfully converged change: created commit f25a9192b316.
    Rebased 1 descendants
    Hint: There are still 1 divergent changes remaining in the specified revset, you can run this command again to converge another one.
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  ca4326e1:  e1
    ○  e1  kmkuslsw  58b94e24:  a
    │ ○  e2  kmkuslsw  67edcfef:  f
    │ ○  f  lylxulpl  f2174e65:  a
    ├─╯
    │ ○  d  znkkpsqq  c5f5c95e:  b1 b2
    │ ○  b1 b2  zsuskuln  f25a9192:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln f25a9192 b2
    ├─╮
    ○ │  zsuskuln/1 3f194323 (hidden) b2
    ├─╯
    ○  zsuskuln/2 48bf33ab (hidden) b2
    ○  zsuskuln/3 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    // Run converge a second time to converge the other divergent change
    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 67edcfef e2 | (divergent) e2
        kmkuslsw/1 58b94e24 e1 | (divergent) e2

    Successfully converged change: created commit cc9ec248cade.
    Rebased 1 descendants
    Working copy  (@) now at: xznxytkn 30f78859 g | g
    Parent commit (@-)      : kmkuslsw cc9ec248 e1 e2 | e2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  30f78859:  e1 e2
    ○  e1 e2  kmkuslsw  cc9ec248:  f
    ○  f  lylxulpl  f2174e65:  a
    │ ○  d  znkkpsqq  c5f5c95e:  b1 b2
    │ ○  b1 b2  zsuskuln  f25a9192:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "e2"), @r"
    ○    kmkuslsw cc9ec248 e2
    ├─╮
    ○ │  kmkuslsw/1 67edcfef (hidden) e2
    ├─╯
    ○  kmkuslsw/2 58b94e24 (hidden) e2
    ○  kmkuslsw/3 c3466dc2 (hidden) (empty) e2
    [EOF]
    ");
}

#[test]
fn test_converge_simple_with_revisions_arg() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @"
    @  d  znkkpsqq  ecbe1d2f:  b1
    ○  b1  zsuskuln  48bf33ab:  a
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 3f194323 (divergent) b2
    ○  zsuskuln/1 48bf33ab (divergent) b2
    ○  zsuskuln/2 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    let output = work_dir.run_jj(["converge", "-r", "a::d"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergent changes found in the specified revset.
    [EOF]
    ");

    let output = work_dir.run_jj(["converge", "-r", "a::"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 3f194323 b2 | (divergent) b2
        zsuskuln/1 48bf33ab b1 | (divergent) b2

    Successfully converged change: created commit 077040e1d7e0.
    Rebased 1 descendants
    Working copy  (@) now at: znkkpsqq ac2086f3 d | d
    Parent commit (@-)      : zsuskuln 077040e1 b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  ac2086f3:  b1 b2
    ○  b1 b2  zsuskuln  077040e1:  c
    ○  c  royxmykx  0fdb9e5a:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 077040e1 b2
    ├─╮
    ○ │  zsuskuln/1 3f194323 (hidden) b2
    ├─╯
    ○  zsuskuln/2 48bf33ab (hidden) b2
    ○  zsuskuln/3 fd685708 (hidden) (empty) b2
    [EOF]
    ");
}

#[test]
fn test_converge_one_side_rebased_one_side_description_changed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file2", "c\n")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "b1", "-m", "blah blah blah"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  kpqxywon  32e1c372:  b1
    ○  b1  zsuskuln  c4f55b2e:  a
    │ ○  b2  zsuskuln  3f194323:  c
    │ ○  c  royxmykx  0fdb9e5a:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/1 3f194323 (divergent) b2
    ○  zsuskuln/2 48bf33ab (hidden) b2
    ○  zsuskuln/3 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 c4f55b2e b1 | (divergent) blah blah blah
        zsuskuln/1 3f194323 b2 | (divergent) b2

    Successfully converged change: created commit 834a15ad8ae4.
    Rebased 1 descendants
    Working copy  (@) now at: kpqxywon 5f790792 d | d
    Parent commit (@-)      : zsuskuln 834a15ad b1 b2 | blah blah blah
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  kpqxywon  5f790792:  b1 b2
    ○  b1 b2  zsuskuln  834a15ad:  c
    ○  c  royxmykx  0fdb9e5a:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 834a15ad blah blah blah
    ├─╮
    │ ○  zsuskuln/2 3f194323 (hidden) b2
    ○ │  zsuskuln/1 c4f55b2e (hidden) blah blah blah
    ├─╯
    ○  zsuskuln/3 48bf33ab (hidden) b2
    ○  zsuskuln/4 fd685708 (hidden) (empty) b2
    [EOF]
    ");
}

#[test]
fn test_converge_description_changed_inconsistently() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "initial\n")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file1", "initial\nb\n")]);
    work_dir
        .run_jj(["describe", "-r", "b2", "-m", "today is a bad day"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "b1", "-m", "today is a good day"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "d\n")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  yostqsxw  12fa18a0:  b1
    ○  b1  zsuskuln  57aa7c1d:  a
    │ ○  b2  zsuskuln  16aa57ac:  a
    ├─╯
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/1 16aa57ac (divergent) today is a bad day
    ○  zsuskuln/2 48bf33ab (hidden) b2
    ○  zsuskuln/3 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    // First check behavior in non-interactive mode.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge", "--no-interactive"]));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 57aa7c1d b1 | (divergent) today is a good day
        zsuskuln/1 16aa57ac b2 | (divergent) today is a bad day

    Could not determine which description to use.
    Internal error: Could not converge change
    [EOF]
    [exit status: 255]
    ");

    // Now check behavior in interactive mode.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("n\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 57aa7c1d b1 | (divergent) today is a good day
        zsuskuln/1 16aa57ac b2 | (divergent) today is a bad day

    There are divergent descriptions. You can choose to merge them now in a
    text editor, or skip merging and use the conflicted description (with
    conflict markers). Do you want to merge them now? (Yn): 

    Successfully converged change: created commit 12954f7d25c7.
    Rebased 1 descendants
    Working copy  (@) now at: yostqsxw b7be53fe d | d
    Parent commit (@-)      : zsuskuln 12954f7d b1 b2 | <<<<<<< conflict 1 of 1
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  yostqsxw  b7be53fe:  b1 b2
    ○  b1 b2  zsuskuln  12954f7d:  a
    ○  a  rlvkpnrz  08789390
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 12954f7d <<<<<<< conflict 1 of 1
    ├─╮
    │ ○  zsuskuln/2 16aa57ac (hidden) today is a bad day
    ○ │  zsuskuln/1 57aa7c1d (hidden) today is a good day
    ├─╯
    ○  zsuskuln/3 48bf33ab (hidden) b2
    ○  zsuskuln/4 fd685708 (hidden) (empty) b2
    [EOF]
    ");

    // Verify the description after converge (it should have conflict markers)
    let output = work_dir.run_jj(["log", "-T", "description", "-r", "b1", "--no-graph"]);
    insta::assert_snapshot!(output, @r#"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: zsuskuln 57aa7c1d "today is a good day"
    \\\\\\\        to: zsuskuln 48bf33ab "b2"
    -today is a good day
    +b2
    %%%%%%% diff from: zsuskuln 16aa57ac "today is a bad day"
    \\\\\\\        to: zsuskuln 48bf33ab "b2"
    -today is a bad day
    +b2
    +++++++ zsuskuln 48bf33ab "b2"
    b2
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    "#);
}

#[must_use]
fn get_long_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = "bookmarks ++ '  ' ++ change_id.shortest(8) ++ '  ' ++ commit_id.shortest(8) \
                    ++ surround(':  ', '', parents.map(|c| c.bookmarks()))";
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_evolog(work_dir: &TestWorkDir, revision: &str) -> CommandOutput {
    let template = r#"format_commit_summary_with_refs(commit, "") ++ "\n""#;
    work_dir.run_jj(["evolog", "-r", revision, "-T", template])
}
