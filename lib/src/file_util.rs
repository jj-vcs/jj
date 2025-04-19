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

use std::fs;
use std::fs::File;
use std::io;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use tempfile::NamedTempFile;
use tempfile::PersistError;
use thiserror::Error;

pub use self::platform::*;

#[derive(Debug, Error)]
#[error("Cannot access {path}")]
pub struct PathError {
    pub path: PathBuf,
    #[source]
    pub error: io::Error,
}

pub trait IoResultExt<T> {
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

/// Creates a directory or does nothing if the directory already exists.
///
/// Returns the underlying error if the directory can't be created.
/// The function will also fail if intermediate directories on the path do not
/// already exist.
pub fn create_or_reuse_dir(dirname: &Path) -> io::Result<()> {
    match fs::create_dir(dirname) {
        Ok(()) => Ok(()),
        Err(_) if dirname.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

/// Removes all files in the directory, but not the directory itself.
///
/// The directory must exist, and there should be no sub directories.
pub fn remove_dir_contents(dirname: &Path) -> Result<(), PathError> {
    for entry in dirname.read_dir().context(dirname)? {
        let entry = entry.context(dirname)?;
        let path = entry.path();
        fs::remove_file(&path).context(&path)?;
    }
    Ok(())
}

/// Expands "~/" to "$HOME/".
pub fn expand_home_path(path_str: &str) -> PathBuf {
    if let Some(remainder) = path_str.strip_prefix("~/") {
        if let Ok(home_dir_str) = std::env::var("HOME") {
            return PathBuf::from(home_dir_str).join(remainder);
        }
    }
    PathBuf::from(path_str)
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
                let mut result = PathBuf::from_iter(std::iter::repeat_n("..", i));
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

/// Like `NamedTempFile::persist()`, but doesn't try to overwrite the existing
/// target on Windows.
pub fn persist_content_addressed_temp_file<P: AsRef<Path>>(
    temp_file: NamedTempFile,
    new_path: P,
) -> io::Result<File> {
    if cfg!(windows) {
        // On Windows, overwriting file can fail if the file is opened without
        // FILE_SHARE_DELETE for example. We don't need to take a risk if the
        // file already exists.
        match temp_file.persist_noclobber(&new_path) {
            Ok(file) => Ok(file),
            Err(PersistError { error, file: _ }) => {
                if let Ok(existing_file) = File::open(new_path) {
                    // TODO: Update mtime to help GC keep this file
                    Ok(existing_file)
                } else {
                    Err(error)
                }
            }
        }
    } else {
        // On Unix, rename() is atomic and should succeed even if the
        // destination file exists. Checking if the target exists might involve
        // non-atomic operation, so don't use persist_noclobber().
        temp_file
            .persist(new_path)
            .map_err(|PersistError { error, file: _ }| error)
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    /// Symlinks are always available on UNIX
    pub fn check_symlink_support() -> io::Result<bool> {
        Ok(true)
    }

    pub fn try_symlink<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
        symlink(original, link)
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::io;
    use std::mem;
    use std::os::windows::fs::symlink_file;
    use std::path::Path;
    use std::ptr;

    type HKEY = *mut c_void;
    type LSTATUS = i32;
    type DWORD = u32;
    type REGSAM = u32;

    const HKEY_LOCAL_MACHINE: HKEY = 0x80000002 as HKEY;
    const KEY_READ: REGSAM = 0x20019;
    const REG_DWORD: DWORD = 4;
    const ERROR_SUCCESS: LSTATUS = 0;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExA(
            hKey: HKEY,
            lpSubKey: *const u8,
            ulOptions: DWORD,
            samDesired: REGSAM,
            phkResult: *mut HKEY,
        ) -> LSTATUS;

        fn RegQueryValueExA(
            hKey: HKEY,
            lpValueName: *const u8,
            lpReserved: *mut DWORD,
            lpType: *mut DWORD,
            lpData: *mut u8,
            lpcbData: *mut DWORD,
        ) -> LSTATUS;

        fn RegCloseKey(hKey: HKEY) -> LSTATUS;
    }

    /// Symlinks may or may not be enabled on Windows. They require the
    /// Developer Mode setting, which is stored in the registry key below.
    pub fn check_symlink_support() -> io::Result<bool> {
        // Registry subkey containing the developer mode setting
        let subkey = b"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock\0";
        // Value name for developer mode setting
        let value_name = b"AllowDevelopmentWithoutDevLicense\0";

        // Return value from registry operations
        let mut key_handle: HKEY = ptr::null_mut();
        let mut developer_mode: DWORD = 0;
        let mut value_type: DWORD = 0;
        let mut data_size: DWORD = mem::size_of::<DWORD>() as DWORD;

        #[allow(unsafe_code)]
        unsafe {
            // Open the registry key
            let result = RegOpenKeyExA(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                0,
                KEY_READ,
                &mut key_handle,
            );

            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result));
            }

            let result = RegQueryValueExA(
                key_handle,
                value_name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                &mut developer_mode as *mut DWORD as *mut u8,
                &mut data_size,
            );

            RegCloseKey(key_handle);

            if result != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(result));
            }

            if value_type != REG_DWORD {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Registry value is not a DWORD",
                ));
            }
        }

        Ok(developer_mode == 1)
    }

    pub fn try_symlink<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
        // this will create a nonfunctional link for directories, but at the moment
        // we don't have enough information in the tree to determine whether the
        // symlink target is a file or a directory
        // note: if developer mode is not enabled the error code will be 1314,
        // ERROR_PRIVILEGE_NOT_HELD

        symlink_file(original, link)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use test_case::test_case;

    use super::*;
    use crate::tests::new_temp_dir;

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
        let temp_dir = new_temp_dir();
        let target = temp_dir.path().join("file");
        let mut temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        temp_file.write_all(b"contents").unwrap();
        assert!(persist_content_addressed_temp_file(temp_file, target).is_ok());
    }

    #[test_case(false ; "existing file open")]
    #[test_case(true ; "existing file closed")]
    fn test_persist_target_exists(existing_file_closed: bool) {
        let temp_dir = new_temp_dir();
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
