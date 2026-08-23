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

#![expect(missing_docs)]

use std::borrow::Borrow;
use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend;
use crate::backend::BackendResult;
use crate::backend::MergedTreeVal;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeEntriesNonRecursiveIterator;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::backend::borrow_tree_value;
use crate::matchers::Matcher;
use crate::merge::Merge;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::store::Store;

#[derive(Clone)]
pub struct Tree {
    store: Arc<Store>,
    dir: RepoPathBuf,
    id: TreeId,
    data: Arc<backend::Tree>,
}

impl Debug for Tree {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("Tree")
            .field("dir", &self.dir)
            .field("id", &self.id)
            .finish()
    }
}

impl PartialEq for Tree {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.dir == other.dir
    }
}

impl Eq for Tree {}

impl Hash for Tree {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dir.hash(state);
        self.id.hash(state);
    }
}

impl Tree {
    pub fn new(store: Arc<Store>, dir: RepoPathBuf, id: TreeId, data: Arc<backend::Tree>) -> Self {
        Self {
            store,
            dir,
            id,
            data,
        }
    }

    pub fn empty(store: Arc<Store>, dir: RepoPathBuf) -> Self {
        let id = store.empty_tree_id().clone();
        Self {
            store,
            dir,
            id,
            data: Arc::new(backend::Tree::default()),
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn dir(&self) -> &RepoPath {
        &self.dir
    }

    pub fn id(&self) -> &TreeId {
        &self.id
    }

    pub fn data(&self) -> &backend::Tree {
        &self.data
    }

    pub fn entries_non_recursive(&self) -> TreeEntriesNonRecursiveIterator<'_> {
        self.data.entries()
    }

    pub fn entries_matching<'matcher>(
        &self,
        matcher: &'matcher dyn Matcher,
    ) -> TreeEntriesIterator<'matcher> {
        TreeEntriesIterator::new(self.clone(), matcher)
    }

    pub fn value(&self, basename: &RepoPathComponent) -> Option<&TreeValue> {
        self.data.value(basename)
    }

    pub async fn path_value(&self, path: &RepoPath) -> BackendResult<Option<TreeValue>> {
        assert_eq!(self.dir(), RepoPath::root());
        match path.split() {
            Some((dir, basename)) => {
                let tree = self.sub_tree_recursive(dir).await?;
                Ok(tree.and_then(|tree| tree.data.value(basename).cloned()))
            }
            None => Ok(Some(TreeValue::Tree(self.id.clone()))),
        }
    }

    pub async fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Self>> {
        if let Some(sub_tree) = self.data.value(name) {
            match sub_tree {
                TreeValue::Tree(sub_tree_id) => {
                    let subdir = self.dir.join(name);
                    let sub_tree = self.store.get_tree(subdir, sub_tree_id).await?;
                    Ok(Some(sub_tree))
                }
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    async fn known_sub_tree(&self, subdir: RepoPathBuf, id: &TreeId) -> Self {
        self.store.get_tree(subdir, id).await.unwrap()
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
        // TODO: It would be nice to be able to return a reference here, but
        // then we would have to figure out how to share Tree instances
        // across threads.
        Ok(Some(current_tree))
    }
}

pub struct TreeEntriesIterator<'matcher> {
    stack: Vec<TreeEntriesDirItem>,
    matcher: &'matcher dyn Matcher,
}

struct TreeEntriesDirItem {
    tree: Tree,
    entries: Vec<(RepoPathBuf, TreeValue)>,
}

impl From<Tree> for TreeEntriesDirItem {
    fn from(tree: Tree) -> Self {
        let mut entries = tree
            .entries_non_recursive()
            .map(|entry| (tree.dir().join(entry.name()), entry.value().clone()))
            .collect_vec();
        entries.reverse();
        Self { tree, entries }
    }
}

impl<'matcher> TreeEntriesIterator<'matcher> {
    fn new(tree: Tree, matcher: &'matcher dyn Matcher) -> Self {
        // TODO: Restrict walk according to Matcher::visit()
        Self {
            stack: vec![TreeEntriesDirItem::from(tree)],
            matcher,
        }
    }
}

impl Iterator for TreeEntriesIterator<'_> {
    type Item = (RepoPathBuf, TreeValue);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(top) = self.stack.last_mut() {
            if let Some((path, value)) = top.entries.pop() {
                match value {
                    TreeValue::Tree(id) => {
                        // TODO: Handle the other cases (specific files and trees)
                        if self.matcher.visit(&path).is_nothing() {
                            continue;
                        }
                        let subtree = top.tree.known_sub_tree(path, &id).block_on();
                        self.stack.push(TreeEntriesDirItem::from(subtree));
                    }
                    value => {
                        if self.matcher.matches(&path) {
                            return Some((path, value));
                        }
                    }
                }
            } else {
                self.stack.pop();
            }
        }
        None
    }
}

/// Extension method for converting a tree-value merge to a `Merge<Tree>`.
// The auto trait bounds of the returned futures leak through the concrete
// impls below, so callers are unaffected by the `async fn` here.
#[expect(async_fn_in_trait)]
pub trait ToTreeMergeExt {
    /// If every non-`None` term of a `MergedTreeValue`
    /// is a `TreeValue::Tree`, this converts it to
    /// a `Merge<Tree>`, with empty trees instead of
    /// any `None` terms. Otherwise, returns `None`.
    async fn to_tree_merge(
        &self,
        store: &Arc<Store>,
        dir: &RepoPath,
    ) -> BackendResult<Option<Merge<Tree>>>;
}

impl<T> ToTreeMergeExt for Merge<Option<T>>
where
    T: Borrow<TreeValue>,
{
    async fn to_tree_merge(
        &self,
        store: &Arc<Store>,
        dir: &RepoPath,
    ) -> BackendResult<Option<Merge<Tree>>> {
        let tree_id_merge = self.try_map(|term| match borrow_tree_value(term.as_ref()) {
            None => Ok(None),
            Some(TreeValue::Tree(id)) => Ok(Some(id)),
            Some(_) => Err(()),
        });
        if let Ok(tree_id_merge) = tree_id_merge {
            Ok(Some(
                tree_id_merge
                    .try_map_async(async |id| {
                        if let Some(id) = id {
                            store.get_tree(dir.to_owned(), id).await
                        } else {
                            Ok(Tree::empty(store.clone(), dir.to_owned()))
                        }
                    })
                    .await?,
            ))
        } else {
            Ok(None)
        }
    }
}

/// Extension methods for `Merge<Tree>`.
// The auto trait bounds of the returned futures leak through the concrete
// impls below, so callers are unaffected by the `async fn` here.
#[expect(async_fn_in_trait)]
pub trait TreeMergeExt: Sized {
    /// The directory that is shared by all trees in the merge.
    fn dir(&self) -> &RepoPath;

    /// The value at the given basename. The value can be `Resolved` even if
    /// `self` is conflicted, which happens if the value at the path can be
    /// trivially merged. Does not recurse, so if `basename` refers to a Tree,
    /// then a `TreeValue::Tree` will be returned.
    fn value(&self, basename: &RepoPathComponent) -> MergedTreeVal<'_>;

    /// Gets the `Merge<Tree>` in a subdirectory of the current tree. If the
    /// path doesn't correspond to a tree in any of the inputs to the merge,
    /// then that entry will be replaced by an empty tree in the result.
    async fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Self>>;

    /// Look up the tree at the given path.
    async fn sub_tree_recursive(&self, path: &RepoPath) -> BackendResult<Option<Self>>;
}

impl TreeMergeExt for Merge<Tree> {
    fn dir(&self) -> &RepoPath {
        debug_assert!(self.iter().map(|tree| tree.dir()).all_equal());
        self.first().dir()
    }

    fn value(&self, basename: &RepoPathComponent) -> MergedTreeVal<'_> {
        if let Some(tree) = self.as_resolved() {
            return Merge::resolved(tree.value(basename));
        }
        let same_change = self.first().store().merge_options().same_change;
        let value = self.map(|tree| tree.value(basename));
        if let Some(resolved) = value.resolve_trivial(same_change) {
            return Merge::resolved(*resolved);
        }
        value
    }

    async fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Self>> {
        let store = self.first().store();
        match self.value(name).into_resolved() {
            Ok(Some(TreeValue::Tree(sub_tree_id))) => {
                let subdir = self.dir().join(name);
                Ok(Some(Self::resolved(
                    store.get_tree(subdir, sub_tree_id).await?,
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
                            store.get_tree(subdir, sub_tree_id).await
                        }
                        Some(_) => unreachable!(),
                        None => {
                            let subdir = self.dir().join(name);
                            Ok(Tree::empty(store.clone(), subdir))
                        }
                    })
                    .await?;
                Ok(Some(trees))
            }
        }
    }

    async fn sub_tree_recursive(&self, path: &RepoPath) -> BackendResult<Option<Self>> {
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
}
