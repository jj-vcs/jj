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

use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::conflict_labels::ConflictLabels;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::merge::MergedTreeValue;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPathBuf;
use crate::tree_builder::TreeBuilder;

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
    pub fn new(base_tree: MergedTree) -> Self {
        Self {
            base_tree,
            overrides: BTreeMap::new(),
        }
    }

    /// Set an override compared to  the base tree. The `values` merge must
    /// either be resolved (i.e. have 1 side) or have the same number of
    /// sides as the `base_tree_ids` used to construct this builder. Use
    /// `Merge::absent()` to remove a value from the tree.
    pub fn set_or_remove(&mut self, path: RepoPathBuf, values: MergedTreeValue) {
        self.overrides.insert(path, values);
    }

    /// Create new tree(s) from the base tree(s) and overrides.
    pub fn write_tree(self) -> BackendResult<MergedTree> {
        let store = self.base_tree.store().clone();
        let labels = self.base_tree.labels().clone();
        let new_tree_ids = self.write_merged_trees()?;
        match new_tree_ids.simplify().into_resolved() {
            Ok(single_tree_id) => Ok(MergedTree::resolved(store, single_tree_id)),
            Err(tree_ids) => {
                let labels = if labels.num_sides() == Some(tree_ids.num_sides()) {
                    labels
                } else {
                    // If the number of sides changed, we need to discard the conflict labels,
                    // otherwise `MergedTree::new` would panic.
                    // TODO: we should preserve conflict labels when setting conflicted tree values
                    // originating from a different tree than the base tree.
                    ConflictLabels::unlabeled()
                };
                let tree = MergedTree::new(store, tree_ids, labels);
                tree.resolve().block_on()
            }
        }
    }

    fn write_merged_trees(self) -> BackendResult<Merge<TreeId>> {
        let store = self.base_tree.store().clone();
        let mut base_tree_ids = self.base_tree.into_tree_ids();
        let num_sides = self
            .overrides
            .values()
            .map(|value| value.num_sides())
            .max()
            .unwrap_or(0);
        base_tree_ids.pad_to(num_sides, store.empty_tree_id());
        // Create a single-tree builder for each base tree
        let mut tree_builders =
            base_tree_ids.map(|base_tree_id| TreeBuilder::new(store.clone(), base_tree_id.clone()));
        for (path, values) in self.overrides {
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
        let merge_builder: MergeBuilder<TreeId> = tree_builders
            .into_iter()
            .map(|builder| builder.write_tree())
            .try_collect()?;
        Ok(merge_builder.build())
    }
}
