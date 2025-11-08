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

use std::path::PathBuf;

use test_case::test_case;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), empty, local_bookmarks, description)"#;
    work_dir.run_jj(["log", "-T", template])
}

#[must_use]
fn get_log_with_summary(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), local_bookmarks, description)"#;
    work_dir.run_jj(["log", "-T", template, "--summary"])
}

#[must_use]
fn get_workspace_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"separate(" ", change_id.short(), working_copies, description)"#;
    work_dir.run_jj(["log", "-T", template, "-r", "all()"])
}

#[must_use]
fn get_recorded_dates(work_dir: &TestWorkDir, revset: &str) -> CommandOutput {
    let template = r#"separate("\n", "Author date:  " ++ author.timestamp(), "Committer date: " ++ committer.timestamp())"#;
    work_dir.run_jj(["log", "--no-graph", "-T", template, "-r", revset])
}

#[test]
fn test_split_by_paths() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo");
    work_dir.write_file("file2", "foo");
    work_dir.write_file("file3", "foo");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  qpvuntsmwlqt false
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");
    insta::assert_snapshot!(get_recorded_dates(&work_dir, "@"), @r"
    Author date:  2001-02-03 04:05:08.000 +07:00
    Committer date: 2001-02-03 04:05:08.000 +07:00[EOF]
    ");

    std::fs::write(
        &edit_script,
        ["dump editor0", "next invocation\n", "dump editor1"].join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Selected changes : qpvuntsm 6dbc7747 (no description set)
    Remaining changes: zsuskuln 42cbbc02 (no description set)
    Working copy  (@) now at: zsuskuln 42cbbc02 (no description set)
    Parent commit (@-)      : qpvuntsm 6dbc7747 (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.


    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    assert!(!test_env.env_root().join("editor1").exists());

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr false
    ○  qpvuntsmwlqt false
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // The author dates of the new commits should be inherited from the commit being
    // split. The committer dates should be newer.
    insta::assert_snapshot!(get_recorded_dates(&work_dir, "@"), @r"
    Author date:  2001-02-03 04:05:08.000 +07:00
    Committer date: 2001-02-03 04:05:10.000 +07:00[EOF]
    ");
    insta::assert_snapshot!(get_recorded_dates(&work_dir, "@-"), @r"
    Author date:  2001-02-03 04:05:08.000 +07:00
    Committer date: 2001-02-03 04:05:10.000 +07:00[EOF]
    ");

    let output = work_dir.run_jj(["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    A file2
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    A file1
    A file3
    [EOF]
    ");

    // Insert an empty commit after @- with "split ."
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["split", "-r", "@-", "."]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: All changes have been selected, so the original revision will become empty
    Rebased 1 descendant commits
    Selected changes : qpvuntsm 9fd1c9e1 (no description set)
    Remaining changes: znkkpsqq 41e0da21 (empty) (no description set)
    Working copy  (@) now at: zsuskuln a06e40b8 (no description set)
    Parent commit (@-)      : znkkpsqq 41e0da21 (empty) (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr false
    ○  znkkpsqqskkl true
    ○  qpvuntsmwlqt false
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    let output = work_dir.run_jj(["diff", "-s", "-r", "@--"]);
    insta::assert_snapshot!(output, @r"
    A file2
    [EOF]
    ");

    // Remove newly created empty commit
    work_dir.run_jj(["abandon", "@-"]).success();

    // Insert an empty commit before @- with "split nonexistent"
    std::fs::write(&edit_script, "").unwrap();
    let output = work_dir.run_jj(["split", "-r", "@-", "nonexistent"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: No changes have been selected, so the new revision will be empty
    Rebased 1 descendant commits
    Selected changes : qpvuntsm 49416632 (empty) (no description set)
    Remaining changes: lylxulpl 718afbf5 (no description set)
    Working copy  (@) now at: zsuskuln 0ed53ee6 (no description set)
    Parent commit (@-)      : lylxulpl 718afbf5 (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  zsuskulnrvyr false
    ○  lylxulplsnyw false
    ○  qpvuntsmwlqt true
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    let output = work_dir.run_jj(["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    A file2
    [EOF]
    ");
}

#[test]
fn test_split_with_non_empty_description() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(r#"ui.default-description = "\n\nTESTED=TODO""#);
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.run_jj(["describe", "-m", "test"]).success();
    std::fs::write(
        edit_script,
        [
            "dump editor1",
            "write\npart 1",
            "next invocation\n",
            "dump editor2",
            "write\npart 2",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    Selected changes : qpvuntsm c7f7b14b part 1
    Remaining changes: kkmpptxz ac33a5a9 part 2
    Working copy  (@) now at: kkmpptxz ac33a5a9 part 2
    Parent commit (@-)      : qpvuntsm c7f7b14b part 1
    [EOF]
    "#);

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.
    test

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the remaining changes.
    test

    JJ: Change ID: kkmpptxz
    JJ: This commit contains the following changes:
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  kkmpptxzrspx false part 2
    ○  qpvuntsmwlqt false part 1
    ◆  zzzzzzzzzzzz true
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
}

#[test]
fn test_split_with_default_description() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(r#"ui.default-description = "\n\nTESTED=TODO""#);
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");

    std::fs::write(
        edit_script,
        ["dump editor1", "next invocation\n", "dump editor2"].join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    Selected changes : qpvuntsm ff633dcc TESTED=TODO
    Remaining changes: rlvkpnrz b1d20b7e (no description set)
    Working copy  (@) now at: rlvkpnrz b1d20b7e (no description set)
    Parent commit (@-)      : qpvuntsm ff633dcc TESTED=TODO
    [EOF]
    "#);

    // Since the commit being split has no description, the user will only be
    // prompted to add a description to the first commit, which will use the
    // default value we set. The second commit will inherit the empty
    // description from the commit being split.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.


    TESTED=TODO

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    assert!(!test_env.env_root().join("editor2").exists());
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  rlvkpnrzqnoo false
    ○  qpvuntsmwlqt false TESTED=TODO
    ◆  zzzzzzzzzzzz true
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
}

#[test]
fn test_split_with_descendants() {
    // Configure the environment and make the initial commits.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // First commit. This is the one we will split later.
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj(["commit", "-m", "Add file1 & file2"])
        .success();
    // Second commit.
    work_dir.write_file("file3", "baz\n");
    work_dir.run_jj(["commit", "-m", "Add file3"]).success();
    // Third commit.
    work_dir.write_file("file4", "foobarbaz\n");
    work_dir.run_jj(["describe", "-m", "Add file4"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r###"
    @  kkmpptxzrspx false Add file4
    ○  rlvkpnrzqnoo false Add file3
    ○  qpvuntsmwlqt false Add file1 & file2
    ◆  zzzzzzzzzzzz true
    [EOF]
    "###);

    // Set up the editor and do the split.
    std::fs::write(
        edit_script,
        [
            "dump editor1",
            "write\nAdd file1",
            "next invocation\n",
            "dump editor2",
            "write\nAdd file2",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "file1", "-r", "qpvu"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : qpvuntsm 74306e35 Add file1
    Remaining changes: royxmykx 0a37745e Add file2
    Working copy  (@) now at: kkmpptxz 7ee84812 Add file4
    Parent commit (@-)      : rlvkpnrz d335bd94 Add file3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kkmpptxzrspx false Add file4
    ○  rlvkpnrzqnoo false Add file3
    ○  royxmykxtrkr false Add file2
    ○  qpvuntsmwlqt false Add file1
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // The commit we're splitting has a description, so the user will be
    // prompted to enter a description for each of the commits.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.
    Add file1 & file2

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the remaining changes.
    Add file1 & file2

    JJ: Change ID: royxmykx
    JJ: This commit contains the following changes:
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // Check the evolog for the first commit. It shows four entries:
    // - The initial empty commit.
    // - The rewritten commit from the snapshot after the files were added.
    // - The rewritten commit once the description is added during `jj commit`.
    // - The rewritten commit after the split.
    let evolog_1 = work_dir.run_jj(["evolog", "-r", "qpvun"]);
    insta::assert_snapshot!(evolog_1, @r"
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 74306e35
    │  Add file1
    │  -- operation 994b490f285d split commit 1d2499e72cefc8a2b87ebb47569140857b96189f
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 1d2499e7
    │  Add file1 & file2
    │  -- operation adf4f33386c9 commit f5700f8ef89e290e4e90ae6adc0908707e0d8c85
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 f5700f8e
    │  (no description set)
    │  -- operation 78ead2155fcc snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");

    // The evolog for the second commit is the same, except that the change id
    // changes after the split.
    let evolog_2 = work_dir.run_jj(["evolog", "-r", "royxm"]);
    insta::assert_snapshot!(evolog_2, @r"
    ○  royxmykx test.user@example.com 2001-02-03 08:05:12 0a37745e
    │  Add file2
    │  -- operation 994b490f285d split commit 1d2499e72cefc8a2b87ebb47569140857b96189f
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 1d2499e7
    │  Add file1 & file2
    │  -- operation adf4f33386c9 commit f5700f8ef89e290e4e90ae6adc0908707e0d8c85
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 f5700f8e
    │  (no description set)
    │  -- operation 78ead2155fcc snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ");
}

// This test makes sure that the children of the commit being split retain any
// other parents which weren't involved in the split.
#[test]
fn test_split_with_merge_child() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir.run_jj(["new", "root()", "-m=a"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj(["new", "description(1)", "description(a)", "-m=2"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    zsuskulnrvyr true 2
    ├─╮
    │ ○  kkmpptxzrspx false a
    ○ │  qpvuntsmwlqt true 1
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // Set up the editor and do the split.
    std::fs::write(
        edit_script,
        ["write\nAdd file1", "next invocation\n", "write\nAdd file2"].join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "-r", "description(a)", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Selected changes : kkmpptxz cc199567 Add file1
    Remaining changes: royxmykx e488409f Add file2
    Working copy  (@) now at: zsuskuln ace61421 (empty) 2
    Parent commit (@-)      : qpvuntsm 884fe9b9 (empty) 1
    Parent commit (@-)      : royxmykx e488409f Add file2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    zsuskulnrvyr true 2
    ├─╮
    │ ○  royxmykxtrkr false Add file2
    │ ○  kkmpptxzrspx false Add file1
    ○ │  qpvuntsmwlqt true 1
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");
}

#[test]
// Split a commit with no descendants into siblings. Also tests that the default
// description is set correctly on the first commit.
fn test_split_parallel_no_descendants() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(r#"ui.default-description = "\n\nTESTED=TODO""#);
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");

    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  qpvuntsmwlqt false
    ◆  zzzzzzzzzzzz true
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);

    std::fs::write(
        edit_script,
        ["dump editor1", "next invocation\n", "dump editor2"].join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "--parallel", "file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    Selected changes : qpvuntsm 7bcd474c TESTED=TODO
    Remaining changes: kkmpptxz 431886f6 (no description set)
    Working copy  (@) now at: kkmpptxz 431886f6 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  kkmpptxzrspx false
    │ ○  qpvuntsmwlqt false TESTED=TODO
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);

    // Since the commit being split has no description, the user will only be
    // prompted to add a description to the first commit, which will use the
    // default value we set. The second commit will inherit the empty
    // description from the commit being split.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.


    TESTED=TODO

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    assert!(!test_env.env_root().join("editor2").exists());

    // Check the evolog for the first commit. It shows three entries:
    // - The initial empty commit.
    // - The rewritten commit from the snapshot after the files were added.
    // - The rewritten commit after the split.
    let evolog_1 = work_dir.run_jj(["evolog", "-r", "qpvun"]);
    insta::assert_snapshot!(evolog_1, @r#"
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:09 7bcd474c
    │  TESTED=TODO
    │  -- operation 2b21c33e1596 split commit f5700f8ef89e290e4e90ae6adc0908707e0d8c85
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 f5700f8e
    │  (no description set)
    │  -- operation 1663cd1cc445 snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);

    // The evolog for the second commit is the same, except that the change id
    // changes after the split.
    let evolog_2 = work_dir.run_jj(["evolog", "-r", "kkmpp"]);
    insta::assert_snapshot!(evolog_2, @r#"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 431886f6
    │  (no description set)
    │  -- operation 2b21c33e1596 split commit f5700f8ef89e290e4e90ae6adc0908707e0d8c85
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 f5700f8e
    │  (no description set)
    │  -- operation 1663cd1cc445 snapshot working copy
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 e8849ae1
       (empty) (no description set)
       -- operation 8f47435a3990 add workspace 'default'
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
}

#[test]
fn test_split_parallel_with_descendants() {
    // Configure the environment and make the initial commits.
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // First commit. This is the one we will split later.
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj(["commit", "-m", "Add file1 & file2"])
        .success();
    // Second commit. This will be the child of the sibling commits after the split.
    work_dir.write_file("file3", "baz\n");
    work_dir.run_jj(["commit", "-m", "Add file3"]).success();
    // Third commit.
    work_dir.write_file("file4", "foobarbaz\n");
    work_dir.run_jj(["describe", "-m", "Add file4"]).success();
    // Move back to the previous commit so that we don't have to pass a revision
    // to the split command.
    work_dir.run_jj(["prev", "--edit"]).success();
    work_dir.run_jj(["prev", "--edit"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx false Add file4
    ○  rlvkpnrzqnoo false Add file3
    @  qpvuntsmwlqt false Add file1 & file2
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // Set up the editor and do the split.
    std::fs::write(
        edit_script,
        [
            "dump editor1",
            "write\nAdd file1",
            "next invocation\n",
            "dump editor2",
            "write\nAdd file2",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "--parallel", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : qpvuntsm 18c85f56 Add file1
    Remaining changes: vruxwmqv cbdfd9cf Add file2
    Working copy  (@) now at: vruxwmqv cbdfd9cf Add file2
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    ○  kkmpptxzrspx false Add file4
    ○    rlvkpnrzqnoo false Add file3
    ├─╮
    │ @  vruxwmqvtpmx false Add file2
    ○ │  qpvuntsmwlqt false Add file1
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // The commit we're splitting has a description, so the user will be
    // prompted to enter a description for each of the sibling commits.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.
    Add file1 & file2

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the remaining changes.
    Add file1 & file2

    JJ: Change ID: vruxwmqv
    JJ: This commit contains the following changes:
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
}

// This test makes sure that the children of the commit being split retain any
// other parents which weren't involved in the split.
#[test]
fn test_split_parallel_with_merge_child() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["describe", "-m=1"]).success();
    work_dir.run_jj(["new", "root()", "-m=a"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir
        .run_jj(["new", "description(1)", "description(a)", "-m=2"])
        .success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @    zsuskulnrvyr true 2
    ├─╮
    │ ○  kkmpptxzrspx false a
    ○ │  qpvuntsmwlqt true 1
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // Set up the editor and do the split.
    std::fs::write(
        edit_script,
        ["write\nAdd file1", "next invocation\n", "write\nAdd file2"].join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["split", "-r", "description(a)", "--parallel", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Selected changes : kkmpptxz cc199567 Add file1
    Remaining changes: royxmykx 82a5c527 Add file2
    Working copy  (@) now at: zsuskuln b7cdcdec (empty) 2
    Parent commit (@-)      : qpvuntsm 884fe9b9 (empty) 1
    Parent commit (@-)      : kkmpptxz cc199567 Add file1
    Parent commit (@-)      : royxmykx 82a5c527 Add file2
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @      zsuskulnrvyr true 2
    ├─┬─╮
    │ │ ○  royxmykxtrkr false Add file2
    │ ○ │  kkmpptxzrspx false Add file1
    │ ├─╯
    ○ │  qpvuntsmwlqt true 1
    ├─╯
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");
}

// Make sure `jj split` would refuse to split an empty commit.
#[test]
fn test_split_empty() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["describe", "--message", "abc"]).success();

    let output = work_dir.run_jj(["split"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Refusing to split empty commit 64eaeeb3e846248efc8b599a2b583b708104fc01.
    Hint: Use `jj new` if you want to create another empty commit.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_split_message_editor_avoids_unc() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo");
    work_dir.write_file("file2", "foo");

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["split", "file2"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
}

#[test]
fn test_split_interactive() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    let diff_editor = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();

    let diff_script = ["rm file2", "dump JJ-INSTRUCTIONS instrs"].join("\0");
    std::fs::write(diff_editor, diff_script).unwrap();

    // Split the working commit interactively and select only file1
    let output = work_dir.run_jj(["split"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Selected changes : qpvuntsm c664a51b (no description set)
    Remaining changes: rlvkpnrz 7e5d65b1 (no description set)
    Working copy  (@) now at: rlvkpnrz 7e5d65b1 (no description set)
    Parent commit (@-)      : qpvuntsm c664a51b (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are splitting a commit into two: qpvuntsm f5700f8e (no description set)

    The diff initially shows the changes in the commit you're splitting.

    Adjust the right side until it shows the contents you want to split into the
    new commit.
    The changes that are not selected will replace the original commit.
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.


    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 7e5d65b1
    │  (no description set)
    │  A file2
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 c664a51b
    │  (no description set)
    │  A file1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_split_interactive_with_paths() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    let diff_editor = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file2", "");
    work_dir.write_file("file3", "");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.write_file("file3", "baz\n");

    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();
    // On the before side, file2 is empty. On the after side, it contains "bar".
    // The "reset file2" copies the empty version from the before side to the
    // after side, effectively "unselecting" the changes and leaving only the
    // changes made to file1. file3 doesn't appear on either side since it isn't
    // in the filesets passed to `jj split`.
    let diff_script = [
        "files-before file2",
        "files-after JJ-INSTRUCTIONS file1 file2",
        "reset file2",
    ]
    .join("\0");
    std::fs::write(diff_editor, diff_script).unwrap();

    // Select file1 and file2 by args, then select file1 interactively via the diff
    // script.
    let output = work_dir.run_jj(["split", "-i", "file1", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Selected changes : rlvkpnrz cdc9960a (no description set)
    Remaining changes: kkmpptxz 7255f070 (no description set)
    Working copy  (@) now at: kkmpptxz 7255f070 (no description set)
    Parent commit (@-)      : rlvkpnrz cdc9960a (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.


    JJ: Change ID: rlvkpnrz
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 7255f070
    │  (no description set)
    │  M file2
    │  M file3
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 cdc9960a
    │  (no description set)
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 ff687a2f
    │  (no description set)
    │  A file2
    │  A file3
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

// When a commit is split, the second commit produced by the split becomes the
// working copy commit for all workspaces whose working copy commit was the
// target of the split. This test does a split where the target commit is the
// working copy commit for two different workspaces.
#[test]
fn test_split_with_multiple_workspaces_same_working_copy() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");
    let secondary_dir = test_env.work_dir("secondary");

    main_dir.run_jj(["desc", "-m", "first-commit"]).success();
    main_dir.write_file("file1", "foo");
    main_dir.write_file("file2", "foo");

    // Create the second workspace and change its working copy commit to match
    // the default workspace.
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    // Change the working copy in the second workspace.
    secondary_dir
        .run_jj(["edit", "-r", "description(first-commit)"])
        .success();
    // Check the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  qpvuntsmwlqt default@ second@ first-commit
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    let setup_opid = main_dir.current_operation_id();

    // Do the split in the default workspace.
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2"]).success();
    // The working copy for both workspaces will be the second split commit.
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  royxmykxtrkr default@ second@ second-commit
    ○  qpvuntsmwlqt first-commit
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Test again with a --parallel split.
    main_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2", "--parallel"]).success();
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  yostqsxwqrlt default@ second@ second-commit
    │ ○  qpvuntsmwlqt first-commit
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

// A workspace should only have its working copy commit updated if the target
// commit is the working copy commit.
#[test]
fn test_split_with_multiple_workspaces_different_working_copy() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.run_jj(["desc", "-m", "first-commit"]).success();
    main_dir.write_file("file1", "foo");
    main_dir.write_file("file2", "foo");

    // Create the second workspace with a different working copy commit.
    main_dir
        .run_jj(["workspace", "add", "--name", "second", "../secondary"])
        .success();
    // Check the working-copy commit in each workspace in the log output. The "@"
    // node in the graph indicates the current workspace's working-copy commit.
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  qpvuntsmwlqt default@ first-commit
    │ ○  pmmvwywvzvvn second@
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    let setup_opid = main_dir.current_operation_id();

    // Do the split in the default workspace.
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2"]).success();
    // Only the working copy commit for the default workspace changes.
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  mzvwutvlkqwt default@ second-commit
    ○  qpvuntsmwlqt first-commit
    │ ○  pmmvwywvzvvn second@
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // Test again with a --parallel split.
    main_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2", "--parallel"]).success();
    insta::assert_snapshot!(get_workspace_log_output(&main_dir), @r"
    @  vruxwmqvtpmx default@ second-commit
    │ ○  qpvuntsmwlqt first-commit
    ├─╯
    │ ○  pmmvwywvzvvn second@
    ├─╯
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

#[test]
fn test_split_with_non_empty_description_and_trailers() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(r#"ui.default-description = "\n\nTESTED=TODO""#);
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.run_jj(["describe", "-m", "test"]).success();
    std::fs::write(
        edit_script,
        [
            "dump editor1",
            "write\npart 1",
            "next invocation\n",
            "dump editor2",
            "write\npart 2",
        ]
        .join("\0"),
    )
    .unwrap();

    test_env.add_config(
        r#"[templates]
        commit_trailers = '''"Signed-off-by: " ++ committer.email()'''"#,
    );
    let output = work_dir.run_jj(["split", "file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    Selected changes : qpvuntsm c7f7b14b part 1
    Remaining changes: kkmpptxz ac33a5a9 part 2
    Working copy  (@) now at: kkmpptxz ac33a5a9 part 2
    Parent commit (@-)      : qpvuntsm c7f7b14b part 1
    [EOF]
    "#);

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the selected changes.
    test

    Signed-off-by: test.user@example.com

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the remaining changes.
    test

    Signed-off-by: test.user@example.com

    JJ: Change ID: kkmpptxz
    JJ: This commit contains the following changes:
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  kkmpptxzrspx false part 2
    ○  qpvuntsmwlqt false part 1
    ◆  zzzzzzzzzzzz true
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
}

#[test]
fn test_split_with_message() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.run_jj(["describe", "-m", "my feature"]).success();
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["split", "-m", "fix in file1", "file1"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Selected changes : qpvuntsm f2a70519 fix in file1
    Remaining changes: kkmpptxz cac11766 my feature
    Working copy  (@) now at: kkmpptxz cac11766 my feature
    Parent commit (@-)      : qpvuntsm f2a70519 fix in file1
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  kkmpptxzrspx false my feature
    ○  qpvuntsmwlqt false fix in file1
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");

    // trailers should be added to the message
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "--config",
        r#"templates.commit_trailers='"CC: " ++ committer.email()'"#,
        "-m",
        "fix in file1",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Selected changes : qpvuntsm d01cf12d fix in file1
    Remaining changes: royxmykx b1556ed9 my feature
    Working copy  (@) now at: royxmykx b1556ed9 my feature
    Parent commit (@-)      : qpvuntsm d01cf12d fix in file1
    [EOF]
    ");

    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  royxmykxtrkr false my feature
    ○  qpvuntsmwlqt false fix in file1
    │
    │  CC: test.user@example.com
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");
}

#[test]
fn test_split_move_first_commit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.run_jj(["commit", "-m", "file2"]).success();
    work_dir.write_file("file3", "bar\n");
    work_dir.run_jj(["commit", "-m", "file3"]).success();
    work_dir.write_file("file4", "bar\n");
    work_dir.run_jj(["commit", "-m", "file4"]).success();
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file5", "bar\n");
    work_dir.run_jj(["commit", "-m", "file5"]).success();

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  qpvuntsmwlqt file2
    ├─╯  A file1
    │    A file2
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // insert the commit before the source commit
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--insert-before",
        "qpvuntsmwlqt",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : vruxwmqv bf94c29a file1
    Remaining changes: qpvuntsm 66b1d4f1 file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  qpvuntsmwlqt file2
    │ │  A file2
    │ ○  vruxwmqvtpmx file1
    ├─╯  A file1
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // insert the commit after the source commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--insert-after",
        "qpvuntsmwlqt",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : kpqxywon 08294e90 file1
    Remaining changes: qpvuntsm 76ebcbb8 file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  kpqxywonksrl file1
    │ │  A file1
    │ ○  qpvuntsmwlqt file2
    ├─╯  A file2
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // create a new branch anywhere in the tree
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--onto",
        "rlvkpnrzqnoo",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : lylxulpl b42b2604 file1
    Remaining changes: qpvuntsm 0f76cbf0 file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ │ ○  lylxulplsnyw file1
    │ ├─╯  A file1
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  qpvuntsmwlqt file2
    ├─╯  A file2
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // create a bubble in the tree
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--insert-after",
        "qpvuntsmwlqt",
        "--insert-before",
        "kkmpptxzrspx",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 2 descendant commits
    Selected changes : uyznsvlq d0338445 file1
    Remaining changes: qpvuntsm 16d41320 file2
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○    kkmpptxzrspx file4
    │ ├─╮  A file4
    │ │ ○  uyznsvlquzzm file1
    │ │ │  A file1
    │ ○ │  rlvkpnrzqnoo file3
    │ ├─╯  A file3
    │ ○  qpvuntsmwlqt file2
    ├─╯  A file2
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // create a commit in another branch
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--before",
        "@",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits
    Selected changes : nmzmmopx 72225233 file1
    Remaining changes: qpvuntsm 98b70782 file2
    Working copy  (@) now at: royxmykx c3dd10b0 (empty) (no description set)
    Parent commit (@-)      : nmzmmopx 72225233 file1
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○  nmzmmopxokps file1
    │  A file1
    ○  mzvwutvlkqwt file5
    │  A file5
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  qpvuntsmwlqt file2
    ├─╯  A file2
    ◆  zzzzzzzzzzzz
    [EOF]
    ");

    // merge two branches with the new commit
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj([
        "split",
        "-m",
        "file1",
        "-r",
        "qpvuntsmwlqt",
        "--after",
        "mzvwutvlkqwt",
        "--after",
        "kkmpptxzrspx",
        "file1",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 3 descendant commits
    Selected changes : nlrtlrxv 1b6975b0 file1
    Remaining changes: qpvuntsm 905586dd file2
    Working copy  (@) now at: royxmykx 85be9860 (empty) (no description set)
    Parent commit (@-)      : nlrtlrxv 1b6975b0 file1
    Added 4 files, modified 0 files, removed 0 files
    [EOF]
    ");

    insta::assert_snapshot!(get_log_with_summary(&work_dir), @r"
    @  royxmykxtrkr
    ○    nlrtlrxvuusk file1
    ├─╮  A file1
    │ ○  kkmpptxzrspx file4
    │ │  A file4
    │ ○  rlvkpnrzqnoo file3
    │ │  A file3
    │ ○  qpvuntsmwlqt file2
    │ │  A file2
    ○ │  mzvwutvlkqwt file5
    ├─╯  A file5
    ◆  zzzzzzzzzzzz
    [EOF]
    ");
}

enum BookmarkBehavior {
    Default,
    MoveBookmarkToChild,
    LeaveBookmarkWithTarget,
}

// TODO: https://github.com/jj-vcs/jj/issues/3419 - Delete params when the config is removed.
#[test_case(BookmarkBehavior::Default; "default_behavior")]
#[test_case(BookmarkBehavior::MoveBookmarkToChild; "move_bookmark_to_child")]
#[test_case(BookmarkBehavior::LeaveBookmarkWithTarget; "leave_bookmark_with_target")]
fn test_split_with_bookmarks(bookmark_behavior: BookmarkBehavior) {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    match bookmark_behavior {
        BookmarkBehavior::LeaveBookmarkWithTarget => {
            test_env.add_config("split.legacy-bookmark-behavior=false");
        }
        BookmarkBehavior::MoveBookmarkToChild => {
            test_env.add_config("split.legacy-bookmark-behavior=true");
        }
        BookmarkBehavior::Default => (),
    }

    // Setup.
    main_dir.run_jj(["desc", "-m", "first-commit"]).success();
    main_dir.write_file("file1", "foo");
    main_dir.write_file("file2", "foo");
    main_dir
        .run_jj(["bookmark", "set", "'*le-signet*'", "-r", "@"])
        .success();
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output(&main_dir), @r#"
    @  qpvuntsmwlqt false "*le-signet*" first-commit
    ◆  zzzzzzzzzzzz true
    [EOF]
    "#);
    }
    let setup_opid = main_dir.current_operation_id();

    // Do the split.
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    let output = main_dir.run_jj(["split", "file2"]);
    match bookmark_behavior {
        BookmarkBehavior::LeaveBookmarkWithTarget => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(output, @r#"
            ------- stderr -------
            Selected changes : qpvuntsm a481fe8a "*le-signet*" | first-commit
            Remaining changes: mzvwutvl 5f597a6e second-commit
            Working copy  (@) now at: mzvwutvl 5f597a6e second-commit
            Parent commit (@-)      : qpvuntsm a481fe8a "*le-signet*" | first-commit
            [EOF]
            "#);
            }
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r#"
            @  mzvwutvlkqwt false second-commit
            ○  qpvuntsmwlqt false "*le-signet*" first-commit
            ◆  zzzzzzzzzzzz true
            [EOF]
            "#);
            }
        }
        BookmarkBehavior::Default | BookmarkBehavior::MoveBookmarkToChild => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(output, @r#"
            ------- stderr -------
            Selected changes : qpvuntsm a481fe8a first-commit
            Remaining changes: mzvwutvl 5f597a6e "*le-signet*" | second-commit
            Working copy  (@) now at: mzvwutvl 5f597a6e "*le-signet*" | second-commit
            Parent commit (@-)      : qpvuntsm a481fe8a first-commit
            [EOF]
            "#);
            }
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r#"
            @  mzvwutvlkqwt false "*le-signet*" second-commit
            ○  qpvuntsmwlqt false first-commit
            ◆  zzzzzzzzzzzz true
            [EOF]
            "#);
            }
        }
    }

    // Test again with a --parallel split.
    main_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2", "--parallel"]).success();
    match bookmark_behavior {
        BookmarkBehavior::LeaveBookmarkWithTarget => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r#"
            @  vruxwmqvtpmx false second-commit
            │ ○  qpvuntsmwlqt false "*le-signet*" first-commit
            ├─╯
            ◆  zzzzzzzzzzzz true
            [EOF]
            "#);
            }
        }
        BookmarkBehavior::Default | BookmarkBehavior::MoveBookmarkToChild => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r#"
            @  vruxwmqvtpmx false "*le-signet*" second-commit
            │ ○  qpvuntsmwlqt false first-commit
            ├─╯
            ◆  zzzzzzzzzzzz true
            [EOF]
            "#);
            }
        }
    }
}
