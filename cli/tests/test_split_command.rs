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
    First part: qpvuntsm 65569ca7 (no description set)
    Second part: zsuskuln 709756f0 (no description set)
    Working copy  (@) now at: zsuskuln 709756f0 (no description set)
    Parent commit (@-)      : qpvuntsm 65569ca7 (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r#"
    JJ: Enter a description for the first commit.


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
    Warning: All changes have been selected, so the second commit will be empty
    Rebased 1 descendant commits
    First part: qpvuntsm 9da0eea0 (no description set)
    Second part: znkkpsqq 5b5714a3 (empty) (no description set)
    Working copy  (@) now at: zsuskuln 0c798ee7 (no description set)
    Parent commit (@-)      : znkkpsqq 5b5714a3 (empty) (no description set)
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
    Warning: No changes have been selected, so the first commit will be empty
    Rebased 1 descendant commits
    First part: qpvuntsm bd42f95a (empty) (no description set)
    Second part: lylxulpl ed55c86b (no description set)
    Working copy  (@) now at: zsuskuln 1e1ed741 (no description set)
    Parent commit (@-)      : lylxulpl ed55c86b (no description set)
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    First part: qpvuntsm 231a3c00 part 1
    Second part: kkmpptxz e96291aa part 2
    Working copy  (@) now at: kkmpptxz e96291aa part 2
    Parent commit (@-)      : qpvuntsm 231a3c00 part 1
    [EOF]
    "#);

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the first commit.
    test

    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the second commit.
    test

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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    First part: qpvuntsm 02ee5d60 TESTED=TODO
    Second part: rlvkpnrz 33cd046b (no description set)
    Working copy  (@) now at: rlvkpnrz 33cd046b (no description set)
    Parent commit (@-)      : qpvuntsm 02ee5d60 TESTED=TODO
    [EOF]
    "#);

    // Since the commit being split has no description, the user will only be
    // prompted to add a description to the first commit, which will use the
    // default value we set. The second commit will inherit the empty
    // description from the commit being split.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the first commit.


    TESTED=TODO

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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
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
    First part: qpvuntsm 34dd141b Add file1
    Second part: royxmykx 465e03d0 Add file2
    Working copy  (@) now at: kkmpptxz 2d5d641f Add file4
    Parent commit (@-)      : rlvkpnrz b3bd9eb7 Add file3
    [EOF]
    ");
    insta::assert_snapshot!(get_log_output(&work_dir), @r###"
    @  kkmpptxzrspx false Add file4
    ○  rlvkpnrzqnoo false Add file3
    ○  royxmykxtrkr false Add file2
    ○  qpvuntsmwlqt false Add file1
    ◆  zzzzzzzzzzzz true
    [EOF]
    "###);

    // The commit we're splitting has a description, so the user will be
    // prompted to enter a description for each of the commits.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the first commit.
    Add file1 & file2

    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the second commit.
    Add file1 & file2

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
    insta::assert_snapshot!(evolog_1, @r###"
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:12 34dd141b
    │  Add file1
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 764d46f1
    │  Add file1 & file2
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 44af2155
    │  (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    "###);

    // The evolog for the second commit is the same, except that the change id
    // changes after the split.
    let evolog_2 = work_dir.run_jj(["evolog", "-r", "royxm"]);
    insta::assert_snapshot!(evolog_2, @r###"
    ○  royxmykx test.user@example.com 2001-02-03 08:05:12 465e03d0
    │  Add file2
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 764d46f1
    │  Add file1 & file2
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 44af2155
    │  (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    "###);
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
    First part: kkmpptxz e8006b47 Add file1
    Second part: royxmykx 5e1b793d Add file2
    Working copy  (@) now at: zsuskuln 696935af (empty) 2
    Parent commit (@-)      : qpvuntsm 8b64ddff (empty) 1
    Parent commit (@-)      : royxmykx 5e1b793d Add file2
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    First part: qpvuntsm 48018df6 TESTED=TODO
    Second part: kkmpptxz 7eddbf93 (no description set)
    Working copy  (@) now at: kkmpptxz 7eddbf93 (no description set)
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);

    // Since the commit being split has no description, the user will only be
    // prompted to add a description to the first commit, which will use the
    // default value we set. The second commit will inherit the empty
    // description from the commit being split.
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the first commit.


    TESTED=TODO

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
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:09 48018df6
    │  TESTED=TODO
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 44af2155
    │  (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    ------- stderr -------
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);

    // The evolog for the second commit is the same, except that the change id
    // changes after the split.
    let evolog_2 = work_dir.run_jj(["evolog", "-r", "kkmpp"]);
    insta::assert_snapshot!(evolog_2, @r#"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 7eddbf93
    │  (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:08 44af2155
    │  (no description set)
    ○  qpvuntsm hidden test.user@example.com 2001-02-03 08:05:07 230dd059
       (empty) (no description set)
    [EOF]
    ------- stderr -------
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
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
    First part: qpvuntsm 84df941d Add file1
    Second part: vruxwmqv 94753be3 Add file2
    Working copy  (@) now at: vruxwmqv 94753be3 Add file2
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
    JJ: Enter a description for the first commit.
    Add file1 & file2

    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the second commit.
    Add file1 & file2

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
    First part: kkmpptxz e8006b47 Add file1
    Second part: royxmykx 2cc60f3d Add file2
    Working copy  (@) now at: zsuskuln 35b5d7eb (empty) 2
    Parent commit (@-)      : qpvuntsm 8b64ddff (empty) 1
    Parent commit (@-)      : kkmpptxz e8006b47 Add file1
    Parent commit (@-)      : royxmykx 2cc60f3d Add file2
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
    Error: Refusing to split empty commit 2ab033062e9fdf7fad2ded8e89c1f145e3698190.
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
    First part: qpvuntsm 0e15949e (no description set)
    Second part: rlvkpnrz 9ed12e4c (no description set)
    Working copy  (@) now at: rlvkpnrz 9ed12e4c (no description set)
    Parent commit (@-)      : qpvuntsm 0e15949e (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are splitting a commit into two: qpvuntsm 44af2155 (no description set)

    The diff initially shows the changes in the commit you're splitting.

    Adjust the right side until it shows the contents you want for the first commit.
    The remainder will be in the second commit.
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    JJ: Enter a description for the first commit.


    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  rlvkpnrz test.user@example.com 2001-02-03 08:05:08 9ed12e4c
    │  (no description set)
    │  A file2
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 0e15949e
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
    First part: rlvkpnrz e3d766b8 (no description set)
    Second part: kkmpptxz 4cf22d3b (no description set)
    Working copy  (@) now at: kkmpptxz 4cf22d3b (no description set)
    Parent commit (@-)      : rlvkpnrz e3d766b8 (no description set)
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    JJ: Enter a description for the first commit.


    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 4cf22d3b
    │  (no description set)
    │  M file2
    │  M file3
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 e3d766b8
    │  (no description set)
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 497ed465
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
    main_dir.run_jj(["undo"]).success();
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
    main_dir.run_jj(["undo"]).success();
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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    First part: qpvuntsm 231a3c00 part 1
    Second part: kkmpptxz e96291aa part 2
    Working copy  (@) now at: kkmpptxz e96291aa part 2
    Parent commit (@-)      : qpvuntsm 231a3c00 part 1
    [EOF]
    "#);

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    JJ: Enter a description for the first commit.
    test

    Signed-off-by: test.user@example.com

    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r#"
    JJ: Enter a description for the second commit.
    test

    Signed-off-by: test.user@example.com

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
    Warning: Deprecated config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
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
    insta::assert_snapshot!(get_log_output(&main_dir), @r"
    @  qpvuntsmwlqt false *le-signet* first-commit
    ◆  zzzzzzzzzzzz true
    [EOF]
    ");
    }

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
            insta::assert_snapshot!(output, @r"
            ------- stderr -------
            First part: qpvuntsm 63d0c5ed *le-signet* | first-commit
            Second part: mzvwutvl a9f5665f second-commit
            Working copy  (@) now at: mzvwutvl a9f5665f second-commit
            Parent commit (@-)      : qpvuntsm 63d0c5ed *le-signet* | first-commit
            [EOF]
            ");
            }
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r"
            @  mzvwutvlkqwt false second-commit
            ○  qpvuntsmwlqt false *le-signet* first-commit
            ◆  zzzzzzzzzzzz true
            [EOF]
            ");
            }
        }
        BookmarkBehavior::Default | BookmarkBehavior::MoveBookmarkToChild => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(output, @r"
            ------- stderr -------
            First part: qpvuntsm 63d0c5ed first-commit
            Second part: mzvwutvl a9f5665f *le-signet* | second-commit
            Working copy  (@) now at: mzvwutvl a9f5665f *le-signet* | second-commit
            Parent commit (@-)      : qpvuntsm 63d0c5ed first-commit
            [EOF]
            ");
            }
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r"
            @  mzvwutvlkqwt false *le-signet* second-commit
            ○  qpvuntsmwlqt false first-commit
            ◆  zzzzzzzzzzzz true
            [EOF]
            ");
            }
        }
    }

    // Test again with a --parallel split.
    main_dir.run_jj(["undo"]).success();
    std::fs::write(
        &edit_script,
        ["", "next invocation\n", "write\nsecond-commit"].join("\0"),
    )
    .unwrap();
    main_dir.run_jj(["split", "file2", "--parallel"]).success();
    match bookmark_behavior {
        BookmarkBehavior::LeaveBookmarkWithTarget => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r"
            @  vruxwmqvtpmx false second-commit
            │ ○  qpvuntsmwlqt false *le-signet* first-commit
            ├─╯
            ◆  zzzzzzzzzzzz true
            [EOF]
            ");
            }
        }
        BookmarkBehavior::Default | BookmarkBehavior::MoveBookmarkToChild => {
            insta::allow_duplicates! {
            insta::assert_snapshot!(get_log_output(&main_dir), @r"
            @  vruxwmqvtpmx false *le-signet* second-commit
            │ ○  qpvuntsmwlqt false first-commit
            ├─╯
            ◆  zzzzzzzzzzzz true
            [EOF]
            ");
            }
        }
    }
}
