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

//! Types for evaluating revsets: the resolved expression tree consumed by
//! `Index` implementations, and the [`Revset`] trait describing the result.
//! The revset language (parsing, symbol resolution, optimization) lives in
//! `crate::revset`.

#![expect(missing_docs)]

use std::any::Any;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use futures::Stream;
use futures::StreamExt as _;
use futures::future::LocalBoxFuture;
use futures::stream::LocalBoxStream;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::fileset_backend::FilesetExpression;
use crate::graph::GraphNode;
use crate::store::Store;
use crate::str_util::StringExpression;
use crate::time_util::DatePattern;

/// Error occurred during revset evaluation.
#[derive(Debug, Error)]
pub enum RevsetEvaluationError {
    #[error("Unexpected error from commit backend")]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl RevsetEvaluationError {
    // TODO: Create a higher-level error instead of putting non-BackendErrors in a
    // BackendError
    pub fn into_backend_error(self) -> BackendError {
        match self {
            Self::Backend(err) => err,
            Self::Other(err) => BackendError::Other(err),
        }
    }
}

// assumes index has less than u64::MAX entries.
pub const GENERATION_RANGE_FULL: Range<u64> = 0..u64::MAX;
pub const GENERATION_RANGE_EMPTY: Range<u64> = 0..0;

pub const PARENTS_RANGE_FULL: Range<u32> = 0..u32::MAX;

/// A custom revset filter expression, defined by an extension.
pub trait RevsetFilterExtension: std::fmt::Debug + Any + Send + Sync {
    /// Returns true iff this filter matches the specified commit.
    fn matches_commit(&self, commit: &Commit) -> bool;
}

impl dyn RevsetFilterExtension {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: RevsetFilterExtension>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

#[derive(Eq, Copy, Clone, Debug, PartialEq)]
pub enum DiffMatchSide {
    Either,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub enum RevsetFilterPredicate {
    /// Commits with number of parents in the range.
    ParentCount(Range<u32>),
    /// Commits with description matching the pattern.
    Description(StringExpression),
    /// Commits with first line of the description matching the pattern.
    Subject(StringExpression),
    /// Commits with author name matching the pattern.
    AuthorName(StringExpression),
    /// Commits with author email matching the pattern.
    AuthorEmail(StringExpression),
    /// Commits with author dates matching the given date pattern.
    AuthorDate(DatePattern),
    /// Commits with committer name matching the pattern.
    CommitterName(StringExpression),
    /// Commits with committer email matching the pattern.
    CommitterEmail(StringExpression),
    /// Commits with committer dates matching the given date pattern.
    CommitterDate(DatePattern),
    /// Commits modifying the paths specified by the fileset.
    File(FilesetExpression),
    /// Commits containing diffs matching the `text` pattern within the `files`.
    DiffLines {
        text: StringExpression,
        files: FilesetExpression,
        side: DiffMatchSide,
    },
    /// Commits with conflicts
    HasConflict,
    /// Commits that are cryptographically signed.
    Signed,
    /// Custom predicates provided by extensions
    Extension(Arc<dyn RevsetFilterExtension>),
}

#[derive(Clone, Debug)]
pub enum ResolvedPredicateExpression {
    /// Pure filter predicate.
    Filter(RevsetFilterPredicate),
    Divergent {
        visible_heads: Vec<CommitId>,
    },
    /// Set expression to be evaluated as filter. This is typically a subtree
    /// node of `Union` with a pure filter predicate.
    Set(Box<ResolvedExpression>),
    NotIn(Box<Self>),
    Union(Box<Self>, Box<Self>),
    Intersection(Box<Self>, Box<Self>),
}

/// Describes evaluation plan of revset expression.
///
/// Unlike `RevsetExpression`, this doesn't contain unresolved symbols or `View`
/// properties.
///
/// Use `RevsetExpression` API to build a query programmatically.
// TODO: rename to BackendExpression?
#[derive(Clone, Debug)]
pub enum ResolvedExpression {
    Commits(Vec<CommitId>),
    Ancestors {
        heads: Box<Self>,
        generation: Range<u64>,
        parents_range: Range<u32>,
    },
    /// Commits that are ancestors of `heads` but not ancestors of `roots`.
    Range {
        roots: Box<Self>,
        heads: Box<Self>,
        generation: Range<u64>,
        // Parents range is only used for traversing heads, not roots
        parents_range: Range<u32>,
    },
    /// Commits that are descendants of `roots` and ancestors of `heads`.
    DagRange {
        roots: Box<Self>,
        heads: Box<Self>,
        generation_from_roots: Range<u64>,
    },
    /// Commits reachable from `sources` within `domain`.
    Reachable {
        sources: Box<Self>,
        domain: Box<Self>,
    },
    Heads(Box<Self>),
    /// Heads of the set of commits which are ancestors of `heads` but are not
    /// ancestors of `roots`, and which also are contained in `filter`.
    HeadsRange {
        roots: Box<Self>,
        heads: Box<Self>,
        parents_range: Range<u32>,
        filter: Option<ResolvedPredicateExpression>,
    },
    Roots(Box<Self>),
    Forks {
        heads: Box<Self>,
    },
    ForkPoint(Box<Self>),
    MergePoint {
        roots: Box<Self>,
        visible_heads: Box<Self>,
    },
    Bisect(Box<Self>),
    HasSize {
        candidates: Box<Self>,
        count: usize,
    },
    Latest {
        candidates: Box<Self>,
        count: usize,
    },
    Coalesce(Box<Self>, Box<Self>),
    Union(Box<Self>, Box<Self>),
    /// Intersects `candidates` with `predicate` by filtering.
    FilterWithin {
        candidates: Box<Self>,
        predicate: ResolvedPredicateExpression,
    },
    /// Intersects expressions by merging.
    Intersection(Box<Self>, Box<Self>),
    Difference(Box<Self>, Box<Self>),
}

pub trait Revset: fmt::Debug {
    /// Streams in topological order with children before parents.
    // TODO: Relax to BoxStream?
    fn stream<'a>(&self) -> LocalBoxStream<'a, Result<CommitId, RevsetEvaluationError>>
    where
        Self: 'a;

    /// Iterates commit/change id pairs in topological order.
    fn commit_change_ids<'a>(
        &self,
    ) -> LocalBoxStream<'a, Result<(CommitId, ChangeId), RevsetEvaluationError>>
    where
        Self: 'a;

    /// Streams graphs nodes (commit ID and edges) in topological order with
    /// children before parents.
    fn stream_graph<'a>(
        &self,
    ) -> LocalBoxStream<'a, Result<GraphNode<CommitId>, RevsetEvaluationError>>
    where
        Self: 'a;

    /// Returns true if iterator will emit no commit.
    fn is_empty(&self) -> Result<bool, RevsetEvaluationError>;

    /// Inclusive lower bound and, optionally, inclusive upper bound of how many
    /// commits are in the revset. The implementation can use its discretion as
    /// to how much effort should be put into the estimation, and how accurate
    /// the resulting estimate should be.
    fn count_estimate(&self) -> Result<(usize, Option<usize>), RevsetEvaluationError>;

    /// Returns a closure that checks if a commit is contained within the
    /// revset.
    ///
    /// The implementation may construct and maintain any necessary internal
    /// context to optimize the performance of the check.
    fn containing_fn<'a>(&self) -> Box<RevsetContainingFn<'a>>
    where
        Self: 'a;
}

/// Function that checks if a commit is contained within the revset.
pub type RevsetContainingFn<'a> =
    dyn Fn(&CommitId) -> LocalBoxFuture<'a, Result<bool, RevsetEvaluationError>> + 'a;

pub trait RevsetStreamExt {
    fn commits(
        self,
        store: &Arc<Store>,
    ) -> impl Stream<Item = Result<Commit, RevsetEvaluationError>> + use<'_, Self>;
}

impl<S: Stream<Item = Result<CommitId, RevsetEvaluationError>>> RevsetStreamExt for S {
    fn commits(
        self,
        store: &Arc<Store>,
    ) -> impl Stream<Item = Result<Commit, RevsetEvaluationError>> + use<'_, S> {
        self.map(async move |result| {
            let commit_id = result?;
            let commit = store
                .get_commit_async(&commit_id)
                .await
                .map_err(RevsetEvaluationError::Backend)?;
            Ok(commit)
        })
        .buffered(store.concurrency())
    }
}
