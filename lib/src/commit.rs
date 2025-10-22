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
use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use futures::future::try_join_all;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::backend;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::MergedTreeId;
use crate::backend::Signature;
use crate::index::IndexResult;
use crate::merged_tree::MergedTree;
use crate::repo::Repo;
use crate::rewrite::merge_commit_trees;
use crate::signing::SignResult;
use crate::signing::Verification;
use crate::store::Store;

#[derive(Clone, serde::Serialize)]
pub struct Commit {
    #[serde(skip)]
    store: Arc<Store>,
    #[serde(rename = "commit_id")]
    id: CommitId,
    #[serde(flatten)]
    data: Arc<backend::Commit>,
}

impl Debug for Commit {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("Commit").field("id", &self.id).finish()
    }
}

impl PartialEq for Commit {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Commit {}

impl Ord for Commit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl PartialOrd for Commit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Commit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Commit {
    pub fn new(store: Arc<Store>, id: CommitId, data: Arc<backend::Commit>) -> Self {
        Self { store, id, data }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn id(&self) -> &CommitId {
        &self.id
    }

    pub fn parent_ids(&self) -> &[CommitId] {
        &self.data.parents
    }

    pub fn parents(&self) -> impl Iterator<Item = BackendResult<Self>> {
        self.data.parents.iter().map(|id| self.store.get_commit(id))
    }

    pub async fn parents_async(&self) -> BackendResult<Vec<Self>> {
        try_join_all(
            self.data
                .parents
                .iter()
                .map(|id| self.store.get_commit_async(id)),
        )
        .await
    }

    pub fn tree(&self) -> BackendResult<MergedTree> {
        self.tree_async().block_on()
    }

    pub async fn tree_async(&self) -> BackendResult<MergedTree> {
        self.store.get_root_tree_async(&self.data.root_tree).await
    }

    pub fn tree_id(&self) -> &MergedTreeId {
        &self.data.root_tree
    }

    pub fn parent_tree(&self, repo: &dyn Repo) -> BackendResult<MergedTree> {
        self.parent_tree_async(repo).block_on()
    }

    /// Return the parent tree, merging the parent trees if there are multiple
    /// parents.
    pub async fn parent_tree_async(&self, repo: &dyn Repo) -> BackendResult<MergedTree> {
        // Avoid merging parent trees if known to be empty. The index could be
        // queried only when parents.len() > 1, but index query would be cheaper
        // than extracting parent commit from the store.
        if is_commit_empty_by_index(repo, &self.id)? == Some(true) {
            return self.tree_async().await;
        }
        let parents: Vec<_> = self.parents_async().await?;
        merge_commit_trees(repo, &parents).await
    }

    /// Returns whether commit's content is empty. Commit description is not
    /// taken into consideration.
    pub fn is_empty(&self, repo: &dyn Repo) -> BackendResult<bool> {
        if let Some(empty) = is_commit_empty_by_index(repo, &self.id)? {
            return Ok(empty);
        }
        is_backend_commit_empty(repo, &self.store, &self.data)
    }

    pub fn has_conflict(&self) -> bool {
        !self.tree_id().as_merge().is_resolved()
    }

    pub fn change_id(&self) -> &ChangeId {
        &self.data.change_id
    }

    pub fn store_commit(&self) -> &Arc<backend::Commit> {
        &self.data
    }

    pub fn description(&self) -> &str {
        &self.data.description
    }

    pub fn author(&self) -> &Signature {
        &self.data.author
    }

    pub fn committer(&self) -> &Signature {
        &self.data.committer
    }

    ///  A commit is hidden if its commit id is not in the change id index.
    pub fn is_hidden(&self, repo: &dyn Repo) -> IndexResult<bool> {
        let maybe_entries = repo.resolve_change_id(self.change_id())?;
        Ok(maybe_entries.is_none_or(|entries| !entries.contains(&self.id)))
    }

    /// A commit is discardable if it has no change from its parent, and an
    /// empty description.
    pub fn is_discardable(&self, repo: &dyn Repo) -> BackendResult<bool> {
        Ok(self.description().is_empty() && self.is_empty(repo)?)
    }

    /// A quick way to just check if a signature is present.
    pub fn is_signed(&self) -> bool {
        self.data.secure_sig.is_some()
    }

    /// A slow (but cached) way to get the full verification.
    pub fn verification(&self) -> SignResult<Option<Verification>> {
        self.data
            .secure_sig
            .as_ref()
            .map(|sig| self.store.signer().verify(&self.id, &sig.data, &sig.sig))
            .transpose()
    }
}

pub(crate) fn is_backend_commit_empty(
    repo: &dyn Repo,
    store: &Arc<Store>,
    commit: &backend::Commit,
) -> BackendResult<bool> {
    if let [parent_id] = &*commit.parents {
        return Ok(commit.root_tree == *store.get_commit(parent_id)?.tree_id());
    }
    let parents: Vec<_> = commit
        .parents
        .iter()
        .map(|id| store.get_commit(id))
        .try_collect()?;
    let parent_tree = merge_commit_trees(repo, &parents).block_on()?;
    Ok(commit.root_tree == parent_tree.id())
}

fn is_commit_empty_by_index(repo: &dyn Repo, id: &CommitId) -> BackendResult<Option<bool>> {
    let maybe_paths = repo
        .index()
        .changed_paths_in_commit(id)
        // TODO: index error shouldn't be a "BackendError"
        .map_err(|err| BackendError::Other(err.into()))?;
    Ok(maybe_paths.map(|mut paths| paths.next().is_none()))
}

pub trait CommitIteratorExt<'c, I> {
    fn ids(self) -> impl Iterator<Item = &'c CommitId>;
}

impl<'c, I> CommitIteratorExt<'c, I> for I
where
    I: Iterator<Item = &'c Commit>,
{
    fn ids(self) -> impl Iterator<Item = &'c CommitId> {
        self.map(|commit| commit.id())
    }
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
