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

//! File system utilities used by the Jujutsu Version Control System and its
//! on-disk storage backends. This crate is deliberately separate from
//! `jj-core` so that code that doesn't touch the local file system (e.g. a
//! server) doesn't have to depend on it.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![deny(unused_must_use)]

pub mod file_util;
pub mod lock;
pub mod stacked_table;

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    // Copied from `testutils::TestResult` to remove dependency cycle.
    pub type TestResult<T = ()> = eyre::Result<T>;

    /// Unlike `testutils::new_temp_dir()`, this function doesn't set up
    /// hermetic Git environment.
    pub fn new_temp_dir() -> TempDir {
        tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap()
    }
}
