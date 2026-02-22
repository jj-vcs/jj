use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::io::FromRawHandle;
use std::os::windows::io::OwnedHandle;
use std::path::PathBuf;
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_DELETE_ON_CLOSE;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows_sys::Win32::Storage::FileSystem::LOCKFILE_EXCLUSIVE_LOCK;
use windows_sys::Win32::Storage::FileSystem::LockFileEx;
use windows_sys::Win32::Storage::FileSystem::OPEN_ALWAYS;
use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::lock::FileLockError;

pub struct FileLock {
    _handle: OwnedHandle,
}

impl FileLock {
    pub fn lock(path: PathBuf) -> Result<Self, FileLockError> {
        let mut path_wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        path_wide.push(0);
        unsafe {
            let mut handle;
            let mut attempts = 0;
            loop {
                // Safety: `path_wide` is a valid, null-terminated wide string
                handle = CreateFileW(
                    path_wide.as_ptr(),
                    // `LockFileEx` requires read or write access
                    GENERIC_READ,
                    // Allow concurrent access attempts to open the lockfile so they can wait on it
                    FILE_SHARE_READ | FILE_SHARE_DELETE,
                    ptr::null(),
                    // Open an existing file or create a new one if none exists
                    OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_DELETE_ON_CLOSE,
                    ptr::null_mut(),
                );
                if handle != INVALID_HANDLE_VALUE {
                    break;
                }
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_ACCESS_DENIED as _) && attempts < 10 {
                    // For unfathomable reasons, deletes aren't atomic and we may have raced with
                    // one. Try again later.
                    std::thread::sleep(Duration::from_millis(100));
                    attempts += 1;
                    continue;
                }
                return Err(FileLockError {
                    message: "Failed to open lock file",
                    path: path,
                    err,
                });
            }
            // Safety: `CreateFileW` is guaranteed to return either INVALID_HANDLE_VALUE or
            // a valid handle, and we excluded the invalid case above.
            let handle = OwnedHandle::from_raw_handle(handle);
            // We use an explicit `LockFileEx` call, rather than playing games with share
            // mode in `CreateFileW`, so that we can block.
            //
            // Safety: `handle` is valid
            let locked = LockFileEx(
                handle.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK,
                0,
                // We must pass some consistent nonempty byte range to get any actual locking
                1,
                0,
                // Zeroed `OVERLAPPED` structure means we wait synchronously
                &mut OVERLAPPED::default(),
            );
            if locked == 0 {
                return Err(FileLockError {
                    message: "Failed to lock lock file",
                    path: path,
                    err: io::Error::last_os_error(),
                });
            }
            Ok(Self { _handle: handle })
        }
    }
}
