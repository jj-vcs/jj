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
use crate::common::TestWorkDir;

/// Returns which of `file1`, `file2` and `file3` are checked out.
fn present_files(work_dir: &TestWorkDir) -> String {
    ["file1", "file2", "file3"]
        .into_iter()
        .filter(|name| work_dir.root().join(name).exists())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn test_sparse_manage_patterns() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Write some files to the working copy
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");

    // By default, all files are tracked
    let output = work_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + all()
    [EOF]
    ");
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    .
    [EOF]
    ");

    // Can stop tracking all files
    let output = work_dir.run_jj(["sparse", "set", "--exclude", "."]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 3 files
    [EOF]
    ");
    // The selection is now empty
    let output = work_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"");
    assert_eq!(present_files(&work_dir), "");
    // But the files are still in the commit
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @"
    file1
    file2
    file3
    [EOF]
    ");

    // Run commands in a sub directory to ensure that patterns are parsed as
    // cwd-relative paths, like in other commands.
    let sub_dir = work_dir.create_dir("sub");

    let output = sub_dir.run_jj(["sparse", "set", "--include=../file2", "--include=../file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + file2
    + file3
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file2 file3");

    // Can combine includes and exclusions. A rule replaces an earlier rule
    // with the same pattern, since that rule could never apply again.
    let output = sub_dir.run_jj([
        "sparse",
        "set",
        "--include=../file1",
        "--exclude=../file2",
        "--exclude=../file3",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 2 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + file1
    - file2
    - file3
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file1");

    // Positional arguments replace the selection
    let output = sub_dir.run_jj(["sparse", "set", "../file2", "../file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 2 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + file2
    + file3
    [EOF]
    ");

    // Can use `--clear` and `--include`
    let output = sub_dir.run_jj(["sparse", "set", "--clear", "--include=../file2"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + file2
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file2");

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
    Added 2 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + all()
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file1 file2 file3");
}

#[test]
fn test_sparse_set_filesets() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");

    let output = work_dir.run_jj(["sparse", "set", "glob:file*", "--exclude", "file3"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file1 file2");

    // `sparse()` refers to the selection the command started from
    let output = work_dir.run_jj(["sparse", "set", "(file3 | sparse()) ~ file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 1 files
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file2 file3");
    let output = work_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + (file3 | glob:file* ~ file3) ~ file1
    [EOF]
    ");

    // A selection that isn't a plain list of paths has no `list` form
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The sparse patterns aren't a plain list of paths
    Hint: Use `jj sparse show` instead.
    [EOF]
    [exit status: 1]
    ");

    // `sparse()` is also available to revsets
    work_dir.run_jj(["sparse", "set", "file1"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "changed");
    let output = work_dir.run_jj(["log", "-r", "files(sparse())", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_sparse_legacy_add_remove() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let dir = work_dir.create_dir("dir");
    dir.write_file("file", "contents");
    work_dir.write_file("file1", "contents");
    let sub_dir = work_dir.create_dir("sub");

    // `--add` paths stay relative to the workspace root, even in a sub
    // directory, so `..` is rejected as it always was
    let output = sub_dir.run_jj(["sparse", "set", "--add=../file1"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    error: invalid value '../file1' for '--add <ADD>': Invalid component ".." in repo-relative path "../file1"

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    "#);

    let output = sub_dir.run_jj(["sparse", "set", "--clear", "--add=dir"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: --add/--remove are deprecated; use --include/--exclude instead. Those take filesets, with paths relative to the current directory.
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");

    // `--remove` deletes an exact entry rather than excluding paths
    let output = sub_dir.run_jj(["sparse", "set", "--remove=dir/file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: --add/--remove are deprecated; use --include/--exclude instead. Those take filesets, with paths relative to the current directory.
    Added 0 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = work_dir.run_jj(["sparse", "list"]);
    insta::assert_snapshot!(output, @"
    dir
    [EOF]
    ");
    assert!(work_dir.root().join("dir/file").exists());

    // `--exclude` is the way to exclude paths under an included prefix
    let output = work_dir.run_jj(["sparse", "set", "--exclude=dir/file"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 1 files
    [EOF]
    ");
    assert!(!work_dir.root().join("dir/file").exists());

    // The deprecated flags can't update a selection they can't represent
    let output = work_dir.run_jj(["sparse", "set", "--add=file1"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: --add/--remove are deprecated; use --include/--exclude instead. Those take filesets, with paths relative to the current directory.
    Error: --add/--remove can only update a plain list of paths
    Hint: Use --include/--exclude instead.
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_sparse_working_copy_format() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let src_dir = work_dir.create_dir("src");
    src_dir.write_file("main.rs", "source");
    src_dir.create_dir("generated").write_file("gen.rs", "gen");
    work_dir.write_file("README.md", "readme");
    let format = || {
        std::fs::read_to_string(work_dir.root().join(".jj/working_copy/type")).unwrap()
    };

    // A plain list of paths stays readable by older jj versions
    work_dir.run_jj(["sparse", "set", "src"]).success();
    assert_eq!(format(), "local");

    // Anything else is gated behind a working copy type older jj rejects, so
    // that it can't read the empty path list as "check out nothing"
    work_dir
        .run_jj(["sparse", "set", "glob:**/*.rs", "--exclude", "src/generated"])
        .success();
    assert_eq!(format(), "local-fileset");
    assert!(work_dir.root().join("src/main.rs").exists());
    assert!(!work_dir.root().join("src/generated/gen.rs").exists());
    assert!(!work_dir.root().join("README.md").exists());

    // A new workspace can inherit the rules
    work_dir
        .run_jj(["workspace", "add", "../copy", "--sparse-patterns=copy", "-r", "@"])
        .success();
    let copy_dir = test_env.work_dir("copy");
    assert!(copy_dir.root().join("src/main.rs").exists());
    assert!(!copy_dir.root().join("src/generated/gen.rs").exists());
    assert_eq!(
        copy_dir.run_jj(["sparse", "show"]).success().stdout.raw(),
        work_dir.run_jj(["sparse", "show"]).success().stdout.raw(),
    );

    // Resetting restores the format older jj can read
    work_dir.run_jj(["sparse", "reset"]).success();
    assert_eq!(format(), "local");
    assert!(work_dir.root().join("README.md").exists());
}

#[test]
fn test_sparse_edit() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.write_file("file3", "contents");
    let sub_dir = work_dir.create_dir("sub");

    let edit_rules = |rules: &[&str]| {
        let mut file = std::fs::File::create(&edit_script).unwrap();
        file.write_all(b"dump rules0\0write\n").unwrap();
        for rule in rules {
            file.write_all(rule.as_bytes()).unwrap();
            file.write_all(b"\n").unwrap();
        }
    };
    let read_rules = || std::fs::read_to_string(test_env.env_root().join("rules0")).unwrap();

    // Paths are relative to the workspace root even in a sub directory, so
    // that editing from there doesn't rewrite the existing rules
    edit_rules(&["+ all()", "- file1", "  -  file2  ", "", "JJ: ignored"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 0 files, modified 0 files, removed 2 files
    [EOF]
    ");
    insta::assert_snapshot!(read_rules(), @r#"
    + all()
    JJ: Lines starting with "+" add, and lines starting with "-" remove, the files
    JJ: matched by a fileset expression, starting from no files. Paths are relative
    JJ: to the workspace root. Lines starting with "JJ:" are ignored.
    "#);
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + all()
    - file1
    - file2
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file3");

    // A later rule can include files an earlier rule excluded
    edit_rules(&["+ all()", "- file1 | file2", "+ file2"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Added 1 files, modified 0 files, removed 0 files
    [EOF]
    ");
    let output = sub_dir.run_jj(["sparse", "show"]);
    insta::assert_snapshot!(output, @"
    + all()
    - file1 | file2
    + file2
    [EOF]
    ");
    assert_eq!(present_files(&work_dir), "file2 file3");

    // Every line needs a marker, so that a stray path can't silently become a
    // rule
    edit_rules(&["file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Sparse rule must start with `+` or `-`: file1
    [EOF]
    [exit status: 1]
    ");

    // Invalid expressions are rejected
    edit_rules(&["+ file1 |"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Failed to parse sparse rule: + file1 |
    Caused by:  --> 1:8
      |
    1 | file1 |
      |        ^---
      |
      = expected `~` or <primary>
    [EOF]
    [exit status: 1]
    ");

    // Paths outside the workspace are rejected
    edit_rules(&["+ ../file1"]);
    let output = sub_dir.run_jj(["sparse", "edit"]);
    insta::assert_snapshot!(output, @"");

    // A failed edit leaves the working copy alone
    assert_eq!(present_files(&work_dir), "file2 file3");
}

#[test]
fn test_sparse_edit_rejects_concurrent_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");

    // The editor changes the selection behind the command's back
    let editor = test_env.env_root().join("concurrent-editor.sh");
    std::fs::write(
        &editor,
        "\"$JJ_TEST_BIN\" sparse set file2 || exit 1\nprintf '+ file1\\n' > \"$1\"\n",
    )
    .unwrap();
    test_env.add_config(format!(
        "ui.editor = [\"sh\", {:?}]",
        editor.to_str().unwrap()
    ));

    let output = work_dir.run_jj_with(|cmd| {
        cmd.env("JJ_TEST_BIN", assert_cmd::cargo::cargo_bin!("jj"))
            .args(["sparse", "edit"])
    });
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The sparse patterns changed concurrently
    Hint: Run the command again to apply it to the new patterns.
    [EOF]
    [exit status: 1]
    ");
    // The concurrent change is kept, rather than being silently overwritten
    assert_eq!(
        work_dir.run_jj(["sparse", "list"]).success().stdout.raw(),
        "file2\n"
    );
    assert_eq!(present_files(&work_dir), "file2");
}

#[cfg(unix)]
#[test]
fn test_sparse_edit_snapshots_file_changes() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.write_file("tracked", "original");

    // The editor also changes a tracked file that the new rules exclude. The
    // change must be committed before the file is removed from disk.
    let editor = test_env.env_root().join("edit-file-and-rules.sh");
    std::fs::write(&editor, "printf edited > tracked\nprintf '' > \"$1\"\n").unwrap();
    test_env.add_config(format!(
        "ui.editor = [\"sh\", {:?}]",
        editor.to_str().unwrap()
    ));

    work_dir.run_jj(["sparse", "edit"]).success();
    assert!(!work_dir.root().join("tracked").exists());
    let output = work_dir.run_jj(["file", "show", "tracked"]);
    insta::assert_snapshot!(output, @"edited[EOF]");
    work_dir.run_jj(["sparse", "reset"]).success();
    assert_eq!(work_dir.read_file("tracked"), "edited");
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
