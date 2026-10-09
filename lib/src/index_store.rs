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

//! Interface for persistent storage of commit indexes.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::index::MutableIndex;
use crate::index::ReadonlyIndex;
use crate::operation::Operation;
use crate::store::Store;

/// Returned by [`IndexStore`] in the event of an error.
#[derive(Debug, Error)]
pub enum IndexStoreError {
    /// Error reading a [`ReadonlyIndex`] from the [`IndexStore`].
    #[error("Failed to read index")]
    Read(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Error writing a [`MutableIndex`] to the [`IndexStore`].
    #[error("Failed to write index")]
    Write(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Result of [`IndexStore`] operations.
pub type IndexStoreResult<T> = Result<T, IndexStoreError>;

/// Defines the interface for types that provide persistent storage for an
/// index.
#[async_trait(?Send)]
pub trait IndexStore: Any + Send + Sync + Debug {
    /// Returns a name representing the type of index that the `IndexStore` is
    /// compatible with. For example, the `IndexStore` for the default index
    /// returns "default".
    fn name(&self) -> &str;

    /// Returns the index at the specified operation.
    async fn get_index_at_op(
        &self,
        op: &Operation,
        store: &Arc<Store>,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>>;

    /// Writes `index` to the index store and returns a read-only version of the
    /// index.
    fn write_index(
        &self,
        index: Box<dyn MutableIndex>,
        op: &Operation,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>>;
}

impl dyn IndexStore {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: IndexStore>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}
