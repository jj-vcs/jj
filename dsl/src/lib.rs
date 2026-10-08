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

//! Parsers for the domain-specific languages used by Jujutsu: revsets,
//! filesets, and templates. This crate only deals with syntax (parsing,
//! alias expansion, diagnostics) and doesn't need a repository, which makes
//! it suitable for e.g. editor integrations.

#![warn(missing_docs)]
#![forbid(unsafe_code)]
#![deny(unused_must_use)]

pub mod dsl_util;
pub mod fileset_parser;
pub mod revset_parser;
pub mod template_parser;

#[cfg(test)]
mod tests {
    // Copied from `testutils::TestResult` to remove dependency cycle.
    pub type TestResult<T = ()> = eyre::Result<T>;
}
