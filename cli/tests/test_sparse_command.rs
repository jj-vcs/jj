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

use std::io::Write as _;

use testutils::TestResult;

use crate::common::TestEnvironment;

#[test]
fn test_sparse_manage_patterns() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Write some files to the working copy
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");

    // By default, all files are tracked
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + all()
    [EOF]
    ");

    // Can stop tracking all files
    let output = work_dir.run_jj(["sparse", "set", "--exclude", "."]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    // The list is now empty
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"");
    // They're removed from the working copy
    assert!(!work_dir.root().join("file1").exists());
    assert!(!work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());
    // But they're still in the commit
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @"
    file1
    file2
    file3
    [EOF]
    ");

    // Run commands in sub directory to ensure that patterns are parsed as
    // cwd-relative paths, and stored as workspace-relative paths.
    let sub_dir = work_dir.create_dir("sub");

    // Can `--include` a few files
    let output = sub_dir.run_jj([
        "sparse",
        "set",
        "--include",
        "../file2",
        "--include",
        "../file3",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + file2
    + file3
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Can combine `--include` and `--exclude`. `--add` and `--remove` are
    // aliases.
    let output = sub_dir.run_jj([
        "sparse",
        "set",
        "--add",
        "../file1",
        "--exclude",
        "../file2",
        "--remove",
        "../file3",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + file2
    + file3
    + file1
    - file2
    - file3
    [EOF]
    ");
    assert!(work_dir.root().join("file1").exists());
    assert!(!work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());

    // Positional arguments replace the patterns
    let output = sub_dir.run_jj(["sparse", "set", "../file2", "../file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + file2
    + file3
    [EOF]
    ");

    // Can use `--clear` and `--include`
    let output = sub_dir.run_jj(["sparse", "set", "--clear", "--include", "../file2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + file2
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(!work_dir.root().join("file3").exists());

    // Can use fileset expressions, including `sparse()` for the current
    // patterns
    let output = work_dir.run_jj(["sparse", "set", "glob:file*", "--exclude", "file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["sparse", "set", "(file3 | sparse()) ~ file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + file3
    + glob:file* ~ file3
    - file1
    [EOF]
    ");
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Positional arguments can't be combined with `--clear`
    let output = work_dir.run_jj(["sparse", "set", "--clear", "file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: the argument '--clear' cannot be used with '[FILESETS]...'

    Usage: jj sparse set --clear [FILESETS]...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");

    // Can reset back to all files
    let output = sub_dir.run_jj(["sparse", "reset"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + all()
    [EOF]
    ");
    assert!(work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Can edit with editor
    let edit_patterns = |patterns: &[&str]| {
        let mut file = std::fs::File::create(&edit_script).unwrap();
        file.write_all(b"dump patterns0\0write\n").unwrap();
        for pattern in patterns {
            file.write_all(pattern.as_bytes()).unwrap();
            file.write_all(b"\n").unwrap();
        }
    };
    let read_patterns = || std::fs::read_to_string(test_env.env_root().join("patterns0")).unwrap();

    // Paths are relative to the workspace root even if run from a sub
    // directory
    edit_patterns(&["+ all()", "- file1", "  -  file2  ", "", "JJ: ignored"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @r#"
    + all()
    JJ: Each line adds (+) or removes (-) the files matched by a fileset expression,
    JJ: starting from no files. Paths are relative to the workspace root.
    JJ: Lines starting with "JJ:" are ignored.
    "#);
    let output = sub_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    + all()
    - file1
    - file2
    [EOF]
    ");

    // Can include previously excluded files
    edit_patterns(&["+ all()", "- file1 | file2", "+ file2"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    insta::assert_snapshot!(read_patterns(), @r#"
    + all()
    - file1
    - file2
    JJ: Each line adds (+) or removes (-) the files matched by a fileset expression,
    JJ: starting from no files. Paths are relative to the workspace root.
    JJ: Lines starting with "JJ:" are ignored.
    "#);
    assert!(!work_dir.root().join("file1").exists());
    assert!(work_dir.root().join("file2").exists());
    assert!(work_dir.root().join("file3").exists());

    // Lines must start with a marker
    edit_patterns(&["file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Sparse pattern must start with '+' or '-': file1
    [EOF]
    [exit status: 1]
    ");

    // Invalid expressions are rejected
    edit_patterns(&["+ file1 |"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Failed to parse sparse pattern: + file1 |
    Caused by:  --> 1:8
      |
    1 | file1 |
      |        ^---
      |
      = expected `~` or <primary>
    [EOF]
    [exit status: 1]
    ");

    // Invalid paths are rejected
    edit_patterns(&["+ ../file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: Failed to parse sparse pattern: + ../file1
    Caused by:
    1:  --> 1:1
      |
    1 | ../file1
      | ^------^
      |
      = Invalid file pattern
    2: Path "../file1" is not in the repo "."
    3: Invalid component ".." in repo-relative path "../file1"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_sparse_editor_avoids_unc() -> TestResult {
    use std::path::PathBuf;

    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(edit_script, "dump-path path")?;
    work_dir.run_jj(["sparse", "edit"]).success();

    let edited_path = PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path"))?);
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
    Ok(())
}
