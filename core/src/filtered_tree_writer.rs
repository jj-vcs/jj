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

use crate::backend::BackendResult;
use crate::backend::TreeValue;
use crate::matchers::Matcher;
use crate::matchers::Visit;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathComponentBuf;
use crate::tree::Tree;
use crate::tree_rewriter::RewrittenEntry;
use crate::tree_rewriter::TreeRewriteVisitor;
use crate::tree_rewriter::rewrite_trees;

/// Writes a version of a given `MergedTree` that's filtered by a `Matcher`.
pub struct FilteredTreeWriter<'matcher> {
    tree: MergedTree,
    matcher: &'matcher dyn Matcher,
}

impl<'matcher> FilteredTreeWriter<'matcher> {
    /// Creates a `FilteredTreeWriter` for writing a filtered version of `tree`
    /// using `matcher`.
    pub fn new(tree: &MergedTree, matcher: &'matcher dyn Matcher) -> Self {
        Self {
            tree: tree.clone(),
            matcher,
        }
    }

    /// Writes the filtered tree and returns it.
    pub async fn write(self) -> BackendResult<MergedTree> {
        let store = self.tree.store().clone();
        let labels = self.tree.labels().clone();
        // Fast paths for when the matcher includes or excludes everything
        if self.matcher.visit(RepoPath::root()) == Visit::AllRecursively {
            return Ok(self.tree);
        }
        if self.matcher.visit(RepoPath::root()).is_nothing() {
            let tree_ids = self.tree.tree_ids().map(|_| store.empty_tree_id().clone());
            return Ok(MergedTree::new(store, tree_ids, labels));
        }

        let roots = self.tree.into_tree_ids();
        let visitor = FilterVisitor {
            matcher: self.matcher,
        };
        let tree_ids = rewrite_trees(&store, roots, &visitor).await?;
        Ok(MergedTree::new(store, tree_ids, labels))
    }
}

struct FilterVisitor<'matcher> {
    matcher: &'matcher dyn Matcher,
}

impl TreeRewriteVisitor for FilterVisitor<'_> {
    fn visit_tree(&self, tree: &Tree) -> Vec<(RepoPathComponentBuf, RewrittenEntry)> {
        let mut new_entries = vec![];
        for entry in tree.entries_non_recursive() {
            let entry_path = tree.dir().join(entry.name());
            let rewritten = if let TreeValue::Tree(sub_tree_id) = entry.value() {
                let visit = self.matcher.visit(&entry_path);
                if visit.is_nothing() {
                    // Skip fully excluded subtrees
                    continue;
                } else if visit == Visit::AllRecursively {
                    // Include entry for fully included subtrees
                    RewrittenEntry::Value(entry.value().clone())
                } else {
                    // Otherwise, we need to filter the subtree
                    RewrittenEntry::Subtree {
                        base_tree_id: sub_tree_id.clone(),
                    }
                }
            } else if self.matcher.matches(&entry_path) {
                // Include matched entry
                RewrittenEntry::Value(entry.value().clone())
            } else {
                // Skip unmatched entry
                continue;
            };
            new_entries.push((entry.name().to_owned(), rewritten));
        }
        new_entries
    }
}
