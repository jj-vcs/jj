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

//! Configuration for working with revsets and filesets in a workspace.

use jj_lib::config::ConfigGetError;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetParseError;
use thiserror::Error;

/// Error from loading a workspace environment.
#[derive(Debug, Error)]
pub enum WorkspaceEnvironmentError {
    /// A config value couldn't be read.
    #[error(transparent)]
    Config(#[from] ConfigGetError),
    /// The configured `immutable_heads()` couldn't be parsed.
    #[error("Invalid `revset-aliases.immutable_heads()`")]
    InvalidImmutableHeads(#[source] RevsetParseError),
    /// The configured `revsets.short-prefixes` couldn't be parsed.
    #[error("Invalid `revsets.short-prefixes`")]
    InvalidShortPrefixes(#[source] RevsetParseError),
    /// The Git repo at the workspace root couldn't be opened.
    #[cfg(feature = "git")]
    #[error(transparent)]
    GitRepoAtWorkdir(#[from] jj_lib::git_backend::GitRepoAtWorkdirError),
}

/// Non-fatal problems found while loading a workspace environment.
#[derive(Debug)]
pub struct WorkspaceEnvironmentDiagnostics {
    /// Diagnostics from parsing `revset-aliases.immutable_heads()`.
    pub immutable_heads: RevsetDiagnostics,
    /// Diagnostics from parsing `revsets.short-prefixes`.
    pub short_prefixes: RevsetDiagnostics,
}

impl WorkspaceEnvironmentDiagnostics {
    /// Creates empty diagnostics.
    pub fn new() -> Self {
        Self {
            immutable_heads: RevsetDiagnostics::new(),
            short_prefixes: RevsetDiagnostics::new(),
        }
    }
}

impl Default for WorkspaceEnvironmentDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}
