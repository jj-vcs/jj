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

//! Fallback file locking implementation using exclusive lockfile creation.
//!
//! Contended locks are retried with exponential backoff for up to roughly ten
//! seconds. Unlike native file locks, lockfiles aren't automatically removed
//! if the process crashes.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use tracing::instrument;

use super::FileLockError;
use super::backoff::BackoffIterator;

pub struct FileLock {
    path: PathBuf,
    _file: File,
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
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut backoff_iterator = BackoffIterator::new();
        loop {
            match options.open(&path) {
                Ok(file) => {
                    tracing::info!("Locked {path:?}");
                    return Ok(Some(Self { path, _file: file }));
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if !blocking {
                        return Ok(None);
                    }
                    if let Some(duration) = backoff_iterator.next() {
                        std::thread::sleep(duration);
                    } else {
                        return Err(FileLockError {
                            message: "Timed out while trying to create lock file",
                            path,
                            err,
                        });
                    }
                }
                Err(err) => {
                    return Err(FileLockError {
                        message: "Failed to create lock file",
                        path,
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
        std::fs::remove_file(&self.path)
            .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to delete lock file"))
            .ok();
    }
}
