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

use bstr::ByteSlice as _;
use indoc::indoc;

use crate::common::TestEnvironment;

#[test]
fn test_diffedit() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2", "a\n");
    work_dir.write_file("file3", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // Test the setup; nothing happens if we make no changes
    std::fs::write(
        &edit_script,
        [
            "files-before file1 file2",
            "files-after JJ-INSTRUCTIONS file2",
            "dump JJ-INSTRUCTIONS instrs",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are editing changes in: kkmpptxz e4245972 (no description set)

    The diff initially shows the commit's changes.

    Adjust the right side until it shows the contents you want. If you
    don't make any changes, then the operation will be aborted.
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Try again with ui.diff-instructions=false
    std::fs::write(&edit_script, "files-before file1 file2\0files-after file2").unwrap();
    let output = work_dir.run_jj(["diffedit", "--config=ui.diff-instructions=false"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Try again with --tool=<name>
    std::fs::write(
        &edit_script,
        "files-before file1 file2\0files-after JJ-INSTRUCTIONS file2",
    )
    .unwrap();
    let output = work_dir.run_jj([
        "diffedit",
        "--config=ui.diff-editor='false'",
        "--tool=fake-diff-editor",
    ]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Nothing happens if the diff-editor exits with an error
    std::fs::write(&edit_script, "rm file2\0fail").unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output.normalize_stderr_exit_status(), @r"
    ------- stderr -------
    Error: Failed to edit diff
    Caused by: Tool exited with exit status: 1 (run with --debug to see the exact invocation)
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Can edit changes to individual files
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 40ad4f80 (no description set)
    Parent commit (@-)      : rlvkpnrz 7e268da3 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Changes to a commit are propagated to descendants
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write file3\nmodified\n").unwrap();
    let output = work_dir.run_jj(["diffedit", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 9f0ebae1 (no description set)
    Parent commit (@-)      : rlvkpnrz 72bcd8e9 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let contents = work_dir.read_file("file3");
    insta::assert_snapshot!(contents, @"modified");

    // Test diffedit --from @--
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(
        &edit_script,
        "files-before file1\0files-after JJ-INSTRUCTIONS file2 file3\0reset file2",
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit", "--from", "@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 215fca5f (no description set)
    Parent commit (@-)      : rlvkpnrz 7e268da3 (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    D file2
    [EOF]
    ");

    // Test with path restriction
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir.write_file("file3", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "modified\n");
    work_dir.write_file("file2", "modified\n");
    work_dir.write_file("file3", "modified\n");

    // Edit only file2 with path argument
    std::fs::write(
        &edit_script,
        "files-before file2\0files-after JJ-INSTRUCTIONS file2\0reset file2",
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: tlkvzzqu 06bdff15 (no description set)
    Parent commit (@-)      : kkmpptxz e4245972 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    C {file3 => file1}
    M file3
    [EOF]
    ");

    // Test reverse-order diffedit --to @-
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(
        &edit_script,
        [
            "files-before file2",
            "files-after JJ-INSTRUCTIONS file1 file2",
            "reset file2",
            "dump JJ-INSTRUCTIONS instrs",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit", "--to", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: kkmpptxz 9a4e9bcc (no description set)
    Parent commit (@-)      : rlvkpnrz fb5c77f4 (no description set)
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are editing changes in: rlvkpnrz 7e268da3 (no description set)

    The diff initially shows the commit's changes relative to:
    kkmpptxz e4245972 (no description set)

    Adjust the right side until it shows the contents you want. If you
    don't make any changes, then the operation will be aborted.
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");
}

#[test]
fn test_diffedit_new_file() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // Test the setup; nothing happens if we make no changes
    std::fs::write(
        &edit_script,
        "files-before file1\0files-after JJ-INSTRUCTIONS file2",
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file2
    [EOF]
    ");

    // Creating `file1` on the right side is noticed by `jj diffedit`
    std::fs::write(&edit_script, "write file1\nmodified\n").unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz c26dcad1 (no description set)
    Parent commit (@-)      : qpvuntsm eb7b8a1f (no description set)
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    M file1
    A file2
    [EOF]
    ");

    // Creating a file that wasn't on either side is ignored by diffedit.
    // TODO(ilyagr) We should decide whether we like this behavior.
    //
    // On one hand, it is unexpected and potentially a minor BUG. On the other
    // hand, this prevents `jj` from loading any backup files the merge tool
    // generates.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write new_file\nnew file\n").unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file2
    [EOF]
    ");
}

#[test]
fn test_diffedit_existing_instructions() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // A diff containing an existing JJ-INSTRUCTIONS file themselves.
    work_dir.write_file("JJ-INSTRUCTIONS", "instruct");

    std::fs::write(&edit_script, "write JJ-INSTRUCTIONS\nmodified\n").unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: qpvuntsm e914aaad (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    // Test that we didn't delete or overwrite the "JJ-INSTRUCTIONS" file.
    let content = work_dir.read_file("JJ-INSTRUCTIONS");
    insta::assert_snapshot!(content, @"modified");
}

#[test]
fn test_diffedit_external_tool_conflict_marker_style() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let file_path = "file";

    // Create a conflict
    work_dir.write_file(
        file_path,
        indoc! {"
            line 1
            line 2
            line 3
            line 4
            line 5
        "},
    );
    work_dir.run_jj(["commit", "-m", "base"]).success();
    work_dir.write_file(
        file_path,
        indoc! {"
            line 1
            line 2.1
            line 2.2
            line 3
            line 4.1
            line 5
        "},
    );
    work_dir.run_jj(["describe", "-m", "side-a"]).success();
    work_dir
        .run_jj(["new", "subject(base)", "-m", "side-b"])
        .success();
    work_dir.write_file(
        file_path,
        indoc! {"
            line 1
            line 2.3
            line 3
            line 4.2
            line 4.3
            line 5
        "},
    );

    // Resolve one of the conflicts in the working copy
    work_dir
        .run_jj(["new", "subject(side-a)", "subject(side-b)"])
        .success();
    work_dir.write_file(
        file_path,
        indoc! {"
            line 1
            line 2.1
            line 2.2
            line 2.3
            line 3
            <<<<<<<
            %%%%%%%
            -line 4
            +line 4.1
            +++++++
            line 4.2
            line 4.3
            >>>>>>>
            line 5
        "},
    );

    // Set up diff editor to use "snapshot" conflict markers
    test_env.add_config(r#"merge-tools.fake-diff-editor.conflict-marker-style = "snapshot""#);

    // We want to see whether the diff editor is using the correct conflict markers,
    // and reset it to make sure that it parses the conflict markers as well
    std::fs::write(
        &edit_script,
        [
            "files-before file",
            "files-after JJ-INSTRUCTIONS file",
            "dump file after-file",
            "reset file",
            "dump file before-file",
        ]
        .join("\0"),
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl b9458f20 (conflict) (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 74e448a1 side-a
    Parent commit (@-)      : zsuskuln 6982bce7 side-b
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    Existing conflicts were resolved or abandoned from 1 commits.
    [EOF]
    ");
    // Conflicts should render using "snapshot" format in diff editor
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("before-file")).unwrap(), @r"
    line 1
    <<<<<<< conflict 1 of 2
    +++++++ side #1
    line 2.1
    line 2.2
    ------- base
    line 2
    +++++++ side #2
    line 2.3
    >>>>>>> conflict 1 of 2 ends
    line 3
    <<<<<<< conflict 2 of 2
    +++++++ side #1
    line 4.1
    ------- base
    line 4
    +++++++ side #2
    line 4.2
    line 4.3
    >>>>>>> conflict 2 of 2 ends
    line 5
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("after-file")).unwrap(), @r"
    line 1
    line 2.1
    line 2.2
    line 2.3
    line 3
    <<<<<<< conflict 1 of 1
    +++++++ side #1
    line 4.1
    ------- base
    line 4
    +++++++ side #2
    line 4.2
    line 4.3
    >>>>>>> conflict 1 of 1 ends
    line 5
    ");
    // Conflicts should be materialized using "diff" format in working copy
    insta::assert_snapshot!(work_dir.read_file(file_path), @r"
    line 1
    <<<<<<< conflict 1 of 2
    +++++++ side #1
    line 2.1
    line 2.2
    %%%%%%% diff from base to side #2
    -line 2
    +line 2.3
    >>>>>>> conflict 1 of 2 ends
    line 3
    <<<<<<< conflict 2 of 2
    %%%%%%% diff from base to side #1
    -line 4
    +line 4.1
    +++++++ side #2
    line 4.2
    line 4.3
    >>>>>>> conflict 2 of 2 ends
    line 5
    ");

    // File should be conflicted with no changes
    let output = work_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy  (@) : mzvwutvl b9458f20 (conflict) (empty) (no description set)
    Parent commit (@-): rlvkpnrz 74e448a1 side-a
    Parent commit (@-): zsuskuln 6982bce7 side-b
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    [EOF]
    ");
}

#[test]
fn test_diffedit_3pane() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file2", "a\n");
    work_dir.write_file("file3", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // 2 configs for a 3-pane setup. In the first, "$right" is passed to what the
    // fake diff editor considers the "after" state.
    let config_with_right_as_after =
        "merge-tools.fake-diff-editor.edit-args=['$left', '$right', '--ignore=$output']";
    let config_with_output_as_after =
        "merge-tools.fake-diff-editor.edit-args=['$left', '$output', '--ignore=$right']";
    std::fs::write(&edit_script, "").unwrap();

    // Nothing happens if we make no changes
    std::fs::write(
        &edit_script,
        "files-before file1 file2\0files-after JJ-INSTRUCTIONS file2",
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit", "--config", config_with_output_as_after]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");
    // Nothing happens if we make no changes, `config_with_right_as_after` version
    let output = work_dir.run_jj(["diffedit", "--config", config_with_right_as_after]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Can edit changes to individual files
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = work_dir.run_jj(["diffedit", "--config", config_with_output_as_after]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 239413bd (no description set)
    Parent commit (@-)      : rlvkpnrz 7e268da3 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Can write something new to `file1`
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write file1\nnew content").unwrap();
    let output = work_dir.run_jj(["diffedit", "--config", config_with_output_as_after]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 95873a91 (no description set)
    Parent commit (@-)      : rlvkpnrz 7e268da3 (no description set)
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    M file1
    M file2
    [EOF]
    ");

    // But nothing happens if we modify the right side
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write file1\nnew content").unwrap();
    let output = work_dir.run_jj(["diffedit", "--config", config_with_right_as_after]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // TODO: test with edit_script of "reset file2". This fails on right side
    // since the file is readonly.
}

#[test]
fn test_diffedit_merge() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();
    work_dir.write_file("file1", "b\n");
    work_dir.write_file("file2", "b\n");
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "c\n");
    work_dir.write_file("file2", "c\n");
    work_dir.run_jj(["new", "@", "b", "-m", "merge"]).success();
    // Resolve the conflict in file1, but leave the conflict in file2
    work_dir.write_file("file1", "d\n");
    work_dir.write_file("file3", "d\n");
    work_dir.run_jj(["new"]).success();
    // Test the setup
    let output = work_dir.run_jj(["diff", "-r", "@-", "-s"]);
    insta::assert_snapshot!(output, @r"
    M file1
    A file3
    [EOF]
    ");

    // Remove file1. The conflict remains in the working copy on top of the merge.
    std::fs::write(
        edit_script,
        "files-before file1\0files-after JJ-INSTRUCTIONS file1 file3\0rm file1",
    )
    .unwrap();
    let output = work_dir.run_jj(["diffedit", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits
    Working copy  (@) now at: yqosqzyt 5e33630e (conflict) (empty) (no description set)
    Parent commit (@-)      : royxmykx 501edbda (conflict) merge
    Added 0 files, modified 0 files, removed 1 files
    Warning: There are unresolved conflicts at these paths:
    file2    2-sided conflict
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file3
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    let output = work_dir.run_jj(["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from base to side #1
    -a
    +c
    +++++++ side #2
    b
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    ");
}

#[test]
fn test_diffedit_old_restore_interactive_tests() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "a\n");
    work_dir.write_file("file2", "a\n");
    work_dir.run_jj(["new"]).success();
    work_dir.remove_file("file1");
    work_dir.write_file("file2", "b\n");
    work_dir.write_file("file3", "b\n");
    work_dir.run_jj(["debug", "snapshot"]).success();
    let setup_opid = work_dir.current_operation_id();

    // Nothing happens if we make no changes
    let output = work_dir.run_jj(["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    C {file2 => file3}
    [EOF]
    ");

    // Nothing happens if the diff-editor exits with an error
    std::fs::write(&edit_script, "rm file2\0fail").unwrap();
    let output = work_dir.run_jj(["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output.normalize_stderr_exit_status(), @r"
    ------- stderr -------
    Error: Failed to edit diff
    Caused by: Tool exited with exit status: 1 (run with --debug to see the exact invocation)
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    C {file2 => file3}
    [EOF]
    ");

    // Can restore changes to individual files
    std::fs::write(&edit_script, "reset file2\0reset file3").unwrap();
    let output = work_dir.run_jj(["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 83b62f75 (no description set)
    Parent commit (@-)      : qpvuntsm fc6f5e82 (no description set)
    Added 0 files, modified 1 files, removed 1 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Can make unrelated edits
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write file3\nunrelated\n").unwrap();
    let output = work_dir.run_jj(["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 8119c685 (no description set)
    Parent commit (@-)      : qpvuntsm fc6f5e82 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output, @r"
    diff --git a/file1 b/file1
    deleted file mode 100644
    index 7898192261..0000000000
    --- a/file1
    +++ /dev/null
    @@ -1,1 +0,0 @@
    -a
    diff --git a/file2 b/file2
    index 7898192261..6178079822 100644
    --- a/file2
    +++ b/file2
    @@ -1,1 +1,1 @@
    -a
    +b
    diff --git a/file3 b/file3
    new file mode 100644
    index 0000000000..c21c9352f7
    --- /dev/null
    +++ b/file3
    @@ -0,0 +1,1 @@
    +unrelated
    [EOF]
    ");
}

#[test]
fn test_diffedit_restore_descendants() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file", "println!(\"foo\")\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "println!(\"bar\")\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file", "println!(\"baz\");\n");

    // Add a ";" after the line with "bar". There should be no conflict.
    std::fs::write(edit_script, "write file\nprintln!(\"bar\");\n").unwrap();
    let output = work_dir.run_jj(["diffedit", "-r", "@-", "--restore-descendants"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content)
    Working copy  (@) now at: kkmpptxz a35ef1a5 (no description set)
    Parent commit (@-)      : rlvkpnrz 2e949a84 (no description set)
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output, @r#"
    diff --git a/file b/file
    index 1a598a8fc9..7b6a85ab5a 100644
    --- a/file
    +++ b/file
    @@ -1,1 +1,1 @@
    -println!("bar");
    +println!("baz");
    [EOF]
    "#);
}

#[test]
fn test_diffedit_external_tool_eol_conversion() {
    // Create 2 changes: one creates a file with a single LF, another changes the
    // file to contain 2 LFs. The diff editor should see the same EOL in both the
    // before file and the after file. And when the diff editor adds another EOL to
    // update, we should always see 3 LFs in the store.

    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let file_path = "file";

    // Use the none eol-conversion setting to check in as is.
    let eol_conversion_none_config = "working-copy.eol-conversion='none'";
    work_dir.write_file(file_path, "\n");
    work_dir
        .run_jj(["commit", "--config", eol_conversion_none_config, "-m", "1"])
        .success();
    work_dir.write_file(file_path, "\n\n");
    work_dir
        .run_jj(["commit", "--config", eol_conversion_none_config, "-m", "2"])
        .success();

    std::fs::write(
        &edit_script,
        [
            "dump file after-file",
            "reset file",
            "dump file before-file",
        ]
        .join("\0"),
    )
    .unwrap();
    let test_eol_conversion_config = "working-copy.eol-conversion='input-output'";
    work_dir
        .run_jj([
            "diffedit",
            "-r",
            "@-",
            "--config",
            test_eol_conversion_config,
        ])
        .success();
    let before_file_contents = std::fs::read(test_env.env_root().join("before-file")).unwrap();
    let before_file_lines = before_file_contents
        .lines_with_terminator()
        .collect::<Vec<_>>();
    let after_file_contents = std::fs::read(test_env.env_root().join("after-file")).unwrap();
    let after_file_lines = after_file_contents
        .lines_with_terminator()
        .collect::<Vec<_>>();
    assert_eq!(before_file_lines[0], after_file_lines[0]);
    fn get_eol(line: &[u8]) -> &'static str {
        if line.ends_with(b"\r\n") {
            "\r\n"
        } else if line.ends_with(b"\n") {
            "\n"
        } else {
            ""
        }
    }
    let first_eol = get_eol(after_file_lines[0]);
    let second_eol = get_eol(after_file_lines[1]);
    assert_eq!(first_eol, second_eol);
    assert_eq!(
        first_eol, "\n",
        "The EOL the external diff editor receives must be LF to align with the builtin diff \
         editor."
    );
    let eol = first_eol;

    // With the previous diffedit command, file now contains the same content as
    // commit 1, i.e., 1 LF. We create another commit 3 with the 2-LF file, so that
    // the file shows up in the next diffedit command.
    work_dir.write_file(file_path, "\n\n");
    work_dir
        .run_jj(["squash", "--config", eol_conversion_none_config, "-m", "2"])
        .success();

    std::fs::write(&edit_script, format!("write file\n{eol}{eol}{eol}")).unwrap();
    work_dir
        .run_jj([
            "diffedit",
            "-r",
            "@-",
            "--config",
            test_eol_conversion_config,
        ])
        .success();

    work_dir
        .run_jj(["new", "root()", "--config", eol_conversion_none_config])
        .success();
    work_dir
        .run_jj(["new", "subject(2)", "--config", eol_conversion_none_config])
        .success();
    let file_content = work_dir.read_file(file_path);
    assert_eq!(file_content, b"\n\n\n");
}
