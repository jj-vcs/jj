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
use std::sync::Arc;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::try_join_all;
use futures::stream;
use futures::stream::FuturesUnordered;
use itertools::Itertools as _;
use thiserror::Error;

use crate::backend;
use crate::backend::BackendResult;
use crate::backend::MergedTreeValue;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::repo_path::RepoPathTree;
use crate::store::Store;
use crate::tree::Tree;
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

/// Error returned when attempting to set an override with a conflict arity that
/// does not match the base tree.
#[derive(Debug, Error)]
#[error("Conflict arity mismatch")]
pub struct ConflictArityMismatchError;

fn collapse_if_all_equal<T: PartialEq>(merge: Merge<T>) -> Merge<T> {
    if !merge.is_resolved() && merge.iter().all_equal() {
        Merge::resolved(merge.into_iter().next().unwrap())
    } else {
        merge
    }
}

fn update_tree_entries<T: Clone>(
    tree_entries: &mut Merge<BTreeMap<RepoPathComponentBuf, TreeValue>>,
    num_sides: usize,
    values: Merge<T>,
    mut apply: impl FnMut(&mut BTreeMap<RepoPathComponentBuf, TreeValue>, T),
) {
    match values.into_resolved() {
        Ok(value) => {
            for entries in tree_entries.iter_mut() {
                apply(entries, value.clone());
            }
        }
        Err(values) => {
            if let Some(entries) = tree_entries.as_resolved() {
                *tree_entries = Merge::repeated(entries.clone(), num_sides);
            }
            for (entries, value) in zip(tree_entries.iter_mut(), values) {
                apply(entries, value);
            }
        }
    }
}

async fn try_map_unique_async<T, U, F, Fut>(merge: Merge<T>, mut f: F) -> BackendResult<Merge<U>>
where
    T: PartialEq,
    U: Clone,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = BackendResult<U>>,
{
    let merge = match merge.into_resolved() {
        Ok(item) => return Ok(Merge::resolved(f(item).await?)),
        Err(merge) => merge,
    };
    let mut unique_inputs: Vec<T> = Vec::new();
    let mut indices: Vec<usize> = Vec::with_capacity(merge.iter().len());
    for item in merge {
        let idx = unique_inputs
            .iter()
            .position(|prev| *prev == item)
            .unwrap_or_else(|| {
                let idx = unique_inputs.len();
                unique_inputs.push(item);
                idx
            });
        indices.push(idx);
    }
    let outputs = try_join_all(unique_inputs.into_iter().map(&mut f)).await?;
    Ok(Merge::from_vec(
        indices
            .into_iter()
            .map(|idx| outputs[idx].clone())
            .collect_vec(),
    ))
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

    /// The store the tree will be written to.
    pub fn store(&self) -> &Arc<Store> {
        self.base_tree.store()
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
        let num_sides = base_tree_ids.num_sides();
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

        let mut trees_to_write =
            Self::get_base_trees(&store, base_tree_ids, &self.overrides).await?;

        // Update entries in parent trees for file overrides
        for (path, values) in self.overrides {
            let (dir, basename) = path.split().unwrap();
            let tree_entries = trees_to_write.get_mut(dir).unwrap();
            update_tree_entries(tree_entries, num_sides, values, |entries, value| {
                if let Some(value) = value {
                    entries.insert(basename.to_owned(), value);
                } else {
                    entries.remove(basename);
                }
            });
        }

        // Write trees in reverse lexicographical order, starting with trees without
        // children.
        // TODO: Writing trees concurrently should help on high-latency backends
        while let Some((dir, cur_entries_merge)) = trees_to_write.pop_last() {
            let cur_entries_merge = collapse_if_all_equal(cur_entries_merge);
            if let Some((parent, basename)) = dir.split() {
                let new_tree_ids =
                    Self::write_cur_trees(&store, &dir, cur_entries_merge, false).await?;
                let parent_entries = trees_to_write.get_mut(parent).unwrap();
                update_tree_entries(
                    parent_entries,
                    num_sides,
                    new_tree_ids,
                    |entries, opt_tree_id| {
                        if let Some(tree_id) = opt_tree_id {
                            entries.insert(basename.to_owned(), TreeValue::Tree(tree_id));
                        } else if let Some(TreeValue::Tree(_)) = entries.get(basename) {
                            entries.remove(basename);
                        }
                    },
                );
            } else {
                // We're writing the root tree. Write it even if empty. Return its ids.
                assert!(trees_to_write.is_empty());
                let root_tree_ids =
                    Self::write_cur_trees(&store, &dir, cur_entries_merge, true).await?;
                return Ok(root_tree_ids.into_map(Option::unwrap));
            }
        }

        unreachable!("trees_to_write must contain the root tree");
    }

    async fn write_cur_trees(
        store: &Arc<Store>,
        dir: &RepoPath,
        cur_entries_merge: Merge<BTreeMap<RepoPathComponentBuf, TreeValue>>,
        write_empty: bool,
    ) -> BackendResult<Merge<Option<TreeId>>> {
        try_map_unique_async(cur_entries_merge, async |cur_entries| {
            if cur_entries.is_empty() && !write_empty {
                Ok(None)
            } else {
                let data = backend::Tree::from_sorted_entries(cur_entries.into_iter().collect());
                let tree = store.write_tree(dir, data).await?;
                Ok(Some(tree.id().clone()))
            }
        })
        .await
    }

    async fn read_trees(
        store: &Arc<Store>,
        dir: RepoPathBuf,
        tree_ids: Merge<Option<TreeId>>,
    ) -> BackendResult<Merge<Tree>> {
        let tree_ids = collapse_if_all_equal(tree_ids);
        try_map_unique_async(tree_ids, async |opt_id| match opt_id {
            Some(id) => store.get_tree(dir.clone(), &id).await,
            None => Ok(Tree::empty(store.clone(), dir.clone())),
        })
        .await
    }

    async fn get_base_trees(
        store: &Arc<Store>,
        base_tree_ids: Merge<TreeId>,
        overrides: &BTreeMap<RepoPathBuf, MergedTreeValue>,
    ) -> BackendResult<BTreeMap<RepoPathBuf, Merge<BTreeMap<RepoPathComponentBuf, TreeValue>>>>
    {
        let mut needed_dirs: RepoPathTree<()> = RepoPathTree::default();
        for path in overrides.keys() {
            if let Some(dir) = path.parent() {
                needed_dirs.add(dir);
            }
        }

        let mut tree_reads = FuturesUnordered::new();

        // Schedule reading the root tree
        let root_tree_ids = base_tree_ids.into_map(Some);
        tree_reads.push(
            async move {
                let root_dir = RepoPathBuf::root();
                let trees = Self::read_trees(store, root_dir.clone(), root_tree_ids).await?;
                BackendResult::Ok((root_dir, trees))
            }
            .boxed(),
        );

        let mut base_trees = BTreeMap::new();
        while let Some((dir, trees)) = tree_reads.try_next().await? {
            if let Some(node) = needed_dirs.get(&dir) {
                for (basename, _child) in node.children() {
                    let basename = basename.to_owned();
                    let sub_dir = dir.join(&basename);
                    let tree_id_at = |tree: &Tree| match tree.value(&basename) {
                        Some(TreeValue::Tree(id)) => Some(id.clone()),
                        _ => None,
                    };
                    let sub_tree_ids = match overrides.get(&sub_dir) {
                        None => trees.map(tree_id_at),
                        Some(ov) => ov.map(|term| match term {
                            Some(TreeValue::Tree(id)) => Some(id.clone()),
                            _ => None,
                        }),
                    };
                    tree_reads.push(
                        async move {
                            let sub_trees =
                                Self::read_trees(store, sub_dir.clone(), sub_tree_ids).await?;
                            BackendResult::Ok((sub_dir, sub_trees))
                        }
                        .boxed(),
                    );
                }
            }
            let entries_merge = trees.map(|tree| {
                tree.data()
                    .entries()
                    .map(|entry| (entry.name().to_owned(), entry.value().clone()))
                    .collect()
            });
            base_trees.insert(dir, entries_merge);
        }

        Ok(base_trees)
    }
}
