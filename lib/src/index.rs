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

//! Interfaces for indexes of the commits in a repository.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use thiserror::Error;

use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::object_id::HexPrefix;
use crate::object_id::PrefixResolution;
use crate::operation::Operation;
use crate::repo_path::RepoPathBuf;
use crate::revset::ResolvedExpression;
use crate::revset::Revset;
use crate::revset::RevsetEvaluationError;
use crate::store::Store;

/// Returned by [`IndexStore`] in the event of an error.
#[derive(Debug, Error)]
pub enum IndexStoreError {
    /// Error reading a [`ReadonlyIndex`] from the [`IndexStore`].
    #[error("Failed to read index")]
    Read(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Error writing a [`MutableIndex`] to the [`IndexStore`].
    #[error("Failed to write index")]
    Write(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Result of [`IndexStore`] operations.
pub type IndexStoreResult<T> = Result<T, IndexStoreError>;

/// Returned by [`Index`] backend in the event of an error.
#[derive(Debug, Error)]
pub enum IndexError {
    /// Error returned if [`Index::all_heads_for_gc()`] is not supported by the
    /// [`Index`] backend.
    #[error("Cannot collect all heads by index of this type")]
    AllHeadsForGcUnsupported,
    /// Some other index error.
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// Result of [`Index`] operations.
pub type IndexResult<T> = Result<T, IndexError>;

/// Defines the interface for types that provide persistent storage for an
/// index.
pub trait IndexStore: Any + Send + Sync + Debug {
    /// Returns a name representing the type of index that the `IndexStore` is
    /// compatible with. For example, the `IndexStore` for the default index
    /// returns "default".
    fn name(&self) -> &str;

    /// Returns the index at the specified operation.
    fn get_index_at_op(
        &self,
        op: &Operation,
        store: &Arc<Store>,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>>;

    /// Writes `index` to the index store and returns a read-only version of the
    /// index.
    fn write_index(
        &self,
        index: Box<dyn MutableIndex>,
        op: &Operation,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>>;
}

impl dyn IndexStore {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: IndexStore>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

/// Defines the interface for types that provide an index of the commits in a
/// repository by [`CommitId`].
pub trait Index: Send + Sync {
    /// Returns the minimum prefix length to disambiguate `commit_id` from other
    /// commits in the index. The length returned is the number of hexadecimal
    /// digits in the minimum prefix.
    ///
    /// If the given `commit_id` doesn't exist, returns the minimum prefix
    /// length which matches none of the commits in the index.
    fn shortest_unique_commit_id_prefix_len(&self, commit_id: &CommitId) -> usize;

    /// Searches the index for commit IDs matching `prefix`. Returns a
    /// [`PrefixResolution`] with a [`CommitId`] if the prefix matches a single
    /// commit.
    fn resolve_commit_id_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<CommitId>;

    /// Returns true if `commit_id` is present in the index.
    fn has_id(&self, commit_id: &CommitId) -> bool;

    /// Returns true if `ancestor_id` commit is an ancestor of the
    /// `descendant_id` commit, or if `ancestor_id` equals `descendant_id`.
    fn is_ancestor(&self, ancestor_id: &CommitId, descendant_id: &CommitId) -> bool;

    /// Returns the best common ancestor or ancestors of the commits in `set1`
    /// and `set2`. A "best common ancestor" has no descendants that are also
    /// common ancestors.
    fn common_ancestors(&self, set1: &[CommitId], set2: &[CommitId]) -> Vec<CommitId>;

    /// Heads among all indexed commits at the associated operation.
    ///
    /// Suppose the index contains all the historical heads and their ancestors
    /// reachable from the associated operation, this function returns the heads
    /// that should be preserved on garbage collection.
    ///
    /// The iteration order is unspecified.
    fn all_heads_for_gc(&self) -> IndexResult<Box<dyn Iterator<Item = CommitId> + '_>>;

    /// Returns the subset of commit IDs in `candidates` which are not ancestors
    /// of other commits in `candidates`. If a commit id is duplicated in the
    /// `candidates` list it will appear at most once in the output.
    fn heads(&self, candidates: &mut dyn Iterator<Item = &CommitId>) -> IndexResult<Vec<CommitId>>;

    /// Returns iterator over paths changed at the specified commit. The paths
    /// are sorted. Returns `None` if the commit wasn't indexed.
    fn changed_paths_in_commit(
        &self,
        commit_id: &CommitId,
    ) -> IndexResult<Option<Box<dyn Iterator<Item = RepoPathBuf> + '_>>>;

    /// Resolves the revset `expression` against the index and corresponding
    /// `store`.
    fn evaluate_revset(
        &self,
        expression: &ResolvedExpression,
        store: &Arc<Store>,
    ) -> Result<Box<dyn Revset + '_>, RevsetEvaluationError>;
}

#[expect(missing_docs)]
pub trait ReadonlyIndex: Any + Send + Sync {
    fn as_index(&self) -> &dyn Index;

    fn change_id_index(&self, heads: &mut dyn Iterator<Item = &CommitId>)
    -> Box<dyn ChangeIdIndex>;

    fn start_modification(&self) -> Box<dyn MutableIndex>;
}

impl dyn ReadonlyIndex {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: ReadonlyIndex>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

#[expect(missing_docs)]
pub trait MutableIndex: Any {
    fn as_index(&self) -> &dyn Index;

    fn change_id_index(
        &self,
        heads: &mut dyn Iterator<Item = &CommitId>,
    ) -> Box<dyn ChangeIdIndex + '_>;

    fn add_commit(&mut self, commit: &Commit) -> IndexResult<()>;

    fn merge_in(&mut self, other: &dyn ReadonlyIndex) -> IndexResult<()>;
}

impl dyn MutableIndex {
    /// Downcasts to the implementation type.
    pub fn downcast<T: MutableIndex>(self: Box<Self>) -> Option<Box<T>> {
        (self as Box<dyn Any>).downcast().ok()
    }

    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: MutableIndex>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

/// Defines the interface for types that provide an index of the commits in a
/// repository by [`ChangeId`].
pub trait ChangeIdIndex: Send + Sync {
    /// Resolve an unambiguous change ID prefix to the commit IDs in the index.
    ///
    /// The order of the returned commit IDs is unspecified.
    fn resolve_prefix(&self, prefix: &HexPrefix) -> IndexResult<PrefixResolution<Vec<CommitId>>>;

    /// This function returns the shortest length of a prefix of `key` that
    /// disambiguates it from every other key in the index.
    ///
    /// The length returned is a number of hexadecimal digits.
    ///
    /// This has some properties that we do not currently make much use of:
    ///
    /// - The algorithm works even if `key` itself is not in the index.
    ///
    /// - In the special case when there are keys in the trie for which our
    ///   `key` is an exact prefix, returns `key.len() + 1`. Conceptually, in
    ///   order to disambiguate, you need every letter of the key *and* the
    ///   additional fact that it's the entire key). This case is extremely
    ///   unlikely for hashes with 12+ hexadecimal characters.
    fn shortest_unique_prefix_len(&self, change_id: &ChangeId) -> IndexResult<usize>;
}
