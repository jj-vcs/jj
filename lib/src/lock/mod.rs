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

#![expect(missing_docs)]

#[cfg(windows)]
mod backoff;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "wasi")]
mod wasi;
#[cfg(windows)]
mod windows;

use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[cfg(unix)]
pub use self::unix::FileLock;
#[cfg(target_os = "wasi")]
pub use self::wasi::FileLock;
#[cfg(windows)]
pub use self::windows::FileLock;

#[derive(Debug, Error)]
#[error("{message}: {path}")]
pub struct FileLockError {
    pub message: &'static str,
    pub path: PathBuf,
    #[source]
    pub err: io::Error,
}

#[cfg(test)]
mod tests {
    use std::cmp::max;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::tests::new_temp_dir;

    #[test]
    fn lock_basic() {
        let temp_dir = new_temp_dir();
        let lock_path = temp_dir.path().join("test.lock");
        assert!(!lock_path.exists());
        {
            let _lock = FileLock::lock(lock_path.clone()).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists());
    }

    #[test]
    fn lock_concurrent() {
        let temp_dir = new_temp_dir();
        let data_path = temp_dir.path().join("test");
        let lock_path = temp_dir.path().join("test.lock");
        fs::write(&data_path, 0_u32.to_le_bytes()).unwrap();
        let num_threads = max(num_cpus::get(), 4);
        thread::scope(|s| {
            for _ in 0..num_threads {
                s.spawn(|| {
                    let _lock = FileLock::lock(lock_path.clone()).unwrap();
                    let data = fs::read(&data_path).unwrap();
                    let value = u32::from_le_bytes(data.try_into().unwrap());
                    thread::sleep(Duration::from_millis(1));
                    fs::write(&data_path, (value + 1).to_le_bytes()).unwrap();
                });
            }
        });
        let data = fs::read(&data_path).unwrap();
        let value = u32::from_le_bytes(data.try_into().unwrap());
        assert_eq!(value, num_threads as u32);
    }

    #[test]
    fn try_lock_succeeds_when_unlocked() {
        let temp_dir = new_temp_dir();
        let lock_path = temp_dir.path().join("test.lock");
        assert!(!lock_path.exists());
        {
            let lock = FileLock::try_lock(lock_path.clone()).unwrap();
            assert!(lock.is_some());
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists());
    }

    #[test]
    fn try_lock_gives_up_when_locked() {
        let temp_dir = new_temp_dir();
        let lock_path = temp_dir.path().join("test.lock");
        let _held = FileLock::lock(lock_path.clone()).unwrap();
        // The lock is already held, so a non-blocking attempt returns `None`
        // instead of blocking. The two lock handles are independent even within
        // a single process, so the second attempt is denied.
        assert!(FileLock::try_lock(lock_path).unwrap().is_none());
    }
}
