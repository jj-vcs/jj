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

//! Concurrently writes rewritten versions of the trees in a `Merge<TreeId>`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::future::poll_fn;
use std::hash::Hash;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use itertools::Itertools as _;

use crate::backend;
use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::merge::Merge;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::store::Store;
use crate::tree::Tree;

/// An entry in a rewritten tree, as returned by
/// [`TreeRewriteVisitor::visit_tree()`].
#[derive(Debug)]
pub(crate) enum RewrittenEntry<C> {
    /// The entry has this value.
    Value(TreeValue),
    /// The entry is the rewritten version of `base_tree_id`, rewritten with
    /// `context`. The entry is omitted if the rewritten tree is empty.
    Subtree { base_tree_id: TreeId, context: C },
}

/// Decides how each tree is rewritten by [`rewrite_trees()`].
pub(crate) trait TreeRewriteVisitor {
    /// The context a tree is rewritten in, in addition to its path and ID. A
    /// tree is rewritten only once per combination of path, ID, and context,
    /// even if it appears in multiple terms of the merge.
    type Context: Clone + Eq + Hash + Send + 'static;

    /// Returns the entries of the rewritten version of `tree`, sorted by name.
    fn visit_tree(
        &self,
        tree: &Tree,
        context: &Self::Context,
    ) -> Vec<(RepoPathComponentBuf, RewrittenEntry<Self::Context>)>;
}

struct LimitedFutures<'a, T> {
    limit: usize,
    running: FuturesUnordered<BoxFuture<'a, T>>,
    queued: VecDeque<BoxFuture<'a, T>>,
}

impl<'a, T: 'a> LimitedFutures<'a, T> {
    fn new(limit: usize) -> Self {
        assert!(limit > 0, "The limit must be positive");
        Self {
            limit,
            running: FuturesUnordered::new(),
            queued: VecDeque::new(),
        }
    }

    fn push(&mut self, future: impl Future<Output = T> + Send + 'a) {
        self.queued.push_back(future.boxed());
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        while self.running.len() < self.limit
            && let Some(future) = self.queued.pop_front()
        {
            self.running.push(future);
        }
        self.running.poll_next_unpin(cx)
    }
}

/// Rewrites each root tree in `roots` with its context, as decided by
/// `visitor`, and returns the IDs of the rewritten trees. Subtrees that are
/// shared between terms are read and written only once.
pub(crate) async fn rewrite_trees<V: TreeRewriteVisitor>(
    store: &Arc<Store>,
    roots: Merge<(TreeId, V::Context)>,
    visitor: &V,
) -> BackendResult<Merge<TreeId>> {
    let mut rewriter = TreeRewriter {
        store: store.clone(),
        visitor,
        work: LimitedFutures::new(store.concurrency()),
        scheduled_trees: HashSet::new(),
        rewritten_trees: HashMap::new(),
        unfinished_trees: HashMap::new(),
        parent_trees: HashMap::new(),
    };
    for (tree_id, context) in roots.iter().unique() {
        rewriter.schedule_read((RepoPathBuf::root(), tree_id.clone(), context.clone()));
    }
    rewriter.run().await?;
    Ok(roots.into_map(|(tree_id, context)| {
        rewriter.rewritten_trees[&(RepoPathBuf::root(), tree_id, context)].clone()
    }))
}

/// Identifies a tree to rewrite by its path, its original ID, and its context.
type TreeKey<C> = (RepoPathBuf, TreeId, C);

/// Identifies an entry in a tree by the tree's key and the entry's index.
type EntryKey<C> = (TreeKey<C>, usize);

#[derive(Debug)]
enum WorkItem<C> {
    // A base tree that has been read.
    ReadTree {
        context: C,
        result: BackendResult<Tree>,
    },
    // A rewritten tree that has been written.
    WrittenTree {
        key: TreeKey<C>,
        result: BackendResult<TreeId>,
    },
}

/// A rewritten tree that's waiting for some of its subtrees to be written.
struct UnfinishedTree {
    base_tree: Tree,
    // Entries of the rewritten tree. Entries for subtrees that have not yet been
    // written, or that turned out to be empty, are `None`.
    entries: Vec<(RepoPathComponentBuf, Option<TreeValue>)>,
    remaining_subtrees: usize,
}

struct TreeRewriter<'visitor, V: TreeRewriteVisitor> {
    store: Arc<Store>,
    visitor: &'visitor V,
    // Pending reads and writes. We use `PrioritizedFutures` only to limit the
    // number of concurrent backend requests. We don't care about the order the
    // work is done in, so the keys are just sequence numbers.
    work: LimitedFutures<'static, WorkItem<V::Context>>,
    scheduled_trees: HashSet<TreeKey<V::Context>>,
    // Map from original tree to the ID of the rewritten tree
    rewritten_trees: HashMap<TreeKey<V::Context>, TreeId>,
    unfinished_trees: HashMap<TreeKey<V::Context>, UnfinishedTree>,
    // Entries in parent trees to update when we have finished writing a subtree
    parent_trees: HashMap<TreeKey<V::Context>, Vec<EntryKey<V::Context>>>,
}

impl<V: TreeRewriteVisitor> TreeRewriter<'_, V> {
    async fn run(&mut self) -> BackendResult<()> {
        while let Some(item) = poll_fn(|cx| self.work.poll_next(cx)).await {
            match item {
                WorkItem::ReadTree { context, result } => {
                    let tree = result?;
                    self.process_tree(&tree, context);
                }
                WorkItem::WrittenTree { key, result } => {
                    let tree_id = result?;
                    let (dir, _, _) = &key;
                    // Add this entry to the parent tree(s) (unless this is a root tree)
                    if !dir.is_root() {
                        let is_empty = tree_id == *self.store.empty_tree_id();
                        for (parent_key, index) in self.parent_trees.remove(&key).unwrap() {
                            let unfinished_tree =
                                self.unfinished_trees.get_mut(&parent_key).unwrap();
                            if !is_empty {
                                unfinished_tree.entries[index].1 =
                                    Some(TreeValue::Tree(tree_id.clone()));
                            }
                            unfinished_tree.remaining_subtrees -= 1;
                            if unfinished_tree.remaining_subtrees == 0 {
                                let unfinished_tree =
                                    self.unfinished_trees.remove(&parent_key).unwrap();
                                self.schedule_write(
                                    parent_key,
                                    &unfinished_tree.base_tree,
                                    unfinished_tree.entries,
                                );
                            }
                        }
                    }
                    self.rewritten_trees.insert(key, tree_id);
                }
            }
        }
        Ok(())
    }

    fn process_tree(&mut self, tree: &Tree, context: V::Context) {
        let key = (tree.dir().to_owned(), tree.id().clone(), context);
        let mut new_entries = vec![];
        let mut remaining_subtrees = 0;
        for (index, (name, entry)) in self
            .visitor
            .visit_tree(tree, &key.2)
            .into_iter()
            .enumerate()
        {
            match entry {
                RewrittenEntry::Value(value) => new_entries.push((name, Some(value))),
                RewrittenEntry::Subtree {
                    base_tree_id,
                    context,
                } => {
                    let subtree_key = (tree.dir().join(&name), base_tree_id, context);
                    if let Some(new_subtree_id) = self.rewritten_trees.get(&subtree_key) {
                        // We already have the rewritten subtree. This can happen when the
                        // current tree is conflicted but the subtree is not and we
                        // processed the subtree when we visited another side of the
                        // current conflict.
                        let value = (new_subtree_id != self.store.empty_tree_id())
                            .then(|| TreeValue::Tree(new_subtree_id.clone()));
                        new_entries.push((name, value));
                    } else {
                        self.parent_trees
                            .entry(subtree_key.clone())
                            .or_default()
                            .push((key.clone(), index));
                        self.schedule_read(subtree_key);
                        new_entries.push((name, None));
                        remaining_subtrees += 1;
                    }
                }
            }
        }
        if remaining_subtrees > 0 {
            self.unfinished_trees.insert(
                key,
                UnfinishedTree {
                    base_tree: tree.clone(),
                    entries: new_entries,
                    remaining_subtrees,
                },
            );
        } else {
            self.schedule_write(key, tree, new_entries);
        }
    }

    fn schedule_read(&mut self, key: TreeKey<V::Context>) {
        if !self.scheduled_trees.insert(key.clone()) {
            // Already scheduled
            return;
        }
        let (dir, tree_id, context) = key;
        let store = self.store.clone();
        if tree_id == *store.empty_tree_id() {
            let result = Ok(Tree::empty(store, dir));
            self.work
                .push(async move { WorkItem::ReadTree { context, result } });
            return;
        }
        self.work.push(async move {
            store
                .get_tree(dir, &tree_id)
                .map(|result| WorkItem::ReadTree { context, result })
                .await
        });
    }

    /// Writes the rewritten tree, or reuses `base_tree` if the rewritten tree
    /// has the same entries.
    fn schedule_write(
        &mut self,
        key: TreeKey<V::Context>,
        base_tree: &Tree,
        entries: Vec<(RepoPathComponentBuf, Option<TreeValue>)>,
    ) {
        let tree = backend::Tree::from_sorted_entries(
            entries
                .into_iter()
                .filter_map(|(name, value)| Some((name, value?)))
                .collect(),
        );
        if &tree == base_tree.data() {
            let result = Ok(base_tree.id().clone());
            self.work
                .push(async move { WorkItem::WrittenTree { key, result } });
            return;
        }
        // TODO: Write trees with identical contents only once. For example, if the
        // only conflict in a merged tree is in `dir/file` and it gets resolved, the
        // `dir/` and root trees end up the same for all terms.
        let store = self.store.clone();
        if tree.is_empty() && !key.0.is_root() {
            // Empty subtrees are omitted from their parents, so there's no need to write
            // them
            let result = Ok(store.empty_tree_id().clone());
            self.work
                .push(async move { WorkItem::WrittenTree { key, result } });
            return;
        }
        self.work.push(async move {
            let result = store
                .write_tree(&key.0, tree)
                .await
                .map(|tree| tree.id().clone());
            WorkItem::WrittenTree { key, result }
        });
    }
}
