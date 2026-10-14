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

use std::fmt;
use std::iter;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use either::Either;
use futures::Stream;
use futures::StreamExt as _;
use futures::future::try_join;
use futures::stream::BoxStream;
use itertools::EitherOrBoth;
use itertools::Itertools as _;

use crate::backend::BackendResult;
use crate::backend::CopyId;
use crate::backend::MergedTreeVal;
use crate::backend::MergedTreeValue;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::conflict_labels::ConflictLabels;
use crate::copies::CopiesTreeDiffEntry;
use crate::copies::CopyHistoryDiffStream;
use crate::copies::CopyHistoryTreeDiffEntry;
use crate::copies::CopyRecords;
use crate::copies::copies_tree_diff_stream;
use crate::filtered_tree_writer::FilteredTreeWriter;
use crate::matchers::EverythingMatcher;
use crate::matchers::Matcher;
use crate::merge::Diff;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::store::Store;
use crate::tree::ToTreeMergeExt as _;
use crate::tree::Tree;
use crate::tree::TreeMergeExt as _;
use crate::tree_merge::merge_trees;
use crate::tree_walk::OrderedTreeWalk;
use crate::tree_walk::TreeWalkQueue;
use crate::tree_walk::TreeWalkVisitor;

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
        Self {
            store,
            tree_ids: Merge::resolved(tree_id),
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
    pub async fn trees(&self) -> BackendResult<Merge<Tree>> {
        self.tree_ids
            .try_map_async(|id| self.store.get_tree(RepoPathBuf::root(), id))
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

    /// Stream of the conflicts in this tree, including subtrees. Recurses into
    /// subtrees and yields conflicts in those, but only if all sides are
    /// trees, so tree/file conflicts will be reported as a single conflict, not
    /// one for each path in the tree.
    pub fn conflicts_stream(&self) -> TreeConflictsStream<'static> {
        self.conflicts_stream_matching(&EverythingMatcher)
    }

    /// Like `conflicts_stream()` but restricted by a matcher.
    pub fn conflicts_stream_matching<'matcher>(
        &self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeConflictsStream<'matcher> {
        let mut queue = TreeWalkQueue::new(self.store().concurrency());
        if !self.tree_ids.is_resolved() {
            let root = RepoPathBuf::root();
            if !matcher.visit(&root).is_nothing() {
                let root_trees_fut = read_trees(
                    self.store.clone(),
                    root.clone(),
                    self.to_merged_tree_value(),
                );
                queue.schedule(root, root_trees_fut);
            }
        }
        let visitor = ConflictsVisitor {
            store: self.store.clone(),
            matcher,
        };
        OrderedTreeWalk::new(visitor, queue).boxed()
    }

    /// Whether this tree has conflicts.
    pub fn has_conflict(&self) -> bool {
        !self.tree_ids.is_resolved()
    }

    /// The value at the given path. The value can be `Resolved` even if
    /// `self` is a `Conflict`, which happens if the value at the path can be
    /// trivially merged.
    pub async fn path_value(&self, path: &RepoPath) -> BackendResult<MergedTreeValue> {
        match path.split() {
            Some((dir, basename)) => {
                let trees = self.trees().await?;
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
        let merged_val = self.path_value(&copy.current_path).await?;
        match merged_val.into_resolved() {
            Ok(Some(val)) if val.copy_id() == Some(id) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    fn to_merged_tree_value(&self) -> MergedTreeValue {
        self.tree_ids
            .map(|tree_id| Some(TreeValue::Tree(tree_id.clone())))
    }

    /// Stream of the entries in this tree. Subtrees are visited recursively.
    /// Subtrees that differ between the current `MergedTree`'s terms are merged
    /// on the fly. Missing terms are treated as empty directories. Subtrees
    /// that conflict with non-trees are not visited. For example, if current
    /// tree is a merge of 3 trees, and the entry for 'foo' is a conflict
    /// between a change subtree and a symlink (i.e. the subdirectory was
    /// replaced by symlink in one side of the conflict), then the entry for
    /// `foo` itself will be emitted, but no entries from inside `foo/` from
    /// either of the trees will be.
    pub fn entries_stream(&self) -> TreeEntriesStream<'static> {
        self.entries_stream_matching(&EverythingMatcher)
    }

    /// Like `entries_stream()` but restricted by a matcher.
    pub fn entries_stream_matching<'matcher>(
        &self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeEntriesStream<'matcher> {
        let mut queue = TreeWalkQueue::new(self.store().concurrency());
        let root = RepoPathBuf::root();
        if !matcher.visit(&root).is_nothing() {
            let root_trees_fut = read_trees(
                self.store.clone(),
                root.clone(),
                self.to_merged_tree_value(),
            );
            queue.schedule(root, root_trees_fut);
        }
        let visitor = TreeEntriesVisitor {
            store: self.store.clone(),
            matcher,
        };
        OrderedTreeWalk::new(visitor, queue).boxed()
    }

    /// Stream of the differences between this tree and another tree.
    fn diff_stream_internal<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        TreeDiffStreamImpl::new(self, other, matcher, self.store().concurrency()).boxed()
    }

    /// Stream of the differences between this tree and another tree.
    pub fn diff_stream<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        stream_without_trees(self.diff_stream_internal(other, matcher))
    }

    /// Like `diff_stream()` but trees with diffs themselves are also included.
    pub fn diff_stream_with_trees<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        self.diff_stream_internal(other, matcher)
    }

    /// Like `diff_stream()` but files in a removed tree will be returned before
    /// a file that replaces it.
    pub fn diff_stream_for_file_system<'matcher>(
        &self,
        other: &Self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeDiffStream<'matcher> {
        DiffStreamForFileSystem::new(self.diff_stream_internal(other, matcher)).boxed()
    }

    /// Returns a slice of this tree filtered by the matcher. The returned tree
    /// has been written to the backend; it is not a lazily calculated  view of
    /// the original tree. The arity and conflict labels will be the same as
    /// this tree's.
    pub async fn filtered(&self, matcher: &dyn Matcher) -> BackendResult<Self> {
        FilteredTreeWriter::new(self, matcher).write().await
    }

    /// Like `diff_stream()` but takes the given copy records into account.
    pub fn diff_stream_with_copies<'a>(
        &self,
        other: &Self,
        matcher: &'a dyn Matcher,
        copy_records: &'a CopyRecords,
    ) -> BoxStream<'a, CopiesTreeDiffEntry> {
        let stream = self.diff_stream(other, matcher);
        copies_tree_diff_stream(stream, self.clone(), other.clone(), copy_records)
    }

    /// Like `diff_stream()` but takes CopyHistory into account.
    pub fn diff_stream_with_copy_history<'a>(
        &'a self,
        other: &'a Self,
        matcher: &'a dyn Matcher,
    ) -> BoxStream<'a, CopyHistoryTreeDiffEntry> {
        let stream = self.diff_stream(other, matcher);
        CopyHistoryDiffStream::new(stream, self, other).boxed()
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
            .into_map(|(tree, _label)| tree.into_tree_ids())
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

/// Type alias for the result from `MergedTree::conflicts_stream()`.
pub type TreeConflictsStream<'matcher> =
    BoxStream<'matcher, (RepoPathBuf, BackendResult<MergedTreeValue>)>;

/// Type alias for the result from `MergedTree::entries_stream()`.
pub type TreeEntriesStream<'matcher> =
    BoxStream<'matcher, (RepoPathBuf, BackendResult<MergedTreeValue>)>;

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

/// Reads the trees for `value`, which must be a tree.
async fn read_trees(
    store: Arc<Store>,
    dir: RepoPathBuf,
    value: MergedTreeValue,
) -> BackendResult<Merge<Tree>> {
    let trees = value
        .to_tree_merge(&store, &dir)
        .await?
        .expect("Should be tree merge");
    Ok(trees)
}

/// Visits the entries in a tree, reading subtrees concurrently.
struct TreeEntriesVisitor<'matcher> {
    store: Arc<Store>,
    matcher: &'matcher dyn Matcher,
}

impl<'matcher> TreeWalkVisitor<'matcher> for TreeEntriesVisitor<'matcher> {
    type Work = Merge<Tree>;
    type Item = MergedTreeValue;

    fn visit(
        &self,
        dir: &RepoPath,
        trees: Self::Work,
        queue: &mut TreeWalkQueue<'matcher, Self::Work, Self::Item>,
    ) {
        for (name, value) in all_tree_entries(&trees) {
            let path = dir.join(name);
            if value.is_tree() {
                // TODO: Handle the other cases (specific files and trees)
                if self.matcher.visit(&path).is_nothing() {
                    continue;
                }
                let trees_fut = read_trees(self.store.clone(), path.clone(), value.cloned());
                queue.schedule(path, trees_fut);
            } else if value.is_present() && self.matcher.matches(&path) {
                queue.emit(path, value.cloned());
            }
        }
    }
}

/// Visits the conflicted entries in a tree, reading subtrees concurrently.
struct ConflictsVisitor<'matcher> {
    store: Arc<Store>,
    matcher: &'matcher dyn Matcher,
}

impl<'matcher> TreeWalkVisitor<'matcher> for ConflictsVisitor<'matcher> {
    type Work = Merge<Tree>;
    type Item = MergedTreeValue;

    fn visit(
        &self,
        dir: &RepoPath,
        trees: Self::Work,
        queue: &mut TreeWalkQueue<'matcher, Self::Work, Self::Item>,
    ) {
        if trees.is_resolved() {
            return;
        }
        for (basename, value) in all_tree_entries(&trees) {
            if value.is_resolved() {
                continue;
            }
            let path = dir.join(basename);
            if value.is_tree() {
                // If all sides are trees or missing, descend into the merged tree
                if self.matcher.visit(&path).is_nothing() {
                    continue;
                }
                let trees_fut = read_trees(self.store.clone(), path.clone(), value.cloned());
                queue.schedule(path, trees_fut);
            } else if self.matcher.matches(&path) {
                // Otherwise this is a conflict between files, trees, etc. If they could be
                // automatically resolved, they should have been when the top-level tree
                // conflict was written, so we assume that they can't be.
                queue.emit(path, value.cloned());
            }
        }
    }
}

/// Stream of differences between two trees.
pub struct TreeDiffStreamImpl<'matcher> {
    walk: OrderedTreeWalk<'matcher, TreeDiffVisitor<'matcher>>,
}

impl<'matcher> TreeDiffStreamImpl<'matcher> {
    /// Creates a iterator over the differences between two trees. Generally
    /// prefer `MergedTree::diff_stream()` of calling this directly.
    ///
    /// `max_concurrent_reads` is the maximum number of paths to read trees for
    /// concurrently. There will often be twice as many pending
    /// `Backend::read_tree()` calls - for the "before" and "after" sides. For
    /// conflicts, there will be even more.
    pub fn new(
        tree1: &MergedTree,
        tree2: &MergedTree,
        matcher: &'matcher dyn Matcher,
        max_concurrent_reads: usize,
    ) -> Self {
        assert!(Arc::ptr_eq(tree1.store(), tree2.store()));
        let store = tree1.store().clone();
        let mut queue = TreeWalkQueue::new(max_concurrent_reads);
        let dir = RepoPathBuf::root();
        let merged_tree1 = tree1.to_merged_tree_value();
        let merged_tree2 = tree2.to_merged_tree_value();
        let root_diff = Diff::new(merged_tree1.clone(), merged_tree2.clone());
        if root_diff.is_changed() {
            if matcher.matches(&dir) {
                queue.emit(dir.clone(), root_diff);
            }
            if !matcher.visit(&dir).is_nothing() {
                let root_trees_fut = try_join(
                    TreeDiffVisitor::trees(store.clone(), dir.clone(), merged_tree1),
                    TreeDiffVisitor::trees(store.clone(), dir.clone(), merged_tree2),
                );
                queue.schedule(dir, root_trees_fut);
            }
        }
        let visitor = TreeDiffVisitor { store, matcher };
        Self {
            walk: OrderedTreeWalk::new(visitor, queue),
        }
    }
}

impl Stream for TreeDiffStreamImpl<'_> {
    type Item = TreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.walk
            .poll_next_unpin(cx)
            .map(|entry| entry.map(|(path, values)| TreeDiffEntry { path, values }))
    }
}

struct TreeDiffVisitor<'matcher> {
    store: Arc<Store>,
    matcher: &'matcher dyn Matcher,
}

impl TreeDiffVisitor<'_> {
    /// Gets the given trees if `values` are trees, otherwise an empty tree.
    async fn trees(
        store: Arc<Store>,
        dir: RepoPathBuf,
        values: MergedTreeValue,
    ) -> BackendResult<Merge<Tree>> {
        let trees = values.to_tree_merge(&store, &dir).await?;
        Ok(trees.unwrap_or_else(|| Merge::resolved(Tree::empty(store, dir))))
    }
}

impl<'matcher> TreeWalkVisitor<'matcher> for TreeDiffVisitor<'matcher> {
    type Work = (Merge<Tree>, Merge<Tree>);
    type Item = Diff<MergedTreeValue>;

    fn visit(
        &self,
        dir: &RepoPath,
        (trees1, trees2): Self::Work,
        queue: &mut TreeWalkQueue<'matcher, Self::Work, Self::Item>,
    ) {
        for (basename, diff) in merged_tree_entry_diff(&trees1, &trees2) {
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

            if file_matches || self.matcher.matches(&path) {
                queue.emit(path.clone(), Diff::new(before.cloned(), after.cloned()));
            }

            // If the path was a tree on either side of the diff, read those trees.
            if tree_matches {
                let before_tree_future =
                    Self::trees(self.store.clone(), path.clone(), before.cloned());
                let after_tree_future =
                    Self::trees(self.store.clone(), path.clone(), after.cloned());
                queue.schedule(path, try_join(before_tree_future, after_tree_future));
            }
        }
    }
}

fn stream_without_trees(stream: TreeDiffStream) -> TreeDiffStream {
    stream
        .filter_map(|mut entry| async move {
            let skip_tree = |merge: MergedTreeValue| {
                if merge.is_tree() {
                    Merge::absent()
                } else {
                    merge
                }
            };
            entry.values = entry.values.map(|diff| diff.map(skip_tree));

            // Filter out entries where neither side is present.
            let any_present = entry.values.as_ref().map_or(true, |diff| {
                diff.before.is_present() || diff.after.is_present()
            });
            any_present.then_some(entry)
        })
        .boxed()
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
            // Filter out changes where neither side (before or after) is_file_like.
            // This ensures we only process file-level changes and transitions.
            if let Ok(diff) = &next.values
                && !diff.before.is_file_like()
                && !diff.after.is_file_like()
            {
                continue;
            }

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
