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

//! Contains the `ReadonlyUserRepo`.

use std::cell::OnceCell;
use std::sync::Arc;

use crate::id_prefix::IdPrefixContext;
use crate::repo::ReadonlyRepo;

/// A ReadonlyRepo along with user-config-dependent derived data. The derived
/// data is lazily loaded.
pub struct ReadonlyUserRepo {
    /// The `ReadonlyRepo` we currently work on.
    // TODO: make private
    pub repo: Arc<ReadonlyRepo>,
    /// The associated `IdPrefixContext`
    // TODO: make private
    pub id_prefix_context: OnceCell<IdPrefixContext>,
}

impl ReadonlyUserRepo {
    /// Create a new `ReadonlyUserRepo` from `repo`.
    pub fn new(repo: Arc<ReadonlyRepo>) -> Self {
        Self {
            repo,
            id_prefix_context: OnceCell::new(),
        }
    }
}
