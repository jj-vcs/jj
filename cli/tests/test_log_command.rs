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
fn test_log_with_empty_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-r="]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: a value is required for '--revisions <REVSETS>' but none was supplied

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_log_with_no_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-T"]);
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
fn test_log_with_or_without_diff() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["describe", "-m", "add a file"]).success();
    work_dir.run_jj(["new", "-m", "a new commit"]).success();
    work_dir.write_file("file1", "foo\nbar\n");

    let output = work_dir.run_jj(["log", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    ○  add a file
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "-p"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    ○  add a file
    │  Added regular file file1:
    │          1: foo
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    a new commit
    add a file
    [EOF]
    ");

    // `-G` is the short name of --no-graph
    let output = work_dir.run_jj(["log", "-T", r#"commit_id.short() ++ "\n""#, "-G"]);
    insta::assert_snapshot!(output, @r"
    58c940c45833
    007859d3ad71
    000000000000
    [EOF]
    ");

    // `-p` for default diff output, `-s` for summary
    let output = work_dir.run_jj(["log", "-T", "description", "-p", "-s"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  M file1
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    ○  add a file
    │  A file1
    │  Added regular file file1:
    │          1: foo
    ◆
    [EOF]
    ");

    // `-s` for summary, `--git` for git diff (which implies `-p`)
    let output = work_dir.run_jj(["log", "-T", "description", "-s", "--git"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  M file1
    │  diff --git a/file1 b/file1
    │  index 257cc5642c..3bd1f0e297 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,1 +1,2 @@
    │   foo
    │  +bar
    ○  add a file
    │  A file1
    │  diff --git a/file1 b/file1
    │  new file mode 100644
    │  index 0000000000..257cc5642c
    │  --- /dev/null
    │  +++ b/file1
    │  @@ -0,0 +1,1 @@
    │  +foo
    ◆
    [EOF]
    ");

    // `-p` for default diff output, `--stat` for diff-stat
    let output = work_dir.run_jj(["log", "-T", "description", "-p", "--stat"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  file1 | 1 +
    │  1 file changed, 1 insertion(+), 0 deletions(-)
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    ○  add a file
    │  file1 | 1 +
    │  1 file changed, 1 insertion(+), 0 deletions(-)
    │  Added regular file file1:
    │          1: foo
    ◆
       0 files changed, 0 insertions(+), 0 deletions(-)
    [EOF]
    ");

    // `--stat` is short format, which should be printed first
    let output = work_dir.run_jj(["log", "-T", "description", "--git", "--stat"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  file1 | 1 +
    │  1 file changed, 1 insertion(+), 0 deletions(-)
    │  diff --git a/file1 b/file1
    │  index 257cc5642c..3bd1f0e297 100644
    │  --- a/file1
    │  +++ b/file1
    │  @@ -1,1 +1,2 @@
    │   foo
    │  +bar
    ○  add a file
    │  file1 | 1 +
    │  1 file changed, 1 insertion(+), 0 deletions(-)
    │  diff --git a/file1 b/file1
    │  new file mode 100644
    │  index 0000000000..257cc5642c
    │  --- /dev/null
    │  +++ b/file1
    │  @@ -0,0 +1,1 @@
    │  +foo
    ◆
       0 files changed, 0 insertions(+), 0 deletions(-)
    [EOF]
    ");

    // `-p` enables default "summary" output, so `-s` is noop
    let output = work_dir.run_jj([
        "log",
        "-T",
        "description",
        "-p",
        "-s",
        "--config=ui.diff-formatter=:summary",
    ]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  M file1
    ○  add a file
    │  A file1
    ◆
    [EOF]
    ");

    // `-p` enables default "color-words" diff output, so `--color-words` is noop
    let output = work_dir.run_jj(["log", "-T", "description", "-p", "--color-words"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  Modified regular file file1:
    │     1    1: foo
    │          2: bar
    ○  add a file
    │  Added regular file file1:
    │          1: foo
    ◆
    [EOF]
    ");

    // `--git` enables git diff, so `-p` is noop
    let output = work_dir.run_jj(["log", "-T", "description", "--no-graph", "-p", "--git"]);
    insta::assert_snapshot!(output, @r"
    a new commit
    diff --git a/file1 b/file1
    index 257cc5642c..3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    add a file
    diff --git a/file1 b/file1
    new file mode 100644
    index 0000000000..257cc5642c
    --- /dev/null
    +++ b/file1
    @@ -0,0 +1,1 @@
    +foo
    [EOF]
    ");

    // Cannot use both `--git` and `--color-words`
    let output = work_dir.run_jj([
        "log",
        "-T",
        "description",
        "--no-graph",
        "-p",
        "--git",
        "--color-words",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '--git' cannot be used with '--color-words'

    Usage: jj log --template <TEMPLATE> --no-graph --patch --git [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // `-s` with or without graph
    let output = work_dir.run_jj(["log", "-T", "description", "-s"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  M file1
    ○  add a file
    │  A file1
    ◆
    [EOF]
    ");
    let output = work_dir.run_jj(["log", "-T", "description", "--no-graph", "-s"]);
    insta::assert_snapshot!(output, @r"
    a new commit
    M file1
    add a file
    A file1
    [EOF]
    ");

    // `--git` implies `-p`, with or without graph
    let output = work_dir.run_jj(["log", "-T", "description", "-r", "@", "--git"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  diff --git a/file1 b/file1
    ~  index 257cc5642c..3bd1f0e297 100644
       --- a/file1
       +++ b/file1
       @@ -1,1 +1,2 @@
        foo
       +bar
    [EOF]
    ");
    let output = work_dir.run_jj(["log", "-T", "description", "-r", "@", "--no-graph", "--git"]);
    insta::assert_snapshot!(output, @r"
    a new commit
    diff --git a/file1 b/file1
    index 257cc5642c..3bd1f0e297 100644
    --- a/file1
    +++ b/file1
    @@ -1,1 +1,2 @@
     foo
    +bar
    [EOF]
    ");

    // `--color-words` implies `-p`, with or without graph
    let output = work_dir.run_jj(["log", "-T", "description", "-r", "@", "--color-words"]);
    insta::assert_snapshot!(output, @r"
    @  a new commit
    │  Modified regular file file1:
    ~     1    1: foo
               2: bar
    [EOF]
    ");
    let output = work_dir.run_jj([
        "log",
        "-T",
        "description",
        "-r",
        "@",
        "--no-graph",
        "--color-words",
    ]);
    insta::assert_snapshot!(output, @r"
    a new commit
    Modified regular file file1:
       1    1: foo
            2: bar
    [EOF]
    ");
}

#[test]
fn test_log_null_terminate_multiline_descriptions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["commit", "-m", "commit 1 line 1", "-m", "commit 1 line 2"])
        .success();
    work_dir
        .run_jj(["commit", "-m", "commit 2 line 1", "-m", "commit 2 line 2"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "commit 3 line 1", "-m", "commit 3 line 2"])
        .success();

    let output = work_dir
        .run_jj([
            "log",
            "-r",
            "~root()",
            "-T",
            r#"description ++ "\0""#,
            "--no-graph",
        ])
        .success();
    insta::assert_debug_snapshot!(
        output.stdout.normalized(),
        @r#""commit 3 line 1\n\ncommit 3 line 2\n\0commit 2 line 1\n\ncommit 2 line 2\n\0commit 1 line 1\n\ncommit 1 line 2\n\0""#
    );
}

#[test]
fn test_log_shortest_accessors() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |rev, template| work_dir.run_jj(["log", "--no-graph", "-r", rev, "-T", template]);
    test_env.add_config(
        r#"
        [template-aliases]
        'format_id(id)' = 'id.shortest(12).prefix() ++ "[" ++ id.shortest(12).rest() ++ "]"'
        "#,
    );

    work_dir.write_file("file", "original file\n");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir
        .run_jj(["bookmark", "c", "-r@", "original"])
        .success();
    insta::assert_snapshot!(
        render("original", r#"format_id(change_id) ++ " " ++ format_id(commit_id)"#),
        @"q[pvuntsmwlqt] 8[216f646c36d][EOF]");

    // Create a chain of 10 commits
    for i in 1..10 {
        work_dir
            .run_jj(["new", "-m", &format!("commit{i}")])
            .success();
        work_dir.write_file("file", format!("file {i}\n"));
    }
    // Create 2^3 duplicates of the chain
    for _ in 0..3 {
        work_dir
            .run_jj(["duplicate", "subject(glob:commit*)"])
            .success();
    }

    insta::assert_snapshot!(
        render("original", r#"format_id(change_id) ++ " " ++ format_id(commit_id)"#),
        @"qpv[untsmwlqt] 82[16f646c36d][EOF]");

    insta::assert_snapshot!(
        render("::@", r#"change_id.shortest() ++ " " ++ commit_id.shortest() ++ "\n""#), @r"
    wq c2
    km 74
    kp 97
    zn 78
    yo 40
    vr bc9
    yq 28
    ro af
    mz 04
    qpv 82
    zzz 00
    [EOF]
    ");

    insta::assert_snapshot!(
        render("::@", r#"format_id(change_id) ++ " " ++ format_id(commit_id) ++ "\n""#), @r"
    wq[nwkozpkust] c2[b4c0bb3362]
    km[kuslswpqwq] 74[fcd50c0643]
    kp[qxywonksrl] 97[dcaada9b8d]
    zn[kkpsqqskkl] 78[c03ab2235b]
    yo[stqsxwqrlt] 40[1119280761]
    vr[uxwmqvtpmx] bc9[e8942b459]
    yq[osqzytrlsw] 28[edbc9658ef]
    ro[yxmykxtrkr] af[3e6a27a1d0]
    mz[vwutvlkqwt] 04[6c6a1df762]
    qpv[untsmwlqt] 82[16f646c36d]
    zzz[zzzzzzzzz] 00[0000000000]
    [EOF]
    ");

    // Can get shorter prefixes in configured revset
    test_env.add_config(r#"revsets.short-prefixes = "(@----)::""#);
    insta::assert_snapshot!(
        render("::@", r#"format_id(change_id) ++ " " ++ format_id(commit_id) ++ "\n""#), @r"
    w[qnwkozpkust] c[2b4c0bb3362]
    km[kuslswpqwq] 74[fcd50c0643]
    kp[qxywonksrl] 9[7dcaada9b8d]
    z[nkkpsqqskkl] 78[c03ab2235b]
    y[ostqsxwqrlt] 4[01119280761]
    vr[uxwmqvtpmx] bc9[e8942b459]
    yq[osqzytrlsw] 28[edbc9658ef]
    ro[yxmykxtrkr] af[3e6a27a1d0]
    mz[vwutvlkqwt] 04[6c6a1df762]
    qpv[untsmwlqt] 82[16f646c36d]
    zzz[zzzzzzzzz] 00[0000000000]
    [EOF]
    ");

    // Can disable short prefixes by setting to empty string
    test_env.add_config(r#"revsets.short-prefixes = """#);
    insta::assert_snapshot!(
        render("::@", r#"format_id(change_id) ++ " " ++ format_id(commit_id) ++ "\n""#), @r"
    wq[nwkozpkust] c2[b4c0bb3362]
    km[kuslswpqwq] 74[fcd50c0643]
    kp[qxywonksrl] 97[dcaada9b8d]
    zn[kkpsqqskkl] 78[c03ab2235b]
    yo[stqsxwqrlt] 401[119280761]
    vr[uxwmqvtpmx] bc9[e8942b459]
    yq[osqzytrlsw] 28[edbc9658ef]
    ro[yxmykxtrkr] af[3e6a27a1d0]
    mz[vwutvlkqwt] 04[6c6a1df762]
    qpv[untsmwlqt] 82[16f646c36d]
    zzz[zzzzzzzzz] 00[0000000000]
    [EOF]
    ");

    // The shortest prefix "zzz" is shadowed by bookmark
    work_dir
        .run_jj(["bookmark", "set", "-r@", "z", "zz", "zzz"])
        .success();
    insta::assert_snapshot!(
        render("root()", r#"format_id(change_id) ++ " " ++ format_id(commit_id) ++ "\n""#), @r"
    zzzz[zzzzzzzz] 00[0000000000]
    [EOF]
    ");
}

#[test]
fn test_log_bad_short_prefixes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Suppress warning in the commit summary template
    test_env.add_config("template-aliases.'format_short_id(id)' = 'id.short(8)'");

    // Error on bad config of short prefixes
    test_env.add_config(r#"revsets.short-prefixes = "!nval!d""#);
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid `revsets.short-prefixes`
    Caused by:  --> 1:1
      |
    1 | !nval!d
      | ^---
      |
      = expected <strict_identifier> or <expression>
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    // Warn on resolution of short prefixes
    test_env.add_config("revsets.short-prefixes = 'missing'");
    let output = work_dir.run_jj(["log", "-Tcommit_id.shortest()"]);
    insta::assert_snapshot!(output, @r"
    @  e
    ◆  0
    [EOF]
    ------- stderr -------
    Warning: In template expression
     --> 1:11
      |
    1 | commit_id.shortest()
      |           ^------^
      |
      = Failed to load short-prefixes index
    Failed to resolve short-prefixes disambiguation revset
    Revision `missing` doesn't exist
    [EOF]
    ");

    // Error on resolution of short prefixes
    test_env.add_config("revsets.short-prefixes = 'missing'");
    let output = work_dir.run_jj(["log", "-r0"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to resolve short-prefixes disambiguation revset
    Caused by: Revision `missing` doesn't exist
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_log_prefix_highlight_styled() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    fn prefix_format(len: Option<usize>) -> String {
        format!(
            r###"
            separate(" ",
              "Change",
              change_id.shortest({0}),
              description.first_line(),
              commit_id.shortest({0}),
              bookmarks,
            )
            "###,
            len.map(|l| l.to_string()).unwrap_or_default()
        )
    }

    work_dir.write_file("file", "original file\n");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir
        .run_jj(["bookmark", "c", "-r@", "original"])
        .success();
    insta::assert_snapshot!(
        work_dir.run_jj(["log", "-r", "original", "-T", &prefix_format(Some(12))]), @r"
    @  Change qpvuntsmwlqt initial 8216f646c36d original
    │
    ~
    [EOF]
    ");

    // Create a chain of 10 commits
    for i in 1..10 {
        work_dir
            .run_jj(["new", "-m", &format!("commit{i}")])
            .success();
        work_dir.write_file("file", format!("file {i}\n"));
    }
    // Create 2^3 duplicates of the chain
    for _ in 0..3 {
        work_dir
            .run_jj(["duplicate", "subject(glob:commit*)"])
            .success();
    }

    insta::assert_snapshot!(
        work_dir.run_jj(["log", "-r", "original", "-T", &prefix_format(Some(12))]), @r"
    ○  Change qpvuntsmwlqt initial 8216f646c36d original
    │
    ~
    [EOF]
    ");
    let output = work_dir.run_jj([
        "--color=always",
        "log",
        "-r",
        "@-----------..@",
        "-T",
        &prefix_format(Some(12)),
    ]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  Change [1m[38;5;5mwq[0m[38;5;8mnwkozpkust[39m commit9 [1m[38;5;4mc2[0m[38;5;8mb4c0bb3362[39m
    ○  Change [1m[38;5;5mkm[0m[38;5;8mkuslswpqwq[39m commit8 [1m[38;5;4m74[0m[38;5;8mfcd50c0643[39m
    ○  Change [1m[38;5;5mkp[0m[38;5;8mqxywonksrl[39m commit7 [1m[38;5;4m97[0m[38;5;8mdcaada9b8d[39m
    ○  Change [1m[38;5;5mzn[0m[38;5;8mkkpsqqskkl[39m commit6 [1m[38;5;4m78[0m[38;5;8mc03ab2235b[39m
    ○  Change [1m[38;5;5myo[0m[38;5;8mstqsxwqrlt[39m commit5 [1m[38;5;4m40[0m[38;5;8m1119280761[39m
    ○  Change [1m[38;5;5mvr[0m[38;5;8muxwmqvtpmx[39m commit4 [1m[38;5;4mbc9[0m[38;5;8me8942b459[39m
    ○  Change [1m[38;5;5myq[0m[38;5;8mosqzytrlsw[39m commit3 [1m[38;5;4m28[0m[38;5;8medbc9658ef[39m
    ○  Change [1m[38;5;5mro[0m[38;5;8myxmykxtrkr[39m commit2 [1m[38;5;4maf[0m[38;5;8m3e6a27a1d0[39m
    ○  Change [1m[38;5;5mmz[0m[38;5;8mvwutvlkqwt[39m commit1 [1m[38;5;4m04[0m[38;5;8m6c6a1df762[39m
    ○  Change [1m[38;5;5mqpv[0m[38;5;8muntsmwlqt[39m initial [1m[38;5;4m82[0m[38;5;8m16f646c36d[39m [38;5;5moriginal[39m
    [1m[38;5;14m◆[0m  Change [1m[38;5;5mzzz[0m[38;5;8mzzzzzzzzz[39m [1m[38;5;4m00[0m[38;5;8m0000000000[39m
    [EOF]
    ");
    let output = work_dir.run_jj([
        "--color=always",
        "log",
        "-r",
        "@-----------..@",
        "-T",
        &prefix_format(Some(3)),
    ]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  Change [1m[38;5;5mwq[0m[38;5;8mn[39m commit9 [1m[38;5;4mc2[0m[38;5;8mb[39m
    ○  Change [1m[38;5;5mkm[0m[38;5;8mk[39m commit8 [1m[38;5;4m74[0m[38;5;8mf[39m
    ○  Change [1m[38;5;5mkp[0m[38;5;8mq[39m commit7 [1m[38;5;4m97[0m[38;5;8md[39m
    ○  Change [1m[38;5;5mzn[0m[38;5;8mk[39m commit6 [1m[38;5;4m78[0m[38;5;8mc[39m
    ○  Change [1m[38;5;5myo[0m[38;5;8ms[39m commit5 [1m[38;5;4m40[0m[38;5;8m1[39m
    ○  Change [1m[38;5;5mvr[0m[38;5;8mu[39m commit4 [1m[38;5;4mbc9[0m
    ○  Change [1m[38;5;5myq[0m[38;5;8mo[39m commit3 [1m[38;5;4m28[0m[38;5;8me[39m
    ○  Change [1m[38;5;5mro[0m[38;5;8my[39m commit2 [1m[38;5;4maf[0m[38;5;8m3[39m
    ○  Change [1m[38;5;5mmz[0m[38;5;8mv[39m commit1 [1m[38;5;4m04[0m[38;5;8m6[39m
    ○  Change [1m[38;5;5mqpv[0m initial [1m[38;5;4m82[0m[38;5;8m1[39m [38;5;5moriginal[39m
    [1m[38;5;14m◆[0m  Change [1m[38;5;5mzzz[0m [1m[38;5;4m00[0m[38;5;8m0[39m
    [EOF]
    ");
    let output = work_dir.run_jj([
        "--color=always",
        "log",
        "-r",
        "@-----------..@",
        "-T",
        &prefix_format(None),
    ]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  Change [1m[38;5;5mwq[0m commit9 [1m[38;5;4mc2[0m
    ○  Change [1m[38;5;5mkm[0m commit8 [1m[38;5;4m74[0m
    ○  Change [1m[38;5;5mkp[0m commit7 [1m[38;5;4m97[0m
    ○  Change [1m[38;5;5mzn[0m commit6 [1m[38;5;4m78[0m
    ○  Change [1m[38;5;5myo[0m commit5 [1m[38;5;4m40[0m
    ○  Change [1m[38;5;5mvr[0m commit4 [1m[38;5;4mbc9[0m
    ○  Change [1m[38;5;5myq[0m commit3 [1m[38;5;4m28[0m
    ○  Change [1m[38;5;5mro[0m commit2 [1m[38;5;4maf[0m
    ○  Change [1m[38;5;5mmz[0m commit1 [1m[38;5;4m04[0m
    ○  Change [1m[38;5;5mqpv[0m initial [1m[38;5;4m82[0m [38;5;5moriginal[39m
    [1m[38;5;14m◆[0m  Change [1m[38;5;5mzzz[0m [1m[38;5;4m00[0m
    [EOF]
    ");
}

#[test]
fn test_log_prefix_highlight_counts_hidden_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(
        r#"
        [revsets]
        short-prefixes = "" # Disable short prefixes
        [template-aliases]
        'format_id(id)' = 'id.shortest(12).prefix() ++ "[" ++ id.shortest(12).rest() ++ "]"'
        "#,
    );

    let prefix_format = r#"
    separate(" ",
      "Change",
      format_id(change_id),
      description.first_line(),
      format_id(commit_id),
      bookmarks,
    )
    "#;

    work_dir.write_file("file", "original file\n");
    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir
        .run_jj(["bookmark", "c", "-r@", "original"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["log", "-r", "all()", "-T", prefix_format]), @r"
    @  Change q[pvuntsmwlqt] initial 8[216f646c36d] original
    ◆  Change z[zzzzzzzzzzz] 00[0000000000]
    [EOF]
    ");

    // Create 2^7 hidden commits
    work_dir.run_jj(["new", "root()", "-m", "extra"]).success();
    for _ in 0..7 {
        work_dir
            .run_jj(["duplicate", "subject(glob:extra)"])
            .success();
    }
    work_dir
        .run_jj(["abandon", "subject(glob:extra)"])
        .success();

    // The unique prefixes became longer.
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", prefix_format]), @r"
    @  Change wq[nwkozpkust] 88[e8407a4f0a]
    │ ○  Change qpv[untsmwlqt] initial 82[16f646c36d] original
    ├─╯
    ◆  Change zzz[zzzzzzzzz] 00[0000000000]
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["log", "-r", "8", "-T", prefix_format]), @r"
    ------- stderr -------
    Error: Commit ID prefix `8` is ambiguous
    [EOF]
    [exit status: 1]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["log", "-r", "88", "-T", prefix_format]), @r"
    @  Change wq[nwkozpkust] 88[e8407a4f0a]
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_log_author_format() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    insta::assert_snapshot!(work_dir.run_jj(["log", "--revisions=@"]), @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");

    let decl = "template-aliases.'format_short_signature(signature)'";
    insta::assert_snapshot!(work_dir.run_jj([
        "--config",
        &format!("{decl}='signature.email().local()'"),
        "log",
        "--revisions=@",
    ]), @r"
    @  qpvuntsm test.user 2001-02-03 08:05:07 e8849ae1
    │  (empty) (no description set)
    ~
    [EOF]
    ");
}

#[test]
fn test_log_divergence() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let template = r#"description.first_line() ++ if(divergent, " !divergence!")"#;

    work_dir.write_file("file", "foo\n");
    work_dir
        .run_jj(["describe", "-m", "description 1"])
        .success();
    // No divergence
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  description 1
    ◆
    [EOF]
    ");

    // Create divergence
    work_dir
        .run_jj(["describe", "-m", "description 2", "--at-operation", "@-"])
        .success();
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  description 1 !divergence!
    │ ○  description 2 !divergence!
    ├─╯
    ◆
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");
}

#[test]
fn test_log_reversed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();

    let output = work_dir.run_jj(["log", "-T", "description", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ◆
    ○  first
    @  second
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "--reversed", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    first
    second
    [EOF]
    ");
}

#[test]
fn test_log_filtered_by_path() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();
    work_dir.write_file("file1", "foo\nbar\n");
    work_dir.write_file("file2", "baz\n");

    // The output filtered to a non-existent file should display a warning.
    let output = work_dir.run_jj(["log", "-r", "@-", "-T", "description", "nonexistent"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    [EOF]
    "#);

    // The output filtered to a non-existent file should display a warning.
    // The warning should be displayed at the beginning of the output.
    let output = work_dir.run_jj(["log", "-T", "description", "nonexistent"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: No matching entries for paths: nonexistent
    Warning: The argument "nonexistent" is being interpreted as a fileset expression. To specify a revset, pass -r "nonexistent" instead.
    [EOF]
    "#);

    let output = work_dir.run_jj(["log", "-T", "description", "file1"]);
    insta::assert_snapshot!(output, @r"
    @  second
    ○  first
    │
    ~
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "file2"]);
    insta::assert_snapshot!(output, @r"
    @  second
    │
    ~
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "-s", "file1"]);
    insta::assert_snapshot!(output, @r"
    @  second
    │  M file1
    ○  first
    │  A file1
    ~
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "-s", "file2", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    second
    A file2
    [EOF]
    ");

    // empty revisions are filtered out by "all()" fileset.
    let output = work_dir.run_jj(["log", "-Tdescription", "-s", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  second
    │  M file1
    │  A file2
    ○  first
    │  A file1
    ~
    [EOF]
    ");

    // "root:<path>" is resolved relative to the workspace root.
    let output = test_env.run_jj_in(
        ".",
        [
            "log",
            "-R",
            work_dir.root().to_str().unwrap(),
            "-Tdescription",
            "-s",
            "root:file1",
        ],
    );
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    @  second
    │  M repo/file1
    ○  first
    │  A repo/file1
    ~
    [EOF]
    ");

    // files() revset doesn't filter the diff.
    let output = work_dir.run_jj([
        "log",
        "-T",
        "description",
        "-s",
        "-rfiles(file2)",
        "--no-graph",
    ]);
    insta::assert_snapshot!(output, @r"
    second
    M file1
    A file2
    [EOF]
    ");
}

#[test]
fn test_log_limit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "a"]).success();
    work_dir.write_file("a", "");
    work_dir.run_jj(["new", "-m", "b"]).success();
    work_dir.write_file("b", "");
    work_dir
        .run_jj(["new", "-m", "c", "subject(glob:a)"])
        .success();
    work_dir.write_file("c", "");
    work_dir
        .run_jj(["new", "-m", "d", "subject(glob:c)", "subject(glob:b)"])
        .success();

    let output = work_dir.run_jj(["log", "-T", "description", "--limit=3"]);
    insta::assert_snapshot!(output, @r"
    @    d
    ├─╮
    │ ○  b
    ○ │  c
    ├─╯
    [EOF]
    ");

    // Applied on sorted DAG
    let output = work_dir.run_jj(["log", "-T", "description", "--limit=2"]);
    insta::assert_snapshot!(output, @r"
    @    d
    ├─╮
    │ ○  b
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-T", "description", "--limit=2", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    d
    c
    [EOF]
    ");

    // Applied on reversed DAG: Because the node "a" is omitted, "b" and "c" are
    // rendered as roots.
    let output = work_dir.run_jj(["log", "-T", "description", "--limit=3", "--reversed"]);
    insta::assert_snapshot!(output, @r"
    ○  c
    │ ○  b
    ├─╯
    @  d
    [EOF]
    ");
    let output = work_dir.run_jj([
        "log",
        "-T",
        "description",
        "--limit=3",
        "--reversed",
        "--no-graph",
    ]);
    insta::assert_snapshot!(output, @r"
    b
    c
    d
    [EOF]
    ");

    // Applied on filtered commits
    let output = work_dir.run_jj(["log", "-T", "description", "--limit=1", "b", "c"]);
    insta::assert_snapshot!(output, @r"
    ○  c
    │
    ~
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: b
    [EOF]
    ");
}

#[test]
fn test_log_warn_path_might_be_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");

    // Don't warn if the file actually exists.
    let output = work_dir.run_jj(["log", "file1", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @
    │
    ~
    [EOF]
    ");

    // Warn for `jj log .` specifically, for former Mercurial users.
    let output = work_dir.run_jj(["log", ".", "-T", "description"]);
    insta::assert_snapshot!(output, @r#"
    @
    │
    ~
    [EOF]
    ------- stderr -------
    Warning: The argument "." is being interpreted as a fileset expression, but this is often not useful because all non-empty commits touch '.'. If you meant to show the working copy commit, pass -r '@' instead.
    [EOF]
    "#);

    // warn when checking `jj log .` in a subdirectory because this folder hasn't
    // been added to the working copy, yet.
    let sub_dir = work_dir.create_dir_all("dir");
    let output = sub_dir.run_jj(["log", "."]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: .
    [EOF]
    ");

    // Warn for `jj log @` instead of `jj log -r @`.
    let output = work_dir.run_jj(["log", "@", "-T", "description"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: No matching entries for paths: @
    Warning: The argument "@" is being interpreted as a fileset expression. To specify a revset, pass -r "@" instead.
    [EOF]
    "#);

    // Warn when there's no path with the provided name.
    let output = work_dir.run_jj(["log", "file2", "-T", "description"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: No matching entries for paths: file2
    Warning: The argument "file2" is being interpreted as a fileset expression. To specify a revset, pass -r "file2" instead.
    [EOF]
    "#);

    // If an explicit revision is provided, then suppress the warning.
    let output = work_dir.run_jj(["log", "@", "-r", "@", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: No matching entries for paths: @
    [EOF]
    ");
}

#[test]
fn test_default_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["describe", "-m", "add a file"]).success();

    // Set configuration to only show the root commit.
    test_env.add_config(r#"revsets.log = "root()""#);

    // Log should only contain one line (for the root commit), and not show the
    // commit created above.
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "commit_id"]), @r"
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");

    // The default revset is not used if a path is specified
    insta::assert_snapshot!(work_dir.run_jj(["log", "file1", "-T", "description"]), @r"
    @  add a file
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_default_revset_per_repo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["describe", "-m", "add a file"]).success();

    // Set configuration to only show the root commit.
    work_dir.write_file(".jj/repo/config.toml", r#"revsets.log = "root()""#);

    // Log should only contain one line (for the root commit), and not show the
    // commit created above.
    insta::assert_snapshot!(work_dir.run_jj(["log", "-T", "commit_id"]), @r"
    ◆  0000000000000000000000000000000000000000
    [EOF]
    ");
}

#[test]
fn test_multiple_revsets() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    for name in ["foo", "bar", "baz"] {
        work_dir.run_jj(["new", "-m", name]).success();
        work_dir
            .run_jj(["bookmark", "create", "-r@", name])
            .success();
    }

    // Default revset should be overridden if one or more -r options are specified.
    test_env.add_config(r#"revsets.log = "root()""#);

    insta::assert_snapshot!(
        work_dir.run_jj(["log", "-T", "bookmarks", "-rfoo"]), @r"
    ○  foo
    │
    ~
    [EOF]
    ");
    insta::assert_snapshot!(
        work_dir.run_jj(["log", "-T", "bookmarks", "-rfoo", "-rbar", "-rbaz"]), @r"
    @  baz
    ○  bar
    ○  foo
    │
    ~
    [EOF]
    ");
    insta::assert_snapshot!(
        work_dir.run_jj(["log", "-T", "bookmarks", "-rfoo", "-rfoo"]), @r"
    ○  foo
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_graph_template_color() {
    // Test that color codes from a multi-line template don't span the graph lines.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["describe", "-m", "first line\nsecond line\nthird line"])
        .success();
    work_dir.run_jj(["new", "-m", "single line"]).success();

    test_env.add_config(
        r#"[colors]
        description = "red"
        "working_copy description" = "green"
        "#,
    );

    // First test without color for comparison
    let template = r#"label(if(current_working_copy, "working_copy"), description)"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  single line
    ○  first line
    │  second line
    │  third line
    ◆
    [EOF]
    ");
    let output = work_dir.run_jj(["--color=always", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;2msingle line[0m
    ○  [38;5;1mfirst line[39m
    │  [38;5;1msecond line[39m
    │  [38;5;1mthird line[39m
    [1m[38;5;14m◆[0m
    [EOF]
    ");
    let output = work_dir.run_jj(["--color=debug", "log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m<<log commit node working_copy mutable::@>>[0m  [1m[38;5;2m<<log commit working_copy description::single line>>[0m
    <<log commit node mutable::○>>  [38;5;1m<<log commit description::first line>>[39m
    │  [38;5;1m<<log commit description::second line>>[39m
    │  [38;5;1m<<log commit description::third line>>[39m
    [1m[38;5;14m<<log commit node immutable::◆>>[0m
    [EOF]
    ");
}

#[test]
fn test_graph_styles() {
    // Test that different graph styles are available.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "initial"]).success();
    work_dir
        .run_jj(["commit", "-m", "main bookmark 1"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "main bookmark 2"])
        .success();
    work_dir
        .run_jj(["new", "-m", "side bookmark\nwith\nlong\ndescription"])
        .success();
    work_dir
        .run_jj(["new", "-m", "merge", "subject(glob:'main bookmark 1')", "@"])
        .success();

    // Default (curved) style
    let output = work_dir.run_jj(["log", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @    merge
    ├─╮
    │ ○  side bookmark
    │ │  with
    │ │  long
    │ │  description
    │ ○  main bookmark 2
    ├─╯
    ○  main bookmark 1
    ○  initial
    ◆
    [EOF]
    ");

    // ASCII style
    test_env.add_config(r#"ui.graph.style = "ascii""#);
    let output = work_dir.run_jj(["log", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @    merge
    |\
    | o  side bookmark
    | |  with
    | |  long
    | |  description
    | o  main bookmark 2
    |/
    o  main bookmark 1
    o  initial
    +
    [EOF]
    ");

    // Large ASCII style
    test_env.add_config(r#"ui.graph.style = "ascii-large""#);
    let output = work_dir.run_jj(["log", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @     merge
    |\
    | \
    |  o  side bookmark
    |  |  with
    |  |  long
    |  |  description
    |  o  main bookmark 2
    | /
    |/
    o  main bookmark 1
    o  initial
    +
    [EOF]
    ");

    // Curved style
    test_env.add_config(r#"ui.graph.style = "curved""#);
    let output = work_dir.run_jj(["log", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @    merge
    ├─╮
    │ ○  side bookmark
    │ │  with
    │ │  long
    │ │  description
    │ ○  main bookmark 2
    ├─╯
    ○  main bookmark 1
    ○  initial
    ◆
    [EOF]
    ");

    // Square style
    test_env.add_config(r#"ui.graph.style = "square""#);
    let output = work_dir.run_jj(["log", "-T=description"]);
    insta::assert_snapshot!(output, @r"
    @    merge
    ├─┐
    │ ○  side bookmark
    │ │  with
    │ │  long
    │ │  description
    │ ○  main bookmark 2
    ├─┘
    ○  main bookmark 1
    ○  initial
    ◆
    [EOF]
    ");

    // Invalid style name
    let output = work_dir.run_jj(["log", "--config=ui.graph.style=unknown"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid type or value for ui.graph.style
    Caused by: unknown variant `unknown`, expected one of `ascii`, `ascii-large`, `curved`, `square`

    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_log_word_wrap() {
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

    work_dir
        .run_jj(["commit", "-m", "main bookmark 1"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "main bookmark 2"])
        .success();
    work_dir.run_jj(["new", "-m", "side"]).success();
    work_dir
        .run_jj(["new", "-m", "merge", "@--", "@"])
        .success();

    // ui.log-word-wrap option applies to both graph/no-graph outputs
    insta::assert_snapshot!(render(&["log", "-r@"], 40, false), @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:11 bafb1ee5
    │  (empty) merge
    ~
    [EOF]
    ");
    insta::assert_snapshot!(render(&["log", "-r@"], 40, true), @r"
    @  mzvwutvl test.user@example.com
    │  2001-02-03 08:05:11 bafb1ee5
    ~  (empty) merge
    [EOF]
    ");
    insta::assert_snapshot!(render(&["log", "--no-graph", "-r@"], 40, false), @r"
    mzvwutvl test.user@example.com 2001-02-03 08:05:11 bafb1ee5
    (empty) merge
    [EOF]
    ");
    insta::assert_snapshot!(render(&["log", "--no-graph", "-r@"], 40, true), @r"
    mzvwutvl test.user@example.com
    2001-02-03 08:05:11 bafb1ee5
    (empty) merge
    [EOF]
    ");

    // Color labels should be preserved
    insta::assert_snapshot!(render(&["log", "-r@", "--color=always"], 40, true), @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mm[38;5;8mzvwutvl[39m [38;5;3mtest.user@example.com[39m[0m
    │  [1m[38;5;14m2001-02-03 08:05:11[39m [38;5;12mb[38;5;8mafb1ee5[39m[0m
    ~  [1m[38;5;10m(empty)[39m merge[0m
    [EOF]
    ");

    // Graph width should be subtracted from the term width
    let template = r#""0 1 2 3 4 5 6 7 8 9""#;
    insta::assert_snapshot!(render(&["log", "-T", template], 10, true), @r"
    @    0 1 2
    ├─╮  3 4 5
    │ │  6 7 8
    │ │  9
    │ ○  0 1 2
    │ │  3 4 5
    │ │  6 7 8
    │ │  9
    │ ○  0 1 2
    ├─╯  3 4 5
    │    6 7 8
    │    9
    ○  0 1 2 3
    │  4 5 6 7
    │  8 9
    ◆  0 1 2 3
       4 5 6 7
       8 9
    [EOF]
    ");

    // Shouldn't panic with $COLUMNS < graph_width
    insta::assert_snapshot!(render(&["log", "-r@"], 0, true), @r"
    @  mzvwutvl
    │  test.user@example.com
    ~  2001-02-03
       08:05:11
       bafb1ee5
       (empty)
       merge
    [EOF]
    ");
    insta::assert_snapshot!(render(&["log", "-r@"], 1, true), @r"
    @  mzvwutvl
    │  test.user@example.com
    ~  2001-02-03
       08:05:11
       bafb1ee5
       (empty)
       merge
    [EOF]
    ");
}

#[test]
fn test_log_diff_stat_width() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |args: &[&str], columns: u32| {
        work_dir.run_jj_with(|cmd| cmd.args(args).env("COLUMNS", columns.to_string()))
    };

    work_dir.write_file("file1", "foo\n".repeat(100));
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file2", "foo\n".repeat(100));

    insta::assert_snapshot!(render(&["log", "--stat", "--no-graph"], 30), @r"
    rlvkpnrz test.user@example.com 2001-02-03 08:05:09 9490cfd3
    (no description set)
    file2 | 100 ++++++++++++++++++
    1 file changed, 100 insertions(+), 0 deletions(-)
    qpvuntsm test.user@example.com 2001-02-03 08:05:08 79f0968d
    (no description set)
    file1 | 100 ++++++++++++++++++
    1 file changed, 100 insertions(+), 0 deletions(-)
    zzzzzzzz root() 00000000
    0 files changed, 0 insertions(+), 0 deletions(-)
    [EOF]
    ");

    // Graph width should be subtracted
    insta::assert_snapshot!(render(&["log", "--stat"], 30), @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 9490cfd3
    │  (no description set)
    │  file2 | 100 +++++++++++++++
    │  1 file changed, 100 insertions(+), 0 deletions(-)
    │ ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 79f0968d
    ├─╯  (no description set)
    │    file1 | 100 +++++++++++++
    │    1 file changed, 100 insertions(+), 0 deletions(-)
    ◆  zzzzzzzz root() 00000000
       0 files changed, 0 insertions(+), 0 deletions(-)
    [EOF]
    ");
}

#[test]
fn test_elided() {
    // Test that elided commits are shown as synthetic nodes.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["new", "-m", "main bookmark 1"]).success();
    work_dir.run_jj(["new", "-m", "main bookmark 2"]).success();
    work_dir
        .run_jj(["new", "@--", "-m", "side bookmark 1"])
        .success();
    work_dir.run_jj(["new", "-m", "side bookmark 2"]).success();
    work_dir
        .run_jj(["new", "-m", "merge", "subject(glob:'main bookmark 2')", "@"])
        .success();

    let get_log = |revs: &str| work_dir.run_jj(["log", "-T", r#"description ++ "\n""#, "-r", revs]);

    // Test the setup
    insta::assert_snapshot!(get_log("::"), @r"
    @    merge
    ├─╮
    │ ○  side bookmark 2
    │ │
    │ ○  side bookmark 1
    │ │
    ○ │  main bookmark 2
    │ │
    ○ │  main bookmark 1
    ├─╯
    ○  initial
    │
    ◆
    [EOF]
    ");

    // Elide some commits from each side of the merge. It's unclear that a revision
    // was skipped on the left side.
    test_env.add_config("ui.log-synthetic-elided-nodes = false");
    insta::assert_snapshot!(get_log("@ | @- | subject(glob:initial)"), @r"
    @    merge
    ├─╮
    │ ○  side bookmark 2
    │ ╷
    ○ ╷  main bookmark 2
    ├─╯
    ○  initial
    │
    ~
    [EOF]
    ");

    // Elide shared commits. It's unclear that a revision was skipped on the right
    // side (#1252).
    insta::assert_snapshot!(get_log("@-- | root()"), @r"
    ○  side bookmark 1
    ╷
    ╷ ○  main bookmark 1
    ╭─╯
    ◆
    [EOF]
    ");

    // Now test the same thing with synthetic nodes for elided commits

    // Elide some commits from each side of the merge
    test_env.add_config("ui.log-synthetic-elided-nodes = true");
    insta::assert_snapshot!(get_log("@ | @- | subject(glob:initial)"), @r"
    @    merge
    ├─╮
    │ ○  side bookmark 2
    │ │
    │ ~  (elided revisions)
    ○ │  main bookmark 2
    │ │
    ~ │  (elided revisions)
    ├─╯
    ○  initial
    │
    ~
    [EOF]
    ");

    // Elide shared commits. To keep the implementation simple, it still gets
    // rendered as two synthetic nodes.
    insta::assert_snapshot!(get_log("@-- | root()"), @r"
    ○  side bookmark 1
    │
    ~  (elided revisions)
    │ ○  main bookmark 1
    │ │
    │ ~  (elided revisions)
    ├─╯
    ◆
    [EOF]
    ");
}

#[test]
fn test_log_with_custom_symbols() {
    // Test that elided commits are shown as synthetic nodes.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["new", "-m", "main bookmark 1"]).success();
    work_dir.run_jj(["new", "-m", "main bookmark 2"]).success();
    work_dir
        .run_jj(["new", "@--", "-m", "side bookmark 1"])
        .success();
    work_dir.run_jj(["new", "-m", "side bookmark 2"]).success();
    work_dir
        .run_jj(["new", "-m", "merge", "subject(glob:'main bookmark 2')", "@"])
        .success();

    let get_log = |revs: &str| work_dir.run_jj(["log", "-T", r#"description ++ "\n""#, "-r", revs]);

    // Simple test with showing default and elided nodes.
    test_env.add_config(
        r###"
        ui.log-synthetic-elided-nodes = true
        templates.log_node = 'if(self, if(current_working_copy, "$", if(root, "┴", "┝")), "🮀")'
        "###,
    );
    insta::assert_snapshot!(get_log("@ | @- | subject(glob:initial) | root()"), @r"
    $    merge
    ├─╮
    │ ┝  side bookmark 2
    │ │
    │ 🮀  (elided revisions)
    ┝ │  main bookmark 2
    │ │
    🮀 │  (elided revisions)
    ├─╯
    ┝  initial
    │
    ┴
    [EOF]
    ");

    // Simple test with showing default and elided nodes, ascii style.
    test_env.add_config(
        r###"
        ui.log-synthetic-elided-nodes = true
        ui.graph.style = 'ascii'
        templates.log_node = 'if(self, if(current_working_copy, "$", if(root, "^", "*")), ":")'
        "###,
    );
    insta::assert_snapshot!(get_log("@ | @- | subject(glob:initial) | root()"), @r"
    $    merge
    |\
    | *  side bookmark 2
    | |
    | :  (elided revisions)
    * |  main bookmark 2
    | |
    : |  (elided revisions)
    |/
    *  initial
    |
    ^
    [EOF]
    ");
}

#[test]
fn test_log_full_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj([
            "commit",
            "-m",
            "this is commit with a multiline description\n\n<full description>",
        ])
        .success();

    let output = work_dir.run_jj(["log", "-T", "builtin_log_compact_full_description"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 3a70504b
    │  (empty) (no description set)
    │
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 37b69cda
    │  (empty) this is commit with a multiline description
    │
    │  <full description>
    │
    ◆  zzzzzzzz root() 00000000

    [EOF]
    ");
}

#[test]
fn test_log_anonymize() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo = origin_dir.root().join(".jj/repo/store/git");
    let origin_git_repo = origin_git_repo.to_str().unwrap();
    origin_dir
        .run_jj([
            "describe",
            "-m",
            "this is commit with a multiline description\n\n<full description>",
        ])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "b1", "b2", "b3"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();

    test_env
        .run_jj_in(".", ["git", "clone", origin_git_repo, "local"])
        .success();
    let work_dir = test_env.work_dir("local");
    work_dir
        .run_jj(["bookmark", "track", "b1@origin", "b2@origin"])
        .success();
    work_dir.run_jj(["new", "b1"]).success();
    work_dir.run_jj(["bookmark", "move", "b1", "-t@"]).success();

    let output = work_dir.run_jj(["log", "-r::", "-Tbuiltin_log_redacted"]);
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt user-78cd 2001-02-03 08:05:13 bookmark-dc8b* de3c47af
    │  (empty) (redacted)
    ◆  qpvuntsm user-78cd 2001-02-03 08:05:08 bookmark-dc8b@remote-86e9 bookmark-56f1 bookmark-ff9e@remote-86e9 37b69cda
    │  (empty) (redacted)
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}
