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

#![expect(missing_docs)]

use std::borrow::Cow;
use std::ffi::OsString;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::Poll;

use tempfile::NamedTempFile;
use tempfile::PersistError;
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;
use tokio::io::ReadBuf;

#[cfg(unix)]
pub use self::platform::check_executable_bit_support;
pub use self::platform::check_symlink_support;
pub use self::platform::try_symlink;

#[derive(Debug, Error)]
#[error("Cannot access {path}")]
pub struct PathError {
    pub path: PathBuf,
    pub source: io::Error,
}

pub trait IoResultExt<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn context(self, path: impl AsRef<Path>) -> Result<T, PathError> {
        self.map_err(|error| PathError {
            path: path.as_ref().to_path_buf(),
            source: error,
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

#[derive(Debug, Error)]
#[error(transparent)]
pub struct BadPathEncoding(platform::BadOsStrEncoding);

/// Constructs [`Path`] from `bytes` in platform-specific manner.
///
/// On Unix, this function never fails because paths are just bytes. On Windows,
/// this may return error if the input wasn't well-formed UTF-8.
pub fn path_from_bytes(bytes: &[u8]) -> Result<&Path, BadPathEncoding> {
    let s = platform::os_str_from_bytes(bytes).map_err(BadPathEncoding)?;
    Ok(Path::new(s))
}

/// Converts `path` to bytes in platform-specific manner.
///
/// On Unix, this function never fails because paths are just bytes. On Windows,
/// this may return error if the input wasn't well-formed UTF-8.
///
/// The returned byte sequence can be considered a superset of ASCII (such as
/// UTF-8 bytes.)
pub fn path_to_bytes(path: &Path) -> Result<&[u8], BadPathEncoding> {
    platform::os_str_to_bytes(path.as_ref()).map_err(BadPathEncoding)
}

/// Expands "~/" to "$HOME/".
pub fn expand_home_path(path_str: &str) -> PathBuf {
    if let Some(remainder) = path_str.strip_prefix("~/")
        && let Ok(home_dir_str) = std::env::var("HOME")
    {
        return PathBuf::from(home_dir_str).join(remainder);
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

/// Converts the given `path` to Unix-like path separated by "/".
///
/// The returned path might not work on Windows if it was canonicalized. On
/// Unix, this function is noop.
pub fn slash_path(path: &Path) -> Cow<'_, Path> {
    if cfg!(windows) {
        Cow::Owned(to_slash_separated(path).into())
    } else {
        Cow::Borrowed(path)
    }
}

fn to_slash_separated(path: &Path) -> OsString {
    let mut buf = OsString::with_capacity(path.as_os_str().len());
    let mut components = path.components();
    match components.next() {
        Some(c) => buf.push(c),
        None => return buf,
    }
    for c in components {
        buf.push("/");
        buf.push(c);
    }
    buf
}

/// Persists the temporary file after synchronizing the content.
///
/// After system crash, the persisted file should have a valid content if
/// existed. However, the persisted file name (or directory entry) could be
/// lost. It's up to caller to synchronize the directory entries.
///
/// See also <https://lwn.net/Articles/457667/> for the behavior on Linux.
pub fn persist_temp_file<P: AsRef<Path>>(
    temp_file: NamedTempFile,
    new_path: P,
) -> io::Result<File> {
    // Ensure persisted file content is flushed to disk.
    temp_file.as_file().sync_data()?;
    temp_file
        .persist(new_path)
        .map_err(|PersistError { error, file: _ }| error)
}

/// Like [`persist_temp_file()`], but doesn't try to overwrite the existing
/// target on Windows.
pub fn persist_content_addressed_temp_file<P: AsRef<Path>>(
    temp_file: NamedTempFile,
    new_path: P,
) -> io::Result<File> {
    // Ensure new file content is flushed to disk, so the old file content
    // wouldn't be lost if existed at the same location.
    temp_file.as_file().sync_data()?;
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

/// Reads from an async source and writes to a sync destination. Does not spawn
/// a task, so writes will block.
pub async fn copy_async_to_sync<R: AsyncRead, W: Write + ?Sized>(
    reader: R,
    writer: &mut W,
) -> io::Result<usize> {
    let mut buf = vec![0; 16 << 10];
    let mut total_written_bytes = 0;

    let mut reader = std::pin::pin!(reader);
    loop {
        let written_bytes = reader.read(&mut buf).await?;
        if written_bytes == 0 {
            return Ok(total_written_bytes);
        }
        writer.write_all(&buf[0..written_bytes])?;
        total_written_bytes += written_bytes;
    }
}

/// `AsyncRead`` implementation backed by a `Read`. It is not actually async;
/// the goal is simply to avoid reading the full contents from the `Read` into
/// memory.
pub struct BlockingAsyncReader<R> {
    reader: R,
}

impl<R: Read + Unpin> BlockingAsyncReader<R> {
    /// Creates a new `BlockingAsyncReader`
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: Read + Unpin> AsyncRead for BlockingAsyncReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let num_bytes_read = self.reader.read(buf.initialize_unfilled())?;
        buf.advance(num_bytes_read);
        Poll::Ready(Ok(()))
    }
}

#[cfg(unix)]
mod platform {
    use std::convert::Infallible;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    pub type BadOsStrEncoding = Infallible;

    pub fn os_str_from_bytes(data: &[u8]) -> Result<&OsStr, BadOsStrEncoding> {
        Ok(OsStr::from_bytes(data))
    }

    pub fn os_str_to_bytes(data: &OsStr) -> Result<&[u8], BadOsStrEncoding> {
        Ok(data.as_bytes())
    }

    /// Whether changing executable bits is permitted on the filesystem of this
    /// directory, and whether attempting to flip one has an observable effect.
    pub fn check_executable_bit_support(path: impl AsRef<Path>) -> io::Result<bool> {
        // Get current permissions and try to flip just the user's executable bit.
        let temp_file = tempfile::tempfile_in(path)?;
        let old_mode = temp_file.metadata()?.permissions().mode();
        let new_mode = old_mode ^ 0o100;
        let result = temp_file.set_permissions(PermissionsExt::from_mode(new_mode));
        match result {
            // If permission was denied, we do not have executable bit support.
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => Ok(false),
            Err(err) => Err(err),
            Ok(()) => {
                // Verify that the permission change was not silently ignored.
                let mode = temp_file.metadata()?.permissions().mode();
                Ok(mode == new_mode)
            }
        }
    }

    /// Symlinks are always available on Unix.
    pub fn check_symlink_support() -> io::Result<bool> {
        Ok(true)
    }

    pub fn try_symlink<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
        symlink(original, link)
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::os::windows::fs::symlink_file;
    use std::path::Path;

    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;

    pub use super::fallback::BadOsStrEncoding;
    pub use super::fallback::os_str_from_bytes;
    pub use super::fallback::os_str_to_bytes;

    /// Symlinks may or may not be enabled on Windows. They require the
    /// Developer Mode setting, which is stored in the registry key below.
    pub fn check_symlink_support() -> io::Result<bool> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let sideloading =
            hklm.open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\AppModelUnlock")?;
        let developer_mode: u32 = sideloading.get_value("AllowDevelopmentWithoutDevLicense")?;
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

#[cfg_attr(unix, expect(dead_code))]
mod fallback {
    use std::ffi::OsStr;

    use thiserror::Error;

    // Define error per platform so we can explicitly say UTF-8 is expected.
    #[derive(Debug, Error)]
    #[error("Invalid UTF-8 sequence")]
    pub struct BadOsStrEncoding;

    pub fn os_str_from_bytes(data: &[u8]) -> Result<&OsStr, BadOsStrEncoding> {
        Ok(str::from_utf8(data).map_err(|_| BadOsStrEncoding)?.as_ref())
    }

    pub fn os_str_to_bytes(data: &OsStr) -> Result<&[u8], BadOsStrEncoding> {
        Ok(data.to_str().ok_or(BadOsStrEncoding)?.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::io::Write as _;

    use itertools::Itertools as _;
    use pollster::FutureExt as _;
    use test_case::test_case;

    use super::*;
    use crate::tests::new_temp_dir;

    #[test]
    #[cfg(unix)]
    fn exec_bit_support_in_temp_dir() {
        // Temporary directories on Unix should always have executable support.
        // Note that it would be problematic to test in a non-temp directory, as
        // a developer's filesystem may or may not have executable bit support.
        let dir = new_temp_dir();
        let supported = check_executable_bit_support(dir.path()).unwrap();
        assert!(supported);
    }

    #[test]
    fn test_path_bytes_roundtrip() {
        let bytes = b"ascii";
        let path = path_from_bytes(bytes).unwrap();
        assert_eq!(path_to_bytes(path).unwrap(), bytes);

        let bytes = b"utf-8.\xc3\xa0";
        let path = path_from_bytes(bytes).unwrap();
        assert_eq!(path_to_bytes(path).unwrap(), bytes);

        let bytes = b"latin1.\xe0";
        if cfg!(unix) {
            let path = path_from_bytes(bytes).unwrap();
            assert_eq!(path_to_bytes(path).unwrap(), bytes);
        } else {
            assert!(path_from_bytes(bytes).is_err());
        }
    }

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
    fn test_slash_path() {
        assert_eq!(slash_path(Path::new("")), Path::new(""));
        assert_eq!(slash_path(Path::new("foo")), Path::new("foo"));
        assert_eq!(slash_path(Path::new("foo/bar")), Path::new("foo/bar"));
        assert_eq!(slash_path(Path::new("foo/bar/..")), Path::new("foo/bar/.."));
        assert_eq!(
            slash_path(Path::new(r"foo\bar")),
            if cfg!(windows) {
                Path::new("foo/bar")
            } else {
                Path::new(r"foo\bar")
            }
        );
        assert_eq!(
            slash_path(Path::new(r"..\foo\bar")),
            if cfg!(windows) {
                Path::new("../foo/bar")
            } else {
                Path::new(r"..\foo\bar")
            }
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

    #[test]
    fn test_copy_async_to_sync_small() {
        let input = b"hello";
        let mut output = vec![];

        let result = copy_async_to_sync(Cursor::new(&input), &mut output).block_on();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5);
        assert_eq!(output, input);
    }

    #[test]
    fn test_copy_async_to_sync_large() {
        // More than 1 buffer worth of data
        let input = (0..100u8).cycle().take(40000).collect_vec();
        let mut output = vec![];

        let result = copy_async_to_sync(Cursor::new(&input), &mut output).block_on();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 40000);
        assert_eq!(output, input);
    }

    #[test]
    fn test_blocking_async_reader() {
        let input = b"hello";
        let sync_reader = Cursor::new(&input);
        let mut async_reader = BlockingAsyncReader::new(sync_reader);

        let mut buf = [0u8; 3];
        let num_bytes_read = async_reader.read(&mut buf).block_on().unwrap();
        assert_eq!(num_bytes_read, 3);
        assert_eq!(&buf, &input[0..3]);

        let num_bytes_read = async_reader.read(&mut buf).block_on().unwrap();
        assert_eq!(num_bytes_read, 2);
        assert_eq!(&buf[0..2], &input[3..5]);
    }

    #[test]
    fn test_blocking_async_reader_read_to_end() {
        let input = b"hello";
        let sync_reader = Cursor::new(&input);
        let mut async_reader = BlockingAsyncReader::new(sync_reader);

        let mut buf = vec![];
        let num_bytes_read = async_reader.read_to_end(&mut buf).block_on().unwrap();
        assert_eq!(num_bytes_read, input.len());
        assert_eq!(&buf, &input);
    }
}
