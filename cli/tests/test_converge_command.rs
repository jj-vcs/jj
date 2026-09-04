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

use test_case::test_case;
use testutils::TestResult;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit_with_files;
use crate::common::force_interactive;

// `jj converge` must runs successfully when there are no divergent changes and
// prints a message to stderr.
#[test]
fn test_converge_no_divergence() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph (without divergent changes)
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "a")]);
    create_commit_with_files(&work_dir, "b", &["a"], &[("file2", "b")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "c")]);

    // Test the setup
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  c  royxmykx  78dcec21 - description: c
    │ ○  b  zsuskuln  056564da - description: b
    ├─╯
    ○  a  rlvkpnrz  3b93fc14 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Run `jj converge` command and check the output.
    let output = work_dir.run_jj(["converge"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergent changes found.
    [EOF]
    ");
}

// A simple `jj converge` scenario where there is a single divergent change with
// two visible commits. In this setup no user input is required.
#[test]
fn test_converge_simple() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with one divergent change (with two visible commits).
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Test the setup: look at the commit graph, commit B is duplicated
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  bf5126ef - description: d
    ○  b1  zsuskuln/1  59a77004 - description: b2
    │ ○  b2  zsuskuln/0  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 2c2bd25d (divergent) b2
    ○  zsuskuln/1 59a77004 (divergent) b2
    ○  zsuskuln/2 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // Run `jj converge` command and check the output. In this case no user input is
    // needed.
    let output = work_dir.run_jj(["converge"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    Attempting to converge change zsuskulnrvyr...

    Successfully converged change: created commit 6b1ce5bc4cbe.
    Rebased 1 descendants
    Working copy  (@) now at: znkkpsqq 696cf5e0 d | d
    Parent commit (@-)      : zsuskuln 6b1ce5bc b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  696cf5e0 - description: d
    ○  b1 b2  zsuskuln  6b1ce5bc - description: b2
    ○  c  royxmykx  4343fc61 - description: c
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 6b1ce5bc b2
    ├─╮
    ○ │  zsuskuln/1 2c2bd25d (hidden) b2
    ├─╯
    ○  zsuskuln/2 59a77004 (hidden) b2
    ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");
}

// When there are multiple divergent changes, the command must prompt the user
// to select one of them. When running in non-interactive mode (jj converge
// --no-interactive) this is not possible.
#[test]
fn test_converge_two_divergent_changes_in_non_interactive_mode() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up: first create a base commit
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);

    // Set up: create commit graph with two divergent changes
    // First divergent change:
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Second divergent change:
    create_commit_with_files(&work_dir, "e2", &["a"], &[("file5", "5")]);
    create_commit_with_files(&work_dir, "f", &["a"], &[("file6", "6")]);
    work_dir.run_jj(["rebase", "-r", "e2", "-o", "f"]).success();
    work_dir
        .run_jj(["bookmark", "create", "e1", "-r", "at_operation(@-, e2)"])
        .success();
    create_commit_with_files(&work_dir, "g", &["e1"], &[("file7", "7")]);

    // Test the setup: look at the commit graph (commit B is duplicated and commit E
    // is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  46658cae - description: g
    ○  e1  kmkuslsw/1  15962bae - description: e2
    │ ○  e2  kmkuslsw/0  b54f15d8 - description: e2
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  b2  zsuskuln/0  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    │ ○  d  znkkpsqq  bf5126ef - description: d
    │ ○  b1  zsuskuln/1  59a77004 - description: b2
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Pass --non-interactive to jj converge command.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge", "--no-interactive"]));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Error: Cannot automatically choose which change to converge
    Hint: Run `jj converge` in interactive mode, or specify a revset that resolves to only one change ID
    [EOF]
    [exit status: 1]
    ");

    // Note: in the test environment jj commands run in non-interactive (quiet) mode
    // by default, so the following also fails but for a different reason: it
    // cannot prompt the user
    let output = work_dir.run_jj(["converge"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Choose which change to converge (jj converge only converges one change at a time):
    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Error: Cannot prompt for input since the output is not connected to a terminal
    [EOF]
    [exit status: 1]
    ");

    // Note: the invocation also fails if stdin is not connected to a terminal
    let output = work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Choose which change to converge (jj converge only converges one change at a time):
    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: Error: Prompt canceled by EOF
    [EOF]
    [exit status: 1]
    ");
}

// This tests scenarios where there are two divergent changes. The command
// prompts the user to choose which change to converge.
#[test]
fn test_converge_two_divergent_changes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up: first create a base commit
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);

    // Set up: create commit graph with two divergent changes
    // First divergent change:
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Second divergent change:
    create_commit_with_files(&work_dir, "e2", &["a"], &[("file5", "5")]);
    create_commit_with_files(&work_dir, "f", &["a"], &[("file6", "6")]);
    work_dir.run_jj(["rebase", "-r", "e2", "-o", "f"]).success();
    work_dir
        .run_jj(["bookmark", "create", "e1", "-r", "at_operation(@-, e2)"])
        .success();
    create_commit_with_files(&work_dir, "g", &["e1"], &[("file7", "7")]);

    // Test the setup: look at the commit graph (commit B is duplicated and commit E
    // is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  46658cae - description: g
    ○  e1  kmkuslsw/1  15962bae - description: e2
    │ ○  e2  kmkuslsw/0  b54f15d8 - description: e2
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  b2  zsuskuln/0  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    │ ○  d  znkkpsqq  bf5126ef - description: d
    │ ○  b1  zsuskuln/1  59a77004 - description: b2
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 2c2bd25d (divergent) b2
    ○  zsuskuln/1 59a77004 (divergent) b2
    ○  zsuskuln/2 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "e2"), @r"
    ○  kmkuslsw/0 b54f15d8 (divergent) e2
    ○  kmkuslsw/1 15962bae (divergent) e2
    ○  kmkuslsw/2 843de29d (hidden) (empty) e2
    [EOF]
    ");

    // If the user chooses to abort the converge operation nothing changes.
    let output = work_dir
        .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("q\n"))
        .success();

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Choose which change to converge (jj converge only converges one change at a time):
    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: 

    Aborting... nothing changed.
    [EOF]
    ");

    // Run the command again, this time the user chooses the first divergent change.
    // This invocation succeeds to automatically converge that change. A hint is
    // printed to inform the user that there is still one divergent change
    // remaining.
    let output = work_dir
        .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("1\n"))
        .success();

    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Choose which change to converge (jj converge only converges one change at a time):
    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: 

    Attempting to converge change zsuskulnrvyr...

    Successfully converged change: created commit ba447f020ee6.
    Rebased 1 descendants
    Hint: There are still 1 divergent changes remaining in the specified revset, you can run this command again to converge another one.
    [EOF]
    ");

    // Verify the commit graph after converging the first divergent change
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  46658cae - description: g
    ○  e1  kmkuslsw/1  15962bae - description: e2
    │ ○  e2  kmkuslsw/0  b54f15d8 - description: e2
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  d  znkkpsqq  b30d892b - description: d
    │ ○  b1 b2  zsuskuln  ba447f02 - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converging the first divergent change
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln ba447f02 b2
    ├─╮
    ○ │  zsuskuln/1 2c2bd25d (hidden) b2
    ├─╯
    ○  zsuskuln/2 59a77004 (hidden) b2
    ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // Run converge a second time to converge the other divergent change
    let output = work_dir.run_jj(["converge"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/0 b54f15d8 e2 | (divergent) e2
        kmkuslsw/1 15962bae e1 | (divergent) e2

    Attempting to converge change kmkuslswpqwq...

    Successfully converged change: created commit 3f08d00a88c4.
    Rebased 1 descendants
    Working copy  (@) now at: xznxytkn f9fd4e7e g | g
    Parent commit (@-)      : kmkuslsw 3f08d00a e1 e2 | e2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converging the second divergent change
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  xznxytkn  f9fd4e7e - description: g
    ○  e1 e2  kmkuslsw  3f08d00a - description: e2
    ○  f  lylxulpl  d50e2761 - description: f
    │ ○  d  znkkpsqq  b30d892b - description: d
    │ ○  b1 b2  zsuskuln  ba447f02 - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converging the second divergent change
    insta::assert_snapshot!(get_evolog(&work_dir, "e2"), @r"
    ○    kmkuslsw 3f08d00a e2
    ├─╮
    ○ │  kmkuslsw/1 b54f15d8 (hidden) e2
    ├─╯
    ○  kmkuslsw/2 15962bae (hidden) e2
    ○  kmkuslsw/3 843de29d (hidden) (empty) e2
    [EOF]
    ");

    // There are no more divergent changes now
    let output = work_dir.run_jj(["converge"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergent changes found.
    [EOF]
    ");
}

// This tests scenarios where the user specifies revisions to converge. More
// precisely, the user specifies a revset that is used as the search space for
// divergent commits.
#[test]
fn test_converge_simple_with_revisions_arg() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  bf5126ef - description: d
    ○  b1  zsuskuln/1  59a77004 - description: b2
    │ ○  b2  zsuskuln/0  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 2c2bd25d (divergent) b2
    ○  zsuskuln/1 59a77004 (divergent) b2
    ○  zsuskuln/2 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // `-r a::d` resolves to {a, b1, d}. b1 IS a divergent commit, but in that
    // revset there are no other commits with that change-id, so by design the
    // command does nothing (we could change that in the future).
    let output = work_dir.run_jj(["converge", "-r", "a::d"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergence found among the specified revisions.
    [EOF]
    ");

    // `-r a::` resolves to {a, b1, b2, c, d}. Now the command "sees" two commits
    // with the same change-id and converges them.
    let output = work_dir.run_jj(["converge", "-r", "a::"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    Attempting to converge change zsuskulnrvyr...

    Successfully converged change: created commit 5b9d32498e06.
    Rebased 1 descendants
    Working copy  (@) now at: znkkpsqq 4080edbe d | d
    Parent commit (@-)      : zsuskuln 5b9d3249 b1 b2 | b2
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  znkkpsqq  4080edbe - description: d
    ○  b1 b2  zsuskuln  5b9d3249 - description: b2
    ○  c  royxmykx  4343fc61 - description: c
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 5b9d3249 b2
    ├─╮
    ○ │  zsuskuln/1 2c2bd25d (hidden) b2
    ├─╯
    ○  zsuskuln/2 59a77004 (hidden) b2
    ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");
}

// This tests scenarios where the user specifies revisions to converge. More
// precisely, the user specifies a revset that is used as the search space for
// divergent commits. This is a variation of
// test_converge_simple_with_revisions_arg: in that test there was a single
// divergent change, here there are two.
#[test]
fn test_converge_simple_with_revisions_arg_and_two_divergent_changes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up: first create a base commit
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);

    // Set up: create commit graph with two divergent changes
    // First divergent change:
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Second divergent change:
    create_commit_with_files(&work_dir, "e3", &["a"], &[("file5", "5")]);
    create_commit_with_files(&work_dir, "f", &["a"], &[("file6", "6")]);
    work_dir.run_jj(["rebase", "-r", "e3", "-o", "f"]).success();
    work_dir
        .run_jj(["bookmark", "create", "e2", "-r", "at_operation(@-, e3)"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "e2", "-m", "blah blah blah"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "e1", "-r", "at_operation(@-, e2)"])
        .success();
    create_commit_with_files(&work_dir, "g", &["e2"], &[("file7", "7")]);

    // Test the setup: look at the commit graph (commit B is duplicated and commit E
    // is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  nmzmmopx  7e4fac7e - description: g
    ○  e2  kmkuslsw/0  c8976369 - description: blah blah blah
    │ ○  e3  kmkuslsw/1  d34ec64c - description: e3
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  e1  kmkuslsw/2  faebbd68 - description: e3
    ├─╯
    │ ○  b2  zsuskuln/0  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    │ ○  d  znkkpsqq  bf5126ef - description: d
    │ ○  b1  zsuskuln/1  59a77004 - description: b2
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/0 2c2bd25d (divergent) b2
    ○  zsuskuln/1 59a77004 (divergent) b2
    ○  zsuskuln/2 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "e3"), @r"
    ○  kmkuslsw/1 d34ec64c (divergent) e3
    ○  kmkuslsw/2 faebbd68 (divergent) e3
    ○  kmkuslsw/3 8f5eb314 (hidden) (empty) e3
    [EOF]
    ");

    // `-r a::d` resolves to {a, b1, d}. b1 IS a divergent commit, but in that
    // revset there are no other commits with that change-id, so by design the
    // command does nothing (we could change that in the future).
    let output = work_dir.run_jj(["converge", "-r", "a::d"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergence found among the specified revisions.
    [EOF]
    ");

    // `-r a::` does resolve to both divergent changes. In this test we simulate the
    // user aborts at the prompt.
    let output = work_dir
        .run_jj_with(|cmd| {
            force_interactive(cmd)
                .args(["converge", "-r", "a::"])
                .write_stdin("q\n")
        })
        .success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 2 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    - Change: kmkuslswpqwq with 3 commits:
        kmkuslsw/0 c8976369 e2 | (divergent) blah blah blah
        kmkuslsw/1 d34ec64c e3 | (divergent) e3
        kmkuslsw/2 faebbd68 e1 | (divergent) e3

    Choose which change to converge (jj converge only converges one change at a time):
    1: zsuskulnrvyr
    2: kmkuslswpqwq
    q: abort
    Enter the index of the change to converge: 

    Aborting... nothing changed.
    [EOF]
    ");

    // `-r b1|e3` resolve to those two commits. Both ARE divergent commits, but in
    // the search space there are no other commits with either change-id so the
    // command does nothing.
    let output = work_dir
        .run_jj_with(|cmd| {
            force_interactive(cmd)
                .args(["converge", "-r", "b1|e3"])
                .write_stdin("q\n")
        })
        .success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No divergence found among the specified revisions.
    [EOF]
    ");

    // Specifying `-r b1|b2` resolves to that divergent change and only that one.
    // There should not be any prompt.
    let output = work_dir
        .run_jj_with(|cmd| {
            force_interactive(cmd)
                .args(["converge", "-r", "b1|b2"])
                .write_stdin("q\n")
        })
        .success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 2c2bd25d b2 | (divergent) b2
        zsuskuln/1 59a77004 b1 | (divergent) b2

    Attempting to converge change zsuskulnrvyr...

    Successfully converged change: created commit 32d4597c081c.
    Rebased 1 descendants
    [EOF]
    ");

    // Look at the resulting commit graph
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  nmzmmopx  7e4fac7e - description: g
    ○  e2  kmkuslsw/0  c8976369 - description: blah blah blah
    │ ○  e3  kmkuslsw/1  d34ec64c - description: e3
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  e1  kmkuslsw/2  faebbd68 - description: e3
    ├─╯
    │ ○  d  znkkpsqq  5aecbdd4 - description: d
    │ ○  b1 b2  zsuskuln  32d4597c - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 32d4597c b2
    ├─╮
    ○ │  zsuskuln/1 2c2bd25d (hidden) b2
    ├─╯
    ○  zsuskuln/2 59a77004 (hidden) b2
    ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    // Lets undo the previous converge operation to try a different scenario.
    work_dir.run_jj(["undo"]).success();

    // The next invocation shows that specifying `-r e1|e3` converges those two
    // divergent commits, but leaves e1 around (by design).
    let output = work_dir
        .run_jj_with(|cmd| {
            force_interactive(cmd)
                .args(["converge", "-r", "e1|e3"])
                .write_stdin("q\n")
        })
        .success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: kmkuslswpqwq with 2 commits:
        kmkuslsw/1 d34ec64c e3 | (divergent) e3
        kmkuslsw/2 faebbd68 e1 | (divergent) e3

    Attempting to converge change kmkuslswpqwq...

    Successfully converged change: created commit b5ce73ed2a0d.
    [EOF]
    ");

    // Look at the resulting commit graph
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  g  nmzmmopx  7e4fac7e - description: g
    ○  e2  kmkuslsw/1  c8976369 - description: blah blah blah
    │ ○  e1 e3  kmkuslsw/0  b5ce73ed - description: e3
    │ ○  f  lylxulpl  d50e2761 - description: f
    ├─╯
    │ ○  b2  zsuskuln/1  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    │ ○  d  znkkpsqq  bf5126ef - description: d
    │ ○  b1  zsuskuln/2  59a77004 - description: b2
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "e3"), @r"
    ○    kmkuslsw/0 b5ce73ed (divergent) e3
    ├─╮
    ○ │  kmkuslsw/2 d34ec64c (hidden) e3
    ├─╯
    ○  kmkuslsw/3 faebbd68 (hidden) e3
    ○  kmkuslsw/4 8f5eb314 (hidden) (empty) e3
    [EOF]
    ");
}

// In this scenario there are two divergent commits. One side changed the
// description, the other side was rebased. In such simple cases `jj converge`
// should be able to automatically combine the new description with the new
// parents.
#[test]
fn test_converge_one_side_rebased_one_side_description_changed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &["a"], &[("file3", "3")]);
    work_dir.run_jj(["rebase", "-r", "b2", "-o", "c"]).success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "b1", "-m", "blah blah blah"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file4", "4")]);

    // Test the setup (commit B is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  kpqxywon  16d29671 - description: d
    ○  b1  zsuskuln/0  d471c689 - description: blah blah blah
    │ ○  b2  zsuskuln/1  2c2bd25d - description: b2
    │ ○  c  royxmykx  4343fc61 - description: c
    ├─╯
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  zsuskuln/1 2c2bd25d (divergent) b2
    ○  zsuskuln/2 59a77004 (hidden) b2
    ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");

    let output = work_dir.run_jj(["converge"]).success();
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: zsuskulnrvyr with 2 commits:
        zsuskuln/0 d471c689 b1 | (divergent) blah blah blah
        zsuskuln/1 2c2bd25d b2 | (divergent) b2

    Attempting to converge change zsuskulnrvyr...

    Successfully converged change: created commit 65226e3f7378.
    Rebased 1 descendants
    Working copy  (@) now at: kpqxywon 405941e7 d | d
    Parent commit (@-)      : zsuskuln 65226e3f b1 b2 | blah blah blah
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d  kpqxywon  405941e7 - description: d
    ○  b1 b2  zsuskuln  65226e3f - description: blah blah blah
    ○  c  royxmykx  4343fc61 - description: c
    ○  a  rlvkpnrz  e9a731d9 - description: a
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    zsuskuln 65226e3f blah blah blah
    ├─╮
    │ ○  zsuskuln/2 2c2bd25d (hidden) b2
    ○ │  zsuskuln/1 d471c689 (hidden) blah blah blah
    ├─╯
    ○  zsuskuln/3 59a77004 (hidden) b2
    ○  zsuskuln/4 b2852eb2 (hidden) (empty) b2
    [EOF]
    ");
}

#[test_case(false; "dont_invoke_text_editor")]
#[test_case(true; "invoke_text_editor")]
fn test_converge_description_changed_inconsistently(invoke_text_editor: bool) -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);
    create_commit_with_files(&work_dir, "b2", &["a"], &[("file2", "2")]);
    work_dir
        .run_jj(["describe", "-r", "b2", "-m", "foo"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "b1", "-r", "at_operation(@-, b2)"])
        .success();
    work_dir
        .run_jj(["describe", "-r", "b1", "-m", "bar"])
        .success();
    create_commit_with_files(&work_dir, "d", &["b1"], &[("file3", "3")]);

    // Test the setup (commit B is duplicated)
    insta::allow_duplicates! {
        insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
        @  d  yostqsxw  a906f67a - description: d
        ○  b1  zsuskuln/0  0ec69b7a - description: bar
        │ ○  b2  zsuskuln/1  08117b18 - description: foo
        ├─╯
        ○  a  rlvkpnrz  e9a731d9 - description: a
        ◆    zzzzzzzz  00000000
        [EOF]
        ");
    }

    insta::allow_duplicates! {
        insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
        ○  zsuskuln/1 08117b18 (divergent) foo
        ○  zsuskuln/2 59a77004 (hidden) b2
        ○  zsuskuln/3 b2852eb2 (hidden) (empty) b2
        [EOF]
        ");
    }

    // First check behavior in non-interactive mode.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge", "--no-interactive"]));
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Found 1 divergent change(s) in the specified revset:
        - Change: zsuskulnrvyr with 2 commits:
            zsuskuln/0 0ec69b7a b1 | (divergent) bar
            zsuskuln/1 08117b18 b2 | (divergent) foo

        Attempting to converge change zsuskulnrvyr...

        Could not determine which description to use.
        Error: Could not converge change
        [EOF]
        [exit status: 1]
        ");
    }

    // Now check behavior in interactive mode.
    if invoke_text_editor {
        std::fs::write(
            &edit_script,
            ["dump editor0", "write\nmy-merged-description"].join("\0"),
        )?;
        let output = work_dir
            .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("y\n"))
            .success();
        insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0"))?, @r#"
        <<<<<<< conflict 1 of 1
        %%%%%%% diff from: zsuskuln 59a77004 "b2"
        \\\\\\\        to: zsuskuln 0ec69b7a "bar"
        -b2
        +bar
        +++++++ zsuskuln 08117b18 "foo"
        foo
        >>>>>>> conflict 1 of 1 ends
        "#);
        insta::assert_snapshot!(output.stdout.normalized(), @"");
        insta::assert_snapshot!(output.stderr.normalized(), @r"
        Found 1 divergent change(s) in the specified revset:
        - Change: zsuskulnrvyr with 2 commits:
            zsuskuln/0 0ec69b7a b1 | (divergent) bar
            zsuskuln/1 08117b18 b2 | (divergent) foo

        Attempting to converge change zsuskulnrvyr...

        There are divergent descriptions. You can choose to merge them now in a
        text editor, or skip merging and use the conflicted description (with
        conflict markers). Do you want to merge them now? (Yn): 

        Successfully converged change: created commit a393891bef3b.
        Rebased 1 descendants
        Working copy  (@) now at: yostqsxw a9dd817f d | d
        Parent commit (@-)      : zsuskuln a393891b b1 b2 | my-merged-description
        ");

        // Verify the commit graph after converge
        insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
        @  d  yostqsxw  a9dd817f - description: d
        ○  b1 b2  zsuskuln  a393891b - description: my-merged-des...
        ○  a  rlvkpnrz  e9a731d9 - description: a
        ◆    zzzzzzzz  00000000
        [EOF]
        ");

        // Verify the evolution history after converge
        insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
        ○    zsuskuln a393891b my-merged-description
        ├─╮
        │ ○  zsuskuln/2 08117b18 (hidden) foo
        ○ │  zsuskuln/1 0ec69b7a (hidden) bar
        ├─╯
        ○  zsuskuln/3 59a77004 (hidden) b2
        ○  zsuskuln/4 b2852eb2 (hidden) (empty) b2
        [EOF]
        ");
    } else {
        let output = work_dir
            .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("n\n"))
            .success();
        insta::assert_snapshot!(output, @r"
        ------- stderr -------
        Found 1 divergent change(s) in the specified revset:
        - Change: zsuskulnrvyr with 2 commits:
            zsuskuln/0 0ec69b7a b1 | (divergent) bar
            zsuskuln/1 08117b18 b2 | (divergent) foo

        Attempting to converge change zsuskulnrvyr...

        There are divergent descriptions. You can choose to merge them now in a
        text editor, or skip merging and use the conflicted description (with
        conflict markers). Do you want to merge them now? (Yn): 

        Successfully converged change: created commit 6fdb1551f127.
        Rebased 1 descendants
        Working copy  (@) now at: yostqsxw 6b108e95 d | d
        Parent commit (@-)      : zsuskuln 6fdb1551 b1 b2 | <<<<<<< conflict 1 of 1
        [EOF]
        ");

        // Verify the commit graph after converge
        insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
        @  d  yostqsxw  6b108e95 - description: d
        ○  b1 b2  zsuskuln  6fdb1551 - description: <<<<<<< confl...
        ○  a  rlvkpnrz  e9a731d9 - description: a
        ◆    zzzzzzzz  00000000
        [EOF]
        ");

        // Verify the evolution history after converge
        insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
        ○    zsuskuln 6fdb1551 <<<<<<< conflict 1 of 1
        ├─╮
        │ ○  zsuskuln/2 08117b18 (hidden) foo
        ○ │  zsuskuln/1 0ec69b7a (hidden) bar
        ├─╯
        ○  zsuskuln/3 59a77004 (hidden) b2
        ○  zsuskuln/4 b2852eb2 (hidden) (empty) b2
        [EOF]
        ");

        // Verify the description after converge (it should have conflict markers)
        let output = work_dir.run_jj(["log", "-T", "description", "-r", "b1", "--no-graph"]);
        insta::assert_snapshot!(output, @r#"
        <<<<<<< conflict 1 of 1
        %%%%%%% diff from: zsuskuln 59a77004 "b2"
        \\\\\\\        to: zsuskuln 0ec69b7a "bar"
        -b2
        +bar
        +++++++ zsuskuln 08117b18 "foo"
        foo
        >>>>>>> conflict 1 of 1 ends
        [EOF]
        "#);
    }
    Ok(())
}

// In this scenario there are two divergent commits. Each side rebased their
// common predecessor on top of different parents. In this case `jj converge`
// cannot automatically determine which parents to use, so it should prompt the
// user.
#[test]
fn test_converge_with_inconsistent_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Set up commit graph with divergent changes
    create_commit_with_files(&work_dir, "a", &[], &[("file1", "1")]);
    create_commit_with_files(&work_dir, "b", &[], &[("file2", "2")]);
    create_commit_with_files(&work_dir, "c", &[], &[("file3", "3")]);
    create_commit_with_files(&work_dir, "d2", &["a"], &[("file4", "4")]);
    work_dir.run_jj(["rebase", "-r", "d2", "-o", "b"]).success();
    work_dir
        .run_jj(["bookmark", "create", "d1", "-r", "at_operation(@-, d2)"])
        .success();
    work_dir.run_jj(["rebase", "-r", "d1", "-o", "c"]).success();

    // Test the setup (commit D is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d2  vruxwmqv/1  c3de3020 - description: d2
    ○  b  zsuskuln  38bded60 - description: b
    │ ○  d1  vruxwmqv/0  4bcd1134 - description: d2
    │ ○  c  royxmykx  b616a3ce - description: c
    ├─╯
    │ ○  a  rlvkpnrz  e9a731d9 - description: a
    ├─╯
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "d1"), @r"
    ○  vruxwmqv/0 4bcd1134 (divergent) d2
    ○  vruxwmqv/2 459038a5 (hidden) d2
    ○  vruxwmqv/3 b31c58cf (hidden) (empty) d2
    [EOF]
    ");

    insta::assert_snapshot!(get_evolog(&work_dir, "d2"), @r"
    @  vruxwmqv/1 c3de3020 (divergent) d2
    ○  vruxwmqv/2 459038a5 (hidden) d2
    ○  vruxwmqv/3 b31c58cf (hidden) (empty) d2
    [EOF]
    ");

    // First check behavior in non-interactive mode. The command cannot determine
    // which parents to use, so it should fail.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge", "--no-interactive"]));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: vruxwmqvtpmx with 2 commits:
        vruxwmqv/0 4bcd1134 d1 | (divergent) d2
        vruxwmqv/1 c3de3020 d2 | (divergent) d2

    Attempting to converge change vruxwmqvtpmx...

    Could not determine which parents to use.
    Error: Could not converge change
    [EOF]
    [exit status: 1]
    ");

    // Run the command again, but this time choose to abort at the prompt.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("q\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: vruxwmqvtpmx with 2 commits:
        vruxwmqv/0 4bcd1134 d1 | (divergent) d2
        vruxwmqv/1 c3de3020 d2 | (divergent) d2

    Attempting to converge change vruxwmqvtpmx...

    Could not determine automatically which parents to use
    1: 4bcd1134ba57:
          Parent: royxmykx b616a3ce c | c
    2: c3de3020707a:
          Parent: zsuskuln 38bded60 b | b
    q: abort
    Enter the index of one of the divergent commits whose parent(s) will be the parents of the solution: 

    Error: Aborting... nothing changed.
    [EOF]
    [exit status: 1]
    ");

    // Run the command one more time, this time the user chooses parents.
    let output =
        work_dir.run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("2\n"));
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Found 1 divergent change(s) in the specified revset:
    - Change: vruxwmqvtpmx with 2 commits:
        vruxwmqv/0 4bcd1134 d1 | (divergent) d2
        vruxwmqv/1 c3de3020 d2 | (divergent) d2

    Attempting to converge change vruxwmqvtpmx...

    Could not determine automatically which parents to use
    1: 4bcd1134ba57:
          Parent: royxmykx b616a3ce c | c
    2: c3de3020707a:
          Parent: zsuskuln 38bded60 b | b
    q: abort
    Enter the index of one of the divergent commits whose parent(s) will be the parents of the solution: 

    Successfully converged change: created commit 5a4258f7aa61.
    Working copy  (@) now at: vruxwmqv 5a4258f7 d1 d2 | d2
    Parent commit (@-)      : zsuskuln 38bded60 b | b
    [EOF]
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  d1 d2  vruxwmqv  5a4258f7 - description: d2
    ○  b  zsuskuln  38bded60 - description: b
    │ ○  c  royxmykx  b616a3ce - description: c
    ├─╯
    │ ○  a  rlvkpnrz  e9a731d9 - description: a
    ├─╯
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "d2"), @r"
    @    vruxwmqv 5a4258f7 d2
    ├─╮
    │ ○  vruxwmqv/2 c3de3020 (hidden) d2
    ○ │  vruxwmqv/1 4bcd1134 (hidden) d2
    ├─╯
    ○  vruxwmqv/3 459038a5 (hidden) d2
    ○  vruxwmqv/4 b31c58cf (hidden) (empty) d2
    [EOF]
    ");
}

// It is possible that a divergent commit is a child of another divergent commit
// (with the same change-id or a different one). Consider the case where both
// parent and child have the same change-id. When converging that change-id the
// algorithm --or the user-- must choose the parent commit(s) of the solution.
// To be concrete, say commit A is the parent of commit B, both with the same
// change-id.
//
// Whether automatically or by user choice, `jj converge` is (currently)
// designed such that the parent(s) of the solution is the parent(s) of one of
// the divergent commits. BUT `jj converge` is "smart" and only considers
// divergent commits that are not descendants of other divergent commits during
// this parent selection process. Please see the implementation for more
// details.
//
// This test is to ensure that the above behavior is correct.
#[test]
fn test_converge_one_divergent_commit_is_a_descendant_of_another_divergent_commit() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Start by setting description to "message 1", then simulate two concurrent
    // operations, one changing the description to "message 2" and the other
    // changing it to "message 3" (--at-op=@- is what allows us to pretend these two
    // operations happen concurrently). At this point we have two divergent commits.
    // Then we set up bookmarks b2 and b3 to point to the commits. Finally we rebase
    // b2 onto b3. This sets the stage for this test's scenario.
    work_dir.run_jj(["describe", "-m", "message 1"]).success();
    work_dir.run_jj(["describe", "-m", "message 2"]).success();
    work_dir
        .run_jj(["describe", "-m", "message 3", "--at-op", "@-"])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "-r",
            "description('message 2*')",
            "b2",
        ])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "-r",
            "description('message 3*')",
            "b3",
        ])
        .success();
    work_dir
        .run_jj(["rebase", "-r", "b2", "-d", "b3", "--keep-divergent"])
        .success();

    // Test the setup: look at the operation log.
    insta::assert_snapshot!(get_op_log_output(&work_dir), @r"
    @  rebase commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj rebase -r b2 -d b3 --keep-divergent
    ○  create bookmark b3 pointing to commit 4734557e78fe9ccdb827e03e71602c685bfe8b53
    │  args: jj bookmark create -r 'description(\'message 3*\')' b3
    ○  create bookmark b2 pointing to commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○    reconcile divergent operations
    ├─╮  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○ │  describe commit a289638d100c5af526559dfafb99f062631771c4
    │ │  args: jj describe -m 'message 2'
    │ ○  describe commit a289638d100c5af526559dfafb99f062631771c4
    ├─╯  args: jj describe -m 'message 3' --at-op @-
    ○  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 1'
    ○  add workspace 'default'
    ○
    [EOF]
    ");

    // Test the setup: look at the commit graph (commit B is duplicated and commit E
    // is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  b2  qpvuntsm/0  cca75b59 - description: message 2
    ○  b3  qpvuntsm/1  4734557e - description: message 3
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    @  qpvuntsm/0 cca75b59 (divergent) (empty) message 2
    ○  qpvuntsm/2 59df0df7 (hidden) (empty) message 2
    ○  qpvuntsm/3 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/4 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b3"), @r"
    ○  qpvuntsm/1 4734557e (divergent) (empty) message 3
    ○  qpvuntsm/3 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/4 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Run `jj converge` command and check the output. In this case the user must
    // merge the descriptions in a text editor. However, the parents are chosen
    // automatically (b2 is ignored because it is a descendant of b3).
    std::fs::write(
        &edit_script,
        ["dump editor0", "write\nmy-merged-description"].join("\0"),
    )?;
    let output = work_dir
        .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("y\n"))
        .success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0"))?, @r#"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: qpvuntsm a289638d "message 1"
    \\\\\\\        to: qpvuntsm cca75b59 "message 2"
    -message 1
    +message 2
    +++++++ qpvuntsm 4734557e "message 3"
    message 3
    >>>>>>> conflict 1 of 1 ends
    "#);
    insta::assert_snapshot!(output.stdout.normalized(), @"");
    insta::assert_snapshot!(output.stderr.normalized(), @r"
    Found 1 divergent change(s) in the specified revset:
    - Change: qpvuntsmwlqt with 2 commits:
        qpvuntsm/0 cca75b59 b2 | (divergent) (empty) message 2
        qpvuntsm/1 4734557e b3 | (divergent) (empty) message 3

    Attempting to converge change qpvuntsmwlqt...

    There are divergent descriptions. You can choose to merge them now in a
    text editor, or skip merging and use the conflicted description (with
    conflict markers). Do you want to merge them now? (Yn): 

    Successfully converged change: created commit ae6fe4a3240c.
    Working copy  (@) now at: qpvuntsm ae6fe4a3 b2 b3 | (empty) my-merged-description
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    ");

    // Verify the commit graph after converge
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @  b2 b3  qpvuntsm  ae6fe4a3 - description: my-merged-des...
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the operation log after converge
    insta::assert_snapshot!(get_op_log_output(&work_dir), @r"
    @  converge qpvuntsmwlqt with 2 predecessors
    │  args: jj converge
    ○  rebase commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj rebase -r b2 -d b3 --keep-divergent
    ○  create bookmark b3 pointing to commit 4734557e78fe9ccdb827e03e71602c685bfe8b53
    │  args: jj bookmark create -r 'description(\'message 3*\')' b3
    ○  create bookmark b2 pointing to commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○    reconcile divergent operations
    ├─╮  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○ │  describe commit a289638d100c5af526559dfafb99f062631771c4
    │ │  args: jj describe -m 'message 2'
    │ ○  describe commit a289638d100c5af526559dfafb99f062631771c4
    ├─╯  args: jj describe -m 'message 3' --at-op @-
    ○  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 1'
    ○  add workspace 'default'
    ○
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    @    qpvuntsm ae6fe4a3 (empty) my-merged-description
    ├─╮
    │ ○  qpvuntsm/2 4734557e (hidden) (empty) message 3
    ○ │  qpvuntsm/1 cca75b59 (hidden) (empty) message 2
    ○ │  qpvuntsm/3 59df0df7 (hidden) (empty) message 2
    ├─╯
    ○  qpvuntsm/4 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/5 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    Ok(())
}

// Similar to
// test_converge_one_divergent_commit_is_a_descendant_of_another_divergent_commit,
// but with a few variations:
// * There is an related commit "in between" the two divergent commits
// * There are other unrelated commits
#[test]
fn test_converge_two_divergent_commits_with_unrelated_commit_in_between() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Start by setting description to "message 1", then simulate two concurrent
    // operations, one changing the description to "message 2" and the other
    // changing it to "message 3" (--at-op=@- is what allows us to pretend these two
    // operations happen concurrently). At this point we have two divergent commits.
    // Then we set up bookmarks b2 and b3 to point to the commits. After that we
    // create foo as a child of b3, then we rebase b2 onto foo. This sets the
    // stage for this test's scenario. We create two other commits (bar and baz)
    // to observe how descendants are rebased.
    work_dir.run_jj(["describe", "-m", "message 1"]).success();
    work_dir.run_jj(["describe", "-m", "message 2"]).success();
    work_dir
        .run_jj(["describe", "-m", "message 3", "--at-op", "@-"])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "-r",
            "description('message 2*')",
            "b2",
        ])
        .success();
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "-r",
            "description('message 3*')",
            "b3",
        ])
        .success();
    work_dir.run_jj(["new", "-r", "b3", "-m", "foo"]).success();
    work_dir
        .run_jj(["rebase", "-r", "b2", "-d", "@", "--keep-divergent"])
        .success();
    work_dir.run_jj(["new", "-r", "b2", "-m", "bar"]).success();
    work_dir.run_jj(["new", "-r", "b3", "-m", "baz"]).success();

    // Test the setup: look at the operation log.
    insta::assert_snapshot!(get_op_log_output(&work_dir), @r"
    @  new empty commit
    │  args: jj new -r b3 -m baz
    ○  new empty commit
    │  args: jj new -r b2 -m bar
    ○  rebase commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj rebase -r b2 -d @ --keep-divergent
    ○  new empty commit
    │  args: jj new -r b3 -m foo
    ○  create bookmark b3 pointing to commit 4734557e78fe9ccdb827e03e71602c685bfe8b53
    │  args: jj bookmark create -r 'description(\'message 3*\')' b3
    ○  create bookmark b2 pointing to commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○    reconcile divergent operations
    ├─╮  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○ │  describe commit a289638d100c5af526559dfafb99f062631771c4
    │ │  args: jj describe -m 'message 2'
    │ ○  describe commit a289638d100c5af526559dfafb99f062631771c4
    ├─╯  args: jj describe -m 'message 3' --at-op @-
    ○  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 1'
    ○  add workspace 'default'
    ○
    [EOF]
    ");

    // Test the setup: look at the commit graph (commit B is duplicated and commit E
    // is duplicated)
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @    znkkpsqq  aac9c864 - description: baz
    │ ○    yostqsxw  38a29791 - description: bar
    │ ○  b2  qpvuntsm/0  2a258b0d - description: message 2
    │ ○    yqosqzyt  f658f253 - description: foo
    ├─╯
    ○  b3  qpvuntsm/1  4734557e - description: message 3
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○  qpvuntsm/0 2a258b0d (divergent) (empty) message 2
    ○  qpvuntsm/2 59df0df7 (hidden) (empty) message 2
    ○  qpvuntsm/3 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/4 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Test the setup: look at the evolog
    insta::assert_snapshot!(get_evolog(&work_dir, "b3"), @r"
    ○  qpvuntsm/1 4734557e (divergent) (empty) message 3
    ○  qpvuntsm/3 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/4 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    // Run `jj converge` command and check the output. In this case the user must
    // merge the descriptions in a text editor. However, the parents are chosen
    // automatically (b2 is ignored because it is a descendant of b3).
    std::fs::write(
        &edit_script,
        ["dump editor0", "write\nmy-merged-description"].join("\0"),
    )?;
    let output = work_dir
        .run_jj_with(|cmd| force_interactive(cmd).args(["converge"]).write_stdin("y\n"))
        .success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0"))?, @r#"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: qpvuntsm a289638d "message 1"
    \\\\\\\        to: qpvuntsm 2a258b0d "message 2"
    -message 1
    +message 2
    +++++++ qpvuntsm 4734557e "message 3"
    message 3
    >>>>>>> conflict 1 of 1 ends
    "#);
    insta::assert_snapshot!(output.stdout.normalized(), @"");
    insta::assert_snapshot!(output.stderr.normalized(), @r"
    Found 1 divergent change(s) in the specified revset:
    - Change: qpvuntsmwlqt with 2 commits:
        qpvuntsm/0 2a258b0d b2 | (divergent) (empty) message 2
        qpvuntsm/1 4734557e b3 | (divergent) (empty) message 3

    Attempting to converge change qpvuntsmwlqt...

    There are divergent descriptions. You can choose to merge them now in a
    text editor, or skip merging and use the conflicted description (with
    conflict markers). Do you want to merge them now? (Yn): 

    Successfully converged change: created commit 605808281071.
    Rebased 3 descendants
    Working copy  (@) now at: znkkpsqq 3ea044e0 (empty) baz
    Parent commit (@-)      : qpvuntsm 60580828 b2 b3 | (empty) my-merged-description
    ");

    insta::assert_snapshot!(work_dir.run_jj(["op", "show"]).success(), @r"
    23ae9388893c test-username@host.example.com default@ 2001-02-03 04:05:21.000 +07:00 - 2001-02-03 04:05:21.000 +07:00
    converge qpvuntsmwlqt with 2 predecessors
    args: jj converge

    Changed commits:
    ○  + znkkpsqq 3ea044e0 (empty) baz
    │  - znkkpsqq/1 aac9c864 (hidden) (empty) baz
    │ ○  + yostqsxw a4dd040e (empty) bar
    ├─╯  - yostqsxw/1 38a29791 (hidden) (empty) bar
    │ ○  + yqosqzyt e85707a5 (empty) foo
    ├─╯  - yqosqzyt/1 f658f253 (hidden) (empty) foo
    ○  + qpvuntsm 60580828 b2 b3 | (empty) my-merged-description
       - qpvuntsm/1 2a258b0d (hidden) (empty) message 2
       - qpvuntsm/2 4734557e (hidden) (empty) message 3

    Changed working copy default@:
    + znkkpsqq 3ea044e0 (empty) baz
    - znkkpsqq/1 aac9c864 (hidden) (empty) baz

    Changed local bookmarks:
    b2:
    + qpvuntsm 60580828 b2 b3 | (empty) my-merged-description
    - qpvuntsm/1 2a258b0d (hidden) (empty) message 2
    b3:
    + qpvuntsm 60580828 b2 b3 | (empty) my-merged-description
    - qpvuntsm/2 4734557e (hidden) (empty) message 3
    [EOF]
    ");

    // Verify the commit graph after converge; notice the commit that was
    // "sandwiched" between b2 and b3 (foo) is now a child of the solution commit.
    insta::assert_snapshot!(get_long_log_output(&work_dir), @r"
    @    znkkpsqq  3ea044e0 - description: baz
    │ ○    yostqsxw  a4dd040e - description: bar
    ├─╯
    │ ○    yqosqzyt  e85707a5 - description: foo
    ├─╯
    ○  b2 b3  qpvuntsm  60580828 - description: my-merged-des...
    ◆    zzzzzzzz  00000000
    [EOF]
    ");

    // Verify the operation log after converge
    insta::assert_snapshot!(get_op_log_output(&work_dir), @r"
    @  converge qpvuntsmwlqt with 2 predecessors
    │  args: jj converge
    ○  new empty commit
    │  args: jj new -r b3 -m baz
    ○  new empty commit
    │  args: jj new -r b2 -m bar
    ○  rebase commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj rebase -r b2 -d @ --keep-divergent
    ○  new empty commit
    │  args: jj new -r b3 -m foo
    ○  create bookmark b3 pointing to commit 4734557e78fe9ccdb827e03e71602c685bfe8b53
    │  args: jj bookmark create -r 'description(\'message 3*\')' b3
    ○  create bookmark b2 pointing to commit 59df0df7968367d456d4438cc68ebe6a316ef8ce
    │  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○    reconcile divergent operations
    ├─╮  args: jj bookmark create -r 'description(\'message 2*\')' b2
    ○ │  describe commit a289638d100c5af526559dfafb99f062631771c4
    │ │  args: jj describe -m 'message 2'
    │ ○  describe commit a289638d100c5af526559dfafb99f062631771c4
    ├─╯  args: jj describe -m 'message 3' --at-op @-
    ○  describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  args: jj describe -m 'message 1'
    ○  add workspace 'default'
    ○
    [EOF]
    ");

    // Verify the evolution history after converge
    insta::assert_snapshot!(get_evolog(&work_dir, "b2"), @r"
    ○    qpvuntsm 60580828 (empty) my-merged-description
    ├─╮
    │ ○  qpvuntsm/2 4734557e (hidden) (empty) message 3
    ○ │  qpvuntsm/1 2a258b0d (hidden) (empty) message 2
    ○ │  qpvuntsm/3 59df0df7 (hidden) (empty) message 2
    ├─╯
    ○  qpvuntsm/4 a289638d (hidden) (empty) message 1
    ○  qpvuntsm/5 e8849ae1 (hidden) (empty) (no description set)
    [EOF]
    ");

    Ok(())
}

#[must_use]
fn get_long_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = "bookmarks ++ '  ' ++ format_short_change_id_with_change_offset(self) ++ '  ' \
                    ++ commit_id.shortest(8) ++ surround(' - description: ', '', truncate_end(16, \
                    description.first_line(), '...'))";
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_op_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    work_dir.run_jj(["op", "log", "-T", "description ++ '\n' ++ attributes"])
}

#[must_use]
fn get_evolog<S: AsRef<str>>(work_dir: &TestWorkDir, revision: S) -> CommandOutput {
    let template = "format_commit_summary_with_refs(commit, '')";
    work_dir.run_jj(["evolog", "-r", revision.as_ref(), "-T", template])
}
