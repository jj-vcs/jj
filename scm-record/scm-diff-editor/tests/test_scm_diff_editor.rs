use std::path::PathBuf;

use insta::assert_debug_snapshot;
use maplit::btreemap;

use scm_diff_editor::testing::{file_info, select_all, TestFilesystem};
use scm_diff_editor::{apply_changes, process_opts, DiffContext, Opts, Result};
use scm_record::{RecordState, Section};

#[test]
fn test_diff() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left") => file_info("\
foo
common1
common2
bar
"),
        PathBuf::from("right") => file_info("\
qux1
common1
common2
qux2
"),
    });
    let DiffContext {
        mut files,
        write_root,
    } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left"),
            right: PathBuf::from("right"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;
    assert_debug_snapshot!(files, @r###"
    [
        File {
            old_path: Some(
                "left",
            ),
            path: "right",
            file_mode: Unix(
                33188,
            ),
            sections: [
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Removed,
                            line: "foo\n",
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "qux1\n",
                        },
                    ],
                },
                Unchanged {
                    lines: [
                        "common1\n",
                        "common2\n",
                    ],
                },
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Removed,
                            line: "bar\n",
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "qux2\n",
                        },
                    ],
                },
            ],
        },
    ]
    "###);

    select_all(&mut files);
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "left": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "foo\ncommon1\ncommon2\nbar\n",
                    hash: "abc123",
                    num_bytes: 24,
                },
            },
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "qux1\ncommon1\ncommon2\nqux2\n",
                    hash: "abc123",
                    num_bytes: 26,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_diff_no_changes() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left") => file_info("\
foo
common1
common2
bar
"),
        PathBuf::from("right") => file_info("\
qux1
common1
common2
qux2
"),
    });
    let DiffContext { files, write_root } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left"),
            right: PathBuf::from("right"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;

    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "left": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "foo\ncommon1\ncommon2\nbar\n",
                    hash: "abc123",
                    num_bytes: 24,
                },
            },
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "foo\ncommon1\ncommon2\nbar\n",
                    hash: "abc123",
                    num_bytes: 24,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_diff_absent_left() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("right") => file_info("right\n"),
    });
    let DiffContext {
        mut files,
        write_root,
    } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left"),
            right: PathBuf::from("right"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;
    assert_debug_snapshot!(files, @r###"
    [
        File {
            old_path: Some(
                "left",
            ),
            path: "right",
            file_mode: Absent,
            sections: [
                FileMode {
                    is_checked: false,
                    mode: Unix(
                        33188,
                    ),
                },
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "right\n",
                        },
                    ],
                },
            ],
        },
    ]
    "###);

    select_all(&mut files);
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "right\n",
                    hash: "abc123",
                    num_bytes: 6,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_diff_absent_right() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left") => file_info("left\n"),
    });
    let DiffContext {
        mut files,
        write_root,
    } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left"),
            right: PathBuf::from("right"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;
    assert_debug_snapshot!(files, @r###"
    [
        File {
            old_path: Some(
                "left",
            ),
            path: "right",
            file_mode: Unix(
                33188,
            ),
            sections: [
                FileMode {
                    is_checked: false,
                    mode: Absent,
                },
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Removed,
                            line: "left\n",
                        },
                    ],
                },
            ],
        },
    ]
    "###);

    select_all(&mut files);
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "left": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "left\n",
                    hash: "abc123",
                    num_bytes: 5,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_reject_diff_non_files() -> Result<()> {
    let filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left/foo") => file_info("left\n"),
        PathBuf::from("right/foo") => file_info("right\n"),
    });
    let result = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left"),
            right: PathBuf::from("right"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    );
    insta::assert_debug_snapshot!(result, @r###"
        Err(
            ReadFile {
                path: "left",
                source: Custom {
                    kind: Other,
                    error: "is a directory",
                },
            },
        )
        "###);

    Ok(())
}

#[test]
fn test_diff_files_in_subdirectories() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left/foo") => file_info("left contents\n"),
        PathBuf::from("right/foo") => file_info("right contents\n"),
    });

    let DiffContext { files, write_root } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left/foo"),
            right: PathBuf::from("right/foo"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;

    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "left/foo": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "left contents\n",
                    hash: "abc123",
                    num_bytes: 14,
                },
            },
            "right/foo": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "left contents\n",
                    hash: "abc123",
                    num_bytes: 14,
                },
            },
        },
        dirs: {
            "",
            "left",
            "right",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_dir_diff_no_changes() -> Result<()> {
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("left/foo") => file_info("left contents\n"),
        PathBuf::from("right/foo") => file_info("right contents\n"),
    });

    let DiffContext { files, write_root } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: PathBuf::from("left/foo"),
            right: PathBuf::from("right/foo"),
            base: None,
            output: None,
            read_only: false,
            dry_run: false,
        },
    )?;

    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;
    assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "left/foo": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "left contents\n",
                    hash: "abc123",
                    num_bytes: 14,
                },
            },
            "right/foo": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "left contents\n",
                    hash: "abc123",
                    num_bytes: 14,
                },
            },
        },
        dirs: {
            "",
            "left",
            "right",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_create_merge() -> Result<()> {
    let base_contents = "\
Hello world 1
Hello world 2
Hello world 3
Hello world 4
";
    let left_contents = "\
Hello world 1
Hello world 2
Hello world L
Hello world 4
";
    let right_contents = "\
Hello world 1
Hello world 2
Hello world R
Hello world 4
";
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("base") => file_info(base_contents),
        PathBuf::from("left") => file_info(left_contents),
        PathBuf::from("right") => file_info(right_contents),
    });

    let DiffContext {
        mut files,
        write_root,
    } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: "left".into(),
            right: "right".into(),
            read_only: false,
            dry_run: false,
            base: Some("base".into()),
            output: Some("output".into()),
        },
    )?;
    insta::assert_debug_snapshot!(files, @r###"
    [
        File {
            old_path: Some(
                "base",
            ),
            path: "output",
            file_mode: Unix(
                33188,
            ),
            sections: [
                Unchanged {
                    lines: [
                        "Hello world 1\n",
                        "Hello world 2\n",
                    ],
                },
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "Hello world L\n",
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Removed,
                            line: "Hello world 3\n",
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "Hello world R\n",
                        },
                    ],
                },
                Unchanged {
                    lines: [
                        "Hello world 4\n",
                    ],
                },
            ],
        },
    ]
    "###);

    select_all(&mut files);
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files,
        },
    )?;

    assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "base": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 1\nHello world 2\nHello world 3\nHello world 4\n",
                    hash: "abc123",
                    num_bytes: 56,
                },
            },
            "left": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 1\nHello world 2\nHello world L\nHello world 4\n",
                    hash: "abc123",
                    num_bytes: 56,
                },
            },
            "output": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 1\nHello world 2\nHello world L\nHello world R\nHello world 4\n",
                    hash: "abc123",
                    num_bytes: 70,
                },
            },
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 1\nHello world 2\nHello world R\nHello world 4\n",
                    hash: "abc123",
                    num_bytes: 56,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}

#[test]
fn test_new_file() -> Result<()> {
    let new_file_contents = "\
Hello world 1
Hello world 2
";
    let mut filesystem = TestFilesystem::new(btreemap! {
        PathBuf::from("right") => file_info(new_file_contents),
    });

    let DiffContext {
        mut files,
        write_root,
    } = process_opts(
        &filesystem,
        &Opts {
            dir_diff: false,
            left: "left".into(),
            right: "right".into(),
            read_only: false,
            dry_run: false,
            base: None,
            output: None,
        },
    )?;
    insta::assert_debug_snapshot!(files, @r###"
    [
        File {
            old_path: Some(
                "left",
            ),
            path: "right",
            file_mode: Absent,
            sections: [
                FileMode {
                    is_checked: false,
                    mode: Unix(
                        33188,
                    ),
                },
                Changed {
                    lines: [
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "Hello world 1\n",
                        },
                        SectionChangedLine {
                            is_checked: false,
                            change_type: Added,
                            line: "Hello world 2\n",
                        },
                    ],
                },
            ],
        },
    ]
    "###);

    // Select no changes from new file.
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files: files.clone(),
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {},
        dirs: {
            "",
        },
    }
    "###);

    // Select all changes from new file.
    select_all(&mut files);
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files: files.clone(),
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 1\nHello world 2\n",
                    hash: "abc123",
                    num_bytes: 28,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    // Select only some changes from new file.
    match files[0].sections.get_mut(1).unwrap() {
        Section::Changed { ref mut lines } => lines[0].is_checked = false,
        _ => panic!("Expected changed section"),
    }
    apply_changes(
        &mut filesystem,
        &write_root,
        RecordState {
            is_read_only: false,
            commits: Default::default(),
            files: files.clone(),
        },
    )?;
    insta::assert_debug_snapshot!(filesystem, @r###"
    TestFilesystem {
        files: {
            "right": FileInfo {
                file_mode: Unix(
                    33188,
                ),
                contents: Text {
                    contents: "Hello world 2\n",
                    hash: "abc123",
                    num_bytes: 14,
                },
            },
        },
        dirs: {
            "",
        },
    }
    "###);

    Ok(())
}
