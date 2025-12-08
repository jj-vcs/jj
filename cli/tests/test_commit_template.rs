// Copyright 2023 The Jujutsu Authors
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

use indoc::indoc;
use regex::Regex;
use testutils::git;

use crate::common::TestEnvironment;

#[test]
fn test_log_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.run_jj(["new", "@", "@-"]).success();

    let template =
        r#"commit_id ++ "\nP: " ++ parents.len() ++ " " ++ parents.map(|c| c.commit_id()) ++ "\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    8b93ef7a3ceefa4e4b1a506945588dd0da2d9e3e
    ├─╮  P: 2 1c1c95df80e53b1e654608d7589f5baabb10ebb2 e8849ae12c709f2321908879bc724fdb2ab8a781
    ○ │  1c1c95df80e53b1e654608d7589f5baabb10ebb2
    ├─╯  P: 1 e8849ae12c709f2321908879bc724fdb2ab8a781
    ○  e8849ae12c709f2321908879bc724fdb2ab8a781
    │  P: 1 0000000000000000000000000000000000000000
    ◆  0000000000000000000000000000000000000000
       P: 0
    [EOF]
    ");

    // List<Commit> can be filtered
    let template =
        r#""P: " ++ parents.filter(|c| !c.root()).map(|c| c.commit_id().short()) ++ "\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    P: 1c1c95df80e5 e8849ae12c70
    ├─╮
    ○ │  P: e8849ae12c70
    ├─╯
    ○  P:
    ◆  P:
    [EOF]
    ");

    let template = r#"parents.map(|c| c.commit_id().shortest(4))"#;
    let output = work_dir.run_jj(["log", "-T", template, "-r@", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;4m1[0m[38;5;8mc1c[39m [1m[38;5;4me[0m[38;5;8m884[39m
    │
    ~
    [EOF]
    ");

    // Commit object isn't printable
    let output = work_dir.run_jj(["log", "-T", "parents"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Failed to parse template: Expected expression of type `Template`, but actual type is `List<Commit>`
    Caused by:  --> 1:1
      |
    1 | parents
      | ^-----^
      |
      = Expected expression of type `Template`, but actual type is `List<Commit>`
    [EOF]
    [exit status: 1]
    ");

    // Redundant argument passed to keyword method
    let template = r#"parents.map(|c| c.commit_id(""))"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Function `commit_id`: Expected 0 arguments
    Caused by:  --> 1:29
      |
    1 | parents.map(|c| c.commit_id(""))
      |                             ^^
      |
      = Function `commit_id`: Expected 0 arguments
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_log_author_timestamp() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();

    let output = work_dir.run_jj(["log", "-T", "author.timestamp()"]);
    insta::assert_snapshot!(output, @r"
    @  2001-02-03 04:05:09.000 +07:00
    ○  2001-02-03 04:05:08.000 +07:00
    ◆  1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");
}

#[test]
fn test_log_author_timestamp_ago() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();

    let template = r#"author.timestamp().ago() ++ "\n""#;
    let output = work_dir
        .run_jj(&["log", "--no-graph", "-T", template])
        .success();
    let line_re = Regex::new(r"[0-9]+ years ago").unwrap();
    assert!(
        output.stdout.raw().lines().all(|x| line_re.is_match(x)),
        "expected every line to match regex"
    );
}

#[test]
fn test_log_author_timestamp_utc() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["log", "-T", "author.timestamp().utc()"]);
    insta::assert_snapshot!(output, @r"
    @  2001-02-02 21:05:07.000 +00:00
    ◆  1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_log_author_timestamp_local() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();

    test_env.add_env_var("TZ", "UTC-05:30");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["log", "-T", "author.timestamp().local()"]);
    insta::assert_snapshot!(output, @r"
    @  2001-02-03 08:05:07.000 +11:00
    ◆  1970-01-01 11:00:00.000 +11:00
    [EOF]
    ");
    test_env.add_env_var("TZ", "UTC+10:00");
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["log", "-T", "author.timestamp().local()"]);
    insta::assert_snapshot!(output, @r"
    @  2001-02-03 08:05:07.000 +11:00
    ◆  1970-01-01 11:00:00.000 +11:00
    [EOF]
    ");
}

#[test]
fn test_log_author_timestamp_after_before() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();

    let template = r#"
    separate(" ",
      author.timestamp(),
      ":",
      if(author.timestamp().after("1969"), "(after 1969)", "(before 1969)"),
      if(author.timestamp().before("1975"), "(before 1975)", "(after 1975)"),
      if(author.timestamp().before("now"), "(before now)", "(after now)")
    ) ++ "\n""#;
    let output = work_dir.run_jj(["log", "--no-graph", "-T", template]);
    insta::assert_snapshot!(output, @r"
    2001-02-03 04:05:08.000 +07:00 : (after 1969) (after 1975) (before now)
    1970-01-01 00:00:00.000 +00:00 : (after 1969) (before 1975) (before now)
    [EOF]
    ");

    // Should display error with invalid date.
    let template = r#"author.timestamp().after("invalid date")"#;
    let output = work_dir.run_jj(["log", "-r@", "--no-graph", "-T", template]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Invalid date pattern
    Caused by:
    1:  --> 1:26
      |
    1 | author.timestamp().after("invalid date")
      |                          ^------------^
      |
      = Invalid date pattern
    2: expected unsupported identifier as position 0..7
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_mine_is_true_when_author_is_user() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj([
            "--config=user.email=johndoe@example.com",
            "--config=user.name=John Doe",
            "new",
        ])
        .success();

    let output = work_dir.run_jj([
        "log",
        "-T",
        r#"coalesce(if(mine, "mine"), author.email(), email_placeholder)"#,
    ]);
    insta::assert_snapshot!(output, @r"
    @  johndoe@example.com
    ○  mine
    ◆  (no email set)
    [EOF]
    ");
}

#[test]
fn test_log_json() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();
    work_dir.run_jj(["new", "-m", "second"]).success();

    let output = work_dir.run_jj(["log", r#"-Tjson(self) ++ "\n""#]);
    insta::assert_snapshot!(output, @r#"
    @  {"commit_id":"b1cb6b2f9141e6ffee18532a8bf9a2075ca02606","parents":["68a505386f936fff6d718f55005e77ea72589bc1"],"change_id":"kkmpptxzrspxrzommnulwmwkkqwworpl","description":"second\n","author":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:09+07:00"},"committer":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:09+07:00"}}
    ○  {"commit_id":"68a505386f936fff6d718f55005e77ea72589bc1","parents":["0000000000000000000000000000000000000000"],"change_id":"qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu","description":"first\n","author":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:08+07:00"},"committer":{"name":"Test User","email":"test.user@example.com","timestamp":"2001-02-03T04:05:08+07:00"}}
    ◆  {"commit_id":"0000000000000000000000000000000000000000","parents":[],"change_id":"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz","description":"","author":{"name":"","email":"","timestamp":"1970-01-01T00:00:00Z"},"committer":{"name":"","email":"","timestamp":"1970-01-01T00:00:00Z"}}
    [EOF]
    "#);
}

#[test]
fn test_log_default() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.run_jj(["describe", "-m", "add a file"]).success();
    work_dir.run_jj(["new", "-m", "description 1"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    // Test default log output format
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 my-bookmark c938c088
    │  (empty) description 1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 007859d3
    │  add a file
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["log", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mk[38;5;8mkmpptxz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:09[39m [38;5;13mmy-bookmark[39m [38;5;12mc[38;5;8m938c088[39m[0m
    │  [1m[38;5;10m(empty)[39m description 1[0m
    ○  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4m007[0m[38;5;8m859d3[39m
    │  add a file
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m000[0m[38;5;8m00000[39m
    [EOF]
    ");

    // Color without graph
    let output = work_dir.run_jj(["log", "--color=always", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;13mk[38;5;8mkmpptxz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:09[39m [38;5;13mmy-bookmark[39m [38;5;12mc[38;5;8m938c088[39m[0m
    [1m[38;5;10m(empty)[39m description 1[0m
    [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4m007[0m[38;5;8m859d3[39m
    add a file
    [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m000[0m[38;5;8m00000[39m
    [EOF]
    ");
}

#[test]
fn test_log_default_without_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["workspace", "forget"]).success();
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_log_builtin_templates() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    // Render without graph to test line ending
    let render = |template| work_dir.run_jj(["log", "-T", template, "--no-graph"]);

    work_dir
        .run_jj(["--config=user.email=''", "--config=user.name=''", "new"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    insta::assert_snapshot!(render(r#"builtin_log_oneline"#), @r"
    rlvkpnrz (no email set) 2001-02-03 08:05:08 my-bookmark aec3ec96 (empty) (no description set)
    qpvuntsm test.user 2001-02-03 08:05:07 e8849ae1 (empty) (no description set)
    zzzzzzzz root() 00000000
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_compact"#), @r"
    rlvkpnrz (no email set) 2001-02-03 08:05:08 my-bookmark aec3ec96
    (empty) (no description set)
    qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)
    zzzzzzzz root() 00000000
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_comfortable"#), @r"
    rlvkpnrz (no email set) 2001-02-03 08:05:08 my-bookmark aec3ec96
    (empty) (no description set)

    qpvuntsm test.user@example.com 2001-02-03 08:05:07 e8849ae1
    (empty) (no description set)

    zzzzzzzz root() 00000000

    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_detailed"#), @r"
    Commit ID: aec3ec964d0771edea9da48a2a170bc6ffa1c725
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Bookmarks: my-bookmark
    Author   : (no name set) <(no email set)> (2001-02-03 08:05:08)
    Committer: (no name set) <(no email set)> (2001-02-03 08:05:08)

        (no description set)

    Commit ID: e8849ae12c709f2321908879bc724fdb2ab8a781
    Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:07)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:07)

        (no description set)

    Commit ID: 0000000000000000000000000000000000000000
    Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    Author   : (no name set) <(no email set)> (1970-01-01 11:00:00)
    Committer: (no name set) <(no email set)> (1970-01-01 11:00:00)

        (no description set)

    [EOF]
    ");
}

#[test]
fn test_log_builtin_templates_colored() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |template| work_dir.run_jj(["--color=always", "log", "-T", template]);

    work_dir
        .run_jj(["--config=user.email=''", "--config=user.name=''", "new"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    insta::assert_snapshot!(render(r#"builtin_log_oneline"#), @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;9m(no email set)[39m [38;5;14m2001-02-03 08:05:08[39m [38;5;13mmy-bookmark[39m [38;5;12ma[38;5;8mec3ec96[39m [38;5;10m(empty)[39m [38;5;10m(no description set)[39m[0m
    ○  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user[39m [38;5;6m2001-02-03 08:05:07[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_compact"#), @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;9m(no email set)[39m [38;5;14m2001-02-03 08:05:08[39m [38;5;13mmy-bookmark[39m [38;5;12ma[38;5;8mec3ec96[39m[0m
    │  [1m[38;5;10m(empty)[39m [38;5;10m(no description set)[39m[0m
    ○  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:07[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m
    │  [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_comfortable"#), @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;9m(no email set)[39m [38;5;14m2001-02-03 08:05:08[39m [38;5;13mmy-bookmark[39m [38;5;12ma[38;5;8mec3ec96[39m[0m
    │  [1m[38;5;10m(empty)[39m [38;5;10m(no description set)[39m[0m
    │
    ○  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:07[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m
    │  [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    │
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m

    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_detailed"#), @r"
    [1m[38;5;2m@[0m  Commit ID: [38;5;4maec3ec964d0771edea9da48a2a170bc6ffa1c725[39m
    │  Change ID: [38;5;5mrlvkpnrzqnoowoytxnquwvuryrwnrmlp[39m
    │  Bookmarks: [38;5;5mmy-bookmark[39m
    │  Author   : [38;5;1m(no name set)[39m <[38;5;1m(no email set)[39m> ([38;5;6m2001-02-03 08:05:08[39m)
    │  Committer: [38;5;1m(no name set)[39m <[38;5;1m(no email set)[39m> ([38;5;6m2001-02-03 08:05:08[39m)
    │
    │  [38;5;2m    (no description set)[39m
    │
    ○  Commit ID: [38;5;4me8849ae12c709f2321908879bc724fdb2ab8a781[39m
    │  Change ID: [38;5;5mqpvuntsmwlqtpsluzzsnyyzlmlwvmlnu[39m
    │  Author   : [38;5;3mTest User[39m <[38;5;3mtest.user@example.com[39m> ([38;5;6m2001-02-03 08:05:07[39m)
    │  Committer: [38;5;3mTest User[39m <[38;5;3mtest.user@example.com[39m> ([38;5;6m2001-02-03 08:05:07[39m)
    │
    │  [38;5;2m    (no description set)[39m
    │
    [1m[38;5;14m◆[0m  Commit ID: [38;5;4m0000000000000000000000000000000000000000[39m
       Change ID: [38;5;5mzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz[39m
       Author   : [38;5;1m(no name set)[39m <[38;5;1m(no email set)[39m> ([38;5;6m1970-01-01 11:00:00[39m)
       Committer: [38;5;1m(no name set)[39m <[38;5;1m(no email set)[39m> ([38;5;6m1970-01-01 11:00:00[39m)

       [38;5;2m    (no description set)[39m

    [EOF]
    ");
}

#[test]
fn test_log_builtin_templates_colored_debug() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let render = |template| work_dir.run_jj(["--color=debug", "log", "-T", template]);

    work_dir
        .run_jj(["--config=user.email=''", "--config=user.name=''", "new"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "my-bookmark"])
        .success();

    insta::assert_snapshot!(render(r#"builtin_log_oneline"#), @r"
    [1m[38;5;2m<<log commit node working_copy mutable::@>>[0m  [1m[38;5;13m<<log commit working_copy mutable change_id shortest prefix::r>>[38;5;8m<<log commit working_copy mutable change_id shortest rest::lvkpnrz>>[39m<<log commit working_copy mutable:: >>[38;5;9m<<log commit working_copy mutable email placeholder::(no email set)>>[39m<<log commit working_copy mutable:: >>[38;5;14m<<log commit working_copy mutable committer timestamp local format::2001-02-03 08:05:08>>[39m<<log commit working_copy mutable:: >>[38;5;13m<<log commit working_copy mutable bookmarks name::my-bookmark>>[39m<<log commit working_copy mutable:: >>[38;5;12m<<log commit working_copy mutable commit_id shortest prefix::a>>[38;5;8m<<log commit working_copy mutable commit_id shortest rest::ec3ec96>>[39m<<log commit working_copy mutable:: >>[38;5;10m<<log commit working_copy mutable empty::(empty)>>[39m<<log commit working_copy mutable:: >>[38;5;10m<<log commit working_copy mutable empty description placeholder::(no description set)>>[39m<<log commit working_copy mutable::>>[0m
    <<log commit node mutable::○>>  [1m[38;5;5m<<log commit mutable change_id shortest prefix::q>>[0m[38;5;8m<<log commit mutable change_id shortest rest::pvuntsm>>[39m<<log commit mutable:: >>[38;5;3m<<log commit mutable author email local::test.user>>[39m<<log commit mutable:: >>[38;5;6m<<log commit mutable committer timestamp local format::2001-02-03 08:05:07>>[39m<<log commit mutable:: >>[1m[38;5;4m<<log commit mutable commit_id shortest prefix::e>>[0m[38;5;8m<<log commit mutable commit_id shortest rest::8849ae1>>[39m<<log commit mutable:: >>[38;5;2m<<log commit mutable empty::(empty)>>[39m<<log commit mutable:: >>[38;5;2m<<log commit mutable empty description placeholder::(no description set)>>[39m<<log commit mutable::>>
    [1m[38;5;14m<<log commit node immutable::◆>>[0m  [1m[38;5;5m<<log commit immutable change_id shortest prefix::z>>[0m[38;5;8m<<log commit immutable change_id shortest rest::zzzzzzz>>[39m<<log commit immutable:: >>[38;5;2m<<log commit immutable root::root()>>[39m<<log commit immutable:: >>[1m[38;5;4m<<log commit immutable commit_id shortest prefix::0>>[0m[38;5;8m<<log commit immutable commit_id shortest rest::0000000>>[39m<<log commit immutable::>>
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_compact"#), @r"
    [1m[38;5;2m<<log commit node working_copy mutable::@>>[0m  [1m[38;5;13m<<log commit working_copy mutable change_id shortest prefix::r>>[38;5;8m<<log commit working_copy mutable change_id shortest rest::lvkpnrz>>[39m<<log commit working_copy mutable:: >>[38;5;9m<<log commit working_copy mutable email placeholder::(no email set)>>[39m<<log commit working_copy mutable:: >>[38;5;14m<<log commit working_copy mutable committer timestamp local format::2001-02-03 08:05:08>>[39m<<log commit working_copy mutable:: >>[38;5;13m<<log commit working_copy mutable bookmarks name::my-bookmark>>[39m<<log commit working_copy mutable:: >>[38;5;12m<<log commit working_copy mutable commit_id shortest prefix::a>>[38;5;8m<<log commit working_copy mutable commit_id shortest rest::ec3ec96>>[39m<<log commit working_copy mutable::>>[0m
    │  [1m[38;5;10m<<log commit working_copy mutable empty::(empty)>>[39m<<log commit working_copy mutable:: >>[38;5;10m<<log commit working_copy mutable empty description placeholder::(no description set)>>[39m<<log commit working_copy mutable::>>[0m
    <<log commit node mutable::○>>  [1m[38;5;5m<<log commit mutable change_id shortest prefix::q>>[0m[38;5;8m<<log commit mutable change_id shortest rest::pvuntsm>>[39m<<log commit mutable:: >>[38;5;3m<<log commit mutable author email local::test.user>><<log commit mutable author email::@>><<log commit mutable author email domain::example.com>>[39m<<log commit mutable:: >>[38;5;6m<<log commit mutable committer timestamp local format::2001-02-03 08:05:07>>[39m<<log commit mutable:: >>[1m[38;5;4m<<log commit mutable commit_id shortest prefix::e>>[0m[38;5;8m<<log commit mutable commit_id shortest rest::8849ae1>>[39m<<log commit mutable::>>
    │  [38;5;2m<<log commit mutable empty::(empty)>>[39m<<log commit mutable:: >>[38;5;2m<<log commit mutable empty description placeholder::(no description set)>>[39m<<log commit mutable::>>
    [1m[38;5;14m<<log commit node immutable::◆>>[0m  [1m[38;5;5m<<log commit immutable change_id shortest prefix::z>>[0m[38;5;8m<<log commit immutable change_id shortest rest::zzzzzzz>>[39m<<log commit immutable:: >>[38;5;2m<<log commit immutable root::root()>>[39m<<log commit immutable:: >>[1m[38;5;4m<<log commit immutable commit_id shortest prefix::0>>[0m[38;5;8m<<log commit immutable commit_id shortest rest::0000000>>[39m<<log commit immutable::>>
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_comfortable"#), @r"
    [1m[38;5;2m<<log commit node working_copy mutable::@>>[0m  [1m[38;5;13m<<log commit working_copy mutable change_id shortest prefix::r>>[38;5;8m<<log commit working_copy mutable change_id shortest rest::lvkpnrz>>[39m<<log commit working_copy mutable:: >>[38;5;9m<<log commit working_copy mutable email placeholder::(no email set)>>[39m<<log commit working_copy mutable:: >>[38;5;14m<<log commit working_copy mutable committer timestamp local format::2001-02-03 08:05:08>>[39m<<log commit working_copy mutable:: >>[38;5;13m<<log commit working_copy mutable bookmarks name::my-bookmark>>[39m<<log commit working_copy mutable:: >>[38;5;12m<<log commit working_copy mutable commit_id shortest prefix::a>>[38;5;8m<<log commit working_copy mutable commit_id shortest rest::ec3ec96>>[39m<<log commit working_copy mutable::>>[0m
    │  [1m[38;5;10m<<log commit working_copy mutable empty::(empty)>>[39m<<log commit working_copy mutable:: >>[38;5;10m<<log commit working_copy mutable empty description placeholder::(no description set)>>[39m<<log commit working_copy mutable::>>[0m
    │  <<log commit::>>
    <<log commit node mutable::○>>  [1m[38;5;5m<<log commit mutable change_id shortest prefix::q>>[0m[38;5;8m<<log commit mutable change_id shortest rest::pvuntsm>>[39m<<log commit mutable:: >>[38;5;3m<<log commit mutable author email local::test.user>><<log commit mutable author email::@>><<log commit mutable author email domain::example.com>>[39m<<log commit mutable:: >>[38;5;6m<<log commit mutable committer timestamp local format::2001-02-03 08:05:07>>[39m<<log commit mutable:: >>[1m[38;5;4m<<log commit mutable commit_id shortest prefix::e>>[0m[38;5;8m<<log commit mutable commit_id shortest rest::8849ae1>>[39m<<log commit mutable::>>
    │  [38;5;2m<<log commit mutable empty::(empty)>>[39m<<log commit mutable:: >>[38;5;2m<<log commit mutable empty description placeholder::(no description set)>>[39m<<log commit mutable::>>
    │  <<log commit::>>
    [1m[38;5;14m<<log commit node immutable::◆>>[0m  [1m[38;5;5m<<log commit immutable change_id shortest prefix::z>>[0m[38;5;8m<<log commit immutable change_id shortest rest::zzzzzzz>>[39m<<log commit immutable:: >>[38;5;2m<<log commit immutable root::root()>>[39m<<log commit immutable:: >>[1m[38;5;4m<<log commit immutable commit_id shortest prefix::0>>[0m[38;5;8m<<log commit immutable commit_id shortest rest::0000000>>[39m<<log commit immutable::>>
       <<log commit::>>
    [EOF]
    ");

    insta::assert_snapshot!(render(r#"builtin_log_detailed"#), @r"
    [1m[38;5;2m<<log commit node working_copy mutable::@>>[0m  <<log commit::Commit ID: >>[38;5;4m<<log commit commit_id::aec3ec964d0771edea9da48a2a170bc6ffa1c725>>[39m<<log commit::>>
    │  <<log commit::Change ID: >>[38;5;5m<<log commit change_id::rlvkpnrzqnoowoytxnquwvuryrwnrmlp>>[39m<<log commit::>>
    │  <<log commit::Bookmarks: >>[38;5;5m<<log commit local_bookmarks name::my-bookmark>>[39m<<log commit::>>
    │  <<log commit::Author   : >>[38;5;1m<<log commit name placeholder::(no name set)>>[39m<<log commit:: <>>[38;5;1m<<log commit email placeholder::(no email set)>>[39m<<log commit::> (>>[38;5;6m<<log commit author timestamp local format::2001-02-03 08:05:08>>[39m<<log commit::)>>
    │  <<log commit::Committer: >>[38;5;1m<<log commit name placeholder::(no name set)>>[39m<<log commit:: <>>[38;5;1m<<log commit email placeholder::(no email set)>>[39m<<log commit::> (>>[38;5;6m<<log commit committer timestamp local format::2001-02-03 08:05:08>>[39m<<log commit::)>>
    │  <<log commit::>>
    │  [38;5;2m<<log commit empty description placeholder::    (no description set)>>[39m<<log commit::>>
    │  <<log commit::>>
    <<log commit node mutable::○>>  <<log commit::Commit ID: >>[38;5;4m<<log commit commit_id::e8849ae12c709f2321908879bc724fdb2ab8a781>>[39m<<log commit::>>
    │  <<log commit::Change ID: >>[38;5;5m<<log commit change_id::qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu>>[39m<<log commit::>>
    │  <<log commit::Author   : >>[38;5;3m<<log commit author name::Test User>>[39m<<log commit:: <>>[38;5;3m<<log commit author email local::test.user>><<log commit author email::@>><<log commit author email domain::example.com>>[39m<<log commit::> (>>[38;5;6m<<log commit author timestamp local format::2001-02-03 08:05:07>>[39m<<log commit::)>>
    │  <<log commit::Committer: >>[38;5;3m<<log commit committer name::Test User>>[39m<<log commit:: <>>[38;5;3m<<log commit committer email local::test.user>><<log commit committer email::@>><<log commit committer email domain::example.com>>[39m<<log commit::> (>>[38;5;6m<<log commit committer timestamp local format::2001-02-03 08:05:07>>[39m<<log commit::)>>
    │  <<log commit::>>
    │  [38;5;2m<<log commit empty description placeholder::    (no description set)>>[39m<<log commit::>>
    │  <<log commit::>>
    [1m[38;5;14m<<log commit node immutable::◆>>[0m  <<log commit::Commit ID: >>[38;5;4m<<log commit commit_id::0000000000000000000000000000000000000000>>[39m<<log commit::>>
       <<log commit::Change ID: >>[38;5;5m<<log commit change_id::zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz>>[39m<<log commit::>>
       <<log commit::Author   : >>[38;5;1m<<log commit name placeholder::(no name set)>>[39m<<log commit:: <>>[38;5;1m<<log commit email placeholder::(no email set)>>[39m<<log commit::> (>>[38;5;6m<<log commit author timestamp local format::1970-01-01 11:00:00>>[39m<<log commit::)>>
       <<log commit::Committer: >>[38;5;1m<<log commit name placeholder::(no name set)>>[39m<<log commit:: <>>[38;5;1m<<log commit email placeholder::(no email set)>>[39m<<log commit::> (>>[38;5;6m<<log commit committer timestamp local format::1970-01-01 11:00:00>>[39m<<log commit::)>>
       <<log commit::>>
       [38;5;2m<<log commit empty description placeholder::    (no description set)>>[39m<<log commit::>>
       <<log commit::>>
    [EOF]
    ");
}

#[test]
fn test_log_evolog_divergence() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "foo\n");
    work_dir
        .run_jj(["describe", "-m", "description 1"])
        .success();
    // No divergence
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:08 556daeb7
    │  description 1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Create divergence
    work_dir
        .run_jj(["describe", "-m", "description 2", "--at-operation", "@-"])
        .success();
    let output = work_dir.run_jj(["log"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm?? test.user@example.com 2001-02-03 08:05:08 556daeb7
    │  description 1
    │ ○  qpvuntsm?? test.user@example.com 2001-02-03 08:05:10 5cea51a1
    ├─╯  description 2
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    [EOF]
    ");

    // Color
    let output = work_dir.run_jj(["log", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[4m[38;5;1mq[24mpvuntsm[38;5;9m??[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:08[39m [38;5;12m55[38;5;8m6daeb7[39m[0m
    │  [1mdescription 1[0m
    │ ○  [1m[4m[38;5;1mq[0m[38;5;1mpvuntsm??[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:10[39m [1m[38;5;4m5c[0m[38;5;8mea51a1[39m
    ├─╯  description 2
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
    [EOF]
    ");

    // Evolog and hidden divergent
    let output = work_dir.run_jj(["evolog"]);
    insta::assert_snapshot!(output, @r"
    @  qpvuntsm?? test.user@example.com 2001-02-03 08:05:08 556daeb7
    │  description 1
    │  -- operation fec5a045b947 describe commit d0c049cd993a8d3a2e69ba6df98788e264ea9fa1
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 d0c049cd
    │  (no description set)
    │  -- operation 911e64a1b666 snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");

    // Colored evolog
    let output = work_dir.run_jj(["evolog", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[4m[38;5;1mq[24mpvuntsm[38;5;9m??[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:08[39m [38;5;12m55[38;5;8m6daeb7[39m[0m
    │  [1mdescription 1[0m
    │  [38;5;8m--[39m operation [38;5;4mfec5a045b947[39m describe commit d0c049cd993a8d3a2e69ba6df98788e264ea9fa1
    ○  [1m[39mq[0m[38;5;8mpvuntsm[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:08[39m [1m[38;5;4md[0m[38;5;8m0c049cd[39m
    │  [38;5;3m(no description set)[39m
    │  [38;5;8m--[39m operation [38;5;4m911e64a1b666[39m snapshot working copy
    ○  [1m[39mq[0m[38;5;8mpvuntsm[39m hidden [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:07[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m
       [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
       [38;5;8m--[39m operation [38;5;4m8f47435a3990[39m add workspace 'default'
    [EOF]
    ");
}

#[test]
fn test_log_bookmarks() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);

    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    // Created some bookmarks on the remote
    origin_dir
        .run_jj(["describe", "-m=description 1"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark1"])
        .success();
    origin_dir
        .run_jj(["new", "root()", "-m=description 2"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark2", "unchanged"])
        .success();
    origin_dir
        .run_jj(["new", "root()", "-m=description 3"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark3"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();
    test_env
        .run_jj_in(
            ".",
            [
                "git",
                "clone",
                origin_git_repo_path.to_str().unwrap(),
                "local",
            ],
        )
        .success();
    let work_dir = test_env.work_dir("local");

    // Track all remote bookmarks, rewrite bookmark1, move bookmark2 forward,
    // create conflict in bookmark3, add new-bookmark
    work_dir
        .run_jj(["bookmark", "track", "glob:*@origin"])
        .success();
    work_dir
        .run_jj(["describe", "bookmark1", "-m", "modified bookmark1 commit"])
        .success();
    work_dir.run_jj(["new", "bookmark2"]).success();
    work_dir
        .run_jj(["bookmark", "set", "bookmark2", "--to=@"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "new-bookmark"])
        .success();
    work_dir
        .run_jj(["describe", "bookmark3", "-m=local"])
        .success();
    origin_dir
        .run_jj(["describe", "bookmark3", "-m=origin"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();
    work_dir.run_jj(["git", "fetch"]).success();

    let template = r#"commit_id.short() ++ " " ++ if(bookmarks, bookmarks, "(no bookmarks)")"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  4bc3723efff8 bookmark2* new-bookmark
    ○  38a204733702 bookmark2@origin unchanged
    │ ○  1c14797dac42 bookmark3?? bookmark3@origin
    ├─╯
    │ ○  8223b15ac1f1 bookmark3??
    ├─╯
    │ ○  a156ef717a61 bookmark1*
    ├─╯
    ◆  000000000000 (no bookmarks)
    [EOF]
    ");

    let template = r#"bookmarks.map(|b| separate("/", b.remote(), b.name())).join(", ")"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  bookmark2, new-bookmark
    ○  origin/bookmark2, unchanged
    │ ○  bookmark3, origin/bookmark3
    ├─╯
    │ ○  bookmark3
    ├─╯
    │ ○  bookmark1
    ├─╯
    ◆
    [EOF]
    ");

    let template = r#"separate(" ", "L:", local_bookmarks, "R:", remote_bookmarks)"#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  L: bookmark2* new-bookmark R:
    ○  L: unchanged R: bookmark2@origin unchanged@origin
    │ ○  L: bookmark3?? R: bookmark3@origin
    ├─╯
    │ ○  L: bookmark3?? R:
    ├─╯
    │ ○  L: bookmark1* R:
    ├─╯
    ◆  L: R:
    [EOF]
    ");

    let template = r#"
    remote_bookmarks.map(|ref| concat(
      ref,
      if(ref.tracked(),
        "(+" ++ ref.tracking_ahead_count().lower()
        ++ "/-" ++ ref.tracking_behind_count().lower() ++ ")"),
    ))
    "#;
    let output = work_dir.run_jj(["log", "-r::remote_bookmarks()", "-T", template]);
    insta::assert_snapshot!(output, @r"
    ○  bookmark3@origin(+0/-1)
    │ ○  bookmark2@origin(+0/-1) unchanged@origin(+0/-0)
    ├─╯
    │ ○  bookmark1@origin(+1/-1)
    ├─╯
    ◆
    [EOF]
    ");
}

#[test]
fn test_log_tags() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-mcommit1"]).success();
    work_dir.run_jj(["commit", "-mcommit2"]).success();
    work_dir.run_jj(["tag", "set", "-r@--", "foo"]).success();
    work_dir.run_jj(["tag", "set", "-r@-", "bar"]).success();
    work_dir.run_jj(["git", "export"]).success();
    work_dir
        .run_jj(["tag", "set", "--allow-move", "-r@-", "foo", "baz"])
        .success();

    let template = r#"commit_id.short() ++ " " ++ if(tags, tags, "(no tags)") ++ "\n""#;
    let output = work_dir.run_jj(["log", "-rall()", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  510df2613fc8 (no tags)
    ◆  3f672e728535 bar baz foo*
    ◆  b876c5f49546 foo@git
    ◆  000000000000 (no tags)
    [EOF]
    ");

    let template = r#"separate(" ", "L:", local_tags, "R:", remote_tags) ++ "\n""#;
    let output = work_dir.run_jj(["log", "-rall()", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  L: R:
    ◆  L: bar baz foo* R: bar@git
    ◆  L: R: foo@git
    ◆  L: R:
    [EOF]
    ");
}

#[test]
fn test_log_git_head() {
    let test_env = TestEnvironment::default();
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    work_dir.run_jj(["new", "-m=initial"]).success();
    work_dir.write_file("file", "foo\n");

    let output = work_dir.run_jj(["log", "-T", "git_head"]);
    insta::assert_snapshot!(output, @r"
    @  false
    ○  true
    ◆  false
    [EOF]
    ------- stderr -------
    Warning: In template expression
     --> 1:1
      |
    1 | git_head
      | ^------^
      |
      = commit.git_head() is deprecated; use .contained_in('first_parent(@)') instead
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "--color=always"]);
    insta::assert_snapshot!(output, @r"
    [1m[38;5;2m@[0m  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 08:05:09[39m [38;5;12m6[38;5;8m87fadfd[39m[0m
    │  [1minitial[0m
    ○  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 08:05:07[39m [1m[38;5;4me[0m[38;5;8m8849ae1[39m
    │  [38;5;2m(empty)[39m [38;5;2m(no description set)[39m
    [1m[38;5;14m◆[0m  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;2mroot()[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
    [EOF]
    ");
}

#[test]
fn test_log_customize_short_id() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "first"]).success();

    // Customize both the commit and the change id
    let decl = "template-aliases.'format_short_id(id)'";
    let output = work_dir.run_jj([
        "log",
        "--config",
        &format!(r#"{decl}='id.shortest(5).prefix().upper() ++ "_" ++ id.shortest(5).rest()'"#),
    ]);
    insta::assert_snapshot!(output, @r"
    @  Q_pvun test.user@example.com 2001-02-03 08:05:08 6_8a50
    │  (empty) first
    ◆  Z_zzzz root() 0_0000
    [EOF]
    ");

    // Customize only the change id
    let output = work_dir.run_jj([
        "log",
        "--config=template-aliases.'format_short_change_id(id)'='format_short_id(id).upper()'",
    ]);
    insta::assert_snapshot!(output, @r"
    @  QPVUNTSM test.user@example.com 2001-02-03 08:05:08 68a50538
    │  (empty) first
    ◆  ZZZZZZZZ root() 00000000
    [EOF]
    ");
}

#[test]
fn test_log_immutable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-mA", "root()"]).success();
    work_dir.run_jj(["new", "-mB"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["new", "-mC"]).success();
    work_dir.run_jj(["new", "-mD", "root()"]).success();

    let template = r#"
    separate(" ",
      description.first_line(),
      bookmarks,
      if(immutable, "[immutable]"),
    ) ++ "\n"
    "#;

    test_env.add_config("revset-aliases.'immutable_heads()' = 'main'");
    let output = work_dir.run_jj(["log", "-r::", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  D
    │ ○  C
    │ ◆  B main [immutable]
    │ ◆  A [immutable]
    ├─╯
    ◆  [immutable]
    [EOF]
    ");

    // Suppress error that could be detected earlier
    test_env.add_config("revsets.short-prefixes = ''");

    test_env.add_config("revset-aliases.'immutable_heads()' = 'unknown_fn()'");
    let output = work_dir.run_jj(["log", "-r::", "-T", template]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by:  --> 1:1
      |
    1 | unknown_fn()
      | ^--------^
      |
      = Function `unknown_fn` doesn't exist
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    test_env.add_config("revset-aliases.'immutable_heads()' = 'unknown_symbol'");
    let output = work_dir.run_jj(["log", "-r::", "-T", template]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Failed to evaluate revset
    Caused by:
    1:  --> 5:10
      |
    5 |       if(immutable, "[immutable]"),
      |          ^-------^
      |
      = Failed to evaluate revset
    2: Revision `unknown_symbol` doesn't exist
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_log_contained_in() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-mA", "root()"]).success();
    work_dir.run_jj(["new", "-mB"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["new", "-mC"]).success();
    work_dir.run_jj(["new", "-mD", "root()"]).success();

    let template_for_revset = |revset: &str| {
        format!(
            r#"
    separate(" ",
      description.first_line(),
      bookmarks,
      if(self.contained_in("{revset}"), "[contained_in]"),
    ) ++ "\n"
    "#
        )
    };

    let output = work_dir.run_jj([
        "log",
        "-r::",
        "-T",
        &template_for_revset("subject(glob:A)::"),
    ]);
    insta::assert_snapshot!(output, @r"
    @  D
    │ ○  C [contained_in]
    │ ○  B main [contained_in]
    │ ○  A [contained_in]
    ├─╯
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj([
        "log",
        "-r::",
        "-T",
        &template_for_revset(r#"visible_heads()"#),
    ]);
    insta::assert_snapshot!(output, @r"
    @  D [contained_in]
    │ ○  C [contained_in]
    │ ○  B main
    │ ○  A
    ├─╯
    ◆
    [EOF]
    ");

    // Suppress error that could be detected earlier
    let output = work_dir.run_jj(["log", "-r::", "-T", &template_for_revset("unknown_fn()")]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: In revset expression
    Caused by:
    1:  --> 5:28
      |
    5 |       if(self.contained_in("unknown_fn()"), "[contained_in]"),
      |                            ^------------^
      |
      = In revset expression
    2:  --> 1:1
      |
    1 | unknown_fn()
      | ^--------^
      |
      = Function `unknown_fn` doesn't exist
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["log", "-r::", "-T", &template_for_revset("author(x:'y')")]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: In revset expression
    Caused by:
    1:  --> 5:28
      |
    5 |       if(self.contained_in("author(x:'y')"), "[contained_in]"),
      |                            ^-------------^
      |
      = In revset expression
    2:  --> 1:8
      |
    1 | author(x:'y')
      |        ^---^
      |
      = Invalid string pattern
    3: Invalid string pattern kind `x:`
    Hint: Try prefixing with one of `exact:`, `glob:`, `regex:`, `substring:`, or one of these with `-i` suffix added (e.g. `glob-i:`) for case-insensitive matching
    [EOF]
    [exit status: 1]
    "#);

    let output = work_dir.run_jj(["log", "-r::", "-T", &template_for_revset("maine")]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Failed to evaluate revset
    Caused by:
    1:  --> 5:28
      |
    5 |       if(self.contained_in("maine"), "[contained_in]"),
      |                            ^-----^
      |
      = Failed to evaluate revset
    2: Revision `maine` doesn't exist
    Hint: Did you mean `main`?
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_short_prefix_in_transaction() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(r#"
        [revsets]
        log = '::subject(glob:test)'

        [templates]
        log = 'summary ++ "\n"'
        commit_summary = 'summary'

        [template-aliases]
        'format_id(id)' = 'id.shortest(12).prefix() ++ "[" ++ id.shortest(12).rest() ++ "]"'
        'summary' = 'separate(" ", format_id(change_id), format_id(commit_id), description.first_line())'
    "#);

    work_dir.write_file("file", "original file\n");
    work_dir.run_jj(["describe", "-m", "initial"]).success();

    // Create a chain of 5 commits
    for i in 0..5 {
        work_dir
            .run_jj(["new", "-m", &format!("commit{i}")])
            .success();
        work_dir.write_file("file", format!("file {i}\n"));
    }
    // Create 2^4 duplicates of the chain
    for _ in 0..4 {
        work_dir
            .run_jj(["duplicate", "subject(glob:commit*)"])
            .success();
    }

    // Short prefix should be used for commit summary inside the transaction
    let parent_id = "c0b41"; // Force id lookup to build index before mutation.
    // If the cached index wasn't invalidated, the
    // newly created commit wouldn't be found in it.
    let output = work_dir.run_jj(["new", parent_id, "--no-edit", "-m", "test"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created new commit km[kuslswpqwq] a[5d12a825adf] test
    [EOF]
    ");

    // Should match log's short prefixes
    let output = work_dir.run_jj(["log", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    km[kuslswpqwq] a[5d12a825adf] test
    y[qosqzytrlsw] c0[b41b9a1b34] commit4
    r[oyxmykxtrkr] 1[2124aa50a07] commit3
    m[zvwutvlkqwt] c7[673aedfb82] commit2
    zs[uskulnrvyr] 4[36497fbfb9d] commit1
    kk[mpptxzrspx] d[70e8b9aa12b] commit0
    q[pvuntsmwlqt] 8[216f646c36d] initial
    zz[zzzzzzzzzz] 0[00000000000]
    [EOF]
    ");

    test_env.add_config(r#"revsets.short-prefixes = """#);

    let output = work_dir.run_jj(["log", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    kmk[uslswpqwq] a5[d12a825adf] test
    yq[osqzytrlsw] c0b[41b9a1b34] commit4
    ro[yxmykxtrkr] 121[24aa50a07] commit3
    mz[vwutvlkqwt] c7[673aedfb82] commit2
    zs[uskulnrvyr] 43[6497fbfb9d] commit1
    kk[mpptxzrspx] d7[0e8b9aa12b] commit0
    qp[vuntsmwlqt] 82[16f646c36d] initial
    zz[zzzzzzzzzz] 00[0000000000]
    [EOF]
    ");
}

#[test]
fn test_log_diff_predefined_formats() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\nb\n");
    work_dir.write_file("file2", "a\n");
    work_dir.write_file("rename-source", "rename");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "a\nb\nc\n");
    work_dir.write_file("file2", "b\nc\n");
    std::fs::rename(
        work_dir.root().join("rename-source"),
        work_dir.root().join("rename-target"),
    )
    .unwrap();

    let template = r#"
    concat(
      "=== color_words ===\n",
      diff.color_words(),
      "=== git ===\n",
      diff.git(),
      "=== stat ===\n",
      diff.stat(80),
      "=== summary ===\n",
      diff.summary(),
    )
    "#;

    // color, without paths
    let output = work_dir.run_jj(["log", "--no-graph", "--color=always", "-r@", "-T", template]);
    insta::assert_snapshot!(output, @r"
    === color_words ===
    [38;5;3mModified regular file file1:[39m
    [38;5;1m   1[39m [38;5;2m   1[39m: a
    [38;5;1m   2[39m [38;5;2m   2[39m: b
         [38;5;2m   3[39m: [4m[38;5;2mc[24m[39m
    [38;5;3mModified regular file file2:[39m
    [38;5;1m   1[39m [38;5;2m   1[39m: [4m[38;5;1ma[38;5;2mb[24m[39m
         [38;5;2m   2[39m: [4m[38;5;2mc[24m[39m
    [38;5;3mModified regular file rename-target (rename-source => rename-target):[39m
    === git ===
    [1mdiff --git a/file1 b/file1[0m
    [1mindex 422c2b7ab3..de980441c3 100644[0m
    [1m--- a/file1[0m
    [1m+++ b/file1[0m
    [38;5;6m@@ -1,2 +1,3 @@[39m
     a
     b
    [38;5;2m+[4mc[24m[39m
    [1mdiff --git a/file2 b/file2[0m
    [1mindex 7898192261..9ddeb5c484 100644[0m
    [1m--- a/file2[0m
    [1m+++ b/file2[0m
    [38;5;6m@@ -1,1 +1,2 @@[39m
    [38;5;1m-[4ma[24m[39m
    [38;5;2m+[4mb[24m[39m
    [38;5;2m+[4mc[24m[39m
    [1mdiff --git a/rename-source b/rename-target[0m
    [1mrename from rename-source[0m
    [1mrename to rename-target[0m
    === stat ===
    file1                            | 1 [38;5;2m+[38;5;1m[39m
    file2                            | 3 [38;5;2m++[38;5;1m-[39m
    {rename-source => rename-target} | 0[38;5;1m[39m
    3 files changed, 3 insertions(+), 1 deletion(-)
    === summary ===
    [38;5;6mM file1[39m
    [38;5;6mM file2[39m
    [38;5;6mR {rename-source => rename-target}[39m
    [EOF]
    ");

    // color labels
    let output = work_dir.run_jj(["log", "--no-graph", "--color=debug", "-r@", "-T", template]);
    insta::assert_snapshot!(output, @r"
    <<log commit::=== color_words ===>>
    [38;5;3m<<log commit diff color_words header::Modified regular file file1:>>[39m
    [38;5;1m<<log commit diff color_words removed line_number::   1>>[39m<<log commit diff color_words:: >>[38;5;2m<<log commit diff color_words added line_number::   1>>[39m<<log commit diff color_words::: a>>
    [38;5;1m<<log commit diff color_words removed line_number::   2>>[39m<<log commit diff color_words:: >>[38;5;2m<<log commit diff color_words added line_number::   2>>[39m<<log commit diff color_words::: b>>
    <<log commit diff color_words::     >>[38;5;2m<<log commit diff color_words added line_number::   3>>[39m<<log commit diff color_words::: >>[4m[38;5;2m<<log commit diff color_words added token::c>>[24m[39m
    [38;5;3m<<log commit diff color_words header::Modified regular file file2:>>[39m
    [38;5;1m<<log commit diff color_words removed line_number::   1>>[39m<<log commit diff color_words:: >>[38;5;2m<<log commit diff color_words added line_number::   1>>[39m<<log commit diff color_words::: >>[4m[38;5;1m<<log commit diff color_words removed token::a>>[38;5;2m<<log commit diff color_words added token::b>>[24m[39m<<log commit diff color_words::>>
    <<log commit diff color_words::     >>[38;5;2m<<log commit diff color_words added line_number::   2>>[39m<<log commit diff color_words::: >>[4m[38;5;2m<<log commit diff color_words added token::c>>[24m[39m
    [38;5;3m<<log commit diff color_words header::Modified regular file rename-target (rename-source => rename-target):>>[39m
    <<log commit::=== git ===>>
    [1m<<log commit diff git file_header::diff --git a/file1 b/file1>>[0m
    [1m<<log commit diff git file_header::index 422c2b7ab3..de980441c3 100644>>[0m
    [1m<<log commit diff git file_header::--- a/file1>>[0m
    [1m<<log commit diff git file_header::+++ b/file1>>[0m
    [38;5;6m<<log commit diff git hunk_header::@@ -1,2 +1,3 @@>>[39m
    <<log commit diff git context:: a>>
    <<log commit diff git context:: b>>
    [38;5;2m<<log commit diff git added::+>>[4m<<log commit diff git added token::c>>[24m[39m
    [1m<<log commit diff git file_header::diff --git a/file2 b/file2>>[0m
    [1m<<log commit diff git file_header::index 7898192261..9ddeb5c484 100644>>[0m
    [1m<<log commit diff git file_header::--- a/file2>>[0m
    [1m<<log commit diff git file_header::+++ b/file2>>[0m
    [38;5;6m<<log commit diff git hunk_header::@@ -1,1 +1,2 @@>>[39m
    [38;5;1m<<log commit diff git removed::->>[4m<<log commit diff git removed token::a>>[24m<<log commit diff git removed::>>[39m
    [38;5;2m<<log commit diff git added::+>>[4m<<log commit diff git added token::b>>[24m<<log commit diff git added::>>[39m
    [38;5;2m<<log commit diff git added::+>>[4m<<log commit diff git added token::c>>[24m[39m
    [1m<<log commit diff git file_header::diff --git a/rename-source b/rename-target>>[0m
    [1m<<log commit diff git file_header::rename from rename-source>>[0m
    [1m<<log commit diff git file_header::rename to rename-target>>[0m
    <<log commit::=== stat ===>>
    <<log commit diff stat::file1                            | 1 >>[38;5;2m<<log commit diff stat added::+>>[38;5;1m<<log commit diff stat removed::>>[39m
    <<log commit diff stat::file2                            | 3 >>[38;5;2m<<log commit diff stat added::++>>[38;5;1m<<log commit diff stat removed::->>[39m
    <<log commit diff stat::{rename-source => rename-target} | 0>>[38;5;1m<<log commit diff stat removed::>>[39m
    <<log commit diff stat stat-summary::3 files changed, 3 insertions(+), 1 deletion(-)>>
    <<log commit::=== summary ===>>
    [38;5;6m<<log commit diff summary modified::M file1>>[39m
    [38;5;6m<<log commit diff summary modified::M file2>>[39m
    [38;5;6m<<log commit diff summary renamed::R {rename-source => rename-target}>>[39m
    [EOF]
    ");

    // cwd != workspace root
    let output = test_env.run_jj_in(".", ["log", "-Rrepo", "--no-graph", "-r@", "-T", template]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    === color_words ===
    Modified regular file repo/file1:
       1    1: a
       2    2: b
            3: c
    Modified regular file repo/file2:
       1    1: ab
            2: c
    Modified regular file repo/rename-target (repo/rename-source => repo/rename-target):
    === git ===
    diff --git a/file1 b/file1
    index 422c2b7ab3..de980441c3 100644
    --- a/file1
    +++ b/file1
    @@ -1,2 +1,3 @@
     a
     b
    +c
    diff --git a/file2 b/file2
    index 7898192261..9ddeb5c484 100644
    --- a/file2
    +++ b/file2
    @@ -1,1 +1,2 @@
    -a
    +b
    +c
    diff --git a/rename-source b/rename-target
    rename from rename-source
    rename to rename-target
    === stat ===
    repo/file1                            | 1 +
    repo/file2                            | 3 ++-
    repo/{rename-source => rename-target} | 0
    3 files changed, 3 insertions(+), 1 deletion(-)
    === summary ===
    M repo/file1
    M repo/file2
    R repo/{rename-source => rename-target}
    [EOF]
    ");

    // with non-default config
    std::fs::write(
        test_env.env_root().join("config-good.toml"),
        indoc! {"
            diff.color-words.context = 0
            diff.color-words.max-inline-alternation = 0
            diff.git.context = 1
        "},
    )
    .unwrap();
    let output = work_dir.run_jj([
        "log",
        "--config-file=../config-good.toml",
        "--no-graph",
        "-r@",
        "-T",
        template,
    ]);
    insta::assert_snapshot!(output, @r"
    === color_words ===
    Modified regular file file1:
        ...
            3: c
    Modified regular file file2:
       1     : a
            1: b
            2: c
    Modified regular file rename-target (rename-source => rename-target):
    === git ===
    diff --git a/file1 b/file1
    index 422c2b7ab3..de980441c3 100644
    --- a/file1
    +++ b/file1
    @@ -2,1 +2,2 @@
     b
    +c
    diff --git a/file2 b/file2
    index 7898192261..9ddeb5c484 100644
    --- a/file2
    +++ b/file2
    @@ -1,1 +1,2 @@
    -a
    +b
    +c
    diff --git a/rename-source b/rename-target
    rename from rename-source
    rename to rename-target
    === stat ===
    file1                            | 1 +
    file2                            | 3 ++-
    {rename-source => rename-target} | 0
    3 files changed, 3 insertions(+), 1 deletion(-)
    === summary ===
    M file1
    M file2
    R {rename-source => rename-target}
    [EOF]
    ");

    // bad config
    std::fs::write(
        test_env.env_root().join("config-bad.toml"),
        "diff.git.context = 'not an integer'\n",
    )
    .unwrap();
    let output = work_dir.run_jj([
        "log",
        "--config-file=../config-bad.toml",
        "-Tself.diff().git()",
    ]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse template: Failed to load diff settings
    Caused by:
    1:  --> 1:13
      |
    1 | self.diff().git()
      |             ^-^
      |
      = Failed to load diff settings
    2: Invalid type or value for diff.git.context
    3: invalid type: string "not an integer", expected usize

    Hint: Check the config file: ../config-bad.toml
    [EOF]
    [exit status: 1]
    "#);

    // color_words() with parameters
    let template = "self.diff('file1').color_words(0)";
    let output = work_dir.run_jj(["log", "--no-graph", "-r@", "-T", template]);
    insta::assert_snapshot!(output, @r"
    Modified regular file file1:
        ...
            3: c
    [EOF]
    ");

    // git() with parameters
    let template = "self.diff('file1').git(1)";
    let output = work_dir.run_jj(["log", "--no-graph", "-r@", "-T", template]);
    insta::assert_snapshot!(output, @r"
    diff --git a/file1 b/file1
    index 422c2b7ab3..de980441c3 100644
    --- a/file1
    +++ b/file1
    @@ -2,1 +2,2 @@
     b
    +c
    [EOF]
    ");

    // custom template with files()
    let template = indoc! {r#"
        concat(
          "=== " ++ commit_id.short() ++ " ===\n",
          diff.files().map(|e| separate(" ",
            e.path(),
            "[" ++ e.status() ++ "]",
            "source=" ++ e.source().path() ++ " [" ++ e.source().file_type() ++ "]",
            "target=" ++ e.target().path() ++ " [" ++ e.target().file_type() ++ "]",
          ) ++ "\n").join(""),
          "* " ++ separate(" ",
            if(diff.files(), "non-empty", "empty"),
            "len=" ++ diff.files().len(),
          ) ++ "\n",
        )
    "#};
    let output = work_dir.run_jj(["log", "--no-graph", "-T", template]);
    insta::assert_snapshot!(output, @r"
    === d9ea8f447a3b ===
    file1 [modified] source=file1 [file] target=file1 [file]
    file2 [modified] source=file2 [file] target=file2 [file]
    rename-target [renamed] source=rename-source [file] target=rename-target [file]
    * non-empty len=3
    === 20bc00d202c2 ===
    file1 [added] source=file1 [] target=file1 [file]
    file2 [added] source=file2 [] target=file2 [file]
    rename-source [added] source=rename-source [] target=rename-source [file]
    * non-empty len=3
    === 000000000000 ===
    * empty len=0
    [EOF]
    ");

    // custom diff stat template
    let template = indoc! {r#"
        concat(
          "=== " ++ commit_id.short() ++ " ===\n",
          "* " ++ separate(" ",
            "total_added=" ++ diff.stat().total_added(),
            "total_removed=" ++ diff.stat().total_removed(),
          ) ++ "\n",
        )
    "#};
    let output = work_dir.run_jj(["log", "--no-graph", "-T", template]);
    insta::assert_snapshot!(output, @r"
    === d9ea8f447a3b ===
    * total_added=3 total_removed=1
    === 20bc00d202c2 ===
    * total_added=4 total_removed=0
    === 000000000000 ===
    * total_added=0 total_removed=0
    [EOF]
    ");
}

#[test]
fn test_file_list_entries() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.create_dir("dir");
    work_dir.write_file("dir/file", "content1");
    work_dir.write_file("exec-file", "content1");
    work_dir.write_file("conflict-exec-file", "content1");
    work_dir.write_file("conflict-file", "content1");
    work_dir
        .run_jj(["file", "chmod", "x", "exec-file", "conflict-exec-file"])
        .success();

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("conflict-exec-file", "content2");
    work_dir.write_file("conflict-file", "content2");
    work_dir
        .run_jj(["file", "chmod", "x", "conflict-exec-file"])
        .success();

    work_dir.run_jj(["new", "visible_heads()"]).success();

    let template = indoc! {r#"
        separate(" ",
          path,
          "[" ++ file_type ++ "]",
          "conflict=" ++ conflict,
          "executable=" ++ executable,
        ) ++ "\n"
    "#};
    let output = work_dir.run_jj(["file", "list", "-T", template]);
    insta::assert_snapshot!(output, @r"
    conflict-exec-file [conflict] conflict=true executable=true
    conflict-file [conflict] conflict=true executable=false
    dir/file [file] conflict=false executable=false
    exec-file [file] conflict=false executable=true
    [EOF]
    ");

    let template = r#"if(files, files.map(|e| e.path()), "(empty)") ++ "\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    conflict-exec-file conflict-file dir/file exec-file
    ├─╮
    │ ○  conflict-exec-file conflict-file dir/file exec-file
    ○ │  conflict-exec-file conflict-file
    ├─╯
    ◆  (empty)
    [EOF]
    ");

    let template = r#"self.files("dir").map(|e| e.path()) ++ "\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @    dir/file
    ├─╮
    │ ○  dir/file
    ○ │
    ├─╯
    ◆
    [EOF]
    ");
}

#[cfg(unix)]
#[test]
fn test_file_list_symlink() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::os::unix::fs::symlink("symlink_target", work_dir.root().join("symlink")).unwrap();

    let template = r#"separate(" ", path, "[" ++ file_type ++ "]") ++ "\n""#;
    let output = work_dir.run_jj(["file", "list", "-T", template]);
    insta::assert_snapshot!(output, @r"
    symlink [symlink]
    [EOF]
    ");
}

#[test]
fn test_signature_templates() {
    let test_env = TestEnvironment::default();

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-m", "unsigned"]).success();
    test_env.add_config("signing.behavior = 'own'");
    test_env.add_config("signing.backend = 'test'");
    work_dir.run_jj(["describe", "-m", "signed"]).success();

    let template = r#"
    if(signature,
      signature.status() ++ " " ++ signature.display(),
      "no",
    ) ++ " signature""#;

    // show that signatures can render
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @r"
    @  good test-display signature
    ○  no signature
    ◆  no signature
    [EOF]
    ");
    let output = work_dir.run_jj(["show", "-T", template]);
    insta::assert_snapshot!(output, @"good test-display signature[EOF]");

    // builtin templates
    test_env.add_config("ui.show-cryptographic-signatures = true");

    let args = ["log", "-r", "..", "-T"];

    let output = work_dir.run_jj_with(|cmd| cmd.args(args).arg("builtin_log_oneline"));
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user 2001-02-03 08:05:09 eb0e9b58 [✓︎] (empty) signed
    ○  qpvuntsm test.user 2001-02-03 08:05:08 0604e056 (empty) unsigned
    │
    ~
    [EOF]
    ");

    let output = work_dir.run_jj_with(|cmd| cmd.args(args).arg("builtin_log_compact"));
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 eb0e9b58 [✓︎]
    │  (empty) signed
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 0604e056
    │  (empty) unsigned
    ~
    [EOF]
    ");

    let output = work_dir.run_jj_with(|cmd| cmd.args(args).arg("builtin_log_detailed"));
    insta::assert_snapshot!(output, @r"
    @  Commit ID: eb0e9b58b724003df03b4277d3066c1c20187ce5
    │  Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    │  Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)
    │  Signature: good signature by test-display
    │
    │      signed
    │
    ○  Commit ID: 0604e056feaf8ee553fae4e06d4bfc57cdd319d6
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    ~  Author   : Test User <test.user@example.com> (2001-02-03 08:05:08)
       Committer: Test User <test.user@example.com> (2001-02-03 08:05:08)
       Signature: (no signature)

           unsigned

    [EOF]
    ");

    // customization point
    let config_val = r#"template-aliases."format_short_cryptographic_signature(signature)"="'status: ' ++ signature.status()""#;
    let output = work_dir.run_jj_with(|cmd| {
        cmd.args(args)
            .arg("builtin_log_oneline")
            .args(["--config", config_val])
    });
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user 2001-02-03 08:05:09 eb0e9b58 status: good (empty) signed
    ○  qpvuntsm test.user 2001-02-03 08:05:08 0604e056 status: <Error: No CryptographicSignature available> (empty) unsigned
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_log_git_format_patch_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj([
            "new",
            "-m",
            "some change\n\nmultiline desc\nsecond line\n\nwith blanks\n",
        ])
        .success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "modified\n");
    work_dir.write_file("file3", "new\n");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        "git_format_patch_email_headers",
        "-r@",
    ]);
    insta::assert_snapshot!(output, @r"
    From fee27496968a4347a49d69c0a634fc0d5cf7fbc0 Mon Sep 17 00:00:00 2001
    From: Test User <test.user@example.com>
    Date: Sat, 3 Feb 2001 04:05:08 +0700
    Subject: [PATCH] some change

    multiline desc
    second line

    with blanks
    ---
     file1 | 1 -
     file2 | 2 +-
     file3 | 1 +
     3 files changed, 2 insertions(+), 2 deletions(-)

    [EOF]
    ");
}

#[test]
fn test_log_format_trailers() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        "format_gerrit_change_id_trailer(self) ++ format_signed_off_by_trailer(self)",
        "-r@",
    ]);
    insta::assert_snapshot!(output, @r"
    Change-Id: I9a45c67d3e96a7e5007c110ede34dec56a6a6964
    Signed-off-by: Test User <test.user@example.com>
    [EOF]
    ");

    work_dir
        .run_jj([
            "describe",
            "-r@",
            "-m",
            "a change with trailers",
            r#"--config=templates.commit_trailers="format_signed_off_by_trailer(self) ++ format_gerrit_change_id_trailer(self)""#,
        ])
        .success();

    let output = work_dir.run_jj(["log", "--no-graph", "-T", r#"trailers ++ "\n""#, "-r@"]);
    insta::assert_snapshot!(output, @r"
    Signed-off-by: Test User <test.user@example.com>
    Change-Id: I9a45c67d3e96a7e5007c110ede34dec56a6a6964
    [EOF]
    ");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        "trailers.map(|t| t.key())",
        "-r@",
    ]);
    insta::assert_snapshot!(output, @"Signed-off-by Change-Id[EOF]");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        "trailers.map(|t| t.value())",
        "-r@",
    ]);
    insta::assert_snapshot!(output, @"Test User <test.user@example.com> I9a45c67d3e96a7e5007c110ede34dec56a6a6964[EOF]");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        r#"self.trailers().contains_key("Signed-off-by")"#,
        "-r@",
    ]);
    insta::assert_snapshot!(output, @"true[EOF]");

    let output = work_dir.run_jj([
        "log",
        "--no-graph",
        "-T",
        r#"self.trailers().contains_key("foo")"#,
        "-r@",
    ]);
    insta::assert_snapshot!(output, @"false[EOF]");
}
