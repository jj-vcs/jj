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

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendResult;
use crate::backend::MergedTreeValue;
use crate::backend::MergedTreeValueExt as _;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::repo_path::RepoPathTree;
use crate::tree::Tree;
use crate::tree_merge::resolve_file_values;
use crate::tree_rewriter::RewrittenEntry;
use crate::tree_rewriter::TreeRewriteVisitor;
use crate::tree_rewriter::rewrite_trees;

/// Helper for writing trees with conflicts.
///
/// You start by creating an instance of this type with one or more
/// base trees. You then add overrides on top. The overrides may be
/// conflicts. Then you can write the result as a merge of trees.
///
/// If there are overrides both at a path and below it, the override at the
/// path takes precedence if it's not a tree. Otherwise, the overrides below it
/// are applied to the tree at the path, or to an empty directory if the path is
/// removed. If there are overrides only below a path that isn't a tree in the
/// base tree, the value at the path is replaced by a directory unless the
/// overrides below it only remove paths.
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
        let visitor = OverridesVisitor::new(self.overrides);
        let root_has_conflict = visitor.overrides.value().has_conflict;
        let roots = Merge::from_vec(
            base_tree_ids
                .into_iter()
                .enumerate()
                .map(|(term_index, tree_id)| (tree_id, root_has_conflict.then_some(term_index)))
                .collect_vec(),
        );
        rewrite_trees(&store, roots, &visitor).await
    }
}

#[derive(Debug, Default)]
struct OverrideNode {
    /// The override at this path, if any.
    value: Option<MergedTreeValue>,
    /// Whether there's a conflicted override at or below this path.
    has_conflict: bool,
}

/// Applies overrides to each term of a merged tree. The context is the index
/// of the term if there are conflicted overrides in the tree, or `None` if all
/// overrides in the tree are resolved and therefore the same for all terms.
///
/// The context is part of the key [`rewrite_trees()`] uses for deduplicating
/// trees. Using `None` where the term doesn't matter means that a tree shared
/// between terms is read and written only once instead of once per term.
struct OverridesVisitor {
    overrides: RepoPathTree<OverrideNode>,
}

impl OverridesVisitor {
    fn new(overrides: BTreeMap<RepoPathBuf, MergedTreeValue>) -> Self {
        let mut tree: RepoPathTree<OverrideNode> = RepoPathTree::default();
        for (path, value) in overrides {
            if !value.is_resolved() {
                for ancestor in path.ancestors() {
                    tree.add(ancestor).value_mut().has_conflict = true;
                }
            }
            tree.add(&path).value_mut().value = Some(value);
        }
        Self { overrides: tree }
    }
}

impl TreeRewriteVisitor for OverridesVisitor {
    type Context = Option<usize>;

    fn visit_tree(
        &self,
        tree: &Tree,
        term_index: &Option<usize>,
    ) -> Vec<(RepoPathComponentBuf, RewrittenEntry<Option<usize>>)> {
        let node = self.overrides.get(tree.dir()).unwrap();
        let children = node.children().sorted_by_key(|(name, _)| *name);
        let mut new_entries = vec![];
        for item in tree
            .data()
            .entries()
            .merge_join_by(children, |entry, (name, _)| entry.name().cmp(name))
        {
            let (name, base_value, child) = match item {
                EitherOrBoth::Left(entry) => {
                    // No overrides at or below this path
                    new_entries.push((
                        entry.name().to_owned(),
                        RewrittenEntry::Value(entry.value().clone()),
                    ));
                    continue;
                }
                EitherOrBoth::Right((name, child)) => (name, None, child),
                EitherOrBoth::Both(entry, (name, child)) => (name, Some(entry.value()), child),
            };
            let override_node = child.value();
            let value = match &override_node.value {
                Some(values) => term_value(values, *term_index).cloned(),
                None => base_value.cloned(),
            };
            if !child.has_children() {
                if let Some(value) = value {
                    new_entries.push((name.to_owned(), RewrittenEntry::Value(value)));
                }
                continue;
            }
            // Overrides below this path are applied to the tree at this path, or
            // to an empty tree if the path is removed. A non-tree override takes
            // precedence over them. A non-tree value from the base tree is
            // replaced by a directory unless they only remove paths.
            let base_tree_id = match value {
                Some(TreeValue::Tree(tree_id)) => tree_id,
                Some(value)
                    if override_node.value.is_some() || !sets_value_below(child, *term_index) =>
                {
                    new_entries.push((name.to_owned(), RewrittenEntry::Value(value)));
                    continue;
                }
                _ => tree.store().empty_tree_id().clone(),
            };
            new_entries.push((
                name.to_owned(),
                RewrittenEntry::Subtree {
                    base_tree_id,
                    context: term_index.filter(|_| override_node.has_conflict),
                },
            ));
        }
        new_entries
    }
}

/// Returns the value of `values` in the term at `term_index`. `term_index` may
/// be `None` only if `values` is resolved.
fn term_value(values: &MergedTreeValue, term_index: Option<usize>) -> Option<&TreeValue> {
    match values.as_resolved() {
        Some(value) => value.as_ref(),
        None => values.as_slice()[term_index.unwrap()].as_ref(),
    }
}

/// Whether any override below `node` sets a value in the term at
/// `term_index`.
fn sets_value_below(node: &RepoPathTree<OverrideNode>, term_index: Option<usize>) -> bool {
    node.children().any(|(_, child)| {
        child
            .value()
            .value
            .as_ref()
            .is_some_and(|values| term_value(values, term_index).is_some())
            || sets_value_below(child, term_index)
    })
}
