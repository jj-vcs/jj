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
//! to `LockFileEx`, with POSIX-semantics delete to avoid "pending delete"
//! races.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::mem;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::PathBuf;

use tracing::instrument;
use windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION;
use windows_sys::Win32::Storage::FileSystem::FILE_DISPOSITION_FLAG_DELETE;
use windows_sys::Win32::Storage::FileSystem::FILE_DISPOSITION_FLAG_POSIX_SEMANTICS;
use windows_sys::Win32::Storage::FileSystem::FILE_DISPOSITION_INFO_EX;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows_sys::Win32::Storage::FileSystem::FileDispositionInfoEx;
use windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle;
use windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle;

use super::FileLockError;

const GENERIC_WRITE: u32 = 0x40000000;
const DELETE: u32 = 0x00010000;

pub struct FileLock {
    path: PathBuf,
    file: File,
}

impl FileLock {
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        tracing::info!("Attempting to lock {path:?}");

        loop {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .access_mode(GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .open(&path)
                .map_err(|err| FileLockError {
                    message: "Failed to open lock file",
                    path: path.clone(),
                    err,
                })?;

            file.lock().map_err(|err| FileLockError {
                message: "Failed to lock lock file",
                path: path.clone(),
                err,
            })?;

            // If the previous holder POSIX-deleted this file, nNumberOfLinks
            // will be 0. We're holding a handle to a deleted inode — retry
            // with a fresh file. (Mirrors the st_nlink check in unix.rs.)
            match get_number_of_links(&file) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(err) => {
                    return Err(FileLockError {
                        message: "Failed to stat lock file",
                        path,
                        err,
                    });
                }
            }

            tracing::info!("Locked {path:?}");
            return Ok(Self { path, file });
        }
    }
}

impl Drop for FileLock {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        // Delete the file while still holding the lock (same order as unix.rs).
        // POSIX delete immediately unlinks the name; waiters that held a handle
        // to the old inode will see nNumberOfLinks == 0 and retry.
        posix_delete(&self.file)
            .or_else(|_| {
                // Fallback for pre-RS1 Windows or non-NTFS.
                std::fs::remove_file(&self.path)
            })
            .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to delete lock file"))
            .ok();
        self.file
            .unlock()
            .inspect_err(|err| tracing::warn!(?err, ?self.path, "Failed to unlock lock file"))
            .ok();
    }
}

fn get_number_of_links(file: &File) -> io::Result<u32> {
    let handle = file.as_raw_handle() as *mut core::ffi::c_void;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.nNumberOfLinks)
}

fn posix_delete(file: &File) -> io::Result<()> {
    let handle = file.as_raw_handle() as *mut core::ffi::c_void;
    let info = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    let ok = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfoEx,
            (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
            mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
