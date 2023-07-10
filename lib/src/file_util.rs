// Copyright 2021 The Jujutsu Authors
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

#![allow(missing_docs)]

use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::{io, iter};

use tempfile::{NamedTempFile, PersistError};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Cannot access {path}")]
pub struct PathError {
    pub path: PathBuf,
    #[source]
    pub error: io::Error,
}

pub(crate) trait IoResultExt<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError> {
        self.map_err(|error| PathError {
            path: path.as_ref().to_path_buf(),
            error,
        })
    }
}

/// Turns the given `to` path into relative path starting from the `from` path.
///
/// Both `from` and `to` paths are supposed to be absolute and normalized in the
/// same manner.
pub fn relative_path(from: &Path, to: &Path) -> PathBuf {
    // Find common prefix.
    for (i, base) in from.ancestors().enumerate() {
        if let Ok(suffix) = to.strip_prefix(base) {
            if i == 0 && suffix.as_os_str().is_empty() {
                return ".".into();
            } else {
                let mut result = PathBuf::from_iter(iter::repeat("..").take(i));
                result.push(suffix);
                return result;
            }
        }
    }

    // No common prefix found. Return the original (absolute) path.
    to.to_owned()
}

/// Consumes as much `..` and `.` as possible without considering symlinks.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(result.components().next_back(), Some(Component::Normal(_))) =>
            {
                // Do not pop ".."
                let popped = result.pop();
                assert!(popped);
            }
            _ => {
                result.push(c);
            }
        }
    }

    if result.as_os_str().is_empty() {
        ".".into()
    } else {
        result
    }
}

// Like NamedTempFile::persist(), but also succeeds if the target already
// exists.
pub fn persist_content_addressed_temp_file<P: AsRef<Path>>(
    temp_file: NamedTempFile,
    new_path: P,
) -> Result<File, PersistError> {
    match temp_file.persist(&new_path) {
        Ok(file) => Ok(file),
        Err(PersistError { error, file }) => {
            if let Ok(existing_file) = File::open(new_path) {
                Ok(existing_file)
            } else {
                Err(PersistError { error, file })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use test_case::test_case;

    use super::*;

    #[test]
    fn normalize_too_many_dot_dot() {
        assert_eq!(normalize_path(Path::new("foo/..")), Path::new("."));
        assert_eq!(normalize_path(Path::new("foo/../..")), Path::new(".."));
        assert_eq!(
            normalize_path(Path::new("foo/../../..")),
            Path::new("../..")
        );
        assert_eq!(
            normalize_path(Path::new("foo/../../../bar/baz/..")),
            Path::new("../../bar")
        );
    }

    #[test]
    fn test_persist_no_existing_file() {
        let temp_dir = testutils::new_temp_dir();
        let target = temp_dir.path().join("file");
        let mut temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        temp_file.write_all(b"contents").unwrap();
        assert!(persist_content_addressed_temp_file(temp_file, target).is_ok());
    }

    #[test_case(false ; "existing file open")]
    #[test_case(true ; "existing file closed")]
    fn test_persist_target_exists(existing_file_closed: bool) {
        let temp_dir = testutils::new_temp_dir();
        let target = temp_dir.path().join("file");
        let mut temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        temp_file.write_all(b"contents").unwrap();

        let mut file = File::create(&target).unwrap();
        file.write_all(b"contents").unwrap();
        if existing_file_closed {
            drop(file);
        }

        assert!(persist_content_addressed_temp_file(temp_file, &target).is_ok());
    }
}
