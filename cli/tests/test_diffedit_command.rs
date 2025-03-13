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

use indoc::indoc;

use crate::common::TestEnvironment;

#[test]
fn test_diffedit() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    std::fs::write(repo_path.join("file3"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();

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
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are editing changes in: kkmpptxz 3d4cce89 (no description set)

    The diff initially shows the commit's changes.

    Adjust the right side until it shows the contents you want. If you
    don't make any changes, then the operation will be aborted.
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Try again with ui.diff-instructions=false
    std::fs::write(&edit_script, "files-before file1 file2\0files-after file2").unwrap();
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config=ui.diff-instructions=false"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
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
    let output = test_env.run_jj_in(
        &repo_path,
        [
            "diffedit",
            "--config=ui.diff-editor='false'",
            "--tool=fake-diff-editor",
        ],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Nothing happens if the diff-editor exits with an error
    std::fs::write(&edit_script, "rm file2\0fail").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output.normalize_stderr_exit_status(), @r"
    ------- stderr -------
    Error: Failed to edit diff
    Caused by: Tool exited with exit status: 1 (run with --debug to see the exact invocation)
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Can edit changes to individual files
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created kkmpptxz cbc7a725 (no description set)
    Working copy now at: kkmpptxz cbc7a725 (no description set)
    Parent commit      : rlvkpnrz a72506cd (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Changes to a commit are propagated to descendants
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(&edit_script, "write file3\nmodified\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created rlvkpnrz d4eef3fc (no description set)
    Rebased 1 descendant commits
    Working copy now at: kkmpptxz 59ef1b95 (no description set)
    Parent commit      : rlvkpnrz d4eef3fc (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let contents = String::from_utf8(std::fs::read(repo_path.join("file3")).unwrap()).unwrap();
    insta::assert_snapshot!(contents, @"modified");

    // Test diffedit --from @--
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(
        &edit_script,
        "files-before file1\0files-after JJ-INSTRUCTIONS file2 file3\0reset file2",
    )
    .unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "--from", "@--"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created kkmpptxz 5b585bd1 (no description set)
    Working copy now at: kkmpptxz 5b585bd1 (no description set)
    Parent commit      : rlvkpnrz a72506cd (no description set)
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    D file2
    [EOF]
    ");
}

#[test]
fn test_diffedit_new_file() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();

    // Test the setup; nothing happens if we make no changes
    std::fs::write(
        &edit_script,
        "files-before file1\0files-after JJ-INSTRUCTIONS file2",
    )
    .unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file2
    [EOF]
    ");

    // Creating `file1` on the right side is noticed by `jj diffedit`
    std::fs::write(&edit_script, "write file1\nmodified\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created rlvkpnrz b0376e2b (no description set)
    Working copy now at: rlvkpnrz b0376e2b (no description set)
    Parent commit      : qpvuntsm b739eb46 (no description set)
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
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
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(&edit_script, "write new_file\nnew file\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file2
    [EOF]
    ");
}

#[test]
fn test_diffedit_external_tool_conflict_marker_style() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");
    let file_path = repo_path.join("file");

    // Create a conflict
    std::fs::write(
        &file_path,
        indoc! {"
        line 1
        line 2
        line 3
        line 4
        line 5
    "},
    )
    .unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m", "base"])
        .success();
    std::fs::write(
        &file_path,
        indoc! {"
        line 1
        line 2.1
        line 2.2
        line 3
        line 4.1
        line 5
    "},
    )
    .unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m", "side-a"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "description(base)", "-m", "side-b"])
        .success();
    std::fs::write(
        &file_path,
        indoc! {"
        line 1
        line 2.3
        line 3
        line 4.2
        line 4.3
        line 5
    "},
    )
    .unwrap();

    // Resolve one of the conflicts in the working copy
    test_env
        .run_jj_in(
            &repo_path,
            ["new", "description(side-a)", "description(side-b)"],
        )
        .success();
    std::fs::write(
        &file_path,
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
    )
    .unwrap();

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
    let output = test_env.run_jj_in(&repo_path, ["diffedit"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created mzvwutvl fb39e804 (conflict) (empty) (no description set)
    Working copy now at: mzvwutvl fb39e804 (conflict) (empty) (no description set)
    Parent commit      : rlvkpnrz 3765cc27 side-a
    Parent commit      : zsuskuln 8b3de837 side-b
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    Existing conflicts were resolved or abandoned from these commits:
      mzvwutvl hidden a813239f (conflict) (no description set)
    [EOF]
    ");
    // Conflicts should render using "snapshot" format in diff editor
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("before-file")).unwrap(), @r"
    line 1
    <<<<<<< Conflict 1 of 2
    +++++++ Contents of side #1
    line 2.1
    line 2.2
    ------- Contents of base
    line 2
    +++++++ Contents of side #2
    line 2.3
    >>>>>>> Conflict 1 of 2 ends
    line 3
    <<<<<<< Conflict 2 of 2
    +++++++ Contents of side #1
    line 4.1
    ------- Contents of base
    line 4
    +++++++ Contents of side #2
    line 4.2
    line 4.3
    >>>>>>> Conflict 2 of 2 ends
    line 5
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("after-file")).unwrap(), @r"
    line 1
    line 2.1
    line 2.2
    line 2.3
    line 3
    <<<<<<< Conflict 1 of 1
    +++++++ Contents of side #1
    line 4.1
    ------- Contents of base
    line 4
    +++++++ Contents of side #2
    line 4.2
    line 4.3
    >>>>>>> Conflict 1 of 1 ends
    line 5
    ");
    // Conflicts should be materialized using "diff" format in working copy
    insta::assert_snapshot!(
        std::fs::read_to_string(&file_path).unwrap(), @r"
    line 1
    <<<<<<< Conflict 1 of 2
    +++++++ Contents of side #1
    line 2.1
    line 2.2
    %%%%%%% Changes from base to side #2
    -line 2
    +line 2.3
    >>>>>>> Conflict 1 of 2 ends
    line 3
    <<<<<<< Conflict 2 of 2
    %%%%%%% Changes from base to side #1
    -line 4
    +line 4.1
    +++++++ Contents of side #2
    line 4.2
    line 4.3
    >>>>>>> Conflict 2 of 2 ends
    line 5
    ");

    // File should be conflicted with no changes
    let output = test_env.run_jj_in(&repo_path, ["st"]);
    insta::assert_snapshot!(output, @r"
    The working copy has no changes.
    Working copy : mzvwutvl fb39e804 (conflict) (empty) (no description set)
    Parent commit: rlvkpnrz 3765cc27 side-a
    Parent commit: zsuskuln 8b3de837 side-b
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
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    std::fs::write(repo_path.join("file3"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();

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
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config", config_with_output_as_after],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");
    // Nothing happens if we make no changes, `config_with_right_as_after` version
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config", config_with_right_as_after],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    [EOF]
    ");

    // Can edit changes to individual files
    std::fs::write(&edit_script, "reset file2").unwrap();
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config", config_with_output_as_after],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created kkmpptxz ed8aada3 (no description set)
    Working copy now at: kkmpptxz ed8aada3 (no description set)
    Parent commit      : rlvkpnrz a72506cd (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Can write something new to `file1`
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(&edit_script, "write file1\nnew content").unwrap();
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config", config_with_output_as_after],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created kkmpptxz 7c19e689 (no description set)
    Working copy now at: kkmpptxz 7c19e689 (no description set)
    Parent commit      : rlvkpnrz a72506cd (no description set)
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    M file1
    M file2
    [EOF]
    ");

    // But nothing happens if we modify the right side
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(&edit_script, "write file1\nnew content").unwrap();
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "--config", config_with_right_as_after],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
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
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "b"])
        .success();
    std::fs::write(repo_path.join("file1"), "b\n").unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new", "@-"]).success();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file1"), "c\n").unwrap();
    std::fs::write(repo_path.join("file2"), "c\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["new", "@", "b", "-m", "merge"])
        .success();
    // Resolve the conflict in file1, but leave the conflict in file2
    std::fs::write(repo_path.join("file1"), "d\n").unwrap();
    std::fs::write(repo_path.join("file3"), "d\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    // Test the setup
    let output = test_env.run_jj_in(&repo_path, ["diff", "-r", "@-", "-s"]);
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
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created royxmykx 0105de4a (conflict) merge
    Rebased 1 descendant commits
    Working copy now at: yqosqzyt abbb78c1 (conflict) (empty) (no description set)
    Parent commit      : royxmykx 0105de4a (conflict) merge
    Added 0 files, modified 0 files, removed 1 files
    Warning: There are unresolved conflicts at these paths:
    file2    2-sided conflict
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    D file1
    A file3
    [EOF]
    ");
    assert!(!repo_path.join("file1").exists());
    let output = test_env.run_jj_in(&repo_path, ["file", "show", "file2"]);
    insta::assert_snapshot!(output, @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -a
    +c
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}

#[test]
fn test_diffedit_old_restore_interactive_tests() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::remove_file(repo_path.join("file1")).unwrap();
    std::fs::write(repo_path.join("file2"), "b\n").unwrap();
    std::fs::write(repo_path.join("file3"), "b\n").unwrap();

    // Nothing happens if we make no changes
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    C {file2 => file3}
    [EOF]
    ");

    // Nothing happens if the diff-editor exits with an error
    std::fs::write(&edit_script, "rm file2\0fail").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output.normalize_stderr_exit_status(), @r"
    ------- stderr -------
    Error: Failed to edit diff
    Caused by: Tool exited with exit status: 1 (run with --debug to see the exact invocation)
    [EOF]
    [exit status: 1]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    M file2
    C {file2 => file3}
    [EOF]
    ");

    // Can restore changes to individual files
    std::fs::write(&edit_script, "reset file2\0reset file3").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created rlvkpnrz 69811eda (no description set)
    Working copy now at: rlvkpnrz 69811eda (no description set)
    Parent commit      : qpvuntsm fc687cb8 (no description set)
    Added 0 files, modified 1 files, removed 1 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "-s"]);
    insta::assert_snapshot!(output, @r"
    D file1
    [EOF]
    ");

    // Can make unrelated edits
    test_env.run_jj_in(&repo_path, ["undo"]).success();
    std::fs::write(&edit_script, "write file3\nunrelated\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "--from", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created rlvkpnrz 2b76a42e (no description set)
    Working copy now at: rlvkpnrz 2b76a42e (no description set)
    Parent commit      : qpvuntsm fc687cb8 (no description set)
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "--git"]);
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
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file"), "println!(\"foo\")\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "println!(\"bar\")\n").unwrap();
    test_env.run_jj_in(&repo_path, ["new"]).success();
    std::fs::write(repo_path.join("file"), "println!(\"baz\");\n").unwrap();

    // Add a ";" after the line with "bar". There should be no conflict.
    std::fs::write(edit_script, "write file\nprintln!(\"bar\");\n").unwrap();
    let output = test_env.run_jj_in(
        &repo_path,
        ["diffedit", "-r", "@-", "--restore-descendants"],
    );
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Created rlvkpnrz 62b8c2ce (no description set)
    Rebased 1 descendant commits (while preserving their content)
    Working copy now at: kkmpptxz 321d1cd1 (no description set)
    Parent commit      : rlvkpnrz 62b8c2ce (no description set)
    [EOF]
    ");
    let output = test_env.run_jj_in(&repo_path, ["diff", "--git"]);
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
