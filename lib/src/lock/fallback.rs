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
    file: File,
}

// Suppress warning on platforms where specialized lock impl is available
#[cfg_attr(all(unix, not(test)), expect(dead_code))]
impl FileLock {
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        tracing::info!("Attempting to lock {path:?}");

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|err| FileLockError {
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
        Ok(Self { path, file })
    }
}

impl Drop for FileLock {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        self.file
            .unlock()
            .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to unlock lock file"))
            .ok();
        // Note: We intentionally don't delete the lock file here. Deleting
        // would cause a race condition where another process waiting
        // for the lock could acquire a lock on the deleted file, while
        // a third process creates a new file with the same path -
        // breaking mutual exclusion. The unix.rs impl handles this with
        // an st_nlink check, but that's not portable.
    }
}
