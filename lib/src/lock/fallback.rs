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

//! Fallback file locking implementation for non-Unix platforms (primarily
//! Windows). Uses std's `File::lock()` which maps to `LockFileEx` on Windows.

use std::fs::File;
use std::fs::OpenOptions;
use std::path::PathBuf;

use tracing::instrument;

use super::FileLockError;

pub struct FileLock {
    path: PathBuf,
    /// `Option` so `Drop` can close the handle before deleting on Windows.
    file: Option<File>,
}

// Suppress warning on platforms where specialized lock impl is available
#[cfg_attr(all(unix, not(test)), expect(dead_code))]
impl FileLock {
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        tracing::info!("Attempting to lock {path:?}");

        let mut options = OpenOptions::new();
        options.create(true).truncate(false).write(true);

        // On Windows, don't share delete access. This ensures that
        // std::fs::remove_file (which uses DeleteFileW) will fail if any
        // other process has the file open — so deletion in Drop only
        // succeeds when we're the last handle holder.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            const FILE_SHARE_READ: u32 = 1;
            const FILE_SHARE_WRITE: u32 = 2;
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        }

        let file = options.open(&path).map_err(|err| FileLockError {
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
        if let Some(file) = &self.file {
            file.unlock()
                .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to unlock lock file"))
                .ok();
        }
        // On Windows, close the handle first so DeleteFileW can succeed.
        // It will still fail if another process has the file open (they
        // open without FILE_SHARE_DELETE), avoiding any race condition.
        #[cfg(windows)]
        self.file.take();
        std::fs::remove_file(&self.path).ok();
    }
}
