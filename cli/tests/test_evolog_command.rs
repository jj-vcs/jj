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
    │  -- operation 017d65b8b10f (2001-02-03 08:05:10) snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 693dfdbac067 (2001-02-03 08:05:09) snapshot working copy
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation c5e0697c6322 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["--color=always", "evolog"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:10[39m [38;5;12m3[38;5;8m3c10ace[39m[0m
    │  [1mmy description[0m
    │  -- operation [38;5;4m017d65b8b10f[39m ([38;5;6m2001-02-03 08:05:10[39m) snapshot working copy
    [1m[38;5;1m×[0m  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m7[0m[38;5;8mf56b2a0[39m [38;5;1mconflict[39m
    │  my description
    │  -- operation [38;5;4m4c04cc664a56[39m ([38;5;6m2001-02-03 08:05:09[39m) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m5[0m[38;5;8m1e08f95[39m
    │  my description
    │  -- operation [38;5;4m693dfdbac067[39m ([38;5;6m2001-02-03 08:05:09[39m) snapshot working copy
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4mb[0m[38;5;8m955b72e[39m
       [38;5;2m(empty)[39m my description
       -- operation [38;5;4mc5e0697c6322[39m ([38;5;6m2001-02-03 08:05:08[39m) new empty commit
    [EOF]
    ");

    // There should be no diff caused by the rebase because it was a pure rebase
    // (even even though it resulted in a conflict).
    let output = work_dir.run_jj(["evolog", "-p"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 017d65b8b10f (2001-02-03 08:05:10) snapshot working copy
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
    │  -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 693dfdbac067 (2001-02-03 08:05:09) snapshot working copy
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    │  Added regular file file2:
    │          1: foo
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation c5e0697c6322 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Test `--limit`
    let output = work_dir.run_jj(["evolog", "--limit=2"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 017d65b8b10f (2001-02-03 08:05:10) snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    [EOF]
    ");

    // Test `--no-graph`
    let output = work_dir.run_jj(["evolog", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 017d65b8b10f (2001-02-03 08:05:10) snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    my description
    -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 693dfdbac067 (2001-02-03 08:05:09) snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
    (empty) my description
    -- operation c5e0697c6322 (2001-02-03 08:05:08) new empty commit
    [EOF]
    ");

    // Test `--git` format, and that it implies `-p`
    let output = work_dir.run_jj(["evolog", "--no-graph", "--git"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 017d65b8b10f (2001-02-03 08:05:10) snapshot working copy
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
    -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 693dfdbac067 (2001-02-03 08:05:09) snapshot working copy
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
    -- operation c5e0697c6322 (2001-02-03 08:05:08) new empty commit
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
    │  -- operation 17ae64119c81 (2001-02-03 08:05:10) snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation 4c04cc664a56 (2001-02-03 08:05:09) rebase commit 068224a797cd07771e1b619d2292a53d37408adf
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 693dfdbac067 (2001-02-03 08:05:09) snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation c5e0697c6322 (2001-02-03 08:05:08) new empty commit
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
    │  -- operation ed907a7a52ab (2001-02-03 08:05:08) describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog"], 40, true), @r"
    @  qpvuntsm test.user@example.com
    │  2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation ed907a7a52ab (2001-02-03
    │  08:05:08) describe commit
    │  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ○  qpvuntsm hidden test.user@example.com
       2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation eac759b9ab75 (2001-02-03
       08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, false), @r"
    qpvuntsm test.user@example.com 2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation ed907a7a52ab (2001-02-03 08:05:08) describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, true), @r"
    qpvuntsm test.user@example.com
    2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation ed907a7a52ab (2001-02-03
    08:05:08) describe commit
    230dd059e1b059aefc0da06a2e5a7dbf22362f22
    qpvuntsm hidden test.user@example.com
    2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation eac759b9ab75 (2001-02-03
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
    │ │ │  -- operation fb6012fdba41 (2001-02-03 08:05:15) squash commits into 1408a0a73aa72183333175341ae317477a59f6ed
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:15 770795d0
    │ │ │  fifth
    │ │ │  -- operation a3babededa01 (2001-02-03 08:05:15) snapshot working copy
    │ │ │  Added regular file file5:
    │ │ │          1: foo5
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:14 2e0123d1
    │ │    (empty) fifth
    │ │    -- operation 14c5b62e4b6d (2001-02-03 08:05:14) new empty commit
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:14 ea8161b6
    │ │  fourth
    │ │  -- operation 6c9f389c8695 (2001-02-03 08:05:14) snapshot working copy
    │ │  Added regular file file4:
    │ │          1: foo4
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:13 1de5fdb6
    │    (empty) fourth
    │    -- operation d2258e5d1f83 (2001-02-03 08:05:13) new empty commit
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:12 5ec0619a
    ├─╮  squashed 2
    │ │  -- operation 8d00b094745a (2001-02-03 08:05:12) squash commits into e3c2a446fa2b94f8fefe76dc5ee05ae5624239bf
    │ │  Removed regular file file2:
    │ │     1     : foo2
    │ │  Removed regular file file3:
    │ │     1     : foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:12 cce957f1
    │ │  third
    │ │  -- operation a3e48035b454 (2001-02-03 08:05:12) snapshot working copy
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
    │ │  -- operation 199d8e83573a (2001-02-03 08:05:11) describe commit 1c7afcb43eb3b3fd8bc47b2326ef32112c4835ac
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:10 ebec10f4
    │    (empty) (no description set)
    │    -- operation 4099bf08da25 (2001-02-03 08:05:10) squash commits into 766420db930c4af683c9261908dfbd7deb3c6e12
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 69085884
    ├─╮  squashed 1
    │ │  -- operation 4099bf08da25 (2001-02-03 08:05:10) squash commits into 766420db930c4af683c9261908dfbd7deb3c6e12
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 a3759c9d
    │ │  second
    │ │  -- operation 5531c6bff23a (2001-02-03 08:05:10) snapshot working copy
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │          2: bar
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 a5b2f625
    │    (empty) second
    │    -- operation cf72f1ca0234 (2001-02-03 08:05:09) new empty commit
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 5878cbe0
    │  first
    │  -- operation 151e5ed47f69 (2001-02-03 08:05:09) snapshot working copy
    │  Added regular file file1:
    │          1: foo
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation ed907a7a52ab (2001-02-03 08:05:08) describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
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
    -- operation eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    (empty) a
    -- operation b7e997652233 (2001-02-03 08:05:08) describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation fcebcae3dc57 (2001-02-03 08:05:09) describe commit d8d5f980a897bec1a085986377897c00e531ebce
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation 53798933e116 (2001-02-03 08:05:10) describe commit b4584f54d2673e89dbc154f792879a3721baae30
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "--limit=2", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation fcebcae3dc57 (2001-02-03 08:05:09) describe commit d8d5f980a897bec1a085986377897c00e531ebce
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation 53798933e116 (2001-02-03 08:05:10) describe commit b4584f54d2673e89dbc154f792879a3721baae30
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
    │  -- operation eac759b9ab75 (2001-02-03 08:05:07) add workspace 'default'
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    │  (empty) a
    │  -- operation b7e997652233 (2001-02-03 08:05:08) describe commit 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    │  (empty) b
    │  -- operation fcebcae3dc57 (2001-02-03 08:05:09) describe commit d8d5f980a897bec1a085986377897c00e531ebce
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 b28cda4b
    │  (empty) c
    │  -- operation 53798933e116 (2001-02-03 08:05:10) describe commit b4584f54d2673e89dbc154f792879a3721baae30
    │ ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    ├─╯  (empty) d
    │    -- operation 5770c9e8f366 (2001-02-03 08:05:11) new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation a03c0f89ffb3 (2001-02-03 08:05:12) new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation dccf783bfb60 (2001-02-03 08:05:13) squash commits into 5cb22a87bdb1bfe6b5701d3ac58712b8a55f0eb5
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-rdescription(c+d+e)", "--limit=3", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    │  (empty) d
    │  -- operation 5770c9e8f366 (2001-02-03 08:05:11) new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation a03c0f89ffb3 (2001-02-03 08:05:12) new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation dccf783bfb60 (2001-02-03 08:05:13) squash commits into 5cb22a87bdb1bfe6b5701d3ac58712b8a55f0eb5
    [EOF]
    ");
}
