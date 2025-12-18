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

use regex::Regex;

use crate::common::TestEnvironment;

#[test]
fn test_show() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Show @ by default
    let output = work_dir.run_jj(["show"]);
    let output = output.normalize_stdout_with(|s| s.split_inclusive('\n').skip(2).collect());

    insta::assert_snapshot!(output, @r"
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:07)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:07)

        (no description set)

    [EOF]
    ");

    // Specify revision with -r
    let output = work_dir.run_jj(["show", "-r@-"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 0000000000000000000000000000000000000000
    Change ID: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
    Author   : (no name set) <(no email set)> (1970-01-01 11:00:00)
    Committer: (no name set) <(no email set)> (1970-01-01 11:00:00)

        (no description set)

    [EOF]
    ");

    // Specify both positional and -r args
    let output = work_dir.run_jj(["show", "@", "-r@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the argument '[REVSET]' cannot be used with '-r <REVSET>'

    Usage: jj show <REVSET>

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
}

#[test]
fn test_show_basic() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "foo\nbaz qux\n");
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "foo\nbar\nbaz quux\n");
    work_dir.write_file("file3", "foo\n");

    let output = work_dir.run_jj(["show"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    Modified regular file file2:
       1    1: foo
            2: bar
       2    3: baz quxquux
    Modified regular file file3 (file1 => file3):
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--context=0"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    Modified regular file file2:
       1    1: foo
            2: bar
       2    3: baz quxquux
    Modified regular file file3 (file1 => file3):
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    <<show commit::Commit ID: >>[38;5;4m<<show commit commit_id::92e687faa4e5b681937f5a9c47feaa33e6b4892c>>[39m<<show commit::>>
    <<show commit::Change ID: >>[38;5;5m<<show commit change_id::rlvkpnrzqnoowoytxnquwvuryrwnrmlp>>[39m<<show commit::>>
    <<show commit::Author   : >>[38;5;3m<<show commit author name::Test User>>[39m<<show commit:: <>>[38;5;3m<<show commit author email local::test.user>><<show commit author email::@>><<show commit author email domain::example.com>>[39m<<show commit::> (>>[38;5;6m<<show commit author timestamp local format::2001-02-03 08:05:09>>[39m<<show commit::)>>
    <<show commit::Committer: >>[38;5;3m<<show commit committer name::Test User>>[39m<<show commit:: <>>[38;5;3m<<show commit committer email local::test.user>><<show commit committer email::@>><<show commit committer email domain::example.com>>[39m<<show commit::> (>>[38;5;6m<<show commit committer timestamp local format::2001-02-03 08:05:09>>[39m<<show commit::)>>
    <<show commit::>>
    [38;5;3m<<show commit description placeholder::    (no description set)>>[39m<<show commit::>>
    <<show commit::>>
    [38;5;3m<<diff header::Modified regular file file2:>>[39m
    [2m[38;5;1m<<diff context removed line_number::   1>>[0m<<diff context:: >>[2m[38;5;2m<<diff context added line_number::   1>>[0m<<diff context::: foo>>
    <<diff::     >>[38;5;2m<<diff added line_number::   2>>[39m<<diff::: >>[4m[38;5;2m<<diff added token::bar>>[24m[39m
    [38;5;1m<<diff removed line_number::   2>>[39m<<diff:: >>[38;5;2m<<diff added line_number::   3>>[39m<<diff::: baz >>[4m[38;5;1m<<diff removed token::qux>>[38;5;2m<<diff added token::quux>>[24m[39m<<diff::>>
    [38;5;3m<<diff header::Modified regular file file3 (file1 => file3):>>[39m
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "-s"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    M file2
    R {file1 => file3}
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--types"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    FF file2
    FF {file1 => file3}
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--git"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    diff --git a/file2 b/file2
    index 523a4a9de8..485b56a572 100644
    --- a/file2
    +++ b/file2
    @@ -1,2 +1,3 @@
     foo
    -baz qux
    +bar
    +baz quux
    diff --git a/file1 b/file3
    rename from file1
    rename to file3
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--git", "--context=0"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    diff --git a/file2 b/file2
    index 523a4a9de8..485b56a572 100644
    --- a/file2
    +++ b/file2
    @@ -2,1 +2,2 @@
    -baz qux
    +bar
    +baz quux
    diff --git a/file1 b/file3
    rename from file1
    rename to file3
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--git", "--color=debug"]);
    insta::assert_snapshot!(output, @r"
    <<show commit::Commit ID: >>[38;5;4m<<show commit commit_id::92e687faa4e5b681937f5a9c47feaa33e6b4892c>>[39m<<show commit::>>
    <<show commit::Change ID: >>[38;5;5m<<show commit change_id::rlvkpnrzqnoowoytxnquwvuryrwnrmlp>>[39m<<show commit::>>
    <<show commit::Author   : >>[38;5;3m<<show commit author name::Test User>>[39m<<show commit:: <>>[38;5;3m<<show commit author email local::test.user>><<show commit author email::@>><<show commit author email domain::example.com>>[39m<<show commit::> (>>[38;5;6m<<show commit author timestamp local format::2001-02-03 08:05:09>>[39m<<show commit::)>>
    <<show commit::Committer: >>[38;5;3m<<show commit committer name::Test User>>[39m<<show commit:: <>>[38;5;3m<<show commit committer email local::test.user>><<show commit committer email::@>><<show commit committer email domain::example.com>>[39m<<show commit::> (>>[38;5;6m<<show commit committer timestamp local format::2001-02-03 08:05:09>>[39m<<show commit::)>>
    <<show commit::>>
    [38;5;3m<<show commit description placeholder::    (no description set)>>[39m<<show commit::>>
    <<show commit::>>
    [1m<<diff file_header::diff --git a/file2 b/file2>>[0m
    [1m<<diff file_header::index 523a4a9de8..485b56a572 100644>>[0m
    [1m<<diff file_header::--- a/file2>>[0m
    [1m<<diff file_header::+++ b/file2>>[0m
    [38;5;6m<<diff hunk_header::@@ -1,2 +1,3 @@>>[39m
    <<diff context:: foo>>
    [38;5;1m<<diff removed::-baz >>[4m<<diff removed token::qux>>[24m<<diff removed::>>[39m
    [38;5;2m<<diff added::+>>[4m<<diff added token::bar>>[24m[39m
    [38;5;2m<<diff added::+baz >>[4m<<diff added token::quux>>[24m<<diff added::>>[39m
    [1m<<diff file_header::diff --git a/file1 b/file3>>[0m
    [1m<<diff file_header::rename from file1>>[0m
    [1m<<diff file_header::rename to file3>>[0m
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "-s", "--git"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    M file2
    R {file1 => file3}
    diff --git a/file2 b/file2
    index 523a4a9de8..485b56a572 100644
    --- a/file2
    +++ b/file2
    @@ -1,2 +1,3 @@
     foo
    -baz qux
    +bar
    +baz quux
    diff --git a/file1 b/file3
    rename from file1
    rename to file3
    [EOF]
    ");

    let output = work_dir.run_jj(["show", "--stat"]);
    insta::assert_snapshot!(output, @r"
    Commit ID: 92e687faa4e5b681937f5a9c47feaa33e6b4892c
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:09)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        (no description set)

    file2            | 3 ++-
    {file1 => file3} | 0
    2 files changed, 2 insertions(+), 1 deletion(-)
    [EOF]
    ");
}

#[test]
fn test_show_with_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-m", "a new commit"]).success();

    let output = work_dir.run_jj(["show", "-T", "description"]);

    insta::assert_snapshot!(output, @r"
    a new commit
    [EOF]
    ");
}

#[test]
fn test_show_with_template_no_patch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-m", "a new commit"]).success();
    work_dir.write_file("file1", "foo\n");

    let output = work_dir.run_jj(["show", "--no-patch", "-T", "description"]);

    insta::assert_snapshot!(output, @r"
    a new commit
    [EOF]
    ");
}

#[test]
fn test_show_with_no_patch() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new", "-m", "a new commit"]).success();
    work_dir.write_file("file1", "foo\n");

    let output = work_dir.run_jj(["show", "--no-patch"]);

    insta::assert_snapshot!(output, @r"
    Commit ID: 86d5fa72f4ecc6d51478941ee9160db9c52b842e
    Change ID: rlvkpnrzqnoowoytxnquwvuryrwnrmlp
    Author   : Test User <test.user@example.com> (2001-02-03 08:05:08)
    Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

        a new commit

    [EOF]
    ");
}

#[test]
fn test_show_with_no_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["show", "-T"]);
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
fn test_show_relative_timestamps() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(
        r#"
        [template-aliases]
        'format_timestamp(timestamp)' = 'timestamp.ago()'
        "#,
    );

    let output = work_dir.run_jj(["show"]);
    let timestamp_re = Regex::new(r"\([0-9]+ years ago\)").unwrap();
    let output = output.normalize_stdout_with(|s| {
        s.split_inclusive('\n')
            .skip(2)
            .map(|x| timestamp_re.replace_all(x, "(...timestamp...)"))
            .collect()
    });

    insta::assert_snapshot!(output, @r"
    Author   : Test User <test.user@example.com> (...timestamp...)
    Committer: Test User <test.user@example.com> (...timestamp...)

        (no description set)

    [EOF]
    ");
}
