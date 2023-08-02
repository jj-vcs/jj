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

use common::TestEnvironment;
use regex::Regex;

pub mod common;

#[test]
fn test_log_parents() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["new", "@-"]);
    test_env.jj_cmd_success(&repo_path, &["new", "@", "@-"]);

    let template = r#"commit_id ++ "\nP: " ++ parents.map(|c| c.commit_id()) ++ "\n""#;
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", template]);
    insta::assert_snapshot!(stdout, @r###"
    @    c067170d4ca1bc6162b64f7550617ec809647f84
    ├─╮  P: 4db490c88528133d579540b6900b8098f0c17701 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◉ │  4db490c88528133d579540b6900b8098f0c17701
    ├─╯  P: 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    ◉  230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  P: 0000000000000000000000000000000000000000
    ◉  0000000000000000000000000000000000000000
       P:
    "###);

    let template = r#"parents.map(|c| c.commit_id().shortest(4))"#;
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &["log", "-T", template, "-r@", "--color=always"],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  [1m[38;5;4m4[0m[38;5;8mdb4[39m [1m[38;5;4m2[0m[38;5;8m30d[39m
    │
    ~
    "###);

    // Commit object isn't printable
    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-T", "parents"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse template:  --> 1:1
      |
    1 | parents
      | ^-----^
      |
      = Expected expression of type "Template"
    "###);

    // Redundant argument passed to keyword method
    let template = r#"parents.map(|c| c.commit_id(""))"#;
    let stderr = test_env.jj_cmd_failure(&repo_path, &["log", "-T", template]);
    insta::assert_snapshot!(stderr, @r###"
    Error: Failed to parse template:  --> 1:29
      |
    1 | parents.map(|c| c.commit_id(""))
      |                             ^^
      |
      = Function "commit_id": Expected 0 arguments
    "###);
}

#[test]
fn test_log_author_timestamp() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "first"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "second"]);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "author.timestamp()"]);
    insta::assert_snapshot!(stdout, @r###"
    @  2001-02-03 04:05:09.000 +07:00
    ◉  2001-02-03 04:05:07.000 +07:00
    ◉  1970-01-01 00:00:00.000 +00:00
    "###);
}

#[test]
fn test_log_author_timestamp_ago() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "first"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "second"]);

    let template = r#"author.timestamp().ago() ++ "\n""#;
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--no-graph", "-T", template]);
    let line_re = Regex::new(r"[0-9]+ years ago").unwrap();
    assert!(
        stdout.lines().all(|x| line_re.is_match(x)),
        "expected every line to match regex"
    );
}

#[test]
fn test_log_default() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "add a file"]);
    test_env.jj_cmd_success(&repo_path, &["new", "-m", "description 1"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "create", "my-branch"]);

    // Test default log output format
    let stdout = test_env.jj_cmd_success(&repo_path, &["log"]);
    insta::assert_snapshot!(stdout, @r###"
    @  kkmpptxz test.user@example.com 2001-02-03 04:05:09.000 +07:00 my-branch 9de54178
    │  (empty) description 1
    ◉  qpvuntsm test.user@example.com 2001-02-03 04:05:08.000 +07:00 4291e264
    │  add a file
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)
    "###);

    // Color
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [1m[38;5;13mk[38;5;8mkmpptxz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 04:05:09.000 +07:00[39m [38;5;13mmy-branch[39m [38;5;12m9[38;5;8mde54178[39m[0m
    │  [1m[38;5;10m(empty)[39m description 1[0m
    ◉  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:08.000 +07:00[39m [1m[38;5;4m4[0m[38;5;8m291e264[39m
    │  add a file
    ◉  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;6m1970-01-01 00:00:00.000 +00:00[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
       [38;5;2m(empty)[39m (no description set)
    "###);

    // Color without graph
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--color=always", "--no-graph"]);
    insta::assert_snapshot!(stdout, @r###"
    [1m[38;5;13mk[38;5;8mkmpptxz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 04:05:09.000 +07:00[39m [38;5;13mmy-branch[39m [38;5;12m9[38;5;8mde54178[39m[0m
    [1m[38;5;10m(empty)[39m description 1[0m
    [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:08.000 +07:00[39m [1m[38;5;4m4[0m[38;5;8m291e264[39m
    add a file
    [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;6m1970-01-01 00:00:00.000 +00:00[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
    [38;5;2m(empty)[39m (no description set)
    "###);
}

#[test]
fn test_log_builtin_templates() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");
    let render = |template| test_env.jj_cmd_success(&repo_path, &["log", "-T", template]);

    insta::assert_snapshot!(render(r#"builtin_log_oneline"#), @r###"
    @  qpvuntsm test.user 2001-02-03 04:05:07.000 +07:00 230dd059 (empty) (no description set)
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000 (empty) (no description set)
    "###);

    insta::assert_snapshot!(render(r#"builtin_log_compact"#), @r###"
    @  qpvuntsm test.user@example.com 2001-02-03 04:05:07.000 +07:00 230dd059
    │  (empty) (no description set)
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)
    "###);

    insta::assert_snapshot!(render(r#"builtin_log_comfortable"#), @r###"
    @  qpvuntsm test.user@example.com 2001-02-03 04:05:07.000 +07:00 230dd059
    │  (empty) (no description set)
    │
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)

    "###);

    insta::assert_snapshot!(render(r#"builtin_log_detailed"#), @r###"
    @  Commit ID: 230dd059e1b059aefc0da06a2e5a7dbf22362f22
    │  Change ID: qpvuntsmwlqtpsluzzsnyyzlmlwvmlnu
    │  Author: Test User <test.user@example.com> (2001-02-03 04:05:07.000 +07:00)
    │  Committer: Test User <test.user@example.com> (2001-02-03 04:05:07.000 +07:00)
    │
    │      (no description set)
    │
    ◉  Commit ID: 0000000000000000000000000000000000000000
       Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
       Author:  <> (1970-01-01 00:00:00.000 +00:00)
       Committer:  <> (1970-01-01 00:00:00.000 +00:00)

           (no description set)

    "###);
}

#[test]
fn test_log_obslog_divergence() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "foo\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "description 1"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["log"]);
    // No divergence
    insta::assert_snapshot!(stdout, @r###"
    @  qpvuntsm test.user@example.com 2001-02-03 04:05:08.000 +07:00 7a17d52e
    │  description 1
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)
    "###);

    // Create divergence
    test_env.jj_cmd_success(
        &repo_path,
        &["describe", "-m", "description 2", "--at-operation", "@-"],
    );
    let stdout = test_env.jj_cmd_success(&repo_path, &["log"]);
    insta::assert_snapshot!(stdout, @r###"
    Concurrent modification detected, resolving automatically.
    ◉  qpvuntsm?? test.user@example.com 2001-02-03 04:05:10.000 +07:00 8979953d
    │  description 2
    │ @  qpvuntsm?? test.user@example.com 2001-02-03 04:05:08.000 +07:00 7a17d52e
    ├─╯  description 1
    ◉  zzzzzzzz 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)
    "###);

    // Color
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    ◉  [1m[4m[38;5;1mq[0m[38;5;1mpvuntsm??[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:10.000 +07:00[39m [1m[38;5;4m8[0m[38;5;8m979953d[39m
    │  description 2
    │ @  [1m[4m[38;5;1mq[24mpvuntsm[38;5;9m??[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 04:05:08.000 +07:00[39m [38;5;12m7[38;5;8ma17d52e[39m[0m
    ├─╯  [1mdescription 1[0m
    ◉  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;6m1970-01-01 00:00:00.000 +00:00[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
       [38;5;2m(empty)[39m (no description set)
    "###);

    // Obslog and hidden divergent
    let stdout = test_env.jj_cmd_success(&repo_path, &["obslog"]);
    insta::assert_snapshot!(stdout, @r###"
    @  qpvuntsm?? test.user@example.com 2001-02-03 04:05:08.000 +07:00 7a17d52e
    │  description 1
    ◉  qpvuntsm?? hidden test.user@example.com 2001-02-03 04:05:08.000 +07:00 3b68ce25
    │  (no description set)
    ◉  qpvuntsm?? hidden test.user@example.com 2001-02-03 04:05:07.000 +07:00 230dd059
       (empty) (no description set)
    "###);

    // Colored obslog
    let stdout = test_env.jj_cmd_success(&repo_path, &["obslog", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [1m[4m[38;5;1mq[24mpvuntsm[38;5;9m??[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 04:05:08.000 +07:00[39m [38;5;12m7[38;5;8ma17d52e[39m[0m
    │  [1mdescription 1[0m
    ◉  [1m[24m[39mq[0m[38;5;8mpvuntsm[1m[39m?? hidden[0m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:08.000 +07:00[39m [1m[38;5;4m3[0m[38;5;8mb68ce25[39m
    │  (no description set)
    ◉  [1m[24m[39mq[0m[38;5;8mpvuntsm[1m[39m?? hidden[0m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:07.000 +07:00[39m [1m[38;5;4m2[0m[38;5;8m30dd059[39m
       [38;5;2m(empty)[39m (no description set)
    "###);
}

#[test]
fn test_log_git_head() {
    let test_env = TestEnvironment::default();
    let repo_path = test_env.env_root().join("repo");
    git2::Repository::init(&repo_path).unwrap();
    test_env.jj_cmd_success(&repo_path, &["init", "--git-repo=."]);

    test_env.jj_cmd_success(&repo_path, &["new", "-m=initial"]);
    std::fs::write(repo_path.join("file"), "foo\n").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "--color=always"]);
    insta::assert_snapshot!(stdout, @r###"
    @  [1m[38;5;13mr[38;5;8mlvkpnrz[39m [38;5;3mtest.user@example.com[39m [38;5;14m2001-02-03 04:05:09.000 +07:00[39m [38;5;12m5[38;5;8m0aaf475[39m[0m
    │  [1minitial[0m
    ◉  [1m[38;5;5mq[0m[38;5;8mpvuntsm[39m [38;5;3mtest.user@example.com[39m [38;5;6m2001-02-03 04:05:07.000 +07:00[39m [38;5;5mmaster[39m [38;5;2mHEAD@git[39m [1m[38;5;4m2[0m[38;5;8m30dd059[39m
    │  [38;5;2m(empty)[39m (no description set)
    ◉  [1m[38;5;5mz[0m[38;5;8mzzzzzzz[39m [38;5;6m1970-01-01 00:00:00.000 +00:00[39m [1m[38;5;4m0[0m[38;5;8m0000000[39m
       [38;5;2m(empty)[39m (no description set)
    "###);
}

#[test]
fn test_log_customize_short_id() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_success(&repo_path, &["describe", "-m", "first"]);

    // Customize both the commit and the change id
    let decl = "template-aliases.'format_short_id(id)'";
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "--config-toml",
            &format!(r#"{decl}='id.shortest(5).prefix().upper() ++ "_" ++ id.shortest(5).rest()'"#),
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  Q_pvun test.user@example.com 2001-02-03 04:05:08.000 +07:00 6_9542
    │  (empty) first
    ◉  Z_zzzz 1970-01-01 00:00:00.000 +00:00 0_0000
       (empty) (no description set)
    "###);

    // Customize only the change id
    let stdout = test_env.jj_cmd_success(
        &repo_path,
        &[
            "log",
            "--config-toml",
            r#"
                [template-aliases]
                'format_short_change_id(id)'='format_short_id(id).upper()'
            "#,
        ],
    );
    insta::assert_snapshot!(stdout, @r###"
    @  QPVUNTSM test.user@example.com 2001-02-03 04:05:08.000 +07:00 69542c19
    │  (empty) first
    ◉  ZZZZZZZZ 1970-01-01 00:00:00.000 +00:00 00000000
       (empty) (no description set)
    "###);
}
