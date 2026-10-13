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

//! Builder for writing a new tree based on an existing one.

use std::sync::Arc;

use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::merged_tree::MergedTree;
use crate::merged_tree_builder::MergedTreeBuilder;
use crate::repo_path::RepoPathBuf;
use crate::store::Store;

/// Builds a new tree by applying file changes on top of a base tree.
#[derive(Debug)]
pub struct TreeBuilder {
    builder: MergedTreeBuilder,
}

impl TreeBuilder {
    /// Creates a builder with `base_tree_id` as the starting point.
    pub fn new(store: Arc<Store>, base_tree_id: TreeId) -> Self {
        Self {
            builder: MergedTreeBuilder::new(MergedTree::resolved(store, base_tree_id)),
        }
    }

    /// The store the tree will be written to.
    pub fn store(&self) -> &Store {
        self.builder.store()
    }

    /// Sets the value at `path`, which must not be the root.
    pub fn set(&mut self, path: RepoPathBuf, value: TreeValue) {
        self.builder.set_or_remove(path, Some(value));
    }

    /// Removes the entry at `path`, which must not be the root.
    pub fn remove(&mut self, path: RepoPathBuf) {
        self.builder.set_or_remove(path, None);
    }

    /// Sets the value at `path` if `value` is `Some`, otherwise removes it.
    /// `path` must not be the root.
    pub fn set_or_remove(&mut self, path: RepoPathBuf, value: Option<TreeValue>) {
        self.builder.set_or_remove(path, value);
    }

    /// Writes the modified trees to the store and returns the ID of the new
    /// root tree. Directories that become empty are removed.
    pub async fn write_tree(self) -> BackendResult<TreeId> {
        let merged_tree = self.builder.write_tree().await?;
        Ok(merged_tree.into_tree_ids().into_resolved().unwrap())
    }
}
