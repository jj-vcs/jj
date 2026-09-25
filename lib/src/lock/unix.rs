// Copyright 2023 The Jujutsu Authors
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

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::thread;

use rustix::fs::FlockOperation;
use tracing::instrument;

use super::FileLockError;
use super::backoff::BackoffIterator;

pub struct FileLock {
    path: PathBuf,
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
        let operation = if blocking {
            FlockOperation::LockExclusive
        } else {
            FlockOperation::NonBlockingLockExclusive
        };
        let mut backoff_iterator = BackoffIterator::new();
        loop {
            let file = match open_lock_file(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    // Another lock holder may have removed the lock path
                    // between our create-or-observe step and our reopen step.
                    // Retry so we do not report a transient deletion as a
                    // lock acquisition failure.
                    let Some(duration) = backoff_iterator.next() else {
                        return Err(FileLockError {
                            message: "Failed to open lock file",
                            path: path.clone(),
                            err,
                        });
                    };
                    thread::sleep(duration);
                    continue;
                }
                Err(err) => {
                    return Err(FileLockError {
                        message: "Failed to open lock file",
                        path: path.clone(),
                        err,
                    });
                }
            };
            // If the lock was already held, block until it's released, or (in
            // non-blocking mode) report that it's currently unavailable.
            match rustix::fs::flock(&file, operation) {
                Ok(()) => {}
                Err(rustix::io::Errno::WOULDBLOCK) if !blocking => return Ok(None),
                Err(errno) => {
                    return Err(FileLockError {
                        message: "Failed to lock lock file",
                        path: path.clone(),
                        err: errno.into(),
                    });
                }
            }

            match rustix::fs::fstat(&file) {
                Ok(stat) => {
                    if stat.st_nlink == 0 {
                        // Lockfile was deleted, probably by the previous holder's `Drop` impl;
                        // create a new one so our ownership is visible,
                        // rather than hidden in an unlinked file. Not
                        // always necessary, since the previous holder might
                        // have exited abruptly.
                        continue;
                    }
                }
                Err(rustix::io::Errno::STALE) => {
                    // The file handle is stale.
                    // This can happen when using NFS,
                    // likely caused by a remote deletion of the lockfile.
                    // Treat this like a normal lockfile deletion and retry.
                    continue;
                }
                Err(errno) => {
                    return Err(FileLockError {
                        message: "failed to stat lock file",
                        path: path.clone(),
                        err: errno.into(),
                    });
                }
            }

            tracing::info!("Locked {path:?}");
            return Ok(Some(Self { path, file }));
        }
    }
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(created_file) => {
            // Docker for Mac's `fakeowner` layer over VirtioFS has allowed two
            // exclusive flock holders when the first holder creates a missing
            // lock file and then flocks the same descriptor. See:
            // https://github.com/docker/for-mac/issues/7004
            //
            // The failure matters when a jj repository lives on a host
            // directory bind-mounted into a Docker for Mac container. A fresh
            // short-lived lock path is often missing because FileLock removes
            // it on drop, so the next process is likely to be the process that
            // creates the file. Taking flock on that creation descriptor can
            // fail to exclude a concurrent process on affected `fakeowner`
            // mounts.
            //
            // On Linux, limit the workaround to filesystem implementations
            // which may be affected. Other Unix systems use the creation
            // descriptor directly.
            if creation_descriptor_may_have_broken_flock(&created_file) {
                // Close the creation descriptor explicitly before reopening the
                // file. The working probe showed that the create-close-open
                // sequence preserves mutual exclusion on affected Docker for Mac
                // bind mounts.
                drop(created_file);
                OpenOptions::new().write(true).open(path)
            } else {
                Ok(created_file)
            }
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            // The lock file already exists, so no creation descriptor was
            // involved in this process. Opening the existing file directly is
            // the pattern that works on the affected `fakeowner` mounts.
            OpenOptions::new().write(true).open(path)
        }
        Err(err) => Err(err),
    }
}

/// Checks whether `file` may be affected by Docker Desktop's `fakeowner` bug.
///
/// The creation and existing-file open paths can use different inode layers,
/// allowing both descriptors to acquire an exclusive flock. See
/// https://github.com/docker/for-mac/issues/7004.
#[cfg(target_os = "linux")]
fn creation_descriptor_may_have_broken_flock(file: &File) -> bool {
    // The undocumented magic value reported by Docker Desktop's out-of-tree
    // `fakeowner` ownership wrapper, which may be placed above VirtioFS. It is
    // not a guaranteed interface and is only a best-effort Docker Desktop signal.
    const DOCKER_DESKTOP_FAKEOWNER_MAGIC: u64 = 0x6a65_6a63;

    match rustix::fs::fstatfs(file) {
        // The suspected bug is in Docker Desktop's `fakeowner` layer, not the
        // underlying VirtioFS filesystem. A raw VirtioFS mount does not exhibit
        // the broken locking behavior described in the linked report above.
        Ok(stat) => stat.f_type as u64 == DOCKER_DESKTOP_FAKEOWNER_MAGIC,
        // A false negative could restore the broken locking behavior. Detection
        // failure is unusual, so retain the safe workaround in that case.
        Err(_) => true,
    }
}

#[cfg(not(target_os = "linux"))]
fn creation_descriptor_may_have_broken_flock(_file: &File) -> bool {
    // The known broken behavior is limited to Linux guests in Docker Desktop.
    false
}

impl Drop for FileLock {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        // Removing the file isn't strictly necessary, but reduces confusion.
        std::fs::remove_file(&self.path).ok();
        // Unblock any processes that tried to acquire the lock while we held it.
        // They're responsible for creating and locking a new lockfile, since we
        // just deleted this one.
        rustix::fs::flock(&self.file, FlockOperation::Unlock).ok();
    }
}
