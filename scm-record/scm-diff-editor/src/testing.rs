//! Testing utilities.
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use scm_record::{File, FileMode};

use crate::{Error, FileContents, FileInfo, Filesystem, Result};

/// In-memory filesystem for testing purposes.
#[derive(Debug)]
pub struct TestFilesystem {
    files: BTreeMap<PathBuf, FileInfo>,
    dirs: BTreeSet<PathBuf>,
}

impl TestFilesystem {
    /// Construct a new [`TestFilesystem`] with the provided set of files.
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

/// Helper function to create a `FileInfo` object containing the provided file
/// contents and a default hash and file mode.
pub fn file_info(contents: impl Into<String>) -> FileInfo {
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

/// Set all checkboxes in the UI.
pub fn select_all(files: &mut [File]) {
    for file in files {
        file.set_checked(true);
    }
}
