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
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    │  my description
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    │  my description
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 068224a7
    │  my description
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 2b023b5f
       (empty) my description
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["--color=always", "evolog"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:10[39m [38;5;12m6[38;5;8m6b42ad3[39m[0m
    │  [1mmy description[0m
    [1m[38;5;1m×[0m  [1m[39mr[0m[38;5;8mlvkpnrz[38;5;1m hidden [38;5;3mtest.user@example.com[38;5;1m [38;5;6m2001-02-03 08:05:09[38;5;1m [1m[38;5;4m07[0m[38;5;8mb18245[38;5;1m conflict[39m
    │  [38;5;1mmy description[39m
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m06[0m[38;5;8m8224a7[39m
    │  my description
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4m2b[0m[38;5;8m023b5f[39m
       [38;5;2m(empty)[39m my description
    [EOF]
    ");

    // There should be no diff caused by the rebase because it was a pure rebase
    // (even even though it resulted in a conflict).
    let output = work_dir.run_jj(["evolog", "-p"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    │  my description
    │  Resolved conflict in file1:
    │     1     : <<<<<<< Conflict 1 of 1
    │     2     : %%%%%%% Changes from base to side #1
    │     3     : -foo
    │     4     : +++++++ Contents of side #2
    │     5     : foo
    │     6     : bar
    │     7    1: >>>>>>> Conflict 1 of 1 endsresolved
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    │  my description
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 068224a7
    │  my description
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    │  Added regular file file2:
    │          1: foo
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 2b023b5f
       (empty) my description
    [EOF]
    ");

    // Test `--limit`
    let output = work_dir.run_jj(["evolog", "--limit=2"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    │  my description
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    │  my description
    [EOF]
    ");

    // Test `--no-graph`
    let output = work_dir.run_jj(["evolog", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    my description
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    my description
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 068224a7
    my description
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 2b023b5f
    (empty) my description
    [EOF]
    ");

    // Test `--git` format, and that it implies `-p`
    let output = work_dir.run_jj(["evolog", "--no-graph", "--git"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    my description
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
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    my description
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 068224a7
    my description
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
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 2b023b5f
    (empty) my description
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
    $  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 66b42ad3
    │  my description
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 07b18245 conflict
    │  my description
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 068224a7
    │  my description
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 2b023b5f
       (empty) my description
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
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 fa15625b
    │  (empty) first
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog"], 40, true), @r"
    @  qpvuntsm test.user@example.com
    │  2001-02-03 08:05:08 fa15625b
    │  (empty) first
    ○  qpvuntsm hidden test.user@example.com
       2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, false), @r"
    qpvuntsm test.user@example.com 2001-02-03 08:05:08 fa15625b
    (empty) first
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
    (empty) (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, true), @r"
    qpvuntsm test.user@example.com
    2001-02-03 08:05:08 fa15625b
    (empty) first
    qpvuntsm hidden test.user@example.com
    2001-02-03 08:05:07 230dd059
    (empty) (no description set)
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
    ○      qpvuntsm test.user@example.com 2001-02-03 08:05:15 d49749bf
    ├─┬─╮  squashed 3
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:15 8f2ae2b5
    │ │ │  fifth
    │ │ │  Added regular file file5:
    │ │ │          1: foo5
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:14 04d28ca9
    │ │    (empty) fifth
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:14 c5801e10
    │ │  fourth
    │ │  Added regular file file4:
    │ │          1: foo4
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:13 bb54a199
    │    (empty) fourth
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:12 1408a0a7
    ├─╮  squashed 2
    │ │  Removed regular file file2:
    │ │     1     : foo2
    │ │  Removed regular file file3:
    │ │     1     : foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:12 c9460789
    │ │  third
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │     2    2: bar
    │ │          3: baz
    │ │  Added regular file file2:
    │ │          1: foo2
    │ │  Added regular file file3:
    │ │          1: foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:11 66645763
    │ │  (empty) third
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:10 1c7afcb4
    │    (empty) (no description set)
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 e3c2a446
    ├─╮  squashed 1
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 46acd22a
    │ │  second
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │          2: bar
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 cba41deb
    │    (empty) second
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 766420db
    │  first
    │  Added regular file file1:
    │          1: foo
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 fa15625b
    │  (empty) first
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
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
    - description_placeholder
    - email_placeholder
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
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
    (empty) (no description set)
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 d8d5f980
    (empty) a
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 b4584f54
    (empty) b
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 5cb22a87
    (empty) c
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "--limit=2", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 b4584f54
    (empty) b
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 5cb22a87
    (empty) c
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
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
    │  (empty) (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 d8d5f980
    │  (empty) a
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 b4584f54
    │  (empty) b
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 5cb22a87
    │  (empty) c
    │ ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 280cbb6e
    ├─╯  (empty) d
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 031df638
    ├─╯  (empty) e
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 a177c2f2
       (empty) c+d+e
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-rdescription(c+d+e)", "--limit=3", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 280cbb6e
    │  (empty) d
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 031df638
    ├─╯  (empty) e
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 a177c2f2
       (empty) c+d+e
    [EOF]
    ");
}
