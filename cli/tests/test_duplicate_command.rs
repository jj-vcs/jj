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

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;
use crate::common::create_commit;

#[test]
fn test_duplicate() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &[]);
    create_commit(&work_dir, "c", &["a", "b"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    387b928721d9   c
    ├─╮
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "all()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Cannot duplicate the root commit
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["duplicate", "none()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to duplicate.
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as kpqxywon 13eb8bd0 a
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    387b928721d9   c
    ├─╮
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
    ├─╯
    │ ○  13eb8bd0a547   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Undid operation: 35843957f59a (2001-02-03 08:05:17) duplicate 1 commit(s)
    Restored to operation: b1024e3a796c (2001-02-03 08:05:13) create bookmark c pointing to commit 387b928721d9f2efff819ccce81868f32537d71f
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @    387b928721d9   c
    ├─╮
    │ ○  d18ca3e87135   b
    ○ │  7d980be7a1d4   a
    ├─╯
    ◆  000000000000
    [EOF]
    ");
}

// https://github.com/jj-vcs/jj/issues/694
#[test]
fn test_rebase_duplicates() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);
    // Test the setup
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  dffaa0d4dacc   c @ 2001-02-03 04:05:13.000 +07:00
    ○  123b4d91f6e5   b @ 2001-02-03 04:05:11.000 +07:00
    ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated dffaa0d4dacc as yostqsxw fc2e8dc2 c
    [EOF]
    ");
    let output = work_dir.run_jj(["duplicate", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated dffaa0d4dacc as znkkpsqq 14e2803a c
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  dffaa0d4dacc   c @ 2001-02-03 04:05:13.000 +07:00
    │ ○  14e2803a4b0e   c @ 2001-02-03 04:05:16.000 +07:00
    ├─╯
    │ ○  fc2e8dc218ab   c @ 2001-02-03 04:05:15.000 +07:00
    ├─╯
    ○  123b4d91f6e5   b @ 2001-02-03 04:05:11.000 +07:00
    ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");

    let output = work_dir.run_jj(["rebase", "-s", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Rebased 4 commits to destination
    Working copy  (@) now at: royxmykx fa60711d c | c
    Parent commit (@-)      : zsuskuln 594e9d32 b | b
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    // Some of the duplicate commits' timestamps were changed a little to make them
    // have distinct commit ids.
    insta::assert_snapshot!(get_log_output_with_ts(&work_dir), @"
    @  fa60711d6bd1   c @ 2001-02-03 04:05:18.000 +07:00
    │ ○  e320e3d23be0   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    │ ○  f9c10a3b2cfd   c @ 2001-02-03 04:05:18.000 +07:00
    ├─╯
    ○  594e9d322230   b @ 2001-02-03 04:05:18.000 +07:00
    │ ○  7d980be7a1d4   a @ 2001-02-03 04:05:09.000 +07:00
    ├─╯
    ◆  000000000000    @ 1970-01-01 00:00:00.000 +00:00
    [EOF]
    ");
}

#[test]
fn test_duplicate_description_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);

    // Test the setup
    insta::assert_snapshot!(get_log_output(&work_dir), @"
    @  dffaa0d4dacc   c
    ○  123b4d91f6e5   b
    ○  7d980be7a1d4   a
    ◆  000000000000
    [EOF]
    ");

    // Test duplicate_commits()
    test_env.add_config(r#"templates.duplicate_description = "concat(description, '\n(cherry picked from commit ', commit_id, ')')""#);
    let output = work_dir.run_jj(["duplicate", "a"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 7d980be7a1d4 as yostqsxw f73017d9 a
    [EOF]
    ");

    // Test duplicate_commits_onto_parents()
    let output = work_dir.run_jj(["duplicate", "a", "-B", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Duplicating commit 7d980be7a1d4 as a descendant of itself
    Duplicated 7d980be7a1d4 as znkkpsqq fdd77a5e (empty) a
    Rebased 2 commits onto duplicated commits
    Working copy  (@) now at: royxmykx 5679a60a c | c
    Parent commit (@-)      : zsuskuln cb58e31e b | b
    [EOF]
    ");

    // Test empty template
    test_env.add_config("templates.duplicate_description = ''");
    let output = work_dir.run_jj(["duplicate", "b", "-o", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated cb58e31ed5d4 as kpqxywon 33044659 (no description set)
    [EOF]
    ");

    // Test `description` as an alias
    test_env.add_config("templates.duplicate_description = 'description'");
    let output = work_dir.run_jj([
        "duplicate",
        "c",
        "-o",
        "root()",
        // Use an argument here so we can actually see the log in the last test
        // (We don't have a way to remove a config in TestEnvironment)
        "--config",
        "template-aliases.description='\"alias\"'",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Duplicated 5679a60ab86b as kmkuslsw e36bebd2 alias
    [EOF]
    ");

    let template = r#"commit_id.short() ++ "\n" ++ description ++ "[END]\n""#;
    let output = work_dir.run_jj(["log", "-T", template]);
    insta::assert_snapshot!(output, @"
    @  5679a60ab86b
    │  c
    │  [END]
    ○  cb58e31ed5d4
    │  b
    │  [END]
    ○  fdd77a5e11d5
    │  a
    │
    │  (cherry picked from commit 7d980be7a1d499e4d316ab4c01242885032f7eaf)
    │  [END]
    ○  7d980be7a1d4
    │  a
    │  [END]
    │ ○  e36bebd28ab6
    ├─╯  alias
    │    [END]
    │ ○  33044659b895
    ├─╯  [END]
    │ ○  f73017d958e7
    ├─╯  a
    │
    │    (cherry picked from commit 7d980be7a1d499e4d316ab4c01242885032f7eaf)
    │    [END]
    ◆  000000000000
       [END]
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ "   " ++ description.first_line()"#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_log_output_with_ts(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"
    commit_id.short() ++ "   " ++ description.first_line() ++ " @ " ++ committer.timestamp()
    "#;
    work_dir.run_jj(["log", "-T", template])
}
