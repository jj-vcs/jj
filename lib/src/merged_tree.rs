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
use futures::future::try_join;
use futures::stream::BoxStream;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendResult;
use crate::backend::MergedTreeId;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::copies::CopiesTreeDiffEntry;
use crate::copies::CopiesTreeDiffStream;
use crate::copies::CopyRecords;
use crate::matchers::EverythingMatcher;
use crate::matchers::Matcher;
use crate::merge::Diff;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::merge::MergedTreeVal;
use crate::merge::MergedTreeValue;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::store::Store;
use crate::tree::Tree;
use crate::tree_builder::TreeBuilder;
use crate::tree_merge::merge_trees;

/// Presents a view of a merged set of trees at the root directory. In the
/// future, this may store additional metadata like conflict labels, so tree IDs
/// should be compared instead when checking for file changes.
#[derive(Clone)]
pub struct MergedTree {
    store: Arc<Store>,
    tree_ids: Merge<TreeId>,
}

impl fmt::Debug for MergedTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergedTree")
            .field("tree_ids", &self.tree_ids)
            .finish_non_exhaustive()
    }
}

impl MergedTree {
    /// Creates a `MergedTree` with the given resolved tree ID.
    pub fn resolved(store: Arc<Store>, tree_id: TreeId) -> Self {
        Self::new(store, Merge::resolved(tree_id))
    }

    /// Creates a `MergedTree` with the given tree IDs.
    pub fn new(store: Arc<Store>, tree_ids: Merge<TreeId>) -> Self {
        Self { store, tree_ids }
    }

    /// The `Store` associated with this tree.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// The underlying tree IDs for this `MergedTree`.
    pub fn tree_ids(&self) -> &Merge<TreeId> {
        &self.tree_ids
    }

    /// Extracts the underlying tree IDs for this `MergedTree`.
    pub fn into_tree_ids(self) -> Merge<TreeId> {
        self.tree_ids
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

    /// Tries to resolve any conflicts, resolving any conflicts that can be
    /// automatically resolved and leaving the rest unresolved.
    pub async fn resolve(self) -> BackendResult<Self> {
        let merged = merge_trees(&self.store, self.tree_ids).await?;
        // If the result can be resolved, then `merge_trees()` above would have returned
        // a resolved merge. However, that function will always preserve the arity of
        // conflicts it cannot resolve. So we simplify the conflict again
        // here to possibly reduce a complex conflict to a simpler one.
        let simplified = merged.simplify();
        // If debug assertions are enabled, check that the merge was idempotent. In
        // particular,  that this last simplification doesn't enable further automatic
        // resolutions
        if cfg!(debug_assertions) {
            let re_merged = merge_trees(&self.store, simplified.clone()).await.unwrap();
            debug_assert_eq!(re_merged, simplified);
        }
        Ok(Self::new(self.store, simplified))
    }

    /// An iterator over the conflicts in this tree, including subtrees.
    /// Recurses into subtrees and yields conflicts in those, but only if
    /// all sides are trees, so tree/file conflicts will be reported as a single
    /// conflict, not one for each path in the tree.
    // TODO: Restrict this by a matcher (or add a separate method for that).
    pub fn conflicts(
        &self,
    ) -> impl Iterator<Item = (RepoPathBuf, BackendResult<MergedTreeValue>)> + use<> {
        ConflictIterator::new(self)
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

    fn to_merged_tree_value(&self) -> MergedTreeValue {
        self.tree_ids
            .map(|tree_id| Some(TreeValue::Tree(tree_id.clone())))
    }

    /// The tree's id
    // TODO: delete this method after deleting `MergedTreeId`
    pub fn id(&self) -> MergedTreeId {
        MergedTreeId::new(self.tree_ids.clone())
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

    /// Merges this tree with `other`, using `base` as base. Any conflicts will
    /// be resolved recursively if possible.
    pub async fn merge(self, base: Self, other: Self) -> BackendResult<Self> {
        self.merge_no_resolve(base, other).resolve().await
    }

    /// Merges this tree with `other`, using `base` as base, without attempting
    /// to resolve file conflicts.
    pub fn merge_no_resolve(self, base: Self, other: Self) -> Self {
        debug_assert!(Arc::ptr_eq(&base.store, &self.store));
        debug_assert!(Arc::ptr_eq(&other.store, &self.store));
        let nested = Merge::from_vec(vec![self.tree_ids, base.tree_ids, other.tree_ids]);
        Self::new(self.store, nested.flatten().simplify())
    }
}

/// A single entry in a tree diff.
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

impl From<&Merge<Tree>> for ConflictsDirItem {
    fn from(trees: &Merge<Tree>) -> Self {
        let dir = trees.first().dir();
        if trees.is_resolved() {
            return Self { entries: vec![] };
        }

        let mut entries = vec![];
        for (basename, value) in all_tree_entries(trees) {
            if !value.is_resolved() {
                entries.push((dir.join(basename), value.cloned()));
            }
        }
        entries.reverse();
        Self { entries }
    }
}

struct ConflictIterator {
    store: Arc<Store>,
    stack: Vec<ConflictsDirItem>,
}

impl ConflictIterator {
    fn new(tree: &MergedTree) -> Self {
        Self {
            store: tree.store().clone(),
            stack: vec![ConflictsDirItem {
                entries: vec![(RepoPathBuf::root(), tree.to_merged_tree_value())],
            }],
        }
    }
}

impl Iterator for ConflictIterator {
    type Item = (RepoPathBuf, BackendResult<MergedTreeValue>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            if let Some((path, tree_values)) = top.entries.pop() {
                match tree_values.to_tree_merge(&self.store, &path).block_on() {
                    Ok(Some(trees)) => {
                        // If all sides are trees or missing, descend into the merged tree
                        self.stack.push(ConflictsDirItem::from(&trees));
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
                let _ = self.pending_trees.remove_entry(&dir).unwrap();
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

/// Helper for writing trees with conflicts.
///
/// You start by creating an instance of this type with one or more
/// base trees. You then add overrides on top. The overrides may be
/// conflicts. Then you can write the result as a merge of trees.
pub struct MergedTreeBuilder {
    base_tree_id: MergedTreeId,
    overrides: BTreeMap<RepoPathBuf, MergedTreeValue>,
}

impl MergedTreeBuilder {
    /// Create a new builder with the given trees as base.
    pub fn new(base_tree_id: MergedTreeId) -> Self {
        Self {
            base_tree_id,
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
    pub fn write_tree(self, store: &Arc<Store>) -> BackendResult<MergedTreeId> {
        let base_tree_ids = self.base_tree_id.as_merge().clone();
        let new_tree_ids = self.write_merged_trees(base_tree_ids, store)?;
        match new_tree_ids.simplify().into_resolved() {
            Ok(single_tree_id) => Ok(MergedTreeId::resolved(single_tree_id)),
            Err(tree_id) => {
                let tree = store.get_root_tree(&MergedTreeId::new(tree_id))?;
                let resolved = tree.resolve().block_on()?;
                Ok(resolved.id())
            }
        }
    }

    fn write_merged_trees(
        self,
        mut base_tree_ids: Merge<TreeId>,
        store: &Arc<Store>,
    ) -> BackendResult<Merge<TreeId>> {
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
                    for builder in tree_builders.iter_mut() {
                        builder.set_or_remove(path.clone(), value.clone());
                    }
                }
                Err(mut values) => {
                    values.pad_to(num_sides, &None);
                    // This path was overridden with a conflicted value. Apply each term to
                    // its corresponding builder.
                    for (builder, value) in zip(tree_builders.iter_mut(), values) {
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
