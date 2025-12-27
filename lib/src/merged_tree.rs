// Copyright 2023 The Jujutsu Authors
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

//! A lazily merged view of a set of trees.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::fmt;
use std::iter;
use std::iter::zip;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::ready;
use std::vec;

use either::Either;
use futures::Stream;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::future::ready;
use futures::future::try_join;
use futures::future::try_join_all;
use futures::stream::BoxStream;
use futures::stream::FuturesOrdered;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CopyHistory;
use crate::backend::CopyId;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::conflict_labels::ConflictLabels;
use crate::copies::CopiesTreeDiffEntry;
use crate::copies::CopiesTreeDiffStream;
use crate::copies::CopyGraph;
use crate::copies::CopyRecords;
use crate::copies::is_descendant;
use crate::copies::traverse_copy_history;
use crate::matchers::EverythingMatcher;
use crate::matchers::Matcher;
use crate::merge::Diff;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::merge::MergedTreeVal;
use crate::merge::MergedTreeValue;
use crate::merge::SameChange;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::store::Store;
use crate::tree::Tree;
use crate::tree_builder::TreeBuilder;
use crate::tree_merge::merge_trees;

/// Presents a view of a merged set of trees at the root directory, as well as
/// conflict labels.
#[derive(Clone)]
pub struct MergedTree {
    store: Arc<Store>,
    tree_ids: Merge<TreeId>,
    labels: ConflictLabels,
}

impl fmt::Debug for MergedTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergedTree")
            .field("tree_ids", &self.tree_ids)
            .field("labels", &self.labels)
            .finish_non_exhaustive()
    }
}

impl MergedTree {
    /// Creates a `MergedTree` with the given resolved tree ID.
    pub fn resolved(store: Arc<Store>, tree_id: TreeId) -> Self {
        Self::unlabeled(store, Merge::resolved(tree_id))
    }

    /// Creates a `MergedTree` with the given tree IDs, without conflict labels.
    // TODO: remove when all callers are migrated to `MergedTree::new`.
    pub fn unlabeled(store: Arc<Store>, tree_ids: Merge<TreeId>) -> Self {
        Self {
            store,
            tree_ids,
            labels: ConflictLabels::unlabeled(),
        }
    }

    /// Creates a `MergedTree` with the given tree IDs.
    pub fn new(store: Arc<Store>, tree_ids: Merge<TreeId>, labels: ConflictLabels) -> Self {
        if let Some(num_sides) = labels.num_sides() {
            assert_eq!(tree_ids.num_sides(), num_sides);
        }
        Self {
            store,
            tree_ids,
            labels,
        }
    }

    /// The `Store` associated with this tree.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// The underlying tree IDs for this `MergedTree`. If there are file changes
    /// between two trees, then the tree IDs will be different.
    pub fn tree_ids(&self) -> &Merge<TreeId> {
        &self.tree_ids
    }

    /// Extracts the underlying tree IDs for this `MergedTree`, discarding any
    /// conflict labels.
    pub fn into_tree_ids(self) -> Merge<TreeId> {
        self.tree_ids
    }

    /// Returns this merge's conflict labels, if any.
    pub fn labels(&self) -> &ConflictLabels {
        &self.labels
    }

    /// Returns both the underlying tree IDs and any conflict labels. This can
    /// be used to check whether there are changes in files to be materialized
    /// in the working copy.
    pub fn tree_ids_and_labels(&self) -> (&Merge<TreeId>, &ConflictLabels) {
        (&self.tree_ids, &self.labels)
    }

    /// Extracts the underlying tree IDs and conflict labels.
    pub fn into_tree_ids_and_labels(self) -> (Merge<TreeId>, ConflictLabels) {
        (self.tree_ids, self.labels)
    }

    /// Reads the merge of tree objects represented by this `MergedTree`.
    pub fn trees(&self) -> BackendResult<Merge<Tree>> {
        self.trees_async().block_on()
    }

    /// Async version of `trees()`.
    pub async fn trees_async(&self) -> BackendResult<Merge<Tree>> {
        self.tree_ids
            .try_map_async(|id| self.store.get_tree_async(RepoPathBuf::root(), id))
            .await
    }

    /// Returns a label for each term in a merge. Resolved merges use the
    /// provided label, while conflicted merges keep their original labels.
    /// Missing labels are indicated by empty strings.
    pub fn labels_by_term<'a>(&'a self, label: &'a str) -> Merge<&'a str> {
        if self.tree_ids.is_resolved() {
            assert!(!self.labels.has_labels());
            Merge::resolved(label)
        } else if self.labels.has_labels() {
            // If the merge is conflicted and it already has labels, then we want to use
            // those labels instead of the provided label. This ensures that rebasing
            // conflicted commits keeps meaningful labels.
            let labels = self.labels.as_merge();
            assert_eq!(labels.num_sides(), self.tree_ids.num_sides());
            labels.map(|label| label.as_str())
        } else {
            // If the merge is conflicted but it doesn't have labels (e.g. conflicts created
            // before labels were added), then we use empty strings to indicate missing
            // labels. We could consider using `label` for all the sides instead, but it
            // might be confusing.
            Merge::repeated("", self.tree_ids.num_sides())
        }
    }

    /// Tries to resolve any conflicts, resolving any conflicts that can be
    /// automatically resolved and leaving the rest unresolved.
    pub async fn resolve(self) -> BackendResult<Self> {
        let merged = merge_trees(&self.store, self.tree_ids).await?;
        // If the result can be resolved, then `merge_trees()` above would have returned
        // a resolved merge. However, that function will always preserve the arity of
        // conflicts it cannot resolve. So we simplify the conflict again
        // here to possibly reduce a complex conflict to a simpler one.
        let (simplified_labels, simplified) = if merged.is_resolved() {
            (ConflictLabels::unlabeled(), merged)
        } else {
            self.labels.simplify_with(&merged)
        };
        // If debug assertions are enabled, check that the merge was idempotent. In
        // particular, that this last simplification doesn't enable further automatic
        // resolutions
        if cfg!(debug_assertions) {
            let re_merged = merge_trees(&self.store, simplified.clone()).await.unwrap();
            debug_assert_eq!(re_merged, simplified);
        }
        Ok(Self {
            store: self.store,
            tree_ids: simplified,
            labels: simplified_labels,
        })
    }

    /// An iterator over the conflicts in this tree, including subtrees.
    /// Recurses into subtrees and yields conflicts in those, but only if
    /// all sides are trees, so tree/file conflicts will be reported as a single
    /// conflict, not one for each path in the tree.
    pub fn conflicts(
        &self,
    ) -> impl Iterator<Item = (RepoPathBuf, BackendResult<MergedTreeValue>)> + use<> {
        self.conflicts_matching(&EverythingMatcher)
    }

    /// Like `conflicts()` but restricted by a matcher.
    pub fn conflicts_matching<'matcher>(
        &self,
        matcher: &'matcher dyn Matcher,
    ) -> impl Iterator<Item = (RepoPathBuf, BackendResult<MergedTreeValue>)> + use<'matcher> {
        ConflictIterator::new(self, matcher)
    }

    /// Whether this tree has conflicts.
    pub fn has_conflict(&self) -> bool {
        !self.tree_ids.is_resolved()
    }

    /// The value at the given path. The value can be `Resolved` even if
    /// `self` is a `Conflict`, which happens if the value at the path can be
    /// trivially merged.
    pub fn path_value(&self, path: &RepoPath) -> BackendResult<MergedTreeValue> {
        self.path_value_async(path).block_on()
    }

    /// Async version of `path_value()`.
    pub async fn path_value_async(&self, path: &RepoPath) -> BackendResult<MergedTreeValue> {
        match path.split() {
            Some((dir, basename)) => {
                let trees = self.trees_async().await?;
                match trees.sub_tree_recursive(dir).await? {
                    None => Ok(Merge::absent()),
                    Some(tree) => Ok(tree.value(basename).cloned()),
                }
            }
            None => Ok(self.to_merged_tree_value()),
        }
    }

    /// Returns the `TreeValue` associated with `id` if it exists at the
    /// expected path and is resolved.
    pub async fn copy_value(&self, id: &CopyId) -> BackendResult<Option<TreeValue>> {
        let copy = self.store().backend().read_copy(id).await?;
        let merged_val = self.path_value_async(&copy.current_path).await?;
        match merged_val.into_resolved() {
            Ok(Some(val)) if val.copy_id() == Some(id) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    fn to_merged_tree_value(&self) -> MergedTreeValue {
        self.tree_ids
            .map(|tree_id| Some(TreeValue::Tree(tree_id.clone())))
    }

    /// Iterator over the entries matching the given matcher. Subtrees are
    /// visited recursively. Subtrees that differ between the current
    /// `MergedTree`'s terms are merged on the fly. Missing terms are treated as
    /// empty directories. Subtrees that conflict with non-trees are not
    /// visited. For example, if current tree is a merge of 3 trees, and the
    /// entry for 'foo' is a conflict between a change subtree and a symlink
    /// (i.e. the subdirectory was replaced by symlink in one side of the
    /// conflict), then the entry for `foo` itself will be emitted, but no
    /// entries from inside `foo/` from either of the trees will be.
    pub fn entries(&self) -> TreeEntriesIterator<'static> {
        self.entries_matching(&EverythingMatcher)
    }

    /// Like `entries()` but restricted by a matcher.
    pub fn entries_matching<'matcher>(
        &self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeEntriesIterator<'matcher> {
        TreeEntriesIterator::new(self, matcher)
    }

    /// Stream of the differences between this tree and another tree.
    ///
    /// Tree entries (`MergedTreeValue::is_tree()`) are included only if the
    /// other side is present and not a tree.
    fn diff_stream_internal<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        let concurrency = self.store().concurrency();
        if concurrency <= 1 {
            Box::pin(futures::stream::iter(TreeDiffIterator::new(
                self, other, matcher,
            )))
        } else {
            Box::pin(TreeDiffStreamImpl::new(self, other, matcher, concurrency))
        }
    }

    /// Stream of the differences between this tree and another tree.
    pub fn diff_stream<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        stream_without_trees(self.diff_stream_internal(other, matcher))
    }

    /// Like `diff_stream()` but files in a removed tree will be returned before
    /// a file that replaces it.
    pub fn diff_stream_for_file_system<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        Box::pin(DiffStreamForFileSystem::new(
            self.diff_stream_internal(other, matcher),
        ))
    }

    /// Like `diff_stream()` but takes the given copy records into account.
    pub fn diff_stream_with_copies<'a>(
        &self,
        other: &Self,
        matcher: &'a dyn Matcher,
        copy_records: &'a CopyRecords,
    ) -> BoxStream<'a, CopiesTreeDiffEntry> {
        let stream = self.diff_stream(other, matcher);
        Box::pin(CopiesTreeDiffStream::new(
            stream,
            self.clone(),
            other.clone(),
            copy_records,
        ))
    }

    /// Like `diff_stream()` but takes CopyHistory into account.
    pub fn diff_stream_with_copy_history<'a>(
        &'a self,
        other: &'a Self,
        matcher: &'a dyn Matcher,
    ) -> BoxStream<'a, CopyHistoryTreeDiffEntry> {
        let stream = self.diff_stream(other, matcher);
        Box::pin(CopyHistoryDiffStream::new(stream, self, other))
    }

    /// Merges this tree with `other`, using `base` as base. Any conflicts will
    /// be resolved recursively if possible. Does not add conflict labels.
    // TODO: remove when all callers are migrated to `MergedTree::merge`.
    pub async fn merge_unlabeled(self, base: Self, other: Self) -> BackendResult<Self> {
        Self::merge(Merge::from_vec(vec![
            (self, String::new()),
            (base, String::new()),
            (other, String::new()),
        ]))
        .await
    }

    /// Merges the provided trees into a single `MergedTree`. Any conflicts will
    /// be resolved recursively if possible. The provided labels are used if a
    /// conflict arises. However, if one of the input trees is already
    /// conflicted, the corresponding label will be ignored, and its existing
    /// labels will be used instead.
    pub async fn merge(merge: Merge<(Self, String)>) -> BackendResult<Self> {
        Self::merge_no_resolve(merge).resolve().await
    }

    /// Merges the provided trees into a single `MergedTree`, without attempting
    /// to resolve file conflicts.
    pub fn merge_no_resolve(merge: Merge<(Self, String)>) -> Self {
        debug_assert!(
            merge
                .iter()
                .map(|(tree, _)| Arc::as_ptr(tree.store()))
                .all_equal()
        );
        let store = merge.first().0.store().clone();
        let flattened_labels = ConflictLabels::from_merge(
            merge
                .map(|(tree, label)| tree.labels_by_term(label))
                .flatten()
                .map(|&label| label.to_owned()),
        );
        let flattened_tree_ids: Merge<TreeId> = merge
            .into_iter()
            .map(|(tree, _label)| tree.into_tree_ids())
            .collect::<MergeBuilder<_>>()
            .build()
            .flatten();

        let (labels, tree_ids) = flattened_labels.simplify_with(&flattened_tree_ids);
        Self::new(store, tree_ids, labels)
    }
}

/// A single entry in a tree diff.
#[derive(Debug)]
pub struct TreeDiffEntry {
    /// The path.
    pub path: RepoPathBuf,
    /// The resolved tree values if available.
    pub values: BackendResult<Diff<MergedTreeValue>>,
}

/// Type alias for the result from `MergedTree::diff_stream()`. We use a
/// `Stream` instead of an `Iterator` so high-latency backends (e.g. cloud-based
/// ones) can fetch trees asynchronously.
pub type TreeDiffStream<'matcher> = BoxStream<'matcher, TreeDiffEntry>;

fn all_tree_entries(
    trees: &Merge<Tree>,
) -> impl Iterator<Item = (&RepoPathComponent, MergedTreeVal<'_>)> {
    if let Some(tree) = trees.as_resolved() {
        let iter = tree
            .entries_non_recursive()
            .map(|entry| (entry.name(), Merge::normal(entry.value())));
        Either::Left(iter)
    } else {
        let same_change = trees.first().store().merge_options().same_change;
        let iter = all_merged_tree_entries(trees).map(move |(name, values)| {
            // TODO: move resolve_trivial() to caller?
            let values = match values.resolve_trivial(same_change) {
                Some(resolved) => Merge::resolved(*resolved),
                None => values,
            };
            (name, values)
        });
        Either::Right(iter)
    }
}

/// Suppose the given `trees` aren't resolved, iterates `(name, values)` pairs
/// non-recursively. This also works if `trees` are resolved, but is more costly
/// than `tree.entries_non_recursive()`.
pub fn all_merged_tree_entries(
    trees: &Merge<Tree>,
) -> impl Iterator<Item = (&RepoPathComponent, MergedTreeVal<'_>)> {
    let mut entries_iters = trees
        .iter()
        .map(|tree| tree.entries_non_recursive().peekable())
        .collect_vec();
    iter::from_fn(move || {
        let next_name = entries_iters
            .iter_mut()
            .filter_map(|iter| iter.peek())
            .map(|entry| entry.name())
            .min()?;
        let values: MergeBuilder<_> = entries_iters
            .iter_mut()
            .map(|iter| {
                let entry = iter.next_if(|entry| entry.name() == next_name)?;
                Some(entry.value())
            })
            .collect();
        Some((next_name, values.build()))
    })
}

fn merged_tree_entry_diff<'a>(
    trees1: &'a Merge<Tree>,
    trees2: &'a Merge<Tree>,
) -> impl Iterator<Item = (&'a RepoPathComponent, Diff<MergedTreeVal<'a>>)> {
    itertools::merge_join_by(
        all_tree_entries(trees1),
        all_tree_entries(trees2),
        |(name1, _), (name2, _)| name1.cmp(name2),
    )
    .map(|entry| match entry {
        EitherOrBoth::Both((name, value1), (_, value2)) => (name, Diff::new(value1, value2)),
        EitherOrBoth::Left((name, value1)) => (name, Diff::new(value1, Merge::absent())),
        EitherOrBoth::Right((name, value2)) => (name, Diff::new(Merge::absent(), value2)),
    })
    .filter(|(_, diff)| diff.is_changed())
}

/// Recursive iterator over the entries in a tree.
pub struct TreeEntriesIterator<'matcher> {
    store: Arc<Store>,
    stack: Vec<TreeEntriesDirItem>,
    matcher: &'matcher dyn Matcher,
}

struct TreeEntriesDirItem {
    entries: Vec<(RepoPathBuf, MergedTreeValue)>,
}

impl TreeEntriesDirItem {
    fn new(trees: &Merge<Tree>, matcher: &dyn Matcher) -> Self {
        let mut entries = vec![];
        let dir = trees.first().dir();
        for (name, value) in all_tree_entries(trees) {
            let path = dir.join(name);
            if value.is_tree() {
                // TODO: Handle the other cases (specific files and trees)
                if matcher.visit(&path).is_nothing() {
                    continue;
                }
            } else if !matcher.matches(&path) {
                continue;
            }
            entries.push((path, value.cloned()));
        }
        entries.reverse();
        Self { entries }
    }
}

impl<'matcher> TreeEntriesIterator<'matcher> {
    fn new(trees: &MergedTree, matcher: &'matcher dyn Matcher) -> Self {
        Self {
            store: trees.store.clone(),
            stack: vec![TreeEntriesDirItem {
                entries: vec![(RepoPathBuf::root(), trees.to_merged_tree_value())],
            }],
            matcher,
        }
    }
}

impl Iterator for TreeEntriesIterator<'_> {
    type Item = (RepoPathBuf, BackendResult<MergedTreeValue>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            if let Some((path, value)) = top.entries.pop() {
                let maybe_trees = match value.to_tree_merge(&self.store, &path).block_on() {
                    Ok(maybe_trees) => maybe_trees,
                    Err(err) => return Some((path, Err(err))),
                };
                if let Some(trees) = maybe_trees {
                    self.stack
                        .push(TreeEntriesDirItem::new(&trees, self.matcher));
                } else {
                    return Some((path, Ok(value)));
                }
            } else {
                self.stack.pop();
            }
        }
        None
    }
}

/// The state for the non-recursive iteration over the conflicted entries in a
/// single directory.
struct ConflictsDirItem {
    entries: Vec<(RepoPathBuf, MergedTreeValue)>,
}

impl ConflictsDirItem {
    fn new(trees: &Merge<Tree>, matcher: &dyn Matcher) -> Self {
        if trees.is_resolved() {
            return Self { entries: vec![] };
        }

        let dir = trees.first().dir();
        let mut entries = vec![];
        for (basename, value) in all_tree_entries(trees) {
            if value.is_resolved() {
                continue;
            }
            let path = dir.join(basename);
            if value.is_tree() {
                if matcher.visit(&path).is_nothing() {
                    continue;
                }
            } else if !matcher.matches(&path) {
                continue;
            }
            entries.push((path, value.cloned()));
        }
        entries.reverse();
        Self { entries }
    }
}

struct ConflictIterator<'matcher> {
    store: Arc<Store>,
    stack: Vec<ConflictsDirItem>,
    matcher: &'matcher dyn Matcher,
}

impl<'matcher> ConflictIterator<'matcher> {
    fn new(tree: &MergedTree, matcher: &'matcher dyn Matcher) -> Self {
        Self {
            store: tree.store().clone(),
            stack: vec![ConflictsDirItem {
                entries: vec![(RepoPathBuf::root(), tree.to_merged_tree_value())],
            }],
            matcher,
        }
    }
}

impl Iterator for ConflictIterator<'_> {
    type Item = (RepoPathBuf, BackendResult<MergedTreeValue>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            if let Some((path, tree_values)) = top.entries.pop() {
                match tree_values.to_tree_merge(&self.store, &path).block_on() {
                    Ok(Some(trees)) => {
                        // If all sides are trees or missing, descend into the merged tree
                        self.stack.push(ConflictsDirItem::new(&trees, self.matcher));
                    }
                    Ok(None) => {
                        // Otherwise this is a conflict between files, trees, etc. If they could
                        // be automatically resolved, they should have been when the top-level
                        // tree conflict was written, so we assume that they can't be.
                        return Some((path, Ok(tree_values)));
                    }
                    Err(err) => {
                        return Some((path, Err(err)));
                    }
                }
            } else {
                self.stack.pop();
            }
        }
        None
    }
}

/// Iterator over the differences between two trees.
///
/// Tree entries (`MergedTreeValue::is_tree()`) are included only if the other
/// side is present and not a tree.
pub struct TreeDiffIterator<'matcher> {
    store: Arc<Store>,
    stack: Vec<TreeDiffDir>,
    matcher: &'matcher dyn Matcher,
}

struct TreeDiffDir {
    entries: Vec<(RepoPathBuf, Diff<MergedTreeValue>)>,
}

impl<'matcher> TreeDiffIterator<'matcher> {
    /// Creates a iterator over the differences between two trees.
    pub fn new(tree1: &MergedTree, tree2: &MergedTree, matcher: &'matcher dyn Matcher) -> Self {
        assert!(Arc::ptr_eq(tree1.store(), tree2.store()));
        let root_dir = RepoPath::root();
        let mut stack = Vec::new();
        if !matcher.visit(root_dir).is_nothing() {
            stack.push(TreeDiffDir {
                entries: vec![(
                    root_dir.to_owned(),
                    Diff::new(tree1.to_merged_tree_value(), tree2.to_merged_tree_value()),
                )],
            });
        };
        Self {
            store: tree1.store().clone(),
            stack,
            matcher,
        }
    }

    /// Gets the given trees if `values` are trees, otherwise an empty tree.
    fn trees(
        store: &Arc<Store>,
        dir: &RepoPath,
        values: &MergedTreeValue,
    ) -> BackendResult<Merge<Tree>> {
        if let Some(trees) = values.to_tree_merge(store, dir).block_on()? {
            Ok(trees)
        } else {
            Ok(Merge::resolved(Tree::empty(store.clone(), dir.to_owned())))
        }
    }
}

impl TreeDiffDir {
    fn from_trees(
        dir: &RepoPath,
        trees1: &Merge<Tree>,
        trees2: &Merge<Tree>,
        matcher: &dyn Matcher,
    ) -> Self {
        let mut entries = vec![];
        for (name, diff) in merged_tree_entry_diff(trees1, trees2) {
            let path = dir.join(name);
            let tree_before = diff.before.is_tree();
            let tree_after = diff.after.is_tree();
            // Check if trees and files match, but only if either side is a tree or a file
            // (don't query the matcher unnecessarily).
            let tree_matches = (tree_before || tree_after) && !matcher.visit(&path).is_nothing();
            let file_matches = (!tree_before || !tree_after) && matcher.matches(&path);

            // Replace trees or files that don't match by `Merge::absent()`
            let before = if (tree_before && tree_matches) || (!tree_before && file_matches) {
                diff.before
            } else {
                Merge::absent()
            };
            let after = if (tree_after && tree_matches) || (!tree_after && file_matches) {
                diff.after
            } else {
                Merge::absent()
            };
            if before.is_absent() && after.is_absent() {
                continue;
            }
            entries.push((path, Diff::new(before.cloned(), after.cloned())));
        }
        entries.reverse();
        Self { entries }
    }
}

impl Iterator for TreeDiffIterator<'_> {
    type Item = TreeDiffEntry;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            let (path, diff) = match top.entries.pop() {
                Some(entry) => entry,
                None => {
                    self.stack.pop().unwrap();
                    continue;
                }
            };

            if diff.before.is_tree() || diff.after.is_tree() {
                let (before_tree, after_tree) = match (
                    Self::trees(&self.store, &path, &diff.before),
                    Self::trees(&self.store, &path, &diff.after),
                ) {
                    (Ok(before_tree), Ok(after_tree)) => (before_tree, after_tree),
                    (Err(before_err), _) => {
                        return Some(TreeDiffEntry {
                            path,
                            values: Err(before_err),
                        });
                    }
                    (_, Err(after_err)) => {
                        return Some(TreeDiffEntry {
                            path,
                            values: Err(after_err),
                        });
                    }
                };
                let subdir =
                    TreeDiffDir::from_trees(&path, &before_tree, &after_tree, self.matcher);
                self.stack.push(subdir);
            };
            if diff.before.is_file_like() || diff.after.is_file_like() {
                return Some(TreeDiffEntry {
                    path,
                    values: Ok(diff),
                });
            }
        }
        None
    }
}

/// Stream of differences between two trees.
///
/// Tree entries (`MergedTreeValue::is_tree()`) are included only if the other
/// side is present and not a tree.
pub struct TreeDiffStreamImpl<'matcher> {
    store: Arc<Store>,
    matcher: &'matcher dyn Matcher,
    /// Pairs of tree values that may or may not be ready to emit, sorted in the
    /// order we want to emit them. If either side is a tree, there will be
    /// a corresponding entry in `pending_trees`. The item is ready to emit
    /// unless there's a smaller or equal path in `pending_trees`.
    items: BTreeMap<RepoPathBuf, BackendResult<Diff<MergedTreeValue>>>,
    // TODO: Is it better to combine this and `items` into a single map?
    #[expect(clippy::type_complexity)]
    pending_trees:
        BTreeMap<RepoPathBuf, BoxFuture<'matcher, BackendResult<(Merge<Tree>, Merge<Tree>)>>>,
    /// The maximum number of trees to request concurrently. However, we do the
    /// accounting per path, so there will often be twice as many pending
    /// `Backend::read_tree()` calls - for the "before" and "after" sides. For
    /// conflicts, there will be even more.
    max_concurrent_reads: usize,
    /// The maximum number of items in `items`. However, we will always add the
    /// full differences from a particular pair of trees, so it may temporarily
    /// go over the limit (until we emit those items). It may also go over the
    /// limit because we have a file item that's blocked by pending subdirectory
    /// items.
    max_queued_items: usize,
}

impl<'matcher> TreeDiffStreamImpl<'matcher> {
    /// Creates a iterator over the differences between two trees. Generally
    /// prefer `MergedTree::diff_stream()` of calling this directly.
    pub fn new(
        tree1: &MergedTree,
        tree2: &MergedTree,
        matcher: &'matcher dyn Matcher,
        max_concurrent_reads: usize,
    ) -> Self {
        assert!(Arc::ptr_eq(tree1.store(), tree2.store()));
        let store = tree1.store().clone();
        let mut stream = Self {
            store: store.clone(),
            matcher,
            items: BTreeMap::new(),
            pending_trees: BTreeMap::new(),
            max_concurrent_reads,
            max_queued_items: 10000,
        };
        let dir = RepoPathBuf::root();
        let root_tree_fut = Box::pin(try_join(
            Self::trees(store.clone(), dir.clone(), tree1.to_merged_tree_value()),
            Self::trees(store, dir.clone(), tree2.to_merged_tree_value()),
        ));
        stream.pending_trees.insert(dir, root_tree_fut);
        stream
    }

    async fn single_tree(
        store: &Arc<Store>,
        dir: RepoPathBuf,
        value: Option<&TreeValue>,
    ) -> BackendResult<Tree> {
        match value {
            Some(TreeValue::Tree(tree_id)) => store.get_tree_async(dir, tree_id).await,
            _ => Ok(Tree::empty(store.clone(), dir.clone())),
        }
    }

    /// Gets the given trees if `values` are trees, otherwise an empty tree.
    async fn trees(
        store: Arc<Store>,
        dir: RepoPathBuf,
        values: MergedTreeValue,
    ) -> BackendResult<Merge<Tree>> {
        if values.is_tree() {
            values
                .try_map_async(|value| Self::single_tree(&store, dir.clone(), value.as_ref()))
                .await
        } else {
            Ok(Merge::resolved(Tree::empty(store, dir)))
        }
    }

    fn add_dir_diff_items(&mut self, dir: &RepoPath, trees1: &Merge<Tree>, trees2: &Merge<Tree>) {
        for (basename, diff) in merged_tree_entry_diff(trees1, trees2) {
            let path = dir.join(basename);
            let tree_before = diff.before.is_tree();
            let tree_after = diff.after.is_tree();
            // Check if trees and files match, but only if either side is a tree or a file
            // (don't query the matcher unnecessarily).
            let tree_matches =
                (tree_before || tree_after) && !self.matcher.visit(&path).is_nothing();
            let file_matches = (!tree_before || !tree_after) && self.matcher.matches(&path);

            // Replace trees or files that don't match by `Merge::absent()`
            let before = if (tree_before && tree_matches) || (!tree_before && file_matches) {
                diff.before
            } else {
                Merge::absent()
            };
            let after = if (tree_after && tree_matches) || (!tree_after && file_matches) {
                diff.after
            } else {
                Merge::absent()
            };
            if before.is_absent() && after.is_absent() {
                continue;
            }

            // If the path was a tree on either side of the diff, read those trees.
            if tree_matches {
                let before_tree_future =
                    Self::trees(self.store.clone(), path.clone(), before.cloned());
                let after_tree_future =
                    Self::trees(self.store.clone(), path.clone(), after.cloned());
                let both_trees_future = try_join(before_tree_future, after_tree_future);
                self.pending_trees
                    .insert(path.clone(), Box::pin(both_trees_future));
            }

            if before.is_file_like() || after.is_file_like() {
                self.items
                    .insert(path, Ok(Diff::new(before.cloned(), after.cloned())));
            }
        }
    }

    fn poll_tree_futures(&mut self, cx: &mut Context<'_>) {
        loop {
            let mut tree_diffs = vec![];
            let mut some_pending = false;
            let mut all_pending = true;
            for (dir, future) in self
                .pending_trees
                .iter_mut()
                .take(self.max_concurrent_reads)
            {
                if let Poll::Ready(tree_diff) = future.as_mut().poll(cx) {
                    all_pending = false;
                    tree_diffs.push((dir.clone(), tree_diff));
                } else {
                    some_pending = true;
                }
            }

            for (dir, tree_diff) in tree_diffs {
                drop(self.pending_trees.remove_entry(&dir).unwrap());
                match tree_diff {
                    Ok((trees1, trees2)) => {
                        self.add_dir_diff_items(&dir, &trees1, &trees2);
                    }
                    Err(err) => {
                        self.items.insert(dir, Err(err));
                    }
                }
            }

            // If none of the futures have been polled and returned `Poll::Pending`, we must
            // not return. If we did, nothing would call the waker so we might never get
            // polled again.
            if all_pending || (some_pending && self.items.len() >= self.max_queued_items) {
                return;
            }
        }
    }
}

impl Stream for TreeDiffStreamImpl<'_> {
    type Item = TreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Go through all pending tree futures and poll them.
        self.poll_tree_futures(cx);

        // Now emit the first file, or the first tree that completed with an error
        if let Some((path, _)) = self.items.first_key_value() {
            // Check if there are any pending trees before this item that we need to finish
            // polling before we can emit this item.
            if let Some((dir, _)) = self.pending_trees.first_key_value()
                && dir < path
            {
                return Poll::Pending;
            }

            let (path, values) = self.items.pop_first().unwrap();
            Poll::Ready(Some(TreeDiffEntry { path, values }))
        } else if self.pending_trees.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

fn stream_without_trees(stream: TreeDiffStream) -> TreeDiffStream {
    Box::pin(stream.map(|mut entry| {
        let skip_tree = |merge: MergedTreeValue| {
            if merge.is_tree() {
                Merge::absent()
            } else {
                merge
            }
        };
        entry.values = entry.values.map(|diff| diff.map(skip_tree));
        entry
    }))
}

/// Adapts a `TreeDiffStream` to emit a added file at a given path after a
/// removed directory at the same path.
struct DiffStreamForFileSystem<'a> {
    inner: TreeDiffStream<'a>,
    next_item: Option<TreeDiffEntry>,
    held_file: Option<TreeDiffEntry>,
}

impl<'a> DiffStreamForFileSystem<'a> {
    fn new(inner: TreeDiffStream<'a>) -> Self {
        Self {
            inner,
            next_item: None,
            held_file: None,
        }
    }
}

impl Stream for DiffStreamForFileSystem<'_> {
    type Item = TreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Some(next) = match self.next_item.take() {
            Some(next) => Some(next),
            None => ready!(self.inner.as_mut().poll_next(cx)),
        } {
            // If there's a held file "foo" and the next item to emit is not "foo/...", then
            // we must be done with the "foo/" directory and it's time to emit "foo" as a
            // removed file.
            if let Some(held_entry) = self
                .held_file
                .take_if(|held_entry| !next.path.starts_with(&held_entry.path))
            {
                self.next_item = Some(next);
                return Poll::Ready(Some(held_entry));
            }

            match next.values {
                Ok(diff) if diff.before.is_tree() => {
                    assert!(diff.after.is_present());
                    assert!(self.held_file.is_none());
                    self.held_file = Some(TreeDiffEntry {
                        path: next.path,
                        values: Ok(Diff::new(Merge::absent(), diff.after)),
                    });
                }
                Ok(diff) if diff.after.is_tree() => {
                    assert!(diff.before.is_present());
                    return Poll::Ready(Some(TreeDiffEntry {
                        path: next.path,
                        values: Ok(Diff::new(diff.before, Merge::absent())),
                    }));
                }
                _ => {
                    return Poll::Ready(Some(next));
                }
            }
        }
        Poll::Ready(self.held_file.take())
    }
}

/// Describes the source of a CopyHistoryDiffTerm
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CopyHistorySource {
    /// The file was copied from a source at a different path
    Copy(RepoPathBuf),
    /// The file was renamed from a source at a different path
    Rename(RepoPathBuf),
    /// The source and target have the same path
    Normal,
}

/// Describes a single term of a copy-aware diff
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct CopyHistoryDiffTerm {
    /// The current value of the target, if present
    pub target_value: Option<TreeValue>,
    /// List of sources, whether they were copied, renamed, or neither, and the
    /// original value
    pub sources: Vec<(CopyHistorySource, MergedTreeValue)>,
}

/// Like a `TreeDiffEntry`, but takes `CopyHistory`s into account
#[derive(Debug)]
pub struct CopyHistoryTreeDiffEntry {
    /// The final source path (after copy/rename if applicable)
    pub target_path: RepoPathBuf,
    /// The resolved values for the target and source(s), if available
    pub diffs: BackendResult<Merge<CopyHistoryDiffTerm>>,
}

impl CopyHistoryTreeDiffEntry {
    // Simple conversion case where no copy tracing is needed
    fn simple(tde: TreeDiffEntry) -> Self {
        let target_path = tde.path;
        let diffs = tde.values.map(|diff| {
            let sources = if diff.before.is_absent() {
                vec![]
            } else {
                vec![(CopyHistorySource::Normal, diff.before)]
            };
            Merge::from_vec(
                diff.after
                    .into_iter()
                    .map(|target_value| CopyHistoryDiffTerm {
                        target_value,
                        sources: sources.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        });
        Self { target_path, diffs }
    }
}

/// Adapts a `TreeDiffStream` to follow copies / renames.
pub struct CopyHistoryDiffStream<'a> {
    inner: Option<TreeDiffStream<'a>>,
    before_tree: &'a MergedTree,
    after_tree: &'a MergedTree,
    completed: VecDeque<CopyHistoryTreeDiffEntry>,
    pending: FuturesOrdered<BoxFuture<'static, Vec<CopyHistoryTreeDiffEntry>>>,
}

impl<'a> CopyHistoryDiffStream<'a> {
    /// Creates an iterator over the differences between two trees, taking copy
    /// history into account. Generally prefer
    /// `MergedTree::diff_stream_with_copy_history()` instead of calling this
    /// directly.
    pub fn new(
        inner: TreeDiffStream<'a>,
        before_tree: &'a MergedTree,
        after_tree: &'a MergedTree,
    ) -> Self {
        Self {
            inner: Some(inner),
            before_tree,
            after_tree,
            completed: VecDeque::new(),
            pending: FuturesOrdered::new(),
        }
    }
}

impl Stream for CopyHistoryDiffStream<'_> {
    type Item = CopyHistoryTreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // First, check if we have completed entries.
            if let Some(result) = self.completed.pop_front() {
                return Poll::Ready(Some(result));
            }

            // Next, check if we have newly-finished futures.
            if !self.pending.is_empty()
                && let Poll::Ready(Some(mut result)) = self.pending.poll_next_unpin(cx)
            {
                let next = result.remove(0); // `result` should always have at least one item.
                self.completed.extend(result);
                return Poll::Ready(Some(next));
            }

            // If we didn't have queued results above, we want to check our wrapped stream
            // for the next result. However, the stream may have previously
            // returned `Poll::Ready(None)`, in which case we're not supposed to
            // poll it again. But we can't just blindly assume that we're done,
            // because we may have pending copy-tracing futures whose results are not
            // ready yet.
            let Some(next_tde) = (match self.inner.as_mut() {
                Some(inner) => ready!(inner.as_mut().poll_next(cx)),
                None => None,
            }) else {
                self.inner = None;
                if self.pending.is_empty() {
                    return Poll::Ready(None);
                } else {
                    return Poll::Pending;
                }
            };

            let Ok(Diff { before, after }) = &next_tde.values else {
                self.pending
                    .push_back(Box::pin(ready(vec![CopyHistoryTreeDiffEntry::simple(
                        next_tde,
                    )])));
                continue;
            };

            let path = next_tde.path.clone();
            let before = before.clone();
            let after = after.clone();

            let simple = CopyHistoryTreeDiffEntry::simple(next_tde);

            // Don't try copy-tracing if we have conflicts on either side.
            let Some(before) = before.as_resolved() else {
                self.pending.push_back(Box::pin(ready(vec![simple])));
                continue;
            };
            let Some(after) = after.as_resolved() else {
                self.pending.push_back(Box::pin(ready(vec![simple])));
                continue;
            };

            match (before, after) {
                // If we have files with matching copy_ids, no need to do copy-tracing.
                (Some(f1 @ TreeValue::File { .. }), Some(f2 @ TreeValue::File { .. }))
                    if f1.copy_id() == f2.copy_id() =>
                {
                    self.pending.push_back(Box::pin(ready(vec![simple])));
                }

                // For files with non-matching copy-ids, or for a non-file that changes to a file,
                // mark the first as deleted and do copy-tracing on the second.
                (other, Some(f @ TreeValue::File { .. })) => {
                    let future = maybe_split_entries(
                        self.before_tree.clone(),
                        self.after_tree.clone(),
                        other.clone(),
                        f.clone(),
                        path.clone(),
                    );
                    self.pending.push_back(Box::pin(future));
                }

                // Anything else (e.g. file => non-file non-tree), issue a simple diff entry.
                _ => self.pending.push_back(Box::pin(ready(vec![simple]))),
            }
        }
    }
}

async fn classify_source(
    tree: &MergedTree,
    current_id: &CopyId,
    parent_path: RepoPathBuf,
    parent_val: &TreeValue,
    copy_graph: &CopyGraph,
) -> BackendResult<CopyHistorySource> {
    let before_id = parent_val
        .copy_id()
        .expect("expected TreeValue::File with a CopyId");

    let current_history = tree.store().backend().read_copy(current_id).await?;

    // First, check to see if we're looking at the same path with different copy
    // IDs, but an ancestor relationship between the histories. If so, this is a
    // "normal" diff source.
    if current_history.current_path == parent_path
        && (is_descendant(copy_graph, current_id, before_id)
            || is_descendant(copy_graph, before_id, current_id))
    {
        return Ok(CopyHistorySource::Normal);
    }

    let after_val = tree.path_value_async(&parent_path).await?;
    // We're getting our arguments from `find_diff_sources_from_copies`, so we
    // shouldn't have to worry about missing paths or conflicts. So let's just
    // be lazy and `.expect()` our way out of all the `Option`s.
    let after_id = after_val
        .to_copy_id_merge()
        .expect("expected merge of `TreeValue::File`s")
        .resolve_trivial(SameChange::Accept)
        .expect("expected no CopyId conflicts")
        .clone()
        // This may be absent, but we check for that later, so use a placeholder for now
        .unwrap_or_else(CopyId::placeholder);

    // Renames can come in two forms:
    // 1) parent_path is no longer present in after_tree, or
    // 2) parent_path in before_tree & after_tree are not ancestors/descendants of
    //    each other
    //
    //    NB: for this case, a file with the same copy_id is considered to be an
    //    ancestor of itself
    if after_val.is_absent()
        || !(is_descendant(copy_graph, before_id, &after_id)
            || is_descendant(copy_graph, &after_id, before_id))
    {
        Ok(CopyHistorySource::Rename(parent_path))
    } else {
        Ok(CopyHistorySource::Copy(parent_path))
    }
}

// TODO: this may emit two diff entries, where the old diffstream would contain
// only one. We could block on the result of the copy-tracing future, and then
// only split the diff entry if a copy ancestor was found, but for now let's
// keep things simple and always split the entry.
async fn maybe_split_entries(
    before_tree: MergedTree,
    after_tree: MergedTree,
    before_val: Option<TreeValue>,
    file: TreeValue,
    target_path: RepoPathBuf,
) -> Vec<CopyHistoryTreeDiffEntry> {
    let mut result = vec![];
    if let Some(other) = before_val {
        result.push(CopyHistoryTreeDiffEntry {
            target_path: target_path.clone(),
            diffs: Ok(Merge::resolved(CopyHistoryDiffTerm {
                target_value: None,
                sources: vec![(
                    CopyHistorySource::Normal,
                    Merge::resolved(Some(other.clone())),
                )],
            })),
        });
    }
    result.push(tree_diff_entry_from_copies(before_tree, after_tree, file, target_path).await);
    result
}

async fn tree_diff_entry_from_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    file: TreeValue,
    target_path: RepoPathBuf,
) -> CopyHistoryTreeDiffEntry {
    CopyHistoryTreeDiffEntry {
        target_path,
        diffs: diffs_from_copies(before_tree, after_tree, file).await,
    }
}

async fn diffs_from_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    file: TreeValue,
) -> BackendResult<Merge<CopyHistoryDiffTerm>> {
    let copy_id = file.copy_id().ok_or(BackendError::Other(
        "Expected TreeValue::File with a CopyId".into(),
    ))?;
    let related_copies = before_tree
        .store()
        .backend()
        .get_related_copies(copy_id)
        .await?;
    let copy_graph: CopyGraph = related_copies.iter().cloned().collect();

    let copies =
        find_diff_sources_from_copies(&before_tree, copy_id, &copy_graph, &related_copies).await?;

    try_join_all(copies.into_iter().map(async |(path, val)| {
        classify_source(&after_tree, copy_id, path, &val, &copy_graph)
            .await
            .map(|source| (source, Merge::resolved(Some(val))))
    }))
    .await
    .map(|sources| {
        Merge::resolved(CopyHistoryDiffTerm {
            target_value: Some(file),
            sources,
        })
    })
}

// Finds at most one related TreeValue::File present in `tree` per parent listed
// in `file`'s CopyHistory.
async fn find_diff_sources_from_copies(
    tree: &MergedTree,
    copy_id: &CopyId,
    copy_graph: &CopyGraph,
    related_copies: &Vec<(CopyId, CopyHistory)>,
) -> BackendResult<Vec<(RepoPathBuf, TreeValue)>> {
    // Related copies MUST contain ancestors AND descendants. It may also contain
    // unrelated copies.
    let history = copy_graph.get(copy_id).ok_or(BackendError::Other(
        "CopyId should be present in `get_related_copies()` result".into(),
    ))?;

    let mut sources = vec![];

    // TODO: this correctly finds the shallowest relative, but it only finds
    // one. I'm not sure what is the best thing to do when one of our parents
    // itself has multiple parents. E.g., if we have a CopyHistory graph like
    //
    //      D
    //      |
    //      C
    //     / \
    //    A   B
    //
    // where D is `file`, C is its parent but is not present in `tree`, but both A
    // and B are present, this will find either A or B, not both. Should we
    // return both A and B instead? I don't think there's a way to do that with
    // the current dag_walk functions. Do we care enough to implement something
    // new there that pays more attention to the depth in the DAG? Perhaps
    // a variant of closest_common_node?
    'parents: for parent_copy_id in &history.parents {
        let mut absent_ancestors = HashSet::new();

        // First, try to find the parent or a direct ancestor in the tree
        for ancestor_id in traverse_copy_history(copy_graph, parent_copy_id) {
            let ancestor_history = copy_graph.get(ancestor_id).ok_or(BackendError::Other(
                "Ancestor CopyId should be present in `get_related_copies()` result".into(),
            ))?;
            if let Some(ancestor) = tree.copy_value(ancestor_id).await? {
                sources.push((ancestor_history.current_path.clone(), ancestor));
                continue 'parents;
            } else {
                absent_ancestors.insert(ancestor_id);
            }
        }

        // If not, then try descendants of the parent
        //
        // TODO: This will find the shallowest relative, when what we really want is
        // probably the "closest" relative.
        for (related_id, related_history) in related_copies {
            if *related_id == *parent_copy_id {
                break;
            }
            if is_descendant(copy_graph, parent_copy_id, related_id)
                && let Some(relative) = tree.copy_value(related_id).await?
            {
                sources.push((related_history.current_path.clone(), relative));
                continue 'parents;
            }
        }

        // Finally, try descendants of any ancestor
        //
        // TODO: This will find the shallowest relative, when what we really want is
        // probably the "closest" relative.
        for (related_id, related_history) in related_copies {
            for ancestor_id in &absent_ancestors {
                if is_descendant(copy_graph, ancestor_id, related_id)
                    && let Some(relative) = tree.copy_value(related_id).await?
                {
                    sources.push((related_history.current_path.clone(), relative));
                    continue 'parents;
                }
            }
        }
    }

    if history.parents.is_empty() {
        // If there are no parents, let's instead look for a descendant (this handles
        // the reverse-diff case of a file rename.
        for (related_id, related_history) in related_copies {
            if *related_id == *copy_id {
                break;
            }
            if is_descendant(copy_graph, copy_id, related_id)
                && let Some(relative) = tree.copy_value(related_id).await?
            {
                sources.push((related_history.current_path.clone(), relative));
                break;
            }
        }
    }

    Ok(sources)
}

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
        let store = self.base_tree.store.clone();
        let labels = self.base_tree.labels().clone();
        let new_tree_ids = self.write_merged_trees()?;
        match new_tree_ids.simplify().into_resolved() {
            Ok(single_tree_id) => Ok(MergedTree::resolved(store, single_tree_id)),
            Err(tree_ids) => {
                let tree = MergedTree::new(store, tree_ids, labels);
                tree.resolve().block_on()
            }
        }
    }

    fn write_merged_trees(self) -> BackendResult<Merge<TreeId>> {
        let store = self.base_tree.store;
        let mut base_tree_ids = self.base_tree.tree_ids;
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
