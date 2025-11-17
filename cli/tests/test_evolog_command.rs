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

use crate::common::TestEnvironment;
use crate::common::to_toml_value;

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
        .run_jj(["rebase", "-r", "@", "-o", "root()"])
        .success();
    work_dir.write_file("file1", "resolved\n");

    let output = work_dir.run_jj(["evolog"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 62777a103786 snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 826347115e2d snapshot working copy
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation e0f8e58b3800 new empty commit
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["--color=always", "evolog"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:10[39m [38;5;12m3[38;5;8m3c10ace[39m[0m
    │  [1mmy description[0m
    │  [38;5;8m--[39m operation [38;5;4m62777a103786[39m snapshot working copy
    [1m[38;5;1m×[0m  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m7[0m[38;5;8mf56b2a0[39m [38;5;1mconflict[39m
    │  my description
    │  [38;5;8m--[39m operation [38;5;4mad81b0a6af14[39m rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:09[39m [1m[38;5;4m5[0m[38;5;8m1e08f95[39m
    │  my description
    │  [38;5;8m--[39m operation [38;5;4m826347115e2d[39m snapshot working copy
    ○  [1m[39mr[0m[38;5;8mlvkpnrz[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4mb[0m[38;5;8m955b72e[39m
       [38;5;2m(empty)[39m my description
       [38;5;8m--[39m operation [38;5;4me0f8e58b3800[39m new empty commit
    [EOF]
    ");

    // There should be no diff caused by the rebase because it was a pure rebase
    // (even even though it resulted in a conflict).
    let output = work_dir.run_jj(["evolog", "-p"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 62777a103786 snapshot working copy
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
    │  -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 826347115e2d snapshot working copy
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    │  Added regular file file2:
    │          1: foo
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation e0f8e58b3800 new empty commit
       Modified commit description:
               1: my description
    [EOF]
    ");

    // Multiple starting revisions
    let output = work_dir.run_jj(["evolog", "-r.."]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 62777a103786 snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 826347115e2d snapshot working copy
    ○  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation e0f8e58b3800 new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 c664a51b
    │  (no description set)
    │  -- operation ca1226de0084 snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");

    // Test `--limit`
    let output = work_dir.run_jj(["evolog", "--limit=2"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 62777a103786 snapshot working copy
    ×  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    [EOF]
    ");

    // Test `--no-graph`
    let output = work_dir.run_jj(["evolog", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 62777a103786 snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    my description
    -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 826347115e2d snapshot working copy
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
    (empty) my description
    -- operation e0f8e58b3800 new empty commit
    [EOF]
    ");

    // Test `--git` format, and that it implies `-p`
    let output = work_dir.run_jj(["evolog", "--no-graph", "--git"]);
    insta::assert_snapshot!(output, @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    my description
    -- operation 62777a103786 snapshot working copy
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
    -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    my description
    -- operation 826347115e2d snapshot working copy
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
    -- operation e0f8e58b3800 new empty commit
    diff --git a/JJ-COMMIT-DESCRIPTION b/JJ-COMMIT-DESCRIPTION
    --- JJ-COMMIT-DESCRIPTION
    +++ JJ-COMMIT-DESCRIPTION
    @@ -0,0 +1,1 @@
    +my description
    [EOF]
    ");
}

#[test]
fn test_evolog_template() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "origin"])
        .success();
    let origin_dir = test_env.work_dir("origin");
    origin_dir
        .run_jj(["bookmark", "set", "-r@", "main"])
        .success();

    test_env
        .run_jj_in(".", ["git", "clone", "origin", "local"])
        .success();
    let work_dir = test_env.work_dir("local");

    // default template with operation
    let output = work_dir.run_jj(["evolog", "-r@"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 2b17ac71
       (empty) (no description set)
       -- operation 2931515731a6 add workspace 'default'
    [EOF]
    ");
    let output = work_dir.run_jj(["evolog", "-r@", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m<<evolog commit node working_copy mutable::@>>[0m  [1m[38;5;13m<<evolog working_copy mutable commit change_id shortest prefix::k>>[38;5;8m<<evolog working_copy mutable commit change_id shortest rest::kmpptxz>>[39m<<evolog working_copy mutable:: >>[38;5;3m<<evolog working_copy mutable commit author email local::test.user>><<evolog working_copy mutable commit author email::@>><<evolog working_copy mutable commit author email domain::example.com>>[39m<<evolog working_copy mutable:: >>[38;5;14m<<evolog working_copy mutable commit committer timestamp local format::2001-02-03 08:05:09>>[39m<<evolog working_copy mutable:: >>[38;5;12m<<evolog working_copy mutable commit commit_id shortest prefix::2>>[38;5;8m<<evolog working_copy mutable commit commit_id shortest rest::b17ac71>>[39m<<evolog working_copy mutable::>>[0m
       [1m[38;5;10m<<evolog working_copy mutable empty::(empty)>>[39m<<evolog working_copy mutable:: >>[38;5;10m<<evolog working_copy mutable empty description placeholder::(no description set)>>[39m<<evolog working_copy mutable::>>[0m
       [38;5;8m<<evolog separator::-->>[39m<<evolog:: operation >>[38;5;4m<<evolog operation id short::2931515731a6>>[39m<<evolog:: >><<evolog operation description first_line::add workspace 'default'>><<evolog::>>
    [EOF]
    ");

    // default template without operation
    let output = work_dir.run_jj(["evolog", "-rmain@origin"]);
    insta::assert_snapshot!(output, @r"
    ◆  qpvuntsm test.user@example.com 2001-02-03 08:05:07 main@origin e8849ae1
       (empty) (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["evolog", "-rmain@origin", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;14m<<evolog commit node immutable::◆>>[0m  [1m[38;5;5m<<evolog immutable commit change_id shortest prefix::q>>[0m[38;5;8m<<evolog immutable commit change_id shortest rest::pvuntsm>>[39m<<evolog immutable:: >>[38;5;3m<<evolog immutable commit author email local::test.user>><<evolog immutable commit author email::@>><<evolog immutable commit author email domain::example.com>>[39m<<evolog immutable:: >>[38;5;6m<<evolog immutable commit committer timestamp local format::2001-02-03 08:05:07>>[39m<<evolog immutable:: >>[38;5;5m<<evolog immutable commit bookmarks name::main>><<evolog immutable commit bookmarks::@>><<evolog immutable commit bookmarks remote::origin>>[39m<<evolog immutable:: >>[1m[38;5;4m<<evolog immutable commit commit_id shortest prefix::e>>[0m[38;5;8m<<evolog immutable commit commit_id shortest rest::8849ae1>>[39m<<evolog immutable::>>
       [38;5;2m<<evolog immutable empty::(empty)>>[39m<<evolog immutable:: >>[38;5;2m<<evolog immutable empty description placeholder::(no description set)>>[39m<<evolog immutable::>>
    [EOF]
    ");

    // default template with root commit
    let output = work_dir.run_jj(["evolog", "-rroot()"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
    let output = work_dir.run_jj(["evolog", "-rroot()", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;14m<<evolog commit node immutable::◆>>[0m  [1m[38;5;5m<<evolog immutable commit change_id shortest prefix::z>>[0m[38;5;8m<<evolog immutable commit change_id shortest rest::zzzzzzz>>[39m<<evolog immutable:: >>[38;5;2m<<evolog immutable root::root()>>[39m<<evolog immutable:: >>[1m[38;5;4m<<evolog immutable commit commit_id shortest prefix::0>>[0m[38;5;8m<<evolog immutable commit commit_id shortest rest::0000000>>[39m<<evolog immutable::>>
    [EOF]
    ");

    // JSON output with operation
    let output = work_dir.run_jj(["evolog", "-r@", "-Tjson(self)", "--no-graph"]);
    insta::assert_snapshot!(output, @r#"{"commit":{"commit_id":"2b17ac719c7db025e2514f5708d2b0328fc6b268","parents":["0000000000000000000000000000000000000000"],"change_id":"kkmpptxzrspxrzommnulwmwkkqwworpl","description":"","author":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:09+07:00"},"committer":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:09+07:00"}},"operation":{"id":"2931515731a6903101194e8e889efb13f7494077d8ec2650e2ec40ad69c32fe45385a3d333d1792ffbc410655f1e98daa404f709062a7908bc0b03a0241825bc","parents":["00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"],"time":{"start":"2001-02-03T04:05:09+07:00","end":"2001-02-03T04:05:09+07:00"},"description":"add workspace 'default'","hostname":"host.example.com","username":"test-username","is_snapshot":false,"tags":{}}}[EOF]"#);

    // JSON output without operation
    let output = work_dir.run_jj(["evolog", "-rmain@origin", "-Tjson(self)", "--no-graph"]);
    insta::assert_snapshot!(output, @r#"{"commit":{"commit_id":"e8849ae12c709f2321908879bc724fdb2ab8a781","parents":["0000000000000000000000000000000000000000"],"change_id":"qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu","description":"","author":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:07+07:00"},"committer":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:07+07:00"}},"operation":null}[EOF]"#);
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
        .run_jj(["rebase", "-r", "@", "-o", "root()"])
        .success();
    work_dir.write_file("file1", "resolved\n");

    let config = "templates.log_node='if(current_working_copy, \"$\", \"┝\")'";
    let output = work_dir.run_jj(["evolog", "--config", config]);

    insta::assert_snapshot!(output, @r"
    $  rlvkpnrz test.user@example.com 2001-02-03 08:05:10 33c10ace
    │  my description
    │  -- operation 2ea9565d2b85 snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 7f56b2a0 conflict
    │  my description
    │  -- operation ad81b0a6af14 rebase commit 51e08f95160c897080d035d330aead3ee6ed5588
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:09 51e08f95
    │  my description
    │  -- operation 826347115e2d snapshot working copy
    ┝  rlvkpnrz hidden test.user@example.com 2001-02-03 08:05:08 b955b72e
       (empty) my description
       -- operation e0f8e58b3800 new empty commit
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
    │  -- operation 75545f7ff2df describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog"], 40, true), @r"
    @  qpvuntsm test.user@example.com
    │  2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation 75545f7ff2df describe
    │  commit
    │  e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com
       2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add
       workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, false), @r"
    qpvuntsm test.user@example.com 2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation 75545f7ff2df describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");
    insta::assert_snapshot!(render(&["evolog", "--no-graph"], 40, true), @r"
    qpvuntsm test.user@example.com
    2001-02-03 08:05:08 68a50538
    (empty) first
    -- operation 75545f7ff2df describe
    commit
    e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com
    2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    -- operation 8f47435a3990 add workspace
    'default'
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
            "--from=subject(glob:fourth)|subject(glob:fifth)",
            "--into=subject(glob:squash*)",
        ])
        .success();

    let output = work_dir.run_jj(["evolog", "-p", "-rsubject(glob:squash*)"]);
    insta::assert_snapshot!(output, @r"
    ○      qpvuntsm test.user@example.com 2001-02-03 08:05:15 5f3281c6
    ├─┬─╮  squashed 3
    │ │ │  -- operation 3049d8383eb2 squash commits into 5ec0619af5cb4f7707a556a71a6f96af0bc294d2
    │ │ │  Modified commit description:
    │ │ │     1     : <<<<<<< Conflict 1 of 1
    │ │ │     2     : +++++++ Contents of side #1
    │ │ │     3    1: squashed 2
    │ │ │     4     : %%%%%%% Changes from base #1 to side #2
    │ │ │     5     : +fourth
    │ │ │     6    1: %%%%%%% Changes from base #2 to side #3
    │ │ │     7     : +fifth
    │ │ │     8     : >>>>>>> Conflict 1 of 1 ends
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:15 770795d0
    │ │ │  fifth
    │ │ │  -- operation 904e204f4bb0 snapshot working copy
    │ │ │  Added regular file file5:
    │ │ │          1: foo5
    │ │ ○  vruxwmqv hidden test.user@example.com 2001-02-03 08:05:14 2e0123d1
    │ │    (empty) fifth
    │ │    -- operation fc852ed87801 new empty commit
    │ │    Modified commit description:
    │ │            1: fifth
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:14 ea8161b6
    │ │  fourth
    │ │  -- operation 3b09d55dfa6e snapshot working copy
    │ │  Added regular file file4:
    │ │          1: foo4
    │ ○  yqosqzyt hidden test.user@example.com 2001-02-03 08:05:13 1de5fdb6
    │    (empty) fourth
    │    -- operation 9404a551035a new empty commit
    │    Modified commit description:
    │            1: fourth
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:12 5ec0619a
    ├─╮  squashed 2
    │ │  -- operation fa9796d12627 squash commits into 690858846504af0e42fde980fdacf9851559ebb8
    │ │  Modified commit description:
    │ │     1     : <<<<<<< Conflict 1 of 1
    │ │     2     : +++++++ Contents of side #1
    │ │     3    1: squashed 1
    │ │     4    1: %%%%%%% Changes from base to side #2
    │ │     5     : +third
    │ │     6     : >>>>>>> Conflict 1 of 1 ends
    │ │  Removed regular file file2:
    │ │     1     : foo2
    │ │  Removed regular file file3:
    │ │     1     : foo3
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:12 cce957f1
    │ │  third
    │ │  -- operation de96267cd621 snapshot working copy
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
    │ │  -- operation 4611a6121e8a describe commit ebec10f449ad7ab92c7293efab5e3db2d8e9fea1
    │ │  Modified commit description:
    │ │          1: third
    │ ○  zsuskuln hidden test.user@example.com 2001-02-03 08:05:10 ebec10f4
    │    (empty) (no description set)
    │    -- operation 65c81703100d squash commits into 5878cbe03cdf599c9353e5a1a52a01f4c5e0e0fa
    ○    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 69085884
    ├─╮  squashed 1
    │ │  -- operation 65c81703100d squash commits into 5878cbe03cdf599c9353e5a1a52a01f4c5e0e0fa
    │ │  Modified commit description:
    │ │     1     : <<<<<<< Conflict 1 of 1
    │ │     2     : %%%%%%% Changes from base to side #1
    │ │     3     : +first
    │ │     4     : +++++++ Contents of side #2
    │ │     5     : second
    │ │     6     : >>>>>>> Conflict 1 of 1 ends
    │ │          1: squashed 1
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:10 a3759c9d
    │ │  second
    │ │  -- operation a7b202f56742 snapshot working copy
    │ │  Modified regular file file1:
    │ │     1    1: foo
    │ │          2: bar
    │ ○  kkmpptxz hidden test.user@example.com 2001-02-03 08:05:09 a5b2f625
    │    (empty) second
    │    -- operation 26f649a0cdfa new empty commit
    │    Modified commit description:
    │            1: second
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 5878cbe0
    │  first
    │  -- operation af15122a5868 snapshot working copy
    │  Added regular file file1:
    │          1: foo
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 68a50538
    │  (empty) first
    │  -- operation 75545f7ff2df describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    │  Modified commit description:
    │          1: first
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");
}

#[test]
fn test_evolog_abandoned_op() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "");
    work_dir.run_jj(["describe", "-mfile1"]).success();
    work_dir.write_file("file2", "");
    work_dir.run_jj(["describe", "-mfile2"]).success();

    insta::assert_snapshot!(work_dir.run_jj(["evolog", "--summary"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:09 e1869e5d
    │  file2
    │  -- operation 043c31d6dd84 describe commit 32cabcfa05c604a36074d74ae59964e4e5eb18e9
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 32cabcfa
    │  file1
    │  -- operation baef907e5b55 snapshot working copy
    │  A file2
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 cb5ebdc6
    │  file1
    │  -- operation c4cf439c43a8 describe commit 093c3c9624b6cfe22b310586f5638792aa80e6d7
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 093c3c96
    │  (no description set)
    │  -- operation f41b80dc73b6 snapshot working copy
    │  A file1
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");

    // Truncate up to the last "describe -mfile2" operation
    work_dir.run_jj(["op", "abandon", "..@-"]).success();

    // Unreachable predecessors are omitted, therefore the bottom commit shows
    // diffs from the empty tree.
    insta::assert_snapshot!(work_dir.run_jj(["evolog", "--summary"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:09 e1869e5d
    │  file2
    │  -- operation ab2192a635be describe commit 32cabcfa05c604a36074d74ae59964e4e5eb18e9
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 32cabcfa
       file1
       A file1
       A file2
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
    - builtin_evolog_compact
    - builtin_log_comfortable
    - builtin_log_compact
    - builtin_log_compact_full_description
    - builtin_log_detailed
    - builtin_log_node
    - builtin_log_node_ascii
    - builtin_log_oneline
    - builtin_log_redacted
    - builtin_op_log_comfortable
    - builtin_op_log_compact
    - builtin_op_log_node
    - builtin_op_log_node_ascii
    - builtin_op_log_oneline
    - builtin_op_log_redacted
    - commit_summary_separator
    - default_commit_description
    - description_placeholder
    - email_placeholder
    - empty_commit_marker
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
    -- operation 8f47435a3990 add workspace 'default'
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    (empty) a
    -- operation ab34d1de4875 describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation 3851e9877d51 describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation 5f4c7b5cb177 describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "--limit=2", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    (empty) b
    -- operation 3851e9877d51 describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    qpvuntsm test.user@example.com 2001-02-03 08:05:10 b28cda4b
    (empty) c
    -- operation 5f4c7b5cb177 describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
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
        .run_jj(["new", "-r", "subject(glob:c)", "-m", "d"])
        .success();
    work_dir
        .run_jj(["new", "-r", "subject(glob:c)", "-m", "e"])
        .success();
    work_dir
        .run_jj([
            "squash",
            "--from=subject(glob:d)|subject(glob:e)",
            "--to=subject(glob:c)",
            "-m",
            "c+d+e",
        ])
        .success();
    let output = work_dir.run_jj(["evolog", "-r", "subject(glob:c+d+e)", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    │  -- operation 8f47435a3990 add workspace 'default'
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 b86e28cd
    │  (empty) a
    │  -- operation ab34d1de4875 describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:09 9f43967b
    │  (empty) b
    │  -- operation 3851e9877d51 describe commit b86e28cd6862624ad77e1aaf31e34b2c7545bebd
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:10 b28cda4b
    │  (empty) c
    │  -- operation 5f4c7b5cb177 describe commit 9f43967b1cdbce4ab322cb7b4636fc0362c38373
    │ ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    ├─╯  (empty) d
    │    -- operation d7ad62552658 new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation e5c6f290120f new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation b391c05c22de squash commits into b28cda4b118fc50495ca34a24f030abc078d032e
    [EOF]
    ");

    let output = work_dir.run_jj(["evolog", "-rsubject(glob:c+d+e)", "--limit=3", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  mzvwutvl hidden test.user@example.com 2001-02-03 08:05:11 6a4ff8aa
    │  (empty) d
    │  -- operation d7ad62552658 new empty commit
    │ ○  royxmykx hidden test.user@example.com 2001-02-03 08:05:12 7dea2d1d
    ├─╯  (empty) e
    │    -- operation e5c6f290120f new empty commit
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:13 78fdd026
       (empty) c+d+e
       -- operation b391c05c22de squash commits into b28cda4b118fc50495ca34a24f030abc078d032e
    [EOF]
    ");
}
