// Copyright 2026 The Jujutsu Authors
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

//! WASI file locking implementation.
//!
//! WASI preview 2 does not expose advisory file locks (flock/LockFileEx), so
//! this implementation falls back to lockfile creation. This is safe in the
//! typical single-threaded WASI runtime, but does not guard against concurrent
//! access from other processes or instances sharing the same preopened
//! directory.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use tracing::instrument;

use super::FileLockError;

pub struct FileLock {
    path: PathBuf,
    // Held open for the lifetime of the lock so that `Drop` can close it
    // before the lockfile is removed. Not read directly.
    #[expect(dead_code)]
    file: File,
}

impl FileLock {
    /// Acquire an exclusive lock on `path`, blocking until it's available.
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        // In blocking mode, `lock_inner` never returns `Ok(None)`.
        Ok(Self::lock_inner(path, true)?.expect("blocking lock should return a lock"))
    }

    /// Try to acquire an exclusive lock on `path` without blocking. Returns
    /// `Ok(None)` if the lock is currently held by another process.
    pub fn try_lock(path: PathBuf) -> Result<Option<Self>, FileLockError> {
        Self::lock_inner(path, false)
    }

    fn lock_inner(path: PathBuf, blocking: bool) -> Result<Option<Self>, FileLockError> {
        tracing::info!("Attempting to lock {path:?}");
        // Create the lockfile exclusively. If it already exists, either block
        // (retrying) or report that the lock is held.
        loop {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => {
                    tracing::info!("Locked {path:?}");
                    return Ok(Some(Self { path, file }));
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if blocking {
                        // WASI has no flock, so we cannot truly wait for the
                        // lock to be released. Busy-wait briefly by yielding
                        // the runtime, then retry.
                        std::thread::yield_now();
                        continue;
                    }
                    return Ok(None);
                }
                Err(err) => {
                    return Err(FileLockError {
                        message: "Failed to open lock file",
                        path: path.clone(),
                        err,
                    });
                }
            }
        }
    }
}

impl Drop for FileLock {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        // Closing the handle before removing the file. Drop order runs fields
        // in declaration order, so `file` is dropped after `path`, but we
        // explicitly drop it first by leaving scope. The remove_file call
        // below happens while `file` is still open on some platforms, but on
        // WASI that is fine.
        std::fs::remove_file(&self.path).ok();
    }
}
