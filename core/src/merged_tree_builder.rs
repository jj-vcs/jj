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

//! Helps build a new `MergedTree` from a base tree and overrides.

use std::collections::BTreeMap;
use std::iter::zip;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::try_join_all;
use futures::stream;
use thiserror::Error;

use crate::backend::BackendResult;
use crate::backend::MergedTreeValue;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPathBuf;
use crate::tree_builder::TreeBuilder;
use crate::tree_merge::resolve_file_values;

/// Helper for writing trees with conflicts.
///
/// You start by creating an instance of this type with one or more
/// base trees. You then add overrides on top. The overrides may be
/// conflicts. Then you can write the result as a merge of trees.
///
/// If there are overrides both at a path and below it, the override at the
/// path takes precedence if it's not a tree. Otherwise, the overrides below it
/// are applied to the tree at the path, or to an empty directory if the path is
/// removed. If there are overrides only below a path that isn't a tree in the
/// base tree, the value at the path is replaced by a directory unless the
/// overrides below it only remove paths.
#[derive(Debug)]
pub struct MergedTreeBuilder {
    base_tree: MergedTree,
    overrides: BTreeMap<RepoPathBuf, MergedTreeValue>,
}

/// Error returned when attempting to set an override with a conflict arity that
/// does not match the base tree.
#[derive(Debug, Error)]
#[error("Conflict arity mismatch")]
pub struct ConflictArityMismatchError;

impl MergedTreeBuilder {
    /// Create a new builder with the given trees as base.
    ///
    /// The `base_tree` is assumed to already have its conflicts resolved (e.g.
    /// via `MergedTree::merge()` or `MergedTree::resolve()`), as `write_tree()`
    /// only resolves overridden paths and simplifies root tree IDs.
    pub fn new(base_tree: MergedTree) -> Self {
        Self {
            base_tree,
            overrides: BTreeMap::new(),
        }
    }

    /// Set an override compared to the base tree. Use `None` to remove a
    /// value from the tree.
    pub fn set_or_remove(&mut self, path: RepoPathBuf, value: Option<TreeValue>) {
        assert!(!path.is_root());
        self.overrides.insert(path, Merge::resolved(value));
    }

    /// Set an override compared to the base tree. The `values` merge must
    /// either be resolved (i.e. have 1 side) or have the same number of
    /// sides as the base tree used to construct this builder. Conflicted
    /// values should also be derived from the base tree's conflict. To
    /// combine paths from a differently conflicted tree, merge the trees
    /// instead (e.g. using `MergedTree::filtered()` and
    /// `MergedTree::merge()`). Unresolved `TreeValue::Tree` merges are not
    /// supported. Use `Merge::absent()` to remove a value from the tree.
    pub fn set_or_remove_merge(
        &mut self,
        path: RepoPathBuf,
        values: MergedTreeValue,
    ) -> Result<(), ConflictArityMismatchError> {
        assert!(!path.is_root());
        assert!(values.is_resolved() || !values.is_tree());
        if !(values.is_resolved() || values.num_sides() == self.base_tree.tree_ids().num_sides()) {
            return Err(ConflictArityMismatchError);
        }
        self.overrides.insert(path, values);
        Ok(())
    }

    /// Create new tree(s) from the base tree(s) and overrides.
    pub async fn write_tree(self) -> BackendResult<MergedTree> {
        if self.overrides.is_empty() {
            return Ok(self.base_tree);
        }
        let store = self.base_tree.store().clone();
        let labels = self.base_tree.labels().clone();
        let new_tree_ids = self.write_merged_trees().await?;
        if let Some(tree_id) = new_tree_ids.resolve_trivial(store.merge_options().same_change) {
            return Ok(MergedTree::resolved(store, tree_id.clone()));
        }
        let (labels, new_tree_ids) = labels.simplify_with(&new_tree_ids);
        Ok(MergedTree::new(store, new_tree_ids, labels))
    }

    async fn write_merged_trees(mut self) -> BackendResult<Merge<TreeId>> {
        let store = self.base_tree.store().clone();
        let base_tree_ids = self.base_tree.into_tree_ids();
        let mut unresolved = Vec::new();
        for (path, values) in &mut self.overrides {
            if !values.is_resolved() {
                let store = &store;
                let old_values = std::mem::replace(values, Merge::absent());
                unresolved.push(async move {
                    *values = resolve_file_values(store, path, old_values).await?;
                    BackendResult::Ok(())
                });
            }
        }
        stream::iter(unresolved)
            .buffer_unordered(store.concurrency())
            .try_collect::<()>()
            .await?;
        // Create a single-tree builder for each base tree
        let mut tree_builders =
            base_tree_ids.into_map(|base_tree_id| TreeBuilder::new(store.clone(), base_tree_id));
        for (path, values) in self.overrides {
            match values.into_resolved() {
                Ok(value) => {
                    // This path was overridden with a resolved value. Apply that to all
                    // builders.
                    for builder in &mut tree_builders {
                        builder.set_or_remove(path.clone(), value.clone());
                    }
                }
                Err(values) => {
                    // This path was overridden with a conflicted value. Apply each term to
                    // its corresponding builder.
                    for (builder, value) in zip(&mut tree_builders, values) {
                        builder.set_or_remove(path.clone(), value);
                    }
                }
            }
        }
        // TODO: This can be made more efficient. If there's a single resolved conflict
        // in `dir/file`, we shouldn't have to write the `dir/` and root trees more than
        // once.
        let tree_ids = try_join_all(
            tree_builders
                .into_iter()
                .map(|builder| builder.write_tree()),
        )
        .await?;
        let merge = Merge::from_vec(tree_ids);
        Ok(merge)
    }
}
