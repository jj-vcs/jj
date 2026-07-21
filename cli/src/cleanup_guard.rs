// Copyright 2023-2026 The Jujutsu Authors
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

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;

use slab::Slab;
use tracing::instrument;

/// Contains the callbacks passed to currently-live [`CleanupGuard`]s
static LIVE_GUARDS: Mutex<GuardTable> = Mutex::new(Slab::new());

type GuardTable = Slab<Box<dyn FnOnce() + Send>>;

/// Prepare to run [`CleanupGuard`]s on `SIGINT`/`SIGTERM`/`SIGHUP`
pub fn init() {
    if let Err(e) = ctrlc::set_handler(|| {
        // We must hold the lock for the remainder of the process's lifetime to avoid a
        // race where a guard is created after we unlock but before we exit.
        let guards = &mut *LIVE_GUARDS.lock().unwrap();
        if let Err(e) = std::panic::catch_unwind(AssertUnwindSafe(|| {
            for guard in guards.drain() {
                guard();
            }
        })) {
            match e.downcast::<String>() {
                Ok(s) => eprintln!("ctrlc handler panicked: {s}"),
                Err(_) => eprintln!("ctrlc handler panicked"),
            }
        }

        #[cfg(feature = "git")]
        gix::tempfile::registry::cleanup_tempfiles();

        std::process::exit(1);
    }) {
        eprintln!("couldn't register signal handler: {e}");
    }
}

/// A drop guard that also runs on `SIGINT`/`SIGTERM`/`SIGHUP`
pub struct CleanupGuard {
    slot: usize,
}

impl CleanupGuard {
    /// Invoke `f` when dropped or killed by `SIGINT`/`SIGTERM`/`SIGHUP`
    pub fn new<F: FnOnce() + Send + 'static>(f: F) -> Self {
        let guards = &mut *LIVE_GUARDS.lock().unwrap();
        Self {
            slot: guards.insert(Box::new(f)),
        }
    }
}

impl Drop for CleanupGuard {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        let guards = &mut *LIVE_GUARDS.lock().unwrap();
        let f = guards.remove(self.slot);
        f();
    }
}
