//! An interactive difftool for use in VCS programs like
//! [Jujutsu](https://github.com/martinvonz/jj) or Git.
#![warn(missing_docs)]
#![warn(
    clippy::all,
    clippy::as_conversions,
    clippy::clone_on_ref_ptr,
    clippy::dbg_macro
)]
#![allow(clippy::too_many_arguments)]

mod render;
pub mod testing;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf, StripPrefixError};

use clap::Parser;
use sha1::Digest;
use thiserror::Error;
use walkdir::WalkDir;

use scm_record::helpers::CrosstermInput;
use scm_record::{
    File, FileMode, RecordError, RecordState, Recorder, SelectedChanges, SelectedContents,
};

/// Render a partial commit selector for use as a difftool or mergetool.
///
/// This can be used to interactively select changes to include as part of a
/// commit, to resolve merge conflicts, or to simply display a diff in a
/// readable way.
#[derive(Debug, Parser)]
pub struct Opts {
    /// Instead of comparing two files, compare two directories recursively.
    #[clap(short = 'd', long = "dir-diff")]
    pub dir_diff: bool,

    /// The left-hand file to compare (or directory if `--dir-diff` is passed).
    pub left: PathBuf,

    /// The right-hand file to compare (or directory if `--dir-diff` is passed).
    pub right: PathBuf,

    /// Disable all editing controls and do not write the selected commit
    /// contents to disk.
    #[clap(long = "read-only")]
    pub read_only: bool,

    /// Show what would have been written to disk as part of the commit
    /// selection, but do not actually write it.
    #[clap(short = 'N', long = "dry-run")]
    pub dry_run: bool,

    /// Render the interface as a mergetool instead of a difftool and use this
    /// file as the base of a three-way diff as part of resolving merge
    /// conflicts.
    #[clap(
        short = 'b',
        long = "base",
        requires("output"),
        conflicts_with("dir_diff")
    )]
    pub base: Option<PathBuf>,

    /// Write the resolved merge conflicts to this file.
    #[clap(short = 'o', long = "output", conflicts_with("dir_diff"))]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("aborted by user")]
    Cancelled,

    #[error("dry run, not writing any files")]
    DryRun,

    #[error("walking directory: {source}")]
    WalkDir { source: walkdir::Error },

    #[error("stripping directory prefix {root} from {path}: {source}")]
    StripPrefix {
        root: PathBuf,
        path: PathBuf,
        source: StripPrefixError,
    },

    #[error("reading file {path}: {source}")]
    ReadFile { path: PathBuf, source: io::Error },

    #[error("removing file {path}: {source}")]
    RemoveFile { path: PathBuf, source: io::Error },

    #[error("copying file {old_path} to {new_path}: {source}")]
    CopyFile {
        old_path: PathBuf,
        new_path: PathBuf,
        source: io::Error,
    },

    #[error("creating directory {path}: {source}")]
    CreateDirAll { path: PathBuf, source: io::Error },

    #[error("writing file {path}: {source}")]
    WriteFile { path: PathBuf, source: io::Error },

    #[error("file did not exist: {path}")]
    MissingMergeFile { path: PathBuf },

    #[error("file was not text: {path}")]
    BinaryMergeFile { path: PathBuf },

    #[error("recording changes: {source}")]
    Record { source: RecordError },
}

/// Result type alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Information about a file that was read from disk. Note that the file may not have existed, in
/// which case its contents will be marked as absent.
#[derive(Clone, Debug)]
pub struct FileInfo {
    /// The file mode (see [`scm_record::FileMode`]).
    pub file_mode: FileMode,

    /// The file contents.
    pub contents: FileContents,
}

/// Representation of a file's contents.
#[derive(Clone, Debug)]
pub enum FileContents {
    /// There is no file. (This is different from the file being present but empty.)
    Absent,

    /// The file is a text file with the given contents.
    Text {
        /// The contents of the file.
        contents: String,

        /// The hash of `contents`.
        hash: String,

        /// The size of `contents`, in bytes.
        num_bytes: u64,
    },

    /// The file is a binary file (not able to be displayed directly in the UI).
    Binary {
        /// The hash of the file's contents.
        hash: String,

        /// The size of the file's contents, in bytes.
        num_bytes: u64,
    },
}

/// Abstraction over the filesystem.
pub trait Filesystem {
    /// Find the set of files that appear in either `left` or `right`.
    fn read_dir_diff_paths(&self, left: &Path, right: &Path) -> Result<BTreeSet<PathBuf>>;

    /// Read the [`FileInfo`] for the provided `path`.
    fn read_file_info(&self, path: &Path) -> Result<FileInfo>;

    /// Write new file contents to `path`.
    fn write_file(&mut self, path: &Path, contents: &str) -> Result<()>;

    /// Copy the file at `old_path` to `new_path`. (This can be more efficient
    /// than reading and writing the entire contents, particularly for large
    /// binary files.)
    fn copy_file(&mut self, old_path: &Path, new_path: &Path) -> Result<()>;

    /// Delete the file at `path`.
    fn remove_file(&mut self, path: &Path) -> Result<()>;

    /// Create the directory `path` and any parent directories as necessary.
    fn create_dir_all(&mut self, path: &Path) -> Result<()>;
}

struct RealFilesystem;

impl Filesystem for RealFilesystem {
    fn read_dir_diff_paths(&self, left: &Path, right: &Path) -> Result<BTreeSet<PathBuf>> {
        fn walk_dir(dir: &Path) -> Result<BTreeSet<PathBuf>> {
            let mut files = BTreeSet::new();
            for entry in WalkDir::new(dir) {
                let entry = entry.map_err(|err| Error::WalkDir { source: err })?;
                if entry.file_type().is_file() || entry.file_type().is_symlink() {
                    let relative_path = match entry.path().strip_prefix(dir) {
                        Ok(path) => path.to_owned(),
                        Err(err) => {
                            return Err(Error::StripPrefix {
                                root: dir.to_owned(),
                                path: entry.path().to_owned(),
                                source: err,
                            })
                        }
                    };
                    files.insert(relative_path);
                }
            }
            Ok(files)
        }
        let left_files = walk_dir(left)?;
        let right_files = walk_dir(right)?;
        let paths = left_files
            .into_iter()
            .chain(right_files)
            .collect::<BTreeSet<_>>();
        Ok(paths)
    }

    fn read_file_info(&self, path: &Path) -> Result<FileInfo> {
        let file_mode = match fs::metadata(path) {
            Ok(metadata) => {
                // TODO: no support for gitlinks (submodules).
                if metadata.is_symlink() {
                    FileMode::Unix(0o120000)
                } else {
                    let permissions = metadata.permissions();
                    #[cfg(unix)]
                    let executable = {
                        use std::os::unix::fs::PermissionsExt;
                        permissions.mode() & 0o001 == 0o001
                    };
                    #[cfg(not(unix))]
                    let executable = false;
                    if executable {
                        FileMode::Unix(0o100755)
                    } else {
                        FileMode::Unix(0o100644)
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => FileMode::Absent,
            Err(err) => {
                return Err(Error::ReadFile {
                    path: path.to_owned(),
                    source: err,
                })
            }
        };
        let contents = match fs::read(path) {
            Ok(contents) => {
                let hash = {
                    let mut hasher = sha1::Sha1::new();
                    hasher.update(&contents);
                    format!("{:x}", hasher.finalize())
                };
                let num_bytes: u64 = contents.len().try_into().unwrap();
                if contents.contains(&0) {
                    FileContents::Binary { hash, num_bytes }
                } else {
                    match String::from_utf8(contents) {
                        Ok(contents) => FileContents::Text {
                            contents,
                            hash,
                            num_bytes,
                        },
                        Err(_) => FileContents::Binary { hash, num_bytes },
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => FileContents::Absent,
            Err(err) => {
                return Err(Error::ReadFile {
                    path: path.to_owned(),
                    source: err,
                })
            }
        };
        Ok(FileInfo {
            file_mode,
            contents,
        })
    }

    fn write_file(&mut self, path: &Path, contents: &str) -> Result<()> {
        fs::write(path, contents).map_err(|err| Error::WriteFile {
            path: path.to_owned(),
            source: err,
        })
    }

    fn copy_file(&mut self, old_path: &Path, new_path: &Path) -> Result<()> {
        fs::copy(old_path, new_path).map_err(|err| Error::CopyFile {
            old_path: old_path.to_owned(),
            new_path: new_path.to_owned(),
            source: err,
        })?;
        Ok(())
    }

    fn remove_file(&mut self, path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(Error::RemoveFile {
                path: path.to_owned(),
                source: err,
            }),
        }
    }

    fn create_dir_all(&mut self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).map_err(|err| Error::CreateDirAll {
            path: path.to_owned(),
            source: err,
        })?;
        Ok(())
    }
}

/// Information about the files to display/diff in the UI.
#[derive(Debug)]
pub struct DiffContext {
    /// The files to diff.
    /// - When diffing a single file, this will have only one entry.
    /// - When diffing a directory, this may have many entries (one for each pair of files).
    pub files: Vec<File<'static>>,

    /// When writing results to the filesystem, this path should be prepended to
    /// each `File`'s path. It may be empty (indicating to overwrite the file
    /// in-place).
    pub write_root: PathBuf,
}

/// Process the command-line options to find the files to diff.
pub fn process_opts(filesystem: &dyn Filesystem, opts: &Opts) -> Result<DiffContext> {
    let result = match opts {
        Opts {
            dir_diff: false,
            left,
            right,
            base: None,
            output: _,
            read_only: _,
            dry_run: _,
        } => {
            let files = vec![render::create_file(
                filesystem,
                left.clone(),
                left.clone(),
                right.clone(),
                right.clone(),
            )?];
            DiffContext {
                files,
                write_root: PathBuf::new(),
            }
        }

        Opts {
            dir_diff: true,
            left,
            right,
            base: None,
            output: _,
            read_only: _,
            dry_run: _,
        } => {
            let display_paths = filesystem.read_dir_diff_paths(left, right)?;
            let mut files = Vec::new();
            for display_path in display_paths {
                files.push(render::create_file(
                    filesystem,
                    left.join(&display_path),
                    display_path.clone(),
                    right.join(&display_path),
                    display_path.clone(),
                )?);
            }
            DiffContext {
                files,
                write_root: right.clone(),
            }
        }

        Opts {
            dir_diff: false,
            left,
            right,
            base: Some(base),
            output: Some(output),
            read_only: _,
            dry_run: _,
        } => {
            let files = vec![render::create_merge_file(
                filesystem,
                base.clone(),
                left.clone(),
                right.clone(),
                output.clone(),
            )?];
            DiffContext {
                files,
                write_root: PathBuf::new(),
            }
        }

        Opts {
            dir_diff: false,
            left: _,
            right: _,
            base: Some(_),
            output: None,
            read_only: _,
            dry_run: _,
        } => {
            unreachable!("--output is required when --base is provided");
        }

        Opts {
            dir_diff: true,
            left: _,
            right: _,
            base: Some(_),
            output: _,
            read_only: _,
            dry_run: _,
        } => {
            unimplemented!("--base cannot be used with --dir-diff");
        }
    };
    Ok(result)
}

fn print_dry_run(write_root: &Path, state: RecordState) {
    let RecordState {
        is_read_only: _,
        commits: _,
        files,
    } = state;
    for file in files {
        let file_path = write_root.join(file.path.clone());
        let (selected_contents, _unselected_contents) = file.get_selected_contents();

        let File {
            file_mode: old_file_mode,
            ..
        } = file;

        let SelectedChanges {
            contents,
            file_mode,
        } = selected_contents;

        if file_mode == FileMode::Absent {
            println!("Would delete file: {}", file_path.display());
            continue;
        }

        let print_file_mode_change = old_file_mode != file_mode;
        if print_file_mode_change {
            println!(
                "Would change file mode from {} to {}: {}",
                old_file_mode,
                file_mode,
                file_path.display()
            );
        }

        match contents {
            SelectedContents::Unchanged => {
                // Printing that the file is unchanged is incorrect (and that the contents
                // is unchanged is just noisy) if we've already printed that the mode changed.
                if !print_file_mode_change {
                    println!("Would leave file unchanged: {}", file_path.display())
                }
            }
            SelectedContents::Binary {
                old_description,
                new_description,
            } => {
                println!("Would update binary file: {}", file_path.display());
                println!("  Old: {old_description:?}");
                println!("  New: {new_description:?}");
            }
            SelectedContents::Text { contents } => {
                println!("Would update text file: {}", file_path.display());
                for line in contents.lines() {
                    println!("  {line}");
                }
            }
        }
    }
}

/// After the user has selected changes in the provided [`RecordState`], write
/// the results to the provided [`Filesystem`].
pub fn apply_changes(
    filesystem: &mut dyn Filesystem,
    write_root: &Path,
    state: RecordState,
) -> Result<()> {
    let RecordState {
        is_read_only,
        commits: _,
        files,
    } = state;
    if is_read_only {
        return Ok(());
    }
    for file in files {
        let file_path = write_root.join(file.path.clone());
        let (selected_changes, _unselected_changes) = file.get_selected_contents();

        let SelectedChanges {
            contents,
            file_mode,
        } = selected_changes;

        if file_mode == FileMode::Absent {
            filesystem.remove_file(&file_path)?;
        }

        match contents {
            SelectedContents::Unchanged => {
                // Do nothing.
            }
            SelectedContents::Binary {
                old_description: _,
                new_description: _,
            } => {
                let new_path = file_path;
                let old_path = match &file.old_path {
                    Some(old_path) => old_path.clone(),
                    None => Cow::Borrowed(new_path.as_path()),
                };
                filesystem.copy_file(&old_path, &new_path)?;
            }
            SelectedContents::Text { contents } => {
                if let Some(parent_dir) = file_path.parent() {
                    filesystem.create_dir_all(parent_dir)?;
                }

                // TODO: Respect executable bit
                filesystem.write_file(&file_path, &contents)?;
            }
        }
    }
    Ok(())
}

/// Select changes interactively and apply them to disk.
pub fn run(opts: Opts) -> Result<()> {
    let filesystem = RealFilesystem;
    let DiffContext { files, write_root } = process_opts(&filesystem, &opts)?;
    let state = RecordState {
        is_read_only: opts.read_only,
        commits: Default::default(),
        files,
    };
    let mut input = CrosstermInput;
    let recorder = Recorder::new(state, &mut input);
    match recorder.run() {
        Ok(state) => {
            if opts.dry_run {
                print_dry_run(&write_root, state);
                Err(Error::DryRun)
            } else {
                let mut filesystem = filesystem;
                apply_changes(&mut filesystem, &write_root, state)?;
                Ok(())
            }
        }
        Err(RecordError::Cancelled) => Err(Error::Cancelled),
        Err(err) => Err(Error::Record { source: err }),
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;
    use maplit::btreemap;
    use std::collections::BTreeMap;

    use scm_record::Section;

    use super::*;

    #[derive(Debug)]
    struct TestFilesystem {
        files: BTreeMap<PathBuf, FileInfo>,
        dirs: BTreeSet<PathBuf>,
    }

    impl TestFilesystem {
        pub fn new(files: BTreeMap<PathBuf, FileInfo>) -> Self {
            let dirs = files
                .keys()
                .flat_map(|path| path.ancestors().skip(1))
                .map(|path| path.to_owned())
                .collect();
            Self { files, dirs }
        }

        fn assert_parent_dir_exists(&self, path: &Path) {
            if let Some(parent_dir) = path.parent() {
                assert!(
                    self.dirs.contains(parent_dir),
                    "parent dir for {path:?} does not exist"
                );
            }
        }
    }

    impl Filesystem for TestFilesystem {
        fn read_dir_diff_paths(&self, left: &Path, right: &Path) -> Result<BTreeSet<PathBuf>> {
            let left_files = self
                .files
                .keys()
                .filter_map(|path| path.strip_prefix(left).ok());
            let right_files = self
                .files
                .keys()
                .filter_map(|path| path.strip_prefix(right).ok());
            Ok(left_files
                .chain(right_files)
                .map(|path| path.to_path_buf())
                .collect())
        }

        fn read_file_info(&self, path: &Path) -> Result<FileInfo> {
            match self.files.get(path) {
                Some(file_info) => Ok(file_info.clone()),
                None => match self.dirs.get(path) {
                    Some(_path) => Err(Error::ReadFile {
                        path: path.to_owned(),
                        source: io::Error::other("is a directory"),
                    }),
                    None => Ok(FileInfo {
                        file_mode: FileMode::Absent,
                        contents: FileContents::Absent,
                    }),
                },
            }
        }

        fn write_file(&mut self, path: &Path, contents: &str) -> Result<()> {
            self.assert_parent_dir_exists(path);
            self.files.insert(path.to_owned(), file_info(contents));
            Ok(())
        }

        fn copy_file(&mut self, old_path: &Path, new_path: &Path) -> Result<()> {
            self.assert_parent_dir_exists(new_path);
            let file_info = self.read_file_info(old_path)?;
            self.files.insert(new_path.to_owned(), file_info);
            Ok(())
        }

        fn remove_file(&mut self, path: &Path) -> Result<()> {
            self.files.remove(path);
            Ok(())
        }

        fn create_dir_all(&mut self, path: &Path) -> Result<()> {
            self.dirs.insert(path.to_owned());
            Ok(())
        }
    }

    fn file_info(contents: impl Into<String>) -> FileInfo {
        let contents = contents.into();
        let num_bytes = contents.len().try_into().unwrap();
        FileInfo {
            file_mode: FileMode::Unix(0o100644),
            contents: FileContents::Text {
                contents,
                hash: "abc123".to_string(),
                num_bytes,
            },
        }
    }

    fn select_all(files: &mut [File]) {
        for file in files {
            file.set_checked(true);
        }
    }

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
}
