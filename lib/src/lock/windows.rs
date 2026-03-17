// Copyright 2020 The Jujutsu Authors
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

//! Windows file locking implementation using std's `File::lock()` which maps
//! to `LockFileEx`.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::thread;

use tracing::instrument;

use super::FileLockError;
use super::backoff::BackoffIterator;

const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const ERROR_SHARING_VIOLATION: u32 = 32;

pub struct FileLock {
    path: PathBuf,
    /// `Option` so `Drop` can close the handle before deleting.
    file: Option<File>,
}

impl FileLock {
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        tracing::info!("Attempting to lock {path:?}");

        let file = try_create_file(&path).map_err(|err| FileLockError {
            message: "Failed to open lock file",
            path: path.clone(),
            err,
        })?;

        // Acquire exclusive lock (blocks until available)
        file.lock().map_err(|err| FileLockError {
            message: "Failed to lock lock file",
            path: path.clone(),
            err,
        })?;

        tracing::info!("Locked {path:?}");
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}

impl Drop for FileLock {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            file.unlock()
                .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to unlock lock file"))
                .ok();
            // file is dropped here, closing the handle so the delete below
            // can succeed. Another process holding the file open prevents
            // deletion (they open without FILE_SHARE_DELETE), avoiding races.
        }
        std::fs::remove_file(&self.path).ok();
    }
}

fn try_create_file(path: &Path) -> io::Result<File> {
    let mut backoff_iterator = BackoffIterator::new();
    let mut options = OpenOptions::new();
    options
        .create(true)
        .write(true)
        // Don't share delete access. This ensures that std::fs::remove_file
        // (which uses DeleteFileW) will fail if any other process has the file
        // open — so deletion in Drop only succeeds when we're the last handle
        // holder.
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    loop {
        match options.open(path) {
            Ok(file) => return Ok(file),
            // The file may be open for deletion in Drop.
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied
                    || err.raw_os_error() == Some(ERROR_SHARING_VIOLATION as _) =>
            {
                let Some(duration) = backoff_iterator.next() else {
                    return Err(err);
                };
                // Ensure that a file can be created in the target directory.
                tempfile::tempfile_in(path.parent().expect("file path should have parent"))?;
                thread::sleep(duration);
            }
            Err(err) => return Err(err),
        }
    }
}
