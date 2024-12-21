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

#![allow(missing_docs)]

use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use itertools::Itertools as _;
use tokio::io::AsyncReadExt as _;
use tracing::instrument;

use crate::backend;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::ConflictId;
use crate::backend::TreeEntriesNonRecursiveIterator;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::files;
use crate::matchers::EverythingMatcher;
use crate::matchers::Matcher;
use crate::merge::MergedTreeVal;
use crate::object_id::ObjectId as _;
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
        Tree {
            store,
            dir,
            id,
            data,
        }
    }

    pub fn empty(store: Arc<Store>, dir: RepoPathBuf) -> Self {
        let id = store.empty_tree_id().clone();
        Tree {
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

    pub fn entries_non_recursive(&self) -> TreeEntriesNonRecursiveIterator {
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

    pub fn path_value(&self, path: &RepoPath) -> BackendResult<Option<TreeValue>> {
        assert_eq!(self.dir(), RepoPath::root());
        match path.split() {
            Some((dir, basename)) => {
                let tree = self.sub_tree_recursive(dir)?;
                Ok(tree.and_then(|tree| tree.data.value(basename).cloned()))
            }
            None => Ok(Some(TreeValue::Tree(self.id.clone()))),
        }
    }

    pub fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Tree>> {
        if let Some(sub_tree) = self.data.value(name) {
            match sub_tree {
                TreeValue::Tree(sub_tree_id) => {
                    let subdir = self.dir.join(name);
                    let sub_tree = self.store.get_tree(subdir, sub_tree_id)?;
                    Ok(Some(sub_tree))
                }
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    fn known_sub_tree(&self, subdir: RepoPathBuf, id: &TreeId) -> Tree {
        self.store.get_tree(subdir, id).unwrap()
    }

    /// Look up the tree at the given path.
    pub fn sub_tree_recursive(&self, path: &RepoPath) -> BackendResult<Option<Tree>> {
        let mut current_tree = self.clone();
        for name in path.components() {
            match current_tree.sub_tree(name)? {
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

    pub fn conflicts_matching(&self, matcher: &dyn Matcher) -> Vec<(RepoPathBuf, ConflictId)> {
        let mut conflicts = vec![];
        for (name, value) in self.entries_matching(matcher) {
            if let TreeValue::Conflict(id) = value {
                conflicts.push((name.clone(), id.clone()));
            }
        }
        conflicts
    }

    #[instrument]
    pub fn conflicts(&self) -> Vec<(RepoPathBuf, ConflictId)> {
        self.conflicts_matching(&EverythingMatcher)
    }

    pub fn has_conflict(&self) -> bool {
        !self.conflicts().is_empty()
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
                        let subtree = top.tree.known_sub_tree(path, &id);
                        self.stack.push(TreeEntriesDirItem::from(subtree));
                    }
                    value => {
                        if self.matcher.matches(&path) {
                            return Some((path, value));
                        }
                    }
                };
            } else {
                self.stack.pop();
            }
        }
        None
    }
}

/// Resolves file-level conflict by merging content hunks.
///
/// The input `conflict` is supposed to be simplified. It shouldn't contain
/// non-file values that cancel each other.
pub async fn try_resolve_file_conflict(
    store: &Store,
    filename: &RepoPath,
    conflict: &MergedTreeVal<'_>,
) -> BackendResult<Option<TreeValue>> {
    // If there are any non-file or any missing parts in the conflict, we can't
    // merge it. We check early so we don't waste time reading file contents if
    // we can't merge them anyway. At the same time we determine whether the
    // resulting file should be executable.
    let Ok(file_id_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id,
            executable: _,
            copy_id: _,
        }) => Ok(id),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    let Ok(executable_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id: _,
            executable,
            copy_id: _,
        }) => Ok(executable),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    let Ok(copy_id_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id: _,
            executable: _,
            copy_id,
        }) => Ok(copy_id),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    let Some(&&executable) = executable_conflict.resolve_trivial() else {
        // We're unable to determine whether the result should be executable
        return Ok(None);
    };
    let Some(&copy_id) = copy_id_conflict.resolve_trivial() else {
        // We're unable to determine the file's copy ID
        return Ok(None);
    };
    if let Some(&resolved_file_id) = file_id_conflict.resolve_trivial() {
        // Don't bother reading the file contents if the conflict can be trivially
        // resolved.
        return Ok(Some(TreeValue::File {
            id: resolved_file_id.clone(),
            executable,
            copy_id: copy_id.clone(),
        }));
    }

    // While the input conflict should be simplified by caller, it might contain
    // terms which only differ in executable bits. Simplify the conflict further
    // for two reasons:
    // 1. Avoid reading unchanged file contents
    // 2. The simplified conflict can sometimes be resolved when the unsimplfied one
    //    cannot
    let file_id_conflict = file_id_conflict.simplify();

    let contents = file_id_conflict
        .try_map_async(|file_id| async {
            let mut content = vec![];
            let mut reader = store.read_file(filename, file_id).await?;
            reader
                .read_to_end(&mut content)
                .await
                .map_err(|err| BackendError::ReadObject {
                    object_type: file_id.object_type(),
                    hash: file_id.hex(),
                    source: err.into(),
                })?;
            BackendResult::Ok(content)
        })
        .await?;
    if let Some(merged_content) = files::try_merge(&contents) {
        let id = store
            .write_file(filename, &mut merged_content.as_slice())
            .await?;
        Ok(Some(TreeValue::File {
            id,
            executable,
            copy_id: copy_id.clone(),
        }))
    } else {
        Ok(None)
    }
}
