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

//! Writes a version of a given `MergedTree` that's filtered by a `Matcher`.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use itertools::Itertools as _;

use crate::backend;
use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::conflict_labels::ConflictLabels;
use crate::matchers::Matcher;
use crate::matchers::Visit;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::store::Store;
use crate::tree::Tree;

/// Writes a version of a given `MergedTree` that's filtered by a `Matcher`.
pub struct FilteredTreeWriter<'matcher> {
    store: Arc<Store>,
    tree_ids: Merge<TreeId>,
    labels: ConflictLabels,
    matcher: &'matcher dyn Matcher,
    work: FuturesUnordered<BoxFuture<'static, FilteredTreeWorkItem>>,
    scheduled_trees: HashSet<(RepoPathBuf, TreeId)>,
}

#[derive(Debug)]
enum FilteredTreeWorkItem {
    // A base tree that has been read. Some part of it will (usually) be included in the
    // result.
    ReadTree {
        result: BackendResult<Tree>,
    },
    // A filtered tree that has been written. The tree ID of the filtered tree is
    // `tree_id`.
    WrittenTree {
        dir: RepoPathBuf,
        // The old tree this tree is a filtered version of
        old_tree_id: TreeId,
        result: BackendResult<TreeId>,
    },
}

impl<'matcher> FilteredTreeWriter<'matcher> {
    /// Creates a `FilteredTreeWriter` for writing a filtered version of `tree`
    /// using `matcher`.
    pub fn new(tree: &MergedTree, matcher: &'matcher dyn Matcher) -> Self {
        Self {
            store: tree.store().clone(),
            tree_ids: tree.tree_ids().clone(),
            labels: tree.labels().clone(),
            matcher,
            work: FuturesUnordered::new(),
            scheduled_trees: HashSet::new(),
        }
    }

    /// Writes the filtered tree and returns it.
    pub async fn write(mut self) -> BackendResult<MergedTree> {
        // Fast paths for when the matcher includes or excludes everything
        if self.matcher.visit(RepoPath::root()) == Visit::AllRecursively {
            return Ok(MergedTree::new(
                self.store.clone(),
                self.tree_ids.clone(),
                self.labels.clone(),
            ));
        }
        if self.matcher.visit(RepoPath::root()).is_nothing() {
            let tree_ids = self.tree_ids.map(|_| self.store.empty_tree_id().clone());
            return Ok(MergedTree::new(
                self.store.clone(),
                tree_ids,
                self.labels.clone(),
            ));
        }

        // Start by reading the base trees for the root directory
        for tree_id in self.tree_ids.clone().into_iter().unique() {
            self.schedule_read(RepoPathBuf::root(), tree_id);
        }

        // Map from old path and tree ID to new tree ID
        let mut filtered_trees: HashMap<(RepoPathBuf, TreeId), TreeId> = HashMap::new();

        struct UnfinishedTree {
            entries: BTreeMap<RepoPathComponentBuf, TreeValue>,
            remaining_entries: usize,
        }
        let mut unfinished_trees: HashMap<(RepoPathBuf, TreeId), UnfinishedTree> = HashMap::new();
        // Parent trees to update when we have finished writing a subtree
        let mut parent_trees: HashMap<(RepoPathBuf, TreeId), Vec<TreeId>> = HashMap::new();

        while let Some(item) = self.work.next().await {
            match item {
                FilteredTreeWorkItem::ReadTree { result } => {
                    let tree = result?;
                    // We should not attempt to read the same tree multiple times
                    debug_assert!(
                        !filtered_trees.contains_key(&(tree.dir().to_owned(), tree.id().clone())),
                        "Requested the same tree {}@{} multiple times",
                        tree.id(),
                        tree.dir().as_internal_file_string()
                    );

                    let mut new_entries = Vec::new();
                    let mut remaining_entries = 0;
                    let mut any_skipped = false;
                    for entry in tree.entries_non_recursive() {
                        let entry_path = tree.dir().join(entry.name());
                        if let TreeValue::Tree(sub_tree_id) = entry.value() {
                            let visit = self.matcher.visit(&entry_path);
                            if visit.is_nothing() {
                                // Skip fully excluded subtrees
                                any_skipped = true;
                            } else if visit == Visit::AllRecursively {
                                // Include entry for fully included subtees
                                new_entries.push((entry.name().to_owned(), entry.value().clone()));
                            } else {
                                // Otherwise, we need to read the tree
                                let subdir_path = tree.dir().join(entry.name());
                                let subdir_key = (subdir_path, sub_tree_id.clone());
                                if let Some(new_subdir_tree_id) = filtered_trees.get(&subdir_key) {
                                    // We already have the filtered subtree for this entry. This can
                                    // happen when current tree is conflicted but the subtree is not
                                    // and we processed the subtree when we visited another side of
                                    // the current conflict.
                                    new_entries.push((
                                        entry.name().to_owned(),
                                        TreeValue::Tree(new_subdir_tree_id.clone()),
                                    ));
                                } else {
                                    self.schedule_read(subdir_key.0.clone(), sub_tree_id.clone());
                                    parent_trees
                                        .entry(subdir_key)
                                        .or_default()
                                        .push(tree.id().clone());
                                    remaining_entries += 1;
                                }
                            }
                        } else if self.matcher.matches(&entry_path) {
                            // Include matched entry
                            new_entries.push((entry.name().to_owned(), entry.value().clone()));
                        } else {
                            // Skip unmatched entry
                            any_skipped = true;
                        }
                    }
                    if !any_skipped {
                        // TODO: add a ready() future instead of writing this
                        // tree (since it already exists). But make sure to
                        // check self.scheduled_trees first, since we might have
                        // already scheduled a write for this tree if it was the
                        // result of filtering a conflicted tree.
                    }
                    if remaining_entries > 0 {
                        unfinished_trees.insert(
                            (tree.dir().to_owned(), tree.id().clone()),
                            UnfinishedTree {
                                entries: new_entries.into_iter().collect(),
                                remaining_entries,
                            },
                        );
                    } else {
                        let new_tree = backend::Tree::from_sorted_entries(new_entries);
                        self.schedule_write(tree.dir().to_owned(), tree.id().clone(), new_tree);
                    }
                }
                FilteredTreeWorkItem::WrittenTree {
                    dir,
                    old_tree_id,
                    result: tree_id,
                } => {
                    let tree_id = tree_id?;

                    // Add this entry to the parent tree(s) (unless this is a root tree)
                    if let Some((parent_dir, basename)) = dir.split() {
                        let old_parent_tree_ids = parent_trees
                            .remove(&(dir.clone(), old_tree_id.clone()))
                            .unwrap();
                        for old_parent_tree_id in old_parent_tree_ids {
                            let key = (parent_dir.to_owned(), old_parent_tree_id.clone());
                            let unfinished_tree = unfinished_trees.get_mut(&key).unwrap();
                            unfinished_tree
                                .entries
                                .insert(basename.to_owned(), TreeValue::Tree(tree_id.clone()));
                            unfinished_tree.remaining_entries -= 1;
                            if unfinished_tree.remaining_entries == 0 {
                                let unfinished_tree = unfinished_trees.remove(&key).unwrap();
                                let new_tree = backend::Tree::from_sorted_entries(
                                    unfinished_tree.entries.into_iter().collect(),
                                );
                                self.schedule_write(
                                    parent_dir.to_owned(),
                                    old_parent_tree_id,
                                    new_tree,
                                );
                            }
                        }
                    }

                    filtered_trees.insert((dir, old_tree_id), tree_id);
                }
            }
        }

        let tree_ids = Merge::from_vec(
            self.tree_ids
                .iter()
                .map(|tree_id| filtered_trees[&(RepoPathBuf::root(), tree_id.clone())].clone())
                .collect_vec(),
        );
        Ok(MergedTree::new(
            self.store.clone(),
            tree_ids,
            self.labels.clone(),
        ))
    }

    fn schedule_read(&mut self, dir: RepoPathBuf, tree_id: TreeId) {
        if !self.scheduled_trees.insert((dir.clone(), tree_id.clone())) {
            // Already scheduled
            return;
        }
        let store = self.store.clone();
        self.work.push(
            async move {
                store
                    .get_tree(dir, &tree_id)
                    .map(|result| FilteredTreeWorkItem::ReadTree { result })
                    .await
            }
            .boxed(),
        );
    }

    fn schedule_write(&mut self, dir: RepoPathBuf, base_tree_id: TreeId, tree: backend::Tree) {
        let store = self.store.clone();
        self.work.push(
            async move {
                store
                    .write_tree(&dir, tree)
                    .map(|result| FilteredTreeWorkItem::WrittenTree {
                        dir: dir.clone(),
                        old_tree_id: base_tree_id,
                        result: result.map(|tree| tree.id().clone()),
                    })
                    .await
            }
            .boxed(),
        );
    }
}
