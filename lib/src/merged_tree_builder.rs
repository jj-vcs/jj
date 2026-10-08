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

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::MergedTreeValue;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeId;
use crate::conflict_labels::ConflictLabels;
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
#[derive(Debug)]
pub struct MergedTreeBuilder {
    base_tree: MergedTree,
    overrides: BTreeMap<RepoPathBuf, MergedTreeValue>,
}

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

    /// Set an override compared to the base tree. The `values` merge must
    /// either be resolved (i.e. have 1 side) or have the same number of
    /// sides as the `base_tree` used to construct this builder (unless the
    /// base tree is resolved). Unresolved `TreeValue::Tree` merges are not
    /// supported. Use `Merge::absent()` to remove a value from the tree.
    pub fn set_or_remove(
        &mut self,
        path: RepoPathBuf,
        values: MergedTreeValue,
    ) -> BackendResult<()> {
        if values.is_tree() && !values.is_resolved() {
            return Err(BackendError::Other(
                "Unresolved TreeValue::Tree merges are not supported in MergedTreeBuilder".into(),
            ));
        }
        if !values.is_resolved()
            && !self.base_tree.tree_ids().is_resolved()
            && values.num_sides() != self.base_tree.tree_ids().num_sides()
        {
            return Err(BackendError::Other(
                format!(
                    "Override at {} has {} sides, which does not match base tree with {} sides",
                    path.as_internal_file_string(),
                    values.num_sides(),
                    self.base_tree.tree_ids().num_sides(),
                )
                .into(),
            ));
        }
        self.overrides.insert(path, values);
        Ok(())
    }

    /// Create new tree(s) from the base tree(s) and overrides.
    pub async fn write_tree(self) -> BackendResult<MergedTree> {
        let store = self.base_tree.store().clone();
        let labels = self.base_tree.labels().clone();
        let new_tree_ids = if self.overrides.is_empty() {
            self.base_tree.into_tree_ids()
        } else {
            self.write_merged_trees().await?
        };
        if let Some(tree_id) = new_tree_ids.resolve_trivial(store.merge_options().same_change) {
            return Ok(MergedTree::resolved(store, tree_id.clone()));
        }
        let labels = if labels.num_sides() == Some(new_tree_ids.num_sides()) {
            labels
        } else {
            // If the number of sides changed, we need to discard the conflict labels,
            // otherwise `MergedTree::new` would panic.
            // TODO: we should preserve conflict labels when setting conflicted tree values
            // originating from a different tree than the base tree.
            ConflictLabels::unlabeled()
        };
        let (labels, new_tree_ids) = labels.simplify_with(&new_tree_ids);
        Ok(MergedTree::new(store, new_tree_ids, labels))
    }

    async fn write_merged_trees(self) -> BackendResult<Merge<TreeId>> {
        let store = self.base_tree.store().clone();
        let mut base_tree_ids = self.base_tree.into_tree_ids();
        let overrides: Vec<_> = stream::iter(self.overrides)
            .map(async |(path, values)| {
                let values = if values.is_resolved() {
                    values
                } else {
                    resolve_file_values(&store, &path, values).await?
                };
                BackendResult::Ok((path, values))
            })
            .buffered(store.concurrency())
            .try_collect()
            .await?;
        let num_sides = overrides
            .iter()
            .map(|(_, value)| value.num_sides())
            .max()
            .unwrap_or(0)
            .max(base_tree_ids.num_sides());
        let pad_tree_id = base_tree_ids.first().clone();
        base_tree_ids.pad_to(num_sides, &pad_tree_id);
        // Create a single-tree builder for each base tree
        let mut tree_builders =
            base_tree_ids.into_map(|base_tree_id| TreeBuilder::new(store.clone(), base_tree_id));
        for (path, values) in overrides {
            match values.into_resolved() {
                Ok(value) => {
                    // This path was overridden with a resolved value. Apply that to all
                    // builders.
                    for builder in &mut tree_builders {
                        builder.set_or_remove(path.clone(), value.clone());
                    }
                }
                Err(mut values) => {
                    values.pad_to(num_sides, &None);
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
