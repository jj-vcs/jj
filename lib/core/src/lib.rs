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

//! The core library powering the Jujutsu Version Control System. It contains
//! all "base" types such as `Commit` and the `Backend` trait.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![deny(unused_must_use)]

// Needed so that proc macros can be used inside jj_lib and by external crates
// that depend on it.
// See:
// - https://github.com/rust-lang/rust/issues/54647#issuecomment-432015102
// - https://github.com/rust-lang/rust/issues/54363
extern crate self as jj_core;

#[macro_use]
pub mod content_hash;

pub mod backend;
pub mod dsl_util;
pub mod file_util;
pub mod hex_util;
pub mod matchers;
pub mod object_id;
pub mod ref_name;
pub mod repo_path;
pub mod revset;
pub mod revset_parser;
pub mod signing;

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
