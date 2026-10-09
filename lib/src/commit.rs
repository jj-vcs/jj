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

use std::cmp::Ordering;
use std::sync::Arc;

use futures::future::try_join_all;
pub use jj_core::commit::*;

use crate::backend;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::index::IndexResult;
use crate::merged_tree::MergedTree;
use crate::repo::Repo;
use crate::rewrite::merge_commit_trees;
use crate::rewrite::merge_commit_trees_no_resolve;
use crate::store::Store;

/// Methods on [`Commit`] that need access to a [`Repo`].
#[expect(async_fn_in_trait)]
pub trait CommitRepoExt {
    /// Return the parent tree, merging the parent trees if there are multiple
    /// parents.
    async fn parent_tree(&self, repo: &dyn Repo) -> BackendResult<MergedTree>;

    /// Returns the parent tree, merging the parent trees if there are multiple
    /// parents, without resolving conflicts.
    async fn parent_tree_no_resolve(&self, repo: &dyn Repo) -> BackendResult<MergedTree>;

    /// Returns whether commit's content is empty. Commit description is not
    /// taken into consideration.
    async fn is_empty(&self, repo: &dyn Repo) -> BackendResult<bool>;

    ///  A commit is hidden if its commit id is not in the change id index.
    async fn is_hidden(&self, repo: &dyn Repo) -> IndexResult<bool>;

    /// A commit is discardable if it has no change from its parent, and an
    /// empty description.
    async fn is_discardable(&self, repo: &dyn Repo) -> BackendResult<bool>;
}

impl CommitRepoExt for Commit {
    async fn parent_tree(&self, repo: &dyn Repo) -> BackendResult<MergedTree> {
        // Avoid merging parent trees if known to be empty. The index could be
        // queried only when parents.len() > 1, but index query would be cheaper
        // than extracting parent commit from the store.
        if is_commit_empty_by_index(repo, self.id()).await? == Some(true) {
            return Ok(self.tree());
        }
        let parents = self.parents().await?;
        merge_commit_trees(repo, &parents).await
    }

    async fn parent_tree_no_resolve(&self, repo: &dyn Repo) -> BackendResult<MergedTree> {
        // Avoid merging parent trees if known to be empty. The index could be
        // queried only when parents.len() > 1, but index query would be cheaper
        // than extracting parent commit from the store.
        if is_commit_empty_by_index(repo, self.id()).await? == Some(true) {
            return Ok(self.tree());
        }
        let parents = self.parents().await?;
        merge_commit_trees_no_resolve(repo, &parents).await
    }

    async fn is_empty(&self, repo: &dyn Repo) -> BackendResult<bool> {
        if let Some(empty) = is_commit_empty_by_index(repo, self.id()).await? {
            return Ok(empty);
        }
        is_backend_commit_empty(repo, self.store(), self.store_commit()).await
    }

    async fn is_hidden(&self, repo: &dyn Repo) -> IndexResult<bool> {
        let maybe_targets = repo.resolve_change_id(self.change_id()).await?;
        Ok(maybe_targets.is_none_or(|targets| !targets.has_visible(self.id())))
    }

    async fn is_discardable(&self, repo: &dyn Repo) -> BackendResult<bool> {
        Ok(self.description().is_empty() && self.is_empty(repo).await?)
    }
}

pub(crate) async fn is_backend_commit_empty(
    repo: &dyn Repo,
    store: &Arc<Store>,
    commit: &backend::Commit,
) -> BackendResult<bool> {
    if let [parent_id] = &*commit.parents {
        return Ok(commit.root_tree == *store.get_commit_async(parent_id).await?.tree_ids());
    }
    let parents = try_join_all(commit.parents.iter().map(|id| store.get_commit_async(id))).await?;
    let parent_tree = merge_commit_trees(repo, &parents).await?;
    Ok(commit.root_tree == *parent_tree.tree_ids())
}

async fn is_commit_empty_by_index(repo: &dyn Repo, id: &CommitId) -> BackendResult<Option<bool>> {
    let maybe_paths = repo
        .index()
        .changed_paths_in_commit(id)
        .await
        // TODO: index error shouldn't be a "BackendError"
        .map_err(|err| BackendError::Other(err.into()))?;
    Ok(maybe_paths.map(|mut paths| paths.next().is_none()))
}

/// Wrapper to sort `Commit` by committer timestamp.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CommitByCommitterTimestamp(pub Commit);

impl Ord for CommitByCommitterTimestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_timestamp = &self.0.committer().timestamp.timestamp;
        let other_timestamp = &other.0.committer().timestamp.timestamp;
        self_timestamp
            .cmp(other_timestamp)
            .then_with(|| self.0.cmp(&other.0)) // to comply with Eq
    }
}

impl PartialOrd for CommitByCommitterTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
