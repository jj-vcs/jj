// Copyright 2026 The Jujutsu Authors
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

use testutils::TestResult;

use crate::common::TestEnvironment;
use crate::common::create_commit_with_files;

#[test]
fn test_file_edit() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(
        &work_dir,
        "base",
        &[],
        &[
            ("file", "base\n"),
            ("dir/file", "dir\n"),
            ("script.sh", "#!/bin/sh\n"),
        ],
    );
    work_dir
        .run_jj(["file", "chmod", "x", "-r=base", "script.sh"])
        .success();
    // The child doesn't touch `file`, so it inherits edits made to `base`.
    create_commit_with_files(&work_dir, "child", &["base"], &[("other", "child\n")]);
    let setup_opid = work_dir.current_operation_id();

    // The editor sees the content from the revision. The edited content is
    // saved back to the revision, and descendants are rebased.
    std::fs::write(&edit_script, "expect\nbase\n\0write\nmodified\n")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl d2c70bd0 child | child
    Parent commit (@-)      : rlvkpnrz 6b6a8e3b base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @r"
    modified
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @r"
    modified
    [EOF]
    ");

    // With --restore-descendants, descendants keep their content.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write\nmodified\n")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "--restore-descendants", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits (while preserving their content).
    Working copy  (@) now at: mzvwutvl 960b31ff child | child
    Parent commit (@-)      : rlvkpnrz 0f101435 base | base
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "file"]), @r"
    modified
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=child", "file"]), @r"
    base
    [EOF]
    ");

    // Edits the working-copy commit by default
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write\nedited\n")?;
    let output = work_dir.run_jj(["file", "edit", "other"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 5501cad3 child | child
    Parent commit (@-)      : rlvkpnrz 75eae605 base | base
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "other"]), @r"
    edited
    [EOF]
    ");

    // The executable bit is preserved
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write\n#!/bin/sh\necho hello\n")?;
    work_dir
        .run_jj(["file", "edit", "-r=base", "script.sh"])
        .success();
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=base", "script.sh"]), @r#"
    script.sh: Ok(Resolved(Some(File { id: FileId("21ba682558a42264518f1e0ba55e8a5cd9d7db0a"), executable: true, copy_id: CopyId("") })))
    [EOF]
    "#);

    // The file is opened at its path in the repo, so the editor can detect the
    // file type from its name
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "dump-path path_dump")?;
    work_dir
        .run_jj(["file", "edit", "-r=base", "dir/file"])
        .success();
    let path = std::fs::read_to_string(test_env.env_root().join("path_dump"))?.replace('\\', "/");
    assert!(path.ends_with("/wc/dir/file"), "unexpected path: {path}");

    // Nothing happens if the editor doesn't change the file
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    std::fs::write(&edit_script, "")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);

    // A path that doesn't exist in the revision is created when the editor
    // saves it
    std::fs::write(&edit_script, "write\nbrand new\n")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "new_file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: mzvwutvl 4f72c7df child | child
    Parent commit (@-)      : rlvkpnrz a7cca3e1 base | base
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=base", "new_file"]), @r"
    brand new
    [EOF]
    ");

    // Nothing happens if the editor doesn't create the new file
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let opid = work_dir.current_operation_id();
    std::fs::write(&edit_script, "")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "new_file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), opid);

    // Directories can't be edited
    let output = work_dir.run_jj(["file", "edit", "-r=base", "dir"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Path is a directory: dir
    [EOF]
    [exit status: 1]
    ");

    // The revision is unchanged if the editor fails
    std::fs::write(&edit_script, "write\nmodified\n\0fail")?;
    let output = work_dir.run_jj(["file", "edit", "-r=base", "file"]);
    insta::with_settings!({
        filters => [
            (r"\bEditor '[^']*'", "Editor '<redacted>'"),
            (r"in .*(jj-file-edit-)[^/]*(/wc/file)\b", "in <redacted>$1<redacted>$2"),
            (r"exit code", "exit status"),
        ]
    }, {
        insta::assert_snapshot!(output.normalize_backslash(), @r"
        ------- stderr -------
        Error: Failed to edit file
        Caused by: Editor '<redacted>' exited with exit status: 1
        Hint: Edited file is left in <redacted>jj-file-edit-<redacted>/wc/file
        [EOF]
        [exit status: 1]
        ");
    });
    assert_eq!(work_dir.current_operation_id(), opid);
    Ok(())
}

#[test]
fn test_file_edit_conflict() -> TestResult {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit_with_files(&work_dir, "base", &[], &[("file", "base\n")]);
    create_commit_with_files(&work_dir, "left", &["base"], &[("file", "left\n")]);
    create_commit_with_files(&work_dir, "right", &["base"], &[("file", "right\n")]);
    create_commit_with_files(&work_dir, "conflict", &["left", "right"], &[]);
    work_dir.run_jj(["new", "conflict"]).success();
    let setup_opid = work_dir.current_operation_id();

    // The editor sees the materialized conflict, the same as `jj file show`.
    // Nothing happens if the conflict is left unchanged.
    std::fs::write(&edit_script, "dump conflict_dump")?;
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
    assert_eq!(work_dir.current_operation_id(), setup_opid);
    let dumped = std::fs::read_to_string(test_env.env_root().join("conflict_dump"))?;
    let shown = work_dir
        .run_jj(["file", "show", "-r=conflict", "file"])
        .stdout
        .into_raw();
    assert_eq!(dumped, shown);

    // Editing a side within the conflict markers keeps the file conflicted
    std::fs::write(
        &edit_script,
        format!("write\n{}", dumped.replace("left\n", "new-left\n")),
    )?;
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: znkkpsqq 89dab22f (conflict) (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 9c77d648 conflict | (conflict) conflict
    Added 0 files, modified 1 files, removed 0 files
    Warning: There are unresolved conflicts at these paths:
    file    2-sided conflict
    New conflicts appeared in 1 commits:
      vruxwmqv 9c77d648 conflict | (conflict) conflict
    Hint: To resolve the conflicts, start by creating a commit on top of
    the conflicted commit:
      jj new vruxwmqv
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you can inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["file", "show", "-r=conflict", "file"]), @r#"
    <<<<<<< conflict 1 of 1
    +++++++ zsuskuln c0778f46 "left"
    new-left
    %%%%%%% diff from: rlvkpnrz 1792382a "base"
    \\\\\\\        to: royxmykx 386edb4d "right"
    -base
    +right
    >>>>>>> conflict 1 of 1 ends
    [EOF]
    "#);

    // Removing the conflict markers resolves the conflict
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    std::fs::write(&edit_script, "write\nresolved\n")?;
    let output = work_dir.run_jj(["file", "edit", "-r=conflict", "file"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Rebased 1 descendant commits.
    Working copy  (@) now at: znkkpsqq f88712e9 (empty) (no description set)
    Parent commit (@-)      : vruxwmqv 732109c7 conflict | conflict
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["debug", "tree", "-r=conflict"]), @r#"
    file: Ok(Resolved(Some(File { id: FileId("2ab19ae607aabda796309682e0448237aab03047"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#);
    Ok(())
}
