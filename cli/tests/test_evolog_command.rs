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

use crate::common::to_toml_value;
use crate::common::TestEnvironment;

#[test]
fn test_evolog_with_or_without_diff() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["new", "-m", "my description"]).success();
    work_dir.write_file("file1", "foo\nbar\n");
    work_dir.write_file("file2", "foo\n");
    work_dir
        .run_jj(["rebase", "-r", "@", "-d", "root()"])
        .success();
    work_dir.write_file("file1", "resolved\n");

    let output = work_dir.run_jj(["evolog"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 025d5a37806a (2001-02-03 08:05:10) snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 7762b5b7d914 (2001-02-03 08:05:09) snapshot working copy
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation b346767d0b59 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["--color=always", "evolog"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:10[39m [38;5;12m3[38;5;8m3c10ace[39m[0m
    │  [1mmy description[0m
    │  -- operation [38;5;4m025d5a37806a[39m ([38;5;6m2001-02-03 08:05:10[39m) snapshot working copy
    [1m[38;5;1m×[0m  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m7[0m[38;5;8mf56b2a0[39m [38;5;1mconflict[39m
    │  my description
    │  -- operation [38;5;4m7e020f09c86f[39m ([38;5;6m2001-02-03 08:05:09[39m) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m5[0m[38;5;8m1e08f95[39m
    │  my description
    │  -- operation [38;5;4m7762b5b7d914[39m ([38;5;6m2001-02-03 08:05:09[39m) snapshot working copy
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4mb[0m[38;5;8m955b72e[39m
       [38;5;2m(empty)[39m my description
       -- operation [38;5;4mb346767d0b59[39m ([38;5;6m2001-02-03 08:05:08[39m) new empty commit
    [EOF]
    ");

    // There should be no diff caused by the rebase because it was a pure rebase
    // (even even though it resulted in a conflict).
    let output = work_dir.run_jj(["evolog", "-p"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 025d5a37806a (2001-02-03 08:05:10) snapshot working copy
    │  Resolved conflict in file1:
    │     1     : <<<<<<< Conflict 1 of 1
    │     2     : %%%%%%% Changes from base to side #1
    │     3     : -foo
    │     4     : +++++++ Contents of side #2
    │     5     : foo
    │     6     : bar
    │     7    1: >>>>>>> Conflict 1 of 1 endsresolved
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 7762b5b7d914 (2001-02-03 08:05:09) snapshot working copy
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    │  Added regular file file2:
    │          1: foo
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation b346767d0b59 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Test `--limit`
    let output = work_dir.run_jj(["evolog", "--limit=2"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 025d5a37806a (2001-02-03 08:05:10) snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    [EOF]
    ");

    // Test `--no-graph`
    let output = work_dir.run_jj(["evolog", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 025d5a37806a (2001-02-03 08:05:10) snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    my description
    -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 7762b5b7d914 (2001-02-03 08:05:09) snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
    (empty) my description
    -- operation b346767d0b59 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Test `--git` format, and that it implies `-p`
    let output = work_dir.run_jj(["evolog", "--no-graph", "--git"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 025d5a37806a (2001-02-03 08:05:10) snapshot working copy
    diff --git a/file1 b/file1
    index 0000000000..2ab19ae607 100644
    --- a/file1
    +++ b/file1
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --foo
    -+++++++ Contents of side #2
    -foo
    -bar
    ->>>>>>> Conflict 1 of 1 ends
    +resolved
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    my description
    -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 7762b5b7d914 (2001-02-03 08:05:09) snapshot working copy
    diff --git a/file1 b/file1
    index 257cc5642c..3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    diff --git a/file2 b/file2
    new file mode 100644
    index 0000000000..257cc5642c
    --- /dev/null
    +++ b/file2
    @@ -0,0 +1,1 @@
    +foo
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
    (empty) my description
    -- operation b346767d0b59 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");
}

#[test]
fn test_evolog_with_custom_symbols() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["new", "-m", "my description"]).success();
    work_dir.write_file("file1", "foo\nbar\n");
    work_dir.write_file("file2", "foo\n");
    work_dir
        .run_jj(["rebase", "-r", "@", "-d", "root()"])
        .success();
    work_dir.write_file("file1", "resolved\n");

    let config = "templates.log_node='if(current_working_copy, \"$\", \"┝\")'";
    let output = work_dir.run_jj(["evolog", "--config", config]);

    insta::assert_snapshot!(output, @r"
    $  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 86a4e29465c2 (2001-02-03 08:05:10) snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 7e020f09c86f (2001-02-03 08:05:09) rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 7762b5b7d914 (2001-02-03 08:05:09) snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation b346767d0b59 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");
}

#[test]
fn test_evolog_word_wrap() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |args: &[&str], columns: u32, word_wrap: bool| {
        let word_wrap = to_toml_value(word_wrap);
        work_dir.run_jj_with(|cmd| {
            cmd.args(args)
                .arg(format!("--config=ui.log-word-wrap={word_wrap}"))
                .env("COLUMNS", columns.to_string())
        })
    };

    work_dir.run_jj(["describe", "-m", "first"]).success();

    // ui.log-word-wrap option applies to both graph/no-graph outputs
    insta::assert_snapshot!(render(&["evolog"], 40, false), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation b1e0f2240b93 (2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog"], 40, true), @r"
    @  qpvuntsm test.user@example.com
    │  2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation b1e0f2240b93 (2001-02-03
    │  08:05:08) describe commit
    │  e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com
       2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 2affa7025254 (2001-02-03
       08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, false), @r"
    qpvuntsm test.user@example.com 2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation b1e0f2240b93 (2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, true), @r"
    qpvuntsm test.user@example.com
    2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation b1e0f2240b93 (2001-02-03
    08:05:08) describe commit
    e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com
    2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation 2affa7025254 (2001-02-03
    08:05:07) add workspace 'default'
    [EOF]
    ");
}

#[test]
fn test_evolog_squash() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["new", "-m", "second"]).success();
    work_dir.write_file("file1", "foo\nbar\n");

    // not partial
    work_dir.run_jj(["squash", "-m", "squashed 1"]).success();

    work_dir.run_jj(["describe", "-m", "third"]).success();
    work_dir.write_file("file1", "foo\nbar\nbaz\n");
    work_dir.write_file("file2", "foo2\n");
    work_dir.write_file("file3", "foo3\n");

    // partial
    work_dir
        .run_jj(["squash", "-m", "squashed 2", "file1"])
        .success();

    work_dir.run_jj(["new", "-m", "fourth"]).success();
    work_dir.write_file("file4", "foo4\n");

    work_dir.run_jj(["new", "-m", "fifth"]).success();
    work_dir.write_file("file5", "foo5\n");

    // multiple sources
    work_dir
        .run_jj([
            "squash",
            "-msquashed 3",
            "--from=description('fourth')|description('fifth')",
            "--into=description('squash')",
        ])
        .success();

    let output = work_dir.run_jj(["evolog", "-p", "-r", "description('squash')"]);
    insta::assert_snapshot!(output, @r"
    ○      qpvuntsm test.user@example.com 2001-02-03 08:05:15 5f3281c6
    ├─┬─╮  squashed 3
    │ │ │  -- operation a7778c702732 (2001-02-03 08:05:15) squash commits into 5ec0619af5cb4f7707a556a71a6f96af0bc294d2
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:15 770795d0
    │ │ │  fifth
    │ │ │  -- operation 16cbcedf76af (2001-02-03 08:05:15) snapshot working copy
    │ │ │  Added regular file file5:
    │ │ │          1: foo5
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:14 2e0123d1
    │ │    (empty) fifth
    │ │    -- operation 74613aebe7de (2001-02-03 08:05:14) new empty commit
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:14 ea8161b6
    │ │  fourth
    │ │  -- operation 59512c921abb (2001-02-03 08:05:14) snapshot working copy
    │ │  Added regular file file4:
    │ │          1: foo4
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:13 1de5fdb6
    │    (empty) fourth
    │    -- operation da069ee4839f (2001-02-03 08:05:13) new empty commit
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:12 5ec0619a
    ├─╮  squashed 2
    │ │  -- operation a1f7c3d1bdb4 (2001-02-03 08:05:12) squash commits into 690858846504af0e42fde980fdacf9851559ebb8
    │ │  Removed regular file file2:
    │ │     1     : foo2
    │ │  Removed regular file file3:
    │ │     1     : foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:12 cce957f1
    │ │  third
    │ │  -- operation beda2c726145 (2001-02-03 08:05:12) snapshot working copy
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │     2    2: bar
    │ │          3: baz
    │ │  Added regular file file2:
    │ │          1: foo2
    │ │  Added regular file file3:
    │ │          1: foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:11 3a2a4253
    │ │  (empty) third
    │ │  -- operation 764bc1a8d6f9 (2001-02-03 08:05:11) describe commit ebec10f449ad7ab92c7293efab5e3db2d8e9fea1
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:10 ebec10f4
    │    (empty) (no description set)
    │    -- operation 8dd70f79d831 (2001-02-03 08:05:10) squash commits into 5878cbe03cdf599c9353e5a1a52a01f4c5e0e0fa
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 69085884
    ├─╮  squashed 1
    │ │  -- operation 8dd70f79d831 (2001-02-03 08:05:10) squash commits into 5878cbe03cdf599c9353e5a1a52a01f4c5e0e0fa
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 a3759c9d
    │ │  second
    │ │  -- operation 18c05d4f1d89 (2001-02-03 08:05:10) snapshot working copy
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │          2: bar
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 a5b2f625
    │    (empty) second
    │    -- operation ddcc7b330378 (2001-02-03 08:05:09) new empty commit
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 5878cbe0
    │  first
    │  -- operation 1a874febba08 (2001-02-03 08:05:09) snapshot working copy
    │  Added regular file file1:
    │          1: foo
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation b1e0f2240b93 (2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
}

#[test]
fn test_evolog_with_no_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["evolog", "-T"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: a value is required for '--template <TEMPLATE>' but none was supplied

    For more information, try '--help'.
    Hint: The following template aliases are defined:
    - builtin_config_list
    - builtin_config_list_detailed
    - builtin_draft_commit_description
    - builtin_log_comfortable
    - builtin_log_compact
    - builtin_log_compact_full_description
    - builtin_log_detailed
    - builtin_log_node
    - builtin_log_node_ascii
    - builtin_log_oneline
    - builtin_op_log_comfortable
    - builtin_op_log_compact
    - builtin_op_log_node
    - builtin_op_log_node_ascii
    - builtin_op_log_oneline
    - commit_summary_separator
    - default_commit_description
    - description_placeholder
    - email_placeholder
    - git_format_patch_email_headers
    - name_placeholder
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_evolog_reversed_no_graph() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "a"]).success();
    work_dir.run_jj(["describe", "-m", "b"]).success();
    work_dir.run_jj(["describe", "-m", "c"]).success();
    let output = work_dir.run_jj(["evolog", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    (empty) a
    -- operation f4c629a20c1b (2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation 61a1d1adc410 (2001-02-03 08:05:09) describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation b1da3d5c6882 (2001-02-03 08:05:10) describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "--limit=2", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation 61a1d1adc410 (2001-02-03 08:05:09) describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation b1da3d5c6882 (2001-02-03 08:05:10) describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
    [EOF]
    ");
}

#[test]
fn test_evolog_reverse_with_graph() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "a"]).success();
    work_dir.run_jj(["describe", "-m", "b"]).success();
    work_dir.run_jj(["describe", "-m", "c"]).success();
    work_dir
        .run_jj(["new", "-r", "description(c)", "-m", "d"])
        .success();
    work_dir
        .run_jj(["new", "-r", "description(c)", "-m", "e"])
        .success();
    work_dir
        .run_jj([
            "squash",
            "--from",
            "description(d)|description(e)",
            "--to",
            "description(c)",
            "-m",
            "c+d+e",
        ])
        .success();
    let output = work_dir.run_jj(["evolog", "-r", "description(c+d+e)", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    │  -- operation 2affa7025254 (2001-02-03 08:05:07) add workspace 'default'
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    │  (empty) a
    │  -- operation f4c629a20c1b (2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    │  (empty) b
    │  -- operation 61a1d1adc410 (2001-02-03 08:05:09) describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 b28cda4b
    │  (empty) c
    │  -- operation b1da3d5c6882 (2001-02-03 08:05:10) describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
    │ ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    ├─╯  (empty) d
    │    -- operation 13ea929f5755 (2001-02-03 08:05:11) new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation bec55e5d0249 (2001-02-03 08:05:12) new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation 0a0d75e9ee99 (2001-02-03 08:05:13) squash commits into b28cda4b118fc50495ca34a24f030abc078d032e
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-rdescription(c+d+e)", "--limit=3", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    │  (empty) d
    │  -- operation 13ea929f5755 (2001-02-03 08:05:11) new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation bec55e5d0249 (2001-02-03 08:05:12) new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation 0a0d75e9ee99 (2001-02-03 08:05:13) squash commits into b28cda4b118fc50495ca34a24f030abc078d032e
    [EOF]
    ");
}
