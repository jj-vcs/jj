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
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
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
use futures::future::try_join_all;
use futures::stream::BoxStream;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CopyHistory;
use crate::backend::CopyId;
use crate::backend::MergedTreeId;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::copies::CopiesTreeDiffEntry;
use crate::copies::CopiesTreeDiffStream;
use crate::copies::CopyRecords;
use crate::dag_walk::topo_order_reverse;
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

/// Presents a view of a merged set of trees.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct MergedTree {
    trees: Merge<Tree>,
}

impl MergedTree {
    /// Creates a new `MergedTree` representing a single tree without conflicts.
    pub fn resolved(tree: Tree) -> Self {
        Self::new(Merge::resolved(tree))
    }

    /// Creates a new `MergedTree` representing a merge of a set of trees. The
    /// individual trees must not have any conflicts.
    pub fn new(trees: Merge<Tree>) -> Self {
        debug_assert!(trees.iter().map(|tree| tree.dir()).all_equal());
        debug_assert!(
            trees
                .iter()
                .map(|tree| Arc::as_ptr(tree.store()))
                .all_equal()
        );
        Self { trees }
    }

    /// Returns the underlying `Merge<Tree>`.
    pub fn as_merge(&self) -> &Merge<Tree> {
        &self.trees
    }

    /// Extracts the underlying `Merge<Tree>`.
    pub fn take(self) -> Merge<Tree> {
        self.trees
    }

    /// This tree's directory
    pub fn dir(&self) -> &RepoPath {
        self.trees.first().dir()
    }

    /// The `Store` associated with this tree.
    pub fn store(&self) -> &Arc<Store> {
        self.trees.first().store()
    }

    /// Base names of entries in this directory.
    pub fn names<'a>(&'a self) -> Box<dyn Iterator<Item = &'a RepoPathComponent> + 'a> {
        Box::new(all_tree_basenames(&self.trees))
    }

    /// The value at the given basename. The value can be `Resolved` even if
    /// `self` is a `Merge`, which happens if the value at the path can be
    /// trivially merged. Does not recurse, so if `basename` refers to a Tree,
    /// then a `TreeValue::Tree` will be returned.
    pub fn value(&self, basename: &RepoPathComponent) -> MergedTreeVal<'_> {
        trees_value(&self.trees, basename)
    }

    /// Tries to resolve any conflicts, resolving any conflicts that can be
    /// automatically resolved and leaving the rest unresolved.
    pub async fn resolve(self) -> BackendResult<Self> {
        let merged = merge_trees(self.trees).await?;
        // If the result can be resolved, then `merge_trees()` above would have returned
        // a resolved merge. However, that function will always preserve the arity of
        // conflicts it cannot resolve. So we simplify the conflict again
        // here to possibly reduce a complex conflict to a simpler one.
        let simplified = merged.simplify();
        // If debug assertions are enabled, check that the merge was idempotent. In
        // particular,  that this last simplification doesn't enable further automatic
        // resolutions
        if cfg!(debug_assertions) {
            let re_merged = merge_trees(simplified.clone()).await.unwrap();
            debug_assert_eq!(re_merged, simplified);
        }
        Ok(Self { trees: simplified })
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
        !self.trees.is_resolved()
    }

    /// Gets the `MergeTree` in a subdirectory of the current tree. If the path
    /// doesn't correspond to a tree in any of the inputs to the merge, then
    /// that entry will be replace by an empty tree in the result.
    pub async fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Self>> {
        match self.value(name).into_resolved() {
            Ok(Some(TreeValue::Tree(sub_tree_id))) => {
                let subdir = self.dir().join(name);
                Ok(Some(Self::resolved(
                    self.store().get_tree_async(subdir, sub_tree_id).await?,
                )))
            }
            Ok(_) => Ok(None),
            Err(merge) => {
                if !merge.is_tree() {
                    return Ok(None);
                }
                let trees = merge
                    .try_map_async(async |value| match value {
                        Some(TreeValue::Tree(sub_tree_id)) => {
                            let subdir = self.dir().join(name);
                            self.store().get_tree_async(subdir, sub_tree_id).await
                        }
                        Some(_) => unreachable!(),
                        None => {
                            let subdir = self.dir().join(name);
                            Ok(Tree::empty(self.store().clone(), subdir))
                        }
                    })
                    .await?;
                Ok(Some(Self { trees }))
            }
        }
    }

    /// The value at the given path. The value can be `Resolved` even if
    /// `self` is a `Conflict`, which happens if the value at the path can be
    /// trivially merged.
    pub fn path_value(&self, path: &RepoPath) -> BackendResult<MergedTreeValue> {
        self.path_value_async(path).block_on()
    }

    /// Async version of `path_value()`.
    pub async fn path_value_async(&self, path: &RepoPath) -> BackendResult<MergedTreeValue> {
        assert_eq!(self.dir(), RepoPath::root());
        match path.split() {
            Some((dir, basename)) => match self.sub_tree_recursive(dir).await? {
                None => Ok(Merge::absent()),
                Some(tree) => Ok(tree.value(basename).cloned()),
            },
            None => Ok(self
                .trees
                .map(|tree| Some(TreeValue::Tree(tree.id().clone())))),
        }
    }

    /// The tree's id
    pub fn id(&self) -> MergedTreeId {
        MergedTreeId::Merge(self.trees.map(|tree| tree.id().clone()))
    }

    /// Look up the tree at the given path.
    pub async fn sub_tree_recursive(&self, path: &RepoPath) -> BackendResult<Option<Self>> {
        let mut current_tree = self.clone();
        for name in path.components() {
            match current_tree.sub_tree(name).await? {
                None => {
                    return Ok(None);
                }
                Some(sub_tree) => {
                    current_tree = sub_tree;
                }
            }
        }
        Ok(Some(current_tree))
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
        TreeEntriesIterator::new(&self.trees, matcher)
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
                &self.trees,
                &other.trees,
                matcher,
            )))
        } else {
            Box::pin(TreeDiffStreamImpl::new(
                &self.trees,
                &other.trees,
                matcher,
                concurrency,
            ))
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
        Box::pin(CopyHistoryDiffStream::new(
            stream,
            self,
            other,
            self.store().concurrency().max(1),
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
        let nested = Merge::from_vec(vec![self.trees, base.trees, other.trees]);
        Self {
            trees: nested.flatten().simplify(),
        }
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

fn all_tree_basenames(trees: &Merge<Tree>) -> impl Iterator<Item = &RepoPathComponent> {
    trees
        .iter()
        .map(|tree| tree.data().names())
        .kmerge()
        .dedup()
}

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

fn trees_value<'a>(trees: &'a Merge<Tree>, basename: &RepoPathComponent) -> MergedTreeVal<'a> {
    if let Some(tree) = trees.as_resolved() {
        return Merge::resolved(tree.value(basename));
    }
    let same_change = trees.first().store().merge_options().same_change;
    let value = trees.map(|tree| tree.value(basename));
    if let Some(resolved) = value.resolve_trivial(same_change) {
        return Merge::resolved(*resolved);
    }
    value
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
    fn new(trees: &Merge<Tree>, matcher: &'matcher dyn Matcher) -> Self {
        Self {
            store: trees.first().store().clone(),
            stack: vec![TreeEntriesDirItem::new(trees, matcher)],
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
            stack: vec![ConflictsDirItem::from(&tree.trees)],
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
    pub fn new(trees1: &Merge<Tree>, trees2: &Merge<Tree>, matcher: &'matcher dyn Matcher) -> Self {
        assert!(Arc::ptr_eq(trees1.first().store(), trees2.first().store()));
        let root_dir = RepoPath::root();
        let mut stack = Vec::new();
        if !matcher.visit(root_dir).is_nothing() {
            stack.push(TreeDiffDir::from_trees(root_dir, trees1, trees2, matcher));
        };
        Self {
            store: trees1.first().store().clone(),
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
        trees1: &Merge<Tree>,
        trees2: &Merge<Tree>,
        matcher: &'matcher dyn Matcher,
        max_concurrent_reads: usize,
    ) -> Self {
        assert!(Arc::ptr_eq(trees1.first().store(), trees2.first().store()));
        let mut stream = Self {
            store: trees1.first().store().clone(),
            matcher,
            items: BTreeMap::new(),
            pending_trees: BTreeMap::new(),
            max_concurrent_reads,
            max_queued_items: 10000,
        };
        stream.add_dir_diff_items(RepoPath::root(), trees1, trees2);
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

/// Describes the source of a CopyHistoryDiffTerm
#[derive(Clone, Debug, PartialEq)]
pub enum CopyHistorySource {
    /// The file was copied from a source at a different path
    Copy(RepoPathBuf),
    /// The file was renamed from a source at a different path
    Rename(RepoPathBuf),
    /// The source and target have the same path
    Normal,
}

/// Describes a single term of a copy-aware diff
#[derive(Debug, PartialEq)]
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
    // Simple conversion case where no copy detection is needed
    fn simple(tde: TreeDiffEntry) -> Self {
        let target_path = tde.path;
        let diffs = tde.values.map(|(from, to)| {
            let sources = if from.is_absent() {
                vec![]
            } else {
                vec![(CopyHistorySource::Normal, from)]
            };
            Merge::from_vec(
                to.into_iter()
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
    max_queued_entries: usize,
    queued_entries: VecDeque<CopyHistoryTreeDiffEntry>,
    max_concurrent_reads: usize,
    pending_copy_detection: Vec<Option<BoxFuture<'a, CopyHistoryTreeDiffEntry>>>,
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
        max_concurrent_reads: usize,
    ) -> Self {
        Self {
            inner: Some(inner),
            before_tree,
            after_tree,
            max_queued_entries: 10000, // Value copied from TreeDiffStreamImpl::new() above
            queued_entries: VecDeque::new(),
            max_concurrent_reads,
            pending_copy_detection: vec![],
        }
    }

    // Returns true when there are results ready in self.queued_entries
    fn poll_copy_detection(&mut self, cx: &mut Context<'_>) -> bool {
        let mut entries = VecDeque::new();

        loop {
            let mut some_pending = false;
            let mut all_pending = true;

            // Check the first `max_concurrent_reads` futures; if they're ready, hold the
            // results. If they're pending, put them back in the list of futures.
            for future_slot in self
                .pending_copy_detection
                .iter_mut()
                .take(self.max_concurrent_reads)
            {
                let Some(mut future) = future_slot.take() else {
                    continue;
                };
                if let Poll::Ready(result) = future.as_mut().poll(cx) {
                    entries.push_back(result);
                    all_pending = false;
                } else {
                    *future_slot = Some(future);
                    some_pending = true;
                }
            }
            self.pending_copy_detection.retain(|o| o.is_some());

            // We can safely return only if at least one future has been polled and returned
            // `Poll::Pending`. Otherwise, nothing would call the waker so we might never
            // get polled again.
            //
            // If everything we polled is Pending, then there's no reason to loop again.
            //
            // If at least one future returned Pending, we *could* return safely, but we
            // might as well keep polling until either everything is Pending
            // or until we hit our limit of queued items.
            if all_pending || (some_pending && self.queued_entries.len() >= self.max_queued_entries)
            {
                break;
            }
        }

        // Unfortunately, this means queued_entries doesn't have a
        // deterministic order, since we can't guarantee the order that the copy
        // detection futures resolve.
        self.queued_entries.append(&mut entries);

        !self.queued_entries.is_empty()
    }

    // Returns true when `self.after_tree` contains a copy-descendant of `file`, and
    // that descendant was either not present in `self.before_tree` or has
    // different contents than in `self.after_tree`.
    async fn is_modified_descendant_present(&self, file: &TreeValue) -> BackendResult<bool> {
        let Some(copy_id) = file.copy_id() else {
            return Err(BackendError::Other(
                "Expected TreeValue::File with a CopyId".into(),
            ));
        };

        // Related copies MUST contain ancestors AND descendants. It may also contain
        // unrelated copies.
        let related_copies = self
            .before_tree
            .store()
            .backend()
            .get_related_copies(&copy_id)
            .await?;
        let related_copies_map: HashMap<CopyId, CopyHistory> =
            related_copies.iter().cloned().collect();

        for (related_id, related_history) in &related_copies {
            let Some(maybe_descendant) =
                match_exact_file_entry(self.after_tree, &related_history.current_path, related_id)
                    .await?
            else {
                continue;
            };
            for ancestor_id in traverse_copy_history(&related_copies_map, related_id) {
                if *ancestor_id == copy_id && self.is_file_modified(&maybe_descendant).await? {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    // Returns true when `file` (which is present in `self.after_tree`) is either
    // missing in `self.before_tree` or has different contents.
    async fn is_file_modified(&self, file: &TreeValue) -> BackendResult<bool> {
        // `file` is a TreeValue::File present in `self.after_tree`
        let Some(copy_id) = file.copy_id() else {
            return Err(BackendError::Other(
                "Expected TreeValue::File with a CopyId".into(),
            ));
        };

        let history = self
            .before_tree
            .store()
            .backend()
            .read_copy(&copy_id)
            .await?;
        let old = match_exact_file_entry(self.before_tree, &history.current_path, &copy_id).await?;

        match old {
            None => Ok(true),
            Some(old_val) => Ok(old_val != *file),
        }
    }
}

impl Stream for CopyHistoryDiffStream<'_> {
    type Item = CopyHistoryTreeDiffEntry;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // First, check if we have queued work or completed futures to return.
            if !self.queued_entries.is_empty() || self.poll_copy_detection(cx) {
                return Poll::Ready(self.queued_entries.pop_front());
            }

            // If we didn't have queued or polled results above, we want to check our
            // wrapped stream for the next result. However, the stream may have previously
            // returned `Poll::Ready(None)`, in which case we're not supposed to poll it
            // again. But we can't just blindly assume that we're done, because we may have
            // pending copy-detection futures whose results are not ready yet.
            let Some(next_tde) = (match self.inner.as_mut() {
                Some(inner) => ready!(inner.as_mut().poll_next(cx)),
                None => None,
            }) else {
                self.inner = None;
                if self.pending_copy_detection.is_empty() {
                    return Poll::Ready(None);
                } else {
                    return Poll::Pending;
                }
            };

            let Ok((before, after)) = &next_tde.values else {
                return Poll::Ready(Some(CopyHistoryTreeDiffEntry::simple(next_tde)));
            };

            // Don't try copy-tracing if we have conflicts on either side.
            //
            // TODO: maybe we could make this work if the CopyIds resolve, even if the files
            // themselves don't?
            let Some(before) = before.as_resolved() else {
                return Poll::Ready(Some(CopyHistoryTreeDiffEntry::simple(next_tde)));
            };
            let Some(after) = after.as_resolved() else {
                return Poll::Ready(Some(CopyHistoryTreeDiffEntry::simple(next_tde)));
            };

            // Want to end up with before/after being None or Some(File)
            let (before, after) = match (before, after) {
                // If we have two Files with matching CopyIDs, we can skip copy-tracing
                (
                    Some(TreeValue::File { copy_id: copy1, .. }),
                    Some(TreeValue::File { copy_id: copy2, .. }),
                ) if copy1 == copy2 => {
                    return Poll::Ready(Some(CopyHistoryTreeDiffEntry::simple(next_tde)));
                }

                // If we only have Files, then we'll do copy-tracing below.
                (Some(TreeValue::File { .. }), None)
                | (Some(TreeValue::File { .. }), Some(TreeValue::File { .. }))
                | (None, Some(TreeValue::File { .. })) => (before, after),

                // If we have a File on one side and something else on the other, we need to
                // split the TreeDiffEntry into two; one with the copy-matched File, and the other
                // with the non-File as added or deleted.
                (Some(TreeValue::File { .. }), Some(other)) => {
                    // We're changing a path from a File to something else. First, queue up a
                    // DiffEntry for the "something else" and then look for renames for the File.
                    self.queued_entries
                        .push_back(CopyHistoryTreeDiffEntry::simple(TreeDiffEntry {
                            path: next_tde.path.clone(),
                            values: Ok((Merge::absent(), Merge::resolved(Some(other.clone())))),
                        }));
                    (before, &None)
                }

                (Some(other), Some(TreeValue::File { .. })) => {
                    self.queued_entries
                        .push_back(CopyHistoryTreeDiffEntry::simple(TreeDiffEntry {
                            path: next_tde.path.clone(),
                            values: Ok((Merge::resolved(Some(other.clone())), Merge::absent())),
                        }));
                    (&None, after)
                }

                _ => return Poll::Ready(Some(CopyHistoryTreeDiffEntry::simple(next_tde))),
            };

            // At this point, at least one of before/after is a TreeValue::File; if both are
            // Files, we know they have different copy IDs. Match against
            // ancestors/descendants in both directions, possibly creating two
            // `DiffStreamEntry`s

            // If we have a "before" file that didn't match at the same path in the "after"
            // tree, then this is either a deletion or a rename. Renames will be
            // handled by the "after" side of whatever descendant file matches
            // via copy-tracking to this "before" file, so we can skip emitting
            // an entry for that case.
            if let Some(f) = before {
                // TODO: is there a better way to use the future rather than `block_on()`?
                match self.is_modified_descendant_present(f).block_on() {
                    Err(e) => {
                        self.queued_entries.push_back(CopyHistoryTreeDiffEntry {
                            target_path: next_tde.path.clone(),
                            diffs: Err(e),
                        });
                    }
                    // If we matched a descendant, then this is a rename, which can be skipped.
                    Ok(true) => {}
                    // If we didn't match a descendant, then this is a deletion.
                    Ok(false) => {
                        self.queued_entries
                            .push_back(CopyHistoryTreeDiffEntry::simple(TreeDiffEntry {
                                path: next_tde.path.clone(),
                                values: Ok((Merge::resolved(Some(f.clone())), Merge::absent())),
                            }));
                    }
                };
            }

            if let Some(f) = after {
                let fut = Box::pin(tree_diff_entry_from_copies(
                    self.before_tree.clone(),
                    self.after_tree.clone(),
                    f.clone(),
                    next_tde.path,
                ));
                self.pending_copy_detection.push(Some(fut));
            }
        }
    }
}

async fn is_copy_descendant(
    store: &Arc<Store>,
    ancestor: &TreeValue,
    maybe_descendant: MergedTreeValue,
) -> BackendResult<bool> {
    // TODO: cleanup error handling?
    // We're getting our arguments from `copy_or_rename()` (ultimately, from
    // `find_related_copies_in_tree`), so we shouldn't have to worry about missing
    // paths or conflicts. So let's just be lazy and `.expect()` our way out of
    // all the Options.
    let ancestor_id = ancestor.copy_id().expect("expected TreeValue::File");
    let descendant_id = maybe_descendant
        .to_copy_id_merge()
        .expect("expected merge of `TreeValue::File`s")
        .resolve_trivial(SameChange::Accept)
        .expect("expected no CopyId conflicts")
        .clone()
        .expect("expected a CopyId");

    // Avoid querying the backend for the case where no renames have happened
    if descendant_id == ancestor_id {
        return Ok(true);
    }

    let related_copies_map: HashMap<_, _> = store
        .backend()
        .get_related_copies(&descendant_id)
        .await?
        .into_iter()
        .collect();
    // get_related_copies() returns children before parents
    for copy_id in traverse_copy_history(&related_copies_map, &ancestor_id) {
        if *copy_id == ancestor_id {
            break;
        } else if *copy_id == descendant_id {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn copy_or_rename(
    tree: &MergedTree,
    path: RepoPathBuf,
    orig_val: &TreeValue,
) -> BackendResult<CopyHistorySource> {
    let treeval = tree.path_value_async(&path).await?;
    if treeval.is_absent() || !is_copy_descendant(tree.store(), orig_val, treeval).await? {
        Ok(CopyHistorySource::Rename(path))
    } else {
        Ok(CopyHistorySource::Copy(path))
    }
}

async fn tree_diff_entry_from_copies(
    before_tree: MergedTree,
    after_tree: MergedTree,
    file: TreeValue,
    target_path: RepoPathBuf,
) -> CopyHistoryTreeDiffEntry {
    let diffs = match find_related_copies_in_tree(&before_tree, file.clone()).await {
        Err(e) => Err(e),
        Ok(copies) => {
            let target_value = Some(file);
            let sources = try_join_all(copies.into_iter().map(async |(path, val)| {
                match copy_or_rename(&after_tree, path, &val).await {
                    Ok(c) => Ok((c, Merge::resolved(Some(val)))),
                    Err(e) => Err(e),
                }
            }))
            .await;

            match sources {
                Err(e) => Err(e),
                Ok(sources) => Ok(Merge::resolved(CopyHistoryDiffTerm {
                    target_value,
                    sources,
                })),
            }
        }
    };

    CopyHistoryTreeDiffEntry { target_path, diffs }
}

// Finds at most one related TreeValue::File present in `tree` per parent listed
// in `file`'s CopyHistory.
//
// TODO: this is not guaranteed to be the "best" choice for any given parent.
async fn find_related_copies_in_tree(
    tree: &MergedTree,
    file: TreeValue,
) -> BackendResult<Vec<(RepoPathBuf, TreeValue)>> {
    let Some(copy_id) = file.copy_id() else {
        return Err(BackendError::Other(
            "Expected TreeValue::File with a CopyId".into(),
        ));
    };

    let history = tree.store().backend().read_copy(&copy_id).await?;

    // Related copies MUST contain ancestors AND descendants. It may also contain
    // unrelated copies.
    let related_copies = tree.store().backend().get_related_copies(&copy_id).await?;
    let related_copies_map: HashMap<CopyId, CopyHistory> = related_copies.iter().cloned().collect();

    let mut found_parents = vec![];
    let mut missing_parents = HashMap::new();

    // TODO: I think this correctly finds the shallowest ancestor, but it only finds
    // one. I'm not sure what is the best thing to do when one of our parents
    // itself has multiple parents. E.g., if we have
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
    for parent_copy_id in &history.parents {
        let mut found = false;
        let mut seen_ancestors = HashSet::new();
        for ancestor_id in traverse_copy_history(&related_copies_map, parent_copy_id) {
            let ancestor_history = tree.store().backend().read_copy(ancestor_id).await?;
            if let Some(ancestor) =
                match_exact_file_entry(tree, &ancestor_history.current_path, ancestor_id).await?
            {
                found_parents.push((ancestor_history.current_path.clone(), ancestor));
                found = true;
                break;
            } else {
                seen_ancestors.insert(ancestor_id);
            }
        }
        if !found {
            missing_parents.insert(parent_copy_id, seen_ancestors);
        }
    }

    // TODO: same issues here; we don't pay attention to the case where our
    // parent(s) have multiple parents themselves. We also pick the first
    // related_history that links back to any known ancestor, which I think is
    // not guaranteed to be the shortest chain. So presumably we want
    // something like closest_common_node here too.
    'missing_parent: for (_, acceptable_ancestors) in missing_parents {
        // For any parents that were not found in `tree` above, see if there are
        // descendants in `tree`.
        for (related_id, related_history) in &related_copies {
            // The only thing that get_related_copies() promises is that children will be
            // listed before parents. We'll have to verify ourselves that
            // they're actually relevant; for example, that the file with that
            // CopyId actually exists in `tree`.
            let Some(ancestor) =
                match_exact_file_entry(tree, &related_history.current_path, related_id).await?
            else {
                continue;
            };

            for ancestor_id in traverse_copy_history(&related_copies_map, related_id) {
                // If we find an ancestor that matches something in missing_ancestors, then we
                // can use this file in the diff entry.
                let ancestor_history = tree.store().backend().read_copy(ancestor_id).await?;
                if acceptable_ancestors.contains(ancestor_id) {
                    found_parents.push((ancestor_history.current_path.clone(), ancestor));
                    continue 'missing_parent;
                }
            }
        }
    }

    Ok(found_parents)
}

// Gets a `TreeValue` from `tree` if the `TreeValue`:
// * is located at the expected path
// * AND is resolved
// * AND has the expected copy ID
async fn match_exact_file_entry(
    tree: &MergedTree,
    expected_path: &RepoPath,
    expected_copy_id: &CopyId,
) -> BackendResult<Option<TreeValue>> {
    let Some(Some(tree_entry)) = tree
        .path_value_async(expected_path)
        .await?
        .as_resolved()
        .cloned()
    else {
        return Ok(None);
    };

    let Some(copy_id) = tree_entry.copy_id() else {
        return Ok(None);
    };

    if copy_id == *expected_copy_id {
        Ok(Some(tree_entry))
    } else {
        Ok(None)
    }
}

fn traverse_copy_history<'a>(
    copies: &'a HashMap<CopyId, CopyHistory>,
    initial_id: &'a CopyId,
) -> Vec<&'a CopyId> {
    topo_order_reverse(
        [initial_id],
        |id| *id,
        |id| copies.get(id).unwrap().parents.iter(),
    )
}

/// Helper for writing trees with conflicts.
///
/// You start by creating an instance of this type with one or more
/// base trees. You then add overrides on top. The overrides may be
/// conflicts. Then you can write the result as a legacy tree
/// (allowing path-level conflicts) or as multiple conflict-free
/// trees.
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
        let base_tree_ids = match self.base_tree_id.clone() {
            MergedTreeId::Legacy(base_tree_id) => Merge::resolved(base_tree_id),
            MergedTreeId::Merge(base_tree_ids) => base_tree_ids,
        };
        let new_tree_ids = self.write_merged_trees(base_tree_ids, store)?;
        match new_tree_ids.simplify().into_resolved() {
            Ok(single_tree_id) => Ok(MergedTreeId::resolved(single_tree_id)),
            Err(tree_id) => {
                let tree = store.get_root_tree(&MergedTreeId::Merge(tree_id))?;
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
