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

use std::cell::RefCell;
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt;
use std::iter;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use bstr::BString;
use futures::StreamExt as _;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use super::composite::AsCompositeIndex;
use super::composite::CompositeIndex;
use super::entry::GlobalCommitPosition;
use super::rev_walk::EagerRevWalk;
use super::rev_walk::PeekableRevWalk;
use super::rev_walk::RevWalk;
use super::rev_walk::RevWalkBuilder;
use super::revset_graph_iterator::RevsetGraphWalk;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::MillisSinceEpoch;
use crate::commit::Commit;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialize_tree_value;
use crate::diff::ContentDiff;
use crate::diff::DiffHunkKind;
use crate::files;
use crate::graph::GraphNode;
use crate::matchers::FilesMatcher;
use crate::matchers::Matcher;
use crate::matchers::Visit;
use crate::merge::Merge;
use crate::object_id::ObjectId as _;
use crate::repo_path::RepoPath;
use crate::revset::GENERATION_RANGE_FULL;
use crate::revset::ResolvedExpression;
use crate::revset::ResolvedPredicateExpression;
use crate::revset::Revset;
use crate::revset::RevsetContainingFn;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetFilterPredicate;
use crate::rewrite;
use crate::store::Store;
use crate::str_util::StringMatcher;
use crate::tree_merge::MergeOptions;
use crate::tree_merge::resolve_file_values;
use crate::union_find;

type BoxedPredicateFn<'a> = Box<
    dyn FnMut(&CompositeIndex, GlobalCommitPosition) -> Result<bool, RevsetEvaluationError> + 'a,
>;
pub(super) type BoxedRevWalk<'a> = Box<
    dyn RevWalk<CompositeIndex, Item = Result<GlobalCommitPosition, RevsetEvaluationError>> + 'a,
>;

trait ToPredicateFn: fmt::Debug {
    /// Creates function that tests if the given entry is included in the set.
    ///
    /// The predicate function is evaluated in order of `RevsetIterator`.
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a;
}

impl<T: ToPredicateFn + ?Sized> ToPredicateFn for Box<T> {
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        <T as ToPredicateFn>::to_predicate_fn(self)
    }
}

trait InternalRevset: fmt::Debug + ToPredicateFn {
    // All revsets currently iterate in order of descending index position
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a;
}

impl<T: InternalRevset + ?Sized> InternalRevset for Box<T> {
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        <T as InternalRevset>::positions(self)
    }
}

pub(super) struct RevsetImpl<I> {
    inner: Box<dyn InternalRevset>,
    index: I,
}

impl<I: AsCompositeIndex + Clone> RevsetImpl<I> {
    fn new(inner: Box<dyn InternalRevset>, index: I) -> Self {
        Self { inner, index }
    }

    fn positions(
        &self,
    ) -> impl Iterator<Item = Result<GlobalCommitPosition, RevsetEvaluationError>> {
        self.inner.positions().attach(self.index.as_composite())
    }

    pub fn iter_graph_impl(
        &self,
        skip_transitive_edges: bool,
    ) -> impl Iterator<Item = Result<GraphNode<CommitId>, RevsetEvaluationError>> + use<I> {
        let index = self.index.clone();
        let walk = self.inner.positions();
        let mut graph_walk = RevsetGraphWalk::new(walk, skip_transitive_edges);
        iter::from_fn(move || graph_walk.next(index.as_composite()))
    }
}

impl<I> fmt::Debug for RevsetImpl<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevsetImpl")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<I: AsCompositeIndex + Clone> Revset for RevsetImpl<I> {
    fn iter<'a>(&self) -> Box<dyn Iterator<Item = Result<CommitId, RevsetEvaluationError>> + 'a>
    where
        Self: 'a,
    {
        let index = self.index.clone();
        let mut walk = self
            .inner
            .positions()
            .map(|index, pos| Ok(index.commits().entry_by_pos(pos?).commit_id()));
        Box::new(iter::from_fn(move || walk.next(index.as_composite())))
    }

    fn commit_change_ids<'a>(
        &self,
    ) -> Box<dyn Iterator<Item = Result<(CommitId, ChangeId), RevsetEvaluationError>> + 'a>
    where
        Self: 'a,
    {
        let index = self.index.clone();
        let mut walk = self.inner.positions().map(|index, pos| {
            let entry = index.commits().entry_by_pos(pos?);
            Ok((entry.commit_id(), entry.change_id()))
        });
        Box::new(iter::from_fn(move || walk.next(index.as_composite())))
    }

    fn iter_graph<'a>(
        &self,
    ) -> Box<dyn Iterator<Item = Result<GraphNode<CommitId>, RevsetEvaluationError>> + 'a>
    where
        Self: 'a,
    {
        let skip_transitive_edges = true;
        Box::new(self.iter_graph_impl(skip_transitive_edges))
    }

    fn is_empty(&self) -> bool {
        self.positions().next().is_none()
    }

    fn count_estimate(&self) -> Result<(usize, Option<usize>), RevsetEvaluationError> {
        if cfg!(feature = "testing") {
            // Exercise the estimation feature in tests. (If we ever have a Revset
            // implementation in production code that returns estimates, we can probably
            // remove this and rewrite the associated tests.)
            let count = self
                .positions()
                .take(10)
                .process_results(|iter| iter.count())?;
            if count < 10 {
                Ok((count, Some(count)))
            } else {
                Ok((10, None))
            }
        } else {
            let count = self.positions().process_results(|iter| iter.count())?;
            Ok((count, Some(count)))
        }
    }

    fn containing_fn<'a>(&self) -> Box<RevsetContainingFn<'a>>
    where
        Self: 'a,
    {
        let positions = PositionsAccumulator::new(self.index.clone(), self.inner.positions());
        Box::new(move |commit_id| positions.contains(commit_id))
    }
}

/// Incrementally consumes `RevWalk` of the revset collecting positions.
struct PositionsAccumulator<'a, I> {
    index: I,
    inner: RefCell<PositionsAccumulatorInner<'a>>,
}

impl<'a, I: AsCompositeIndex> PositionsAccumulator<'a, I> {
    fn new(index: I, walk: BoxedRevWalk<'a>) -> Self {
        let inner = RefCell::new(PositionsAccumulatorInner {
            walk,
            consumed_positions: Vec::new(),
        });
        Self { index, inner }
    }

    /// Checks whether the commit is in the revset.
    fn contains(&self, commit_id: &CommitId) -> Result<bool, RevsetEvaluationError> {
        let index = self.index.as_composite();
        let Some(position) = index.commits().commit_id_to_pos(commit_id) else {
            return Ok(false);
        };

        let mut inner = self.inner.borrow_mut();
        inner.consume_to(index, position)?;
        let found = inner
            .consumed_positions
            .binary_search_by(|p| p.cmp(&position).reverse())
            .is_ok();
        Ok(found)
    }

    #[cfg(test)]
    fn consumed_len(&self) -> usize {
        self.inner.borrow().consumed_positions.len()
    }
}

/// Helper struct for [`PositionsAccumulator`] to simplify interior mutability.
struct PositionsAccumulatorInner<'a> {
    walk: BoxedRevWalk<'a>,
    consumed_positions: Vec<GlobalCommitPosition>,
}

impl PositionsAccumulatorInner<'_> {
    /// Consumes `RevWalk` to a desired position but not deeper.
    fn consume_to(
        &mut self,
        index: &CompositeIndex,
        desired_position: GlobalCommitPosition,
    ) -> Result<(), RevsetEvaluationError> {
        let last_position = self.consumed_positions.last();
        if last_position.is_some_and(|&pos| pos <= desired_position) {
            return Ok(());
        }
        while let Some(position) = self.walk.next(index).transpose()? {
            self.consumed_positions.push(position);
            if position <= desired_position {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Adapter for precomputed `GlobalCommitPosition`s.
#[derive(Debug)]
struct EagerRevset {
    positions: Vec<GlobalCommitPosition>,
}

impl EagerRevset {
    pub const fn empty() -> Self {
        Self {
            positions: Vec::new(),
        }
    }
}

impl InternalRevset for EagerRevset {
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        let walk = EagerRevWalk::new(self.positions.clone().into_iter());
        Box::new(walk.map(|_index, pos| Ok(pos)))
    }
}

impl ToPredicateFn for EagerRevset {
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let walk = EagerRevWalk::new(self.positions.clone().into_iter());
        predicate_fn_from_rev_walk(walk)
    }
}

/// Adapter for infallible `RevWalk` of `GlobalCommitPosition`s.
struct RevWalkRevset<W> {
    walk: W,
}

impl<W> fmt::Debug for RevWalkRevset<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevWalkRevset").finish_non_exhaustive()
    }
}

impl<W> InternalRevset for RevWalkRevset<W>
where
    W: RevWalk<CompositeIndex, Item = GlobalCommitPosition> + Clone,
{
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        Box::new(self.walk.clone().map(|_index, pos| Ok(pos)))
    }
}

impl<W> ToPredicateFn for RevWalkRevset<W>
where
    W: RevWalk<CompositeIndex, Item = GlobalCommitPosition> + Clone,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        predicate_fn_from_rev_walk(self.walk.clone())
    }
}

fn predicate_fn_from_rev_walk<'a, W>(walk: W) -> BoxedPredicateFn<'a>
where
    W: RevWalk<CompositeIndex, Item = GlobalCommitPosition> + 'a,
{
    let mut walk = walk.peekable();
    Box::new(move |index, entry_pos| {
        while walk.next_if(index, |&pos| pos > entry_pos).is_some() {
            continue;
        }
        Ok(walk.next_if(index, |&pos| pos == entry_pos).is_some())
    })
}

#[derive(Debug)]
struct FilterRevset<S, P> {
    candidates: S,
    predicate: P,
}

impl<S, P> InternalRevset for FilterRevset<S, P>
where
    S: InternalRevset,
    P: ToPredicateFn,
{
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        let mut p = self.predicate.to_predicate_fn();
        Box::new(self.candidates.positions().filter_map(move |index, pos| {
            pos.and_then(|pos| Ok(p(index, pos)?.then_some(pos)))
                .transpose()
        }))
    }
}

impl<S, P> ToPredicateFn for FilterRevset<S, P>
where
    S: ToPredicateFn,
    P: ToPredicateFn,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let mut p1 = self.candidates.to_predicate_fn();
        let mut p2 = self.predicate.to_predicate_fn();
        Box::new(move |index, pos| Ok(p1(index, pos)? && p2(index, pos)?))
    }
}

#[derive(Debug)]
struct NotInPredicate<S>(S);

impl<S: ToPredicateFn> ToPredicateFn for NotInPredicate<S> {
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let mut p = self.0.to_predicate_fn();
        Box::new(move |index, pos| Ok(!p(index, pos)?))
    }
}

#[derive(Debug)]
struct UnionRevset<S1, S2> {
    set1: S1,
    set2: S2,
}

impl<S1, S2> InternalRevset for UnionRevset<S1, S2>
where
    S1: InternalRevset,
    S2: InternalRevset,
{
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        Box::new(union_by(
            self.set1.positions(),
            self.set2.positions(),
            |pos1, pos2| pos1.cmp(pos2).reverse(),
        ))
    }
}

impl<S1, S2> ToPredicateFn for UnionRevset<S1, S2>
where
    S1: ToPredicateFn,
    S2: ToPredicateFn,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let mut p1 = self.set1.to_predicate_fn();
        let mut p2 = self.set2.to_predicate_fn();
        Box::new(move |index, pos| Ok(p1(index, pos)? || p2(index, pos)?))
    }
}

/// `RevWalk` node that merges two sorted walk nodes.
///
/// The input items should be sorted in ascending order by the `cmp` function.
struct UnionRevWalk<I: ?Sized, W1: RevWalk<I>, W2: RevWalk<I>, C> {
    walk1: PeekableRevWalk<I, W1>,
    walk2: PeekableRevWalk<I, W2>,
    cmp: C,
}

impl<I, T, E, W1, W2, C> RevWalk<I> for UnionRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    type Item = W1::Item;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        match (self.walk1.peek(index), self.walk2.peek(index)) {
            (None, _) => self.walk2.next(index),
            (_, None) => self.walk1.next(index),
            (Some(Ok(item1)), Some(Ok(item2))) => match (self.cmp)(item1, item2) {
                Ordering::Less => self.walk1.next(index),
                Ordering::Equal => {
                    self.walk2.next(index);
                    self.walk1.next(index)
                }
                Ordering::Greater => self.walk2.next(index),
            },
            (Some(Err(_)), _) => self.walk1.next(index),
            (_, Some(Err(_))) => self.walk2.next(index),
        }
    }
}

fn union_by<I, T, E, W1, W2, C>(walk1: W1, walk2: W2, cmp: C) -> UnionRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    UnionRevWalk {
        walk1: walk1.peekable(),
        walk2: walk2.peekable(),
        cmp,
    }
}

#[derive(Debug)]
struct IntersectionRevset<S1, S2> {
    set1: S1,
    set2: S2,
}

impl<S1, S2> InternalRevset for IntersectionRevset<S1, S2>
where
    S1: InternalRevset,
    S2: InternalRevset,
{
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        Box::new(intersection_by(
            self.set1.positions(),
            self.set2.positions(),
            |pos1, pos2| pos1.cmp(pos2).reverse(),
        ))
    }
}

impl<S1, S2> ToPredicateFn for IntersectionRevset<S1, S2>
where
    S1: ToPredicateFn,
    S2: ToPredicateFn,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let mut p1 = self.set1.to_predicate_fn();
        let mut p2 = self.set2.to_predicate_fn();
        Box::new(move |index, pos| Ok(p1(index, pos)? && p2(index, pos)?))
    }
}

/// `RevWalk` node that intersects two sorted walk nodes.
///
/// The input items should be sorted in ascending order by the `cmp` function.
struct IntersectionRevWalk<I: ?Sized, W1: RevWalk<I>, W2: RevWalk<I>, C> {
    walk1: PeekableRevWalk<I, W1>,
    walk2: PeekableRevWalk<I, W2>,
    cmp: C,
}

impl<I, T, E, W1, W2, C> RevWalk<I> for IntersectionRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    type Item = W1::Item;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        loop {
            match (self.walk1.peek(index), self.walk2.peek(index)) {
                (None, _) => {
                    return None;
                }
                (_, None) => {
                    return None;
                }
                (Some(Ok(item1)), Some(Ok(item2))) => match (self.cmp)(item1, item2) {
                    Ordering::Less => {
                        self.walk1.next(index);
                    }
                    Ordering::Equal => {
                        self.walk2.next(index);
                        return self.walk1.next(index);
                    }
                    Ordering::Greater => {
                        self.walk2.next(index);
                    }
                },
                (Some(Err(_)), _) => {
                    return self.walk1.next(index);
                }
                (_, Some(Err(_))) => {
                    return self.walk2.next(index);
                }
            }
        }
    }
}

fn intersection_by<I, T, E, W1, W2, C>(
    walk1: W1,
    walk2: W2,
    cmp: C,
) -> IntersectionRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    IntersectionRevWalk {
        walk1: walk1.peekable(),
        walk2: walk2.peekable(),
        cmp,
    }
}

#[derive(Debug)]
struct DifferenceRevset<S1, S2> {
    // The minuend (what to subtract from)
    set1: S1,
    // The subtrahend (what to subtract)
    set2: S2,
}

impl<S1, S2> InternalRevset for DifferenceRevset<S1, S2>
where
    S1: InternalRevset,
    S2: InternalRevset,
{
    fn positions<'a>(&self) -> BoxedRevWalk<'a>
    where
        Self: 'a,
    {
        Box::new(difference_by(
            self.set1.positions(),
            self.set2.positions(),
            |pos1, pos2| pos1.cmp(pos2).reverse(),
        ))
    }
}

impl<S1, S2> ToPredicateFn for DifferenceRevset<S1, S2>
where
    S1: ToPredicateFn,
    S2: ToPredicateFn,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        let mut p1 = self.set1.to_predicate_fn();
        let mut p2 = self.set2.to_predicate_fn();
        Box::new(move |index, pos| Ok(p1(index, pos)? && !p2(index, pos)?))
    }
}

/// `RevWalk` node that subtracts `walk2` items from `walk1`.
///
/// The input items should be sorted in ascending order by the `cmp` function.
struct DifferenceRevWalk<I: ?Sized, W1: RevWalk<I>, W2: RevWalk<I>, C> {
    walk1: PeekableRevWalk<I, W1>,
    walk2: PeekableRevWalk<I, W2>,
    cmp: C,
}

impl<I, T, E, W1, W2, C> RevWalk<I> for DifferenceRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    type Item = W1::Item;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        loop {
            match (self.walk1.peek(index), self.walk2.peek(index)) {
                (None, _) => {
                    return None;
                }
                (_, None) => {
                    return self.walk1.next(index);
                }
                (Some(Ok(item1)), Some(Ok(item2))) => match (self.cmp)(item1, item2) {
                    Ordering::Less => {
                        return self.walk1.next(index);
                    }
                    Ordering::Equal => {
                        self.walk2.next(index);
                        self.walk1.next(index);
                    }
                    Ordering::Greater => {
                        self.walk2.next(index);
                    }
                },
                (Some(Err(_)), _) => {
                    return self.walk1.next(index);
                }
                (_, Some(Err(_))) => {
                    return self.walk2.next(index);
                }
            }
        }
    }
}

fn difference_by<I, T, E, W1, W2, C>(
    walk1: W1,
    walk2: W2,
    cmp: C,
) -> DifferenceRevWalk<I, W1, W2, C>
where
    I: ?Sized,
    W1: RevWalk<I, Item = Result<T, E>>,
    W2: RevWalk<I, Item = Result<T, E>>,
    C: FnMut(&T, &T) -> Ordering,
{
    DifferenceRevWalk {
        walk1: walk1.peekable(),
        walk2: walk2.peekable(),
        cmp,
    }
}

pub(super) fn evaluate<I: AsCompositeIndex + Clone>(
    expression: &ResolvedExpression,
    store: &Arc<Store>,
    index: I,
) -> Result<RevsetImpl<I>, RevsetEvaluationError> {
    let context = EvaluationContext {
        store: store.clone(),
        index: index.as_composite(),
    };
    let internal_revset = context.evaluate(expression)?;
    Ok(RevsetImpl::new(internal_revset, index))
}

struct EvaluationContext<'index> {
    store: Arc<Store>,
    index: &'index CompositeIndex,
}

fn to_u32_generation_range(range: &Range<u64>) -> Result<Range<u32>, RevsetEvaluationError> {
    let start = range.start.try_into().map_err(|_| {
        RevsetEvaluationError::Other(
            format!("Lower bound of generation ({}) is too large", range.start).into(),
        )
    })?;
    let end = range.end.try_into().unwrap_or(u32::MAX);
    Ok(start..end)
}

impl EvaluationContext<'_> {
    fn evaluate(
        &self,
        expression: &ResolvedExpression,
    ) -> Result<Box<dyn InternalRevset>, RevsetEvaluationError> {
        let index = self.index;
        match expression {
            ResolvedExpression::Commits(commit_ids) => {
                Ok(Box::new(self.revset_for_commit_ids(commit_ids)?))
            }
            ResolvedExpression::Ancestors {
                heads,
                generation,
                parents_range,
            } => {
                let head_set = self.evaluate(heads)?;
                let head_positions = head_set.positions().attach(index);
                let builder = RevWalkBuilder::new(index)
                    .wanted_heads(head_positions.try_collect()?)
                    .wanted_parents_range(parents_range.clone());
                if generation == &GENERATION_RANGE_FULL {
                    let walk = builder.ancestors().detach();
                    Ok(Box::new(RevWalkRevset { walk }))
                } else {
                    let generation = to_u32_generation_range(generation)?;
                    let walk = builder
                        .ancestors_filtered_by_generation(generation)
                        .detach();
                    Ok(Box::new(RevWalkRevset { walk }))
                }
            }
            ResolvedExpression::Range {
                roots,
                heads,
                generation,
                parents_range,
            } => {
                let root_set = self.evaluate(roots)?;
                let root_positions: Vec<_> = root_set.positions().attach(index).try_collect()?;
                // Pre-filter heads so queries like 'immutable_heads()..' can
                // terminate early. immutable_heads() usually includes some
                // visible heads, which can be trivially rejected.
                let head_set = self.evaluate(heads)?;
                let head_positions = difference_by(
                    head_set.positions(),
                    EagerRevWalk::new(root_positions.iter().copied().map(Ok)),
                    |pos1, pos2| pos1.cmp(pos2).reverse(),
                )
                .attach(index);
                let builder = RevWalkBuilder::new(index)
                    .wanted_heads(head_positions.try_collect()?)
                    .wanted_parents_range(parents_range.clone())
                    .unwanted_roots(root_positions);
                if generation == &GENERATION_RANGE_FULL {
                    let walk = builder.ancestors().detach();
                    Ok(Box::new(RevWalkRevset { walk }))
                } else {
                    let generation = to_u32_generation_range(generation)?;
                    let walk = builder
                        .ancestors_filtered_by_generation(generation)
                        .detach();
                    Ok(Box::new(RevWalkRevset { walk }))
                }
            }
            ResolvedExpression::DagRange {
                roots,
                heads,
                generation_from_roots,
            } => {
                let root_set = self.evaluate(roots)?;
                let root_positions = root_set.positions().attach(index);
                let head_set = self.evaluate(heads)?;
                let head_positions = head_set.positions().attach(index);
                let builder =
                    RevWalkBuilder::new(index).wanted_heads(head_positions.try_collect()?);
                if generation_from_roots == &(1..2) {
                    let root_positions: HashSet<_> = root_positions.try_collect()?;
                    let walk = builder
                        .ancestors_until_roots(root_positions.iter().copied())
                        .detach();
                    let candidates = RevWalkRevset { walk };
                    let predicate = as_pure_predicate_fn(move |index, pos| {
                        Ok(index
                            .commits()
                            .entry_by_pos(pos)
                            .parent_positions()
                            .iter()
                            .any(|parent_pos| root_positions.contains(parent_pos)))
                    });
                    // TODO: Suppose heads include all visible heads, ToPredicateFn version can be
                    // optimized to only test the predicate()
                    Ok(Box::new(FilterRevset {
                        candidates,
                        predicate,
                    }))
                } else if generation_from_roots == &GENERATION_RANGE_FULL {
                    let mut positions = builder
                        .descendants(root_positions.try_collect()?)
                        .collect_vec();
                    positions.reverse();
                    Ok(Box::new(EagerRevset { positions }))
                } else {
                    // For small generation range, it might be better to build a reachable map
                    // with generation bit set, which can be calculated incrementally from roots:
                    //   reachable[pos] = (reachable[parent_pos] | ...) << 1
                    let mut positions = builder
                        .descendants_filtered_by_generation(
                            root_positions.try_collect()?,
                            to_u32_generation_range(generation_from_roots)?,
                        )
                        .map(|Reverse(pos)| pos)
                        .collect_vec();
                    positions.reverse();
                    Ok(Box::new(EagerRevset { positions }))
                }
            }
            ResolvedExpression::Reachable { sources, domain } => {
                let mut sets = union_find::UnionFind::<GlobalCommitPosition>::new();

                // Compute all reachable subgraphs.
                let domain_revset = self.evaluate(domain)?;
                let domain_vec: Vec<_> = domain_revset.positions().attach(index).try_collect()?;
                let domain_set: HashSet<_> = domain_vec.iter().copied().collect();
                for pos in &domain_set {
                    for parent_pos in index.commits().entry_by_pos(*pos).parent_positions() {
                        if domain_set.contains(&parent_pos) {
                            sets.union(*pos, parent_pos);
                        }
                    }
                }
                // `UnionFind::find` is somewhat slow, so it's faster to only do this once and
                // then cache the result.
                let domain_reps = domain_vec.iter().map(|&pos| sets.find(pos)).collect_vec();

                // Identify disjoint sets reachable from sources. Using a predicate here can be
                // significantly faster for cases like `reachable(filter, X)`, since the filter
                // can be checked for only commits in `X` instead of for all visible commits,
                // and the difference is usually negligible for non-filter revsets.
                let sources_revset = self.evaluate(sources)?;
                let mut sources_predicate = sources_revset.to_predicate_fn();
                let mut set_reps = HashSet::new();
                for (&pos, &rep) in domain_vec.iter().zip(&domain_reps) {
                    // Skip evaluating predicate if `rep` has already been added.
                    if set_reps.contains(&rep) {
                        continue;
                    }
                    if sources_predicate(index, pos)? {
                        set_reps.insert(rep);
                    }
                }

                let positions = domain_vec
                    .into_iter()
                    .zip(domain_reps)
                    .filter_map(|(pos, rep)| set_reps.contains(&rep).then_some(pos))
                    .collect_vec();
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Heads(candidates) => {
                let candidate_set = self.evaluate(candidates)?;
                let positions = index
                    .commits()
                    .heads_pos(candidate_set.positions().attach(index).try_collect()?);
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::HeadsRange {
                roots,
                heads,
                parents_range,
                filter,
            } => {
                let root_set = self.evaluate(roots)?;
                let root_positions: Vec<_> = root_set.positions().attach(index).try_collect()?;
                // Pre-filter heads so queries like 'immutable_heads()..' can
                // terminate early. immutable_heads() usually includes some
                // visible heads, which can be trivially rejected.
                let head_set = self.evaluate(heads)?;
                let head_positions = difference_by(
                    head_set.positions(),
                    EagerRevWalk::new(root_positions.iter().copied().map(Ok)),
                    |pos1, pos2| pos1.cmp(pos2).reverse(),
                )
                .attach(index)
                .try_collect()?;
                let positions = if let Some(filter) = filter {
                    let mut filter = self.evaluate_predicate(filter)?.to_predicate_fn();
                    index.commits().heads_from_range_and_filter(
                        root_positions,
                        head_positions,
                        parents_range,
                        |pos| filter(index, pos),
                    )?
                } else {
                    let Ok(positions) = index.commits().heads_from_range_and_filter::<Infallible>(
                        root_positions,
                        head_positions,
                        parents_range,
                        |_| Ok(true),
                    );
                    positions
                };
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Roots(candidates) => {
                let mut positions: Vec<_> = self
                    .evaluate(candidates)?
                    .positions()
                    .attach(index)
                    .try_collect()?;
                let filled = RevWalkBuilder::new(index)
                    .wanted_heads(positions.clone())
                    .descendants(positions.iter().copied().collect())
                    .collect_positions_set();
                positions.retain(|&pos| {
                    !index
                        .commits()
                        .entry_by_pos(pos)
                        .parent_positions()
                        .iter()
                        .any(|parent| filled.contains(parent))
                });
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::ForkPoint(expression) => {
                let expression_set = self.evaluate(expression)?;
                let mut expression_positions_iter = expression_set.positions().attach(index);
                let Some(position) = expression_positions_iter.next() else {
                    return Ok(Box::new(EagerRevset::empty()));
                };
                let mut positions = vec![position?];
                for position in expression_positions_iter {
                    positions = index
                        .commits()
                        .common_ancestors_pos(positions, vec![position?]);
                }
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Bisect(candidates) => {
                let set = self.evaluate(candidates)?;
                // TODO: Make this more correct in non-linear history
                let candidate_positions: Vec<_> = set.positions().attach(index).try_collect()?;
                let positions = if candidate_positions.is_empty() {
                    candidate_positions
                } else {
                    vec![candidate_positions[candidate_positions.len() / 2]]
                };
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Latest { candidates, count } => {
                let candidate_set = self.evaluate(candidates)?;
                Ok(Box::new(self.take_latest_revset(&*candidate_set, *count)?))
            }
            ResolvedExpression::HasSize { candidates, count } => {
                let set = self.evaluate(candidates)?;
                let positions: Vec<_> = set
                    .positions()
                    .attach(index)
                    .take(count.saturating_add(1))
                    .try_collect()?;
                if positions.len() != *count {
                    // https://github.com/jj-vcs/jj/pull/7252#pullrequestreview-3236259998
                    // in the default engine we have to evaluate the entire
                    // revset (which may be very large) to get an exact count;
                    // we would need to remove .take() above. instead just give
                    // a vaguely approximate error message
                    let determiner = if positions.len() > *count {
                        "more"
                    } else {
                        "fewer"
                    };
                    return Err(RevsetEvaluationError::Other(
                        format!("The revset has {determiner} than the expected {count} revisions")
                            .into(),
                    ));
                }
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Coalesce(expression1, expression2) => {
                let set1 = self.evaluate(expression1)?;
                if set1.positions().attach(index).next().is_some() {
                    Ok(set1)
                } else {
                    self.evaluate(expression2)
                }
            }
            ResolvedExpression::Union(expression1, expression2) => {
                let set1 = self.evaluate(expression1)?;
                let set2 = self.evaluate(expression2)?;
                Ok(Box::new(UnionRevset { set1, set2 }))
            }
            ResolvedExpression::FilterWithin {
                candidates,
                predicate,
            } => Ok(Box::new(FilterRevset {
                candidates: self.evaluate(candidates)?,
                predicate: self.evaluate_predicate(predicate)?,
            })),
            ResolvedExpression::Intersection(expression1, expression2) => {
                let set1 = self.evaluate(expression1)?;
                let set2 = self.evaluate(expression2)?;
                Ok(Box::new(IntersectionRevset { set1, set2 }))
            }
            ResolvedExpression::Difference(expression1, expression2) => {
                let set1 = self.evaluate(expression1)?;
                let set2 = self.evaluate(expression2)?;
                Ok(Box::new(DifferenceRevset { set1, set2 }))
            }
        }
    }

    fn evaluate_predicate(
        &self,
        expression: &ResolvedPredicateExpression,
    ) -> Result<Box<dyn ToPredicateFn>, RevsetEvaluationError> {
        match expression {
            ResolvedPredicateExpression::Filter(predicate) => {
                Ok(build_predicate_fn(self.store.clone(), predicate))
            }
            ResolvedPredicateExpression::Set(expression) => Ok(self.evaluate(expression)?),
            ResolvedPredicateExpression::NotIn(complement) => {
                let set = self.evaluate_predicate(complement)?;
                Ok(Box::new(NotInPredicate(set)))
            }
            ResolvedPredicateExpression::Union(expression1, expression2) => {
                let set1 = self.evaluate_predicate(expression1)?;
                let set2 = self.evaluate_predicate(expression2)?;
                Ok(Box::new(UnionRevset { set1, set2 }))
            }
            ResolvedPredicateExpression::Intersection(expression1, expression2) => {
                let set1 = self.evaluate_predicate(expression1)?;
                let set2 = self.evaluate_predicate(expression2)?;
                Ok(Box::new(IntersectionRevset { set1, set2 }))
            }
        }
    }

    fn revset_for_commit_ids(
        &self,
        commit_ids: &[CommitId],
    ) -> Result<EagerRevset, RevsetEvaluationError> {
        let mut positions: Vec<_> = commit_ids
            .iter()
            .map(|id| {
                // Invalid commit IDs should be rejected by the revset frontend,
                // but there are a few edge cases that break the precondition.
                // For example, in jj <= 0.22, the root commit doesn't exist in
                // the root operation.
                self.index.commits().commit_id_to_pos(id).ok_or_else(|| {
                    RevsetEvaluationError::Other(
                        format!(
                            "Commit ID {} not found in index (index or view might be corrupted)",
                            id.hex()
                        )
                        .into(),
                    )
                })
            })
            .try_collect()?;
        positions.sort_unstable_by_key(|&pos| Reverse(pos));
        positions.dedup();
        Ok(EagerRevset { positions })
    }

    fn take_latest_revset(
        &self,
        candidate_set: &dyn InternalRevset,
        count: usize,
    ) -> Result<EagerRevset, RevsetEvaluationError> {
        if count == 0 {
            return Ok(EagerRevset::empty());
        }

        #[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
        struct Item {
            timestamp: MillisSinceEpoch,
            pos: GlobalCommitPosition, // tie-breaker
        }

        let make_rev_item = |pos| -> Result<_, RevsetEvaluationError> {
            let entry = self.index.commits().entry_by_pos(pos?);
            let commit = self.store.get_commit(&entry.commit_id())?;
            Ok(Reverse(Item {
                timestamp: commit.committer().timestamp.timestamp,
                pos: entry.position(),
            }))
        };

        // Maintain min-heap containing the latest (greatest) count items. For small
        // count and large candidate set, this is probably cheaper than building vec
        // and applying selection algorithm.
        let mut candidate_iter = candidate_set
            .positions()
            .attach(self.index)
            .map(make_rev_item)
            .fuse();
        let mut latest_items: BinaryHeap<_> = candidate_iter.by_ref().take(count).try_collect()?;
        for item in candidate_iter {
            let item = item?;
            let mut earliest = latest_items.peek_mut().unwrap();
            if earliest.0 < item.0 {
                *earliest = item;
            }
        }

        assert!(latest_items.len() <= count);
        let mut positions = latest_items
            .into_iter()
            .map(|item| item.0.pos)
            .collect_vec();
        positions.sort_unstable_by_key(|&pos| Reverse(pos));
        Ok(EagerRevset { positions })
    }
}

struct PurePredicateFn<F>(F);

impl<F> fmt::Debug for PurePredicateFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PurePredicateFn").finish_non_exhaustive()
    }
}

impl<F> ToPredicateFn for PurePredicateFn<F>
where
    F: Fn(&CompositeIndex, GlobalCommitPosition) -> Result<bool, RevsetEvaluationError> + Clone,
{
    fn to_predicate_fn<'a>(&self) -> BoxedPredicateFn<'a>
    where
        Self: 'a,
    {
        Box::new(self.0.clone())
    }
}

fn as_pure_predicate_fn<F>(f: F) -> PurePredicateFn<F>
where
    F: Fn(&CompositeIndex, GlobalCommitPosition) -> Result<bool, RevsetEvaluationError> + Clone,
{
    PurePredicateFn(f)
}

fn box_pure_predicate_fn<'a, F>(f: F) -> Box<dyn ToPredicateFn + 'a>
where
    F: Fn(&CompositeIndex, GlobalCommitPosition) -> Result<bool, RevsetEvaluationError>
        + Clone
        + 'a,
{
    Box::new(PurePredicateFn(f))
}

fn build_predicate_fn(
    store: Arc<Store>,
    predicate: &RevsetFilterPredicate,
) -> Box<dyn ToPredicateFn> {
    match predicate {
        RevsetFilterPredicate::ParentCount(parent_count_range) => {
            let parent_count_range = parent_count_range.clone();
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                Ok(parent_count_range.contains(&entry.num_parents()))
            })
        }
        RevsetFilterPredicate::Description(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(commit.description()))
            })
        }
        RevsetFilterPredicate::Subject(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(commit.description().lines().next().unwrap_or_default()))
            })
        }
        RevsetFilterPredicate::AuthorName(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(&commit.author().name))
            })
        }
        RevsetFilterPredicate::AuthorEmail(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(&commit.author().email))
            })
        }
        RevsetFilterPredicate::AuthorDate(expression) => {
            let expression = *expression;
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                let author_date = &commit.author().timestamp;
                Ok(expression.matches(author_date))
            })
        }
        RevsetFilterPredicate::CommitterName(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(&commit.committer().name))
            })
        }
        RevsetFilterPredicate::CommitterEmail(pattern) => {
            let matcher = Rc::new(pattern.to_matcher());
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(matcher.is_match(&commit.committer().email))
            })
        }
        RevsetFilterPredicate::CommitterDate(expression) => {
            let expression = *expression;
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                let committer_date = &commit.committer().timestamp;
                Ok(expression.matches(committer_date))
            })
        }
        RevsetFilterPredicate::File(expr) => {
            let matcher: Rc<dyn Matcher> = expr.to_matcher().into();
            box_pure_predicate_fn(move |index, pos| {
                if let Some(mut paths) = index.changed_paths().changed_paths(pos) {
                    return Ok(paths.any(|path| matcher.matches(path)));
                }
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(has_diff_from_parent(&store, index, &commit, &*matcher).block_on()?)
            })
        }
        RevsetFilterPredicate::DiffContains { text, files } => {
            let text_matcher = Rc::new(text.to_matcher());
            let files_matcher: Rc<dyn Matcher> = files.to_matcher().into();
            box_pure_predicate_fn(move |index, pos| {
                let narrowed_files_matcher;
                let files_matcher = if let Some(paths) = index.changed_paths().changed_paths(pos) {
                    let matched_paths = paths
                        .filter(|path| files_matcher.matches(path))
                        .collect_vec();
                    if matched_paths.is_empty() {
                        return Ok(false);
                    }
                    narrowed_files_matcher = FilesMatcher::new(matched_paths);
                    &narrowed_files_matcher
                } else {
                    &*files_matcher
                };
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(
                    matches_diff_from_parent(&store, index, &commit, &text_matcher, files_matcher)
                        .block_on()?,
                )
            })
        }
        RevsetFilterPredicate::HasConflict => box_pure_predicate_fn(move |index, pos| {
            let entry = index.commits().entry_by_pos(pos);
            let commit = store.get_commit(&entry.commit_id())?;
            Ok(commit.has_conflict())
        }),
        RevsetFilterPredicate::Signed => box_pure_predicate_fn(move |index, pos| {
            let entry = index.commits().entry_by_pos(pos);
            let commit = store.get_commit(&entry.commit_id())?;
            Ok(commit.is_signed())
        }),
        RevsetFilterPredicate::Extension(ext) => {
            let ext = ext.clone();
            box_pure_predicate_fn(move |index, pos| {
                let entry = index.commits().entry_by_pos(pos);
                let commit = store.get_commit(&entry.commit_id())?;
                Ok(ext.matches_commit(&commit))
            })
        }
    }
}

async fn has_diff_from_parent(
    store: &Arc<Store>,
    index: &CompositeIndex,
    commit: &Commit,
    matcher: &dyn Matcher,
) -> BackendResult<bool> {
    let parents: Vec<_> = commit.parents_async().await?;
    if let [parent] = parents.as_slice() {
        // Fast path: no need to load the root tree
        let unchanged = commit.tree_id() == parent.tree_id();
        if matcher.visit(RepoPath::root()) == Visit::AllRecursively {
            return Ok(!unchanged);
        } else if unchanged {
            return Ok(false);
        }
    }

    // Conflict resolution is expensive, try that only for matched files.
    let from_tree =
        rewrite::merge_commit_trees_no_resolve_without_repo(store, index, &parents).await?;
    let to_tree = commit.tree_async().await?;
    // TODO: handle copy tracking
    let mut tree_diff = from_tree.diff_stream(&to_tree, matcher);
    // TODO: Resolve values concurrently
    while let Some(entry) = tree_diff.next().await {
        let mut values = entry.values?;
        values.before = resolve_file_values(store, &entry.path, values.before).await?;
        if !values.is_changed() {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

async fn matches_diff_from_parent(
    store: &Arc<Store>,
    index: &CompositeIndex,
    commit: &Commit,
    text_matcher: &StringMatcher,
    files_matcher: &dyn Matcher,
) -> BackendResult<bool> {
    let parents: Vec<_> = commit.parents_async().await?;
    // Conflict resolution is expensive, try that only for matched files.
    let from_tree =
        rewrite::merge_commit_trees_no_resolve_without_repo(store, index, &parents).await?;
    let to_tree = commit.tree_async().await?;
    // TODO: handle copy tracking
    let mut tree_diff = from_tree.diff_stream(&to_tree, files_matcher);
    // TODO: Resolve values concurrently
    while let Some(entry) = tree_diff.next().await {
        let mut values = entry.values?;
        values.before = resolve_file_values(store, &entry.path, values.before).await?;
        if !values.is_changed() {
            continue;
        }
        let left_future = materialize_tree_value(store, &entry.path, values.before);
        let right_future = materialize_tree_value(store, &entry.path, values.after);
        let (left_value, right_value) = futures::try_join!(left_future, right_future)?;
        let left_contents = to_file_content(&entry.path, left_value).await?;
        let right_contents = to_file_content(&entry.path, right_value).await?;
        let merge_options = store.merge_options();
        if diff_match_lines(&left_contents, &right_contents, text_matcher, merge_options)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn diff_match_lines(
    lefts: &Merge<BString>,
    rights: &Merge<BString>,
    matcher: &StringMatcher,
    merge_options: &MergeOptions,
) -> BackendResult<bool> {
    // Filter lines prior to comparison. This might produce inferior hunks due
    // to lack of contexts, but is way faster than full diff.
    if let (Some(left), Some(right)) = (lefts.as_resolved(), rights.as_resolved()) {
        let left_lines = match_lines(left, matcher);
        let right_lines = match_lines(right, matcher);
        Ok(left_lines.ne(right_lines))
    } else {
        let lefts: Merge<BString> = lefts.map(|text| match_lines(text, matcher).collect());
        let rights: Merge<BString> = rights.map(|text| match_lines(text, matcher).collect());
        let lefts = files::merge(&lefts, merge_options);
        let rights = files::merge(&rights, merge_options);
        let diff = ContentDiff::by_line(lefts.iter().chain(rights.iter()));
        let different = files::conflict_diff_hunks(diff.hunks(), lefts.as_slice().len())
            .any(|hunk| hunk.kind == DiffHunkKind::Different);
        Ok(different)
    }
}

fn match_lines<'a>(text: &'a [u8], matcher: &StringMatcher) -> impl Iterator<Item = &'a [u8]> {
    // The pattern is matched line by line so that it can be anchored to line
    // start/end. For example, exact:"" will match blank lines.
    text.split_inclusive(|b| *b == b'\n').filter(|line| {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        matcher.is_match_bytes(line)
    })
}

async fn to_file_content(
    path: &RepoPath,
    value: MaterializedTreeValue,
) -> BackendResult<Merge<BString>> {
    let empty = || Merge::resolved(BString::default());
    match value {
        MaterializedTreeValue::Absent => Ok(empty()),
        MaterializedTreeValue::AccessDenied(_) => Ok(empty()),
        MaterializedTreeValue::File(mut file) => {
            Ok(Merge::resolved(file.read_all(path).await?.into()))
        }
        MaterializedTreeValue::Symlink { id: _, target } => Ok(Merge::resolved(target.into())),
        MaterializedTreeValue::GitSubmodule(_) => Ok(empty()),
        MaterializedTreeValue::FileConflict(file) => Ok(file.contents),
        MaterializedTreeValue::OtherConflict { .. } => Ok(empty()),
        MaterializedTreeValue::Tree(id) => {
            panic!("Unexpected tree with id {id:?} in diff at path {path:?}");
        }
    }
}

#[cfg(test)]
#[rustversion::attr(
    since(1.89),
    expect(clippy::cloned_ref_to_slice_refs, reason = "makes tests more readable")
)]
mod tests {
    use indoc::indoc;

    use super::*;
    use crate::default_index::DefaultMutableIndex;
    use crate::default_index::readonly::FieldLengths;
    use crate::files::FileMergeHunkLevel;
    use crate::merge::SameChange;
    use crate::str_util::StringPattern;

    const TEST_FIELD_LENGTHS: FieldLengths = FieldLengths {
        commit_id: 3,
        change_id: 16,
    };

    /// Generator of unique 16-byte ChangeId excluding root id
    fn change_id_generator() -> impl FnMut() -> ChangeId {
        let mut iter = (1_u128..).map(|n| ChangeId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    fn try_collect_vec<T, E>(iter: impl IntoIterator<Item = Result<T, E>>) -> Result<Vec<T>, E> {
        iter.into_iter().collect()
    }

    #[test]
    fn test_revset_combinator() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_3.clone()]);

        let index = index.as_composite();
        let get_pos = |id: &CommitId| index.commits().commit_id_to_pos(id).unwrap();
        let make_positions = |ids: &[&CommitId]| ids.iter().copied().map(get_pos).collect_vec();
        let make_set = |ids: &[&CommitId]| -> Box<dyn InternalRevset> {
            let positions = make_positions(ids);
            Box::new(EagerRevset { positions })
        };

        let set = make_set(&[&id_4, &id_3, &id_2, &id_0]);
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_4)).unwrap());
        assert!(p(index, get_pos(&id_3)).unwrap());
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).unwrap());
        // Uninteresting entries can be skipped
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_3)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).unwrap());

        let set = FilterRevset {
            candidates: make_set(&[&id_4, &id_2, &id_0]),
            predicate: as_pure_predicate_fn(|index, pos| {
                Ok(index.commits().entry_by_pos(pos).commit_id() != id_4)
            }),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index)).unwrap(),
            make_positions(&[&id_2, &id_0])
        );
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_4)).unwrap());
        assert!(!p(index, get_pos(&id_3)).unwrap());
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).unwrap());

        // Intersection by FilterRevset
        let set = FilterRevset {
            candidates: make_set(&[&id_4, &id_2, &id_0]),
            predicate: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index)).unwrap(),
            make_positions(&[&id_2])
        );
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_4)).unwrap());
        assert!(!p(index, get_pos(&id_3)).unwrap());
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(!p(index, get_pos(&id_0)).unwrap());

        let set = UnionRevset {
            set1: make_set(&[&id_4, &id_2]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index)).unwrap(),
            make_positions(&[&id_4, &id_3, &id_2, &id_1])
        );
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_4)).unwrap());
        assert!(p(index, get_pos(&id_3)).unwrap());
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).unwrap());
        assert!(!p(index, get_pos(&id_0)).unwrap());

        let set = IntersectionRevset {
            set1: make_set(&[&id_4, &id_2, &id_0]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index)).unwrap(),
            make_positions(&[&id_2])
        );
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_4)).unwrap());
        assert!(!p(index, get_pos(&id_3)).unwrap());
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(!p(index, get_pos(&id_0)).unwrap());

        let set = DifferenceRevset {
            set1: make_set(&[&id_4, &id_2, &id_0]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index)).unwrap(),
            make_positions(&[&id_4, &id_0])
        );
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_4)).unwrap());
        assert!(!p(index, get_pos(&id_3)).unwrap());
        assert!(!p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).unwrap());
    }

    #[test]
    fn test_revset_combinator_error_propagation() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);

        let index = index.as_composite();
        let get_pos = |id: &CommitId| index.commits().commit_id_to_pos(id).unwrap();
        let make_positions = |ids: &[&CommitId]| ids.iter().copied().map(get_pos).collect_vec();
        let make_good_set = |ids: &[&CommitId]| -> Box<dyn InternalRevset> {
            let positions = make_positions(ids);
            Box::new(EagerRevset { positions })
        };
        let make_bad_set = |ids: &[&CommitId], bad_id: &CommitId| -> Box<dyn InternalRevset> {
            let positions = make_positions(ids);
            let bad_id = bad_id.clone();
            Box::new(FilterRevset {
                candidates: EagerRevset { positions },
                predicate: as_pure_predicate_fn(move |index, pos| {
                    if index.commits().entry_by_pos(pos).commit_id() == bad_id {
                        Err(RevsetEvaluationError::Other("bad".into()))
                    } else {
                        Ok(true)
                    }
                }),
            })
        };

        // Error from filter predicate
        let set = make_bad_set(&[&id_2, &id_1, &id_0], &id_1);
        assert_eq!(
            try_collect_vec(set.positions().attach(index).take(1)).unwrap(),
            make_positions(&[&id_2])
        );
        assert!(try_collect_vec(set.positions().attach(index).take(2)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).is_err());

        // Error from filter candidates
        let set = FilterRevset {
            candidates: make_bad_set(&[&id_2, &id_1, &id_0], &id_1),
            predicate: as_pure_predicate_fn(|_, _| Ok(true)),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index).take(1)).unwrap(),
            make_positions(&[&id_2])
        );
        assert!(try_collect_vec(set.positions().attach(index).take(2)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).is_err());

        // Error from left side of union, immediately
        let set = UnionRevset {
            set1: make_bad_set(&[&id_1], &id_1),
            set2: make_good_set(&[&id_2, &id_1]),
        };
        assert!(try_collect_vec(set.positions().attach(index).take(1)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_2)).unwrap()); // works because bad id isn't visited
        assert!(p(index, get_pos(&id_1)).is_err());

        // Error from right side of union, lazily
        let set = UnionRevset {
            set1: make_good_set(&[&id_2, &id_1]),
            set2: make_bad_set(&[&id_1, &id_0], &id_0),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index).take(2)).unwrap(),
            make_positions(&[&id_2, &id_1])
        );
        assert!(try_collect_vec(set.positions().attach(index).take(3)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).is_err());

        // Error from left side of intersection, immediately
        let set = IntersectionRevset {
            set1: make_bad_set(&[&id_1], &id_1),
            set2: make_good_set(&[&id_2, &id_1]),
        };
        assert!(try_collect_vec(set.positions().attach(index).take(1)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).is_err());

        // Error from right side of intersection, lazily
        let set = IntersectionRevset {
            set1: make_good_set(&[&id_2, &id_1, &id_0]),
            set2: make_bad_set(&[&id_1, &id_0], &id_0),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index).take(1)).unwrap(),
            make_positions(&[&id_1])
        );
        assert!(try_collect_vec(set.positions().attach(index).take(2)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).is_err());

        // Error from left side of difference, immediately
        let set = DifferenceRevset {
            set1: make_bad_set(&[&id_1], &id_1),
            set2: make_good_set(&[&id_2, &id_1]),
        };
        assert!(try_collect_vec(set.positions().attach(index).take(1)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(!p(index, get_pos(&id_2)).unwrap());
        assert!(p(index, get_pos(&id_1)).is_err());

        // Error from right side of difference, lazily
        let set = DifferenceRevset {
            set1: make_good_set(&[&id_2, &id_1, &id_0]),
            set2: make_bad_set(&[&id_1, &id_0], &id_0),
        };
        assert_eq!(
            try_collect_vec(set.positions().attach(index).take(1)).unwrap(),
            make_positions(&[&id_2])
        );
        assert!(try_collect_vec(set.positions().attach(index).take(2)).is_err());
        let mut p = set.to_predicate_fn();
        assert!(p(index, get_pos(&id_2)).unwrap());
        assert!(!p(index, get_pos(&id_1)).unwrap());
        assert!(p(index, get_pos(&id_0)).is_err());
    }

    #[test]
    fn test_positions_accumulator() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_3.clone()]);

        let index = index.as_composite();
        let get_pos = |id: &CommitId| index.commits().commit_id_to_pos(id).unwrap();
        let make_positions = |ids: &[&CommitId]| ids.iter().copied().map(get_pos).collect_vec();
        let make_set = |ids: &[&CommitId]| -> Box<dyn InternalRevset> {
            let positions = make_positions(ids);
            Box::new(EagerRevset { positions })
        };

        let full_set = make_set(&[&id_4, &id_3, &id_2, &id_1, &id_0]);

        // Consumes entries incrementally
        let positions_accum = PositionsAccumulator::new(index, full_set.positions());

        assert!(positions_accum.contains(&id_3).unwrap());
        assert_eq!(positions_accum.consumed_len(), 2);

        assert!(positions_accum.contains(&id_0).unwrap());
        assert_eq!(positions_accum.consumed_len(), 5);

        assert!(positions_accum.contains(&id_3).unwrap());
        assert_eq!(positions_accum.consumed_len(), 5);

        // Does not consume positions for unknown commits
        let positions_accum = PositionsAccumulator::new(index, full_set.positions());

        assert!(
            !positions_accum
                .contains(&CommitId::from_hex("999999"))
                .unwrap()
        );
        assert_eq!(positions_accum.consumed_len(), 0);

        // Does not consume without necessity
        let set = make_set(&[&id_3, &id_2, &id_1]);
        let positions_accum = PositionsAccumulator::new(index, set.positions());

        assert!(!positions_accum.contains(&id_4).unwrap());
        assert_eq!(positions_accum.consumed_len(), 1);

        assert!(positions_accum.contains(&id_3).unwrap());
        assert_eq!(positions_accum.consumed_len(), 1);

        assert!(!positions_accum.contains(&id_0).unwrap());
        assert_eq!(positions_accum.consumed_len(), 3);

        assert!(positions_accum.contains(&id_1).unwrap());
    }

    fn diff_match_lines_samples() -> (Merge<BString>, Merge<BString>) {
        // left2      left1      base       right1      right2
        // ---------- ---------- ---------- ----------- -----------
        // "left 1.1" "line 1"   "line 1"   "line 1"    "line 1"
        // "line 2"   "line 2"   "line 2"   "line 2"    "line 2"
        // "left 3.1" "left 3.1" "line 3"   "right 3.1" "right 3.1"
        // "left 3.2" "left 3.2"
        // "left 3.3"
        // "line 4"   "line 4"   "line 4"   "line 4"    "line 4"
        // "line 5"   "line 5"              "line 5"
        let base = indoc! {"
            line 1
            line 2
            line 3
            line 4
        "};
        let left1 = indoc! {"
            line 1
            line 2
            left 3.1
            left 3.2
            line 4
            line 5
        "};
        let left2 = indoc! {"
            left 1.1
            line 2
            left 3.1
            left 3.2
            left 3.3
            line 4
            line 5
        "};
        let right1 = indoc! {"
            line 1
            line 2
            right 3.1
            line 4
            line 5
        "};
        let right2 = indoc! {"
            line 1
            line 2
            right 3.1
            line 4
        "};

        let conflict1 = Merge::from_vec([left1, base, right1].map(BString::from).to_vec());
        let conflict2 = Merge::from_vec([left2, base, right2].map(BString::from).to_vec());
        (conflict1, conflict2)
    }

    #[test]
    fn test_diff_match_lines_between_resolved() {
        let (conflict1, conflict2) = diff_match_lines_samples();
        let left1 = Merge::resolved(conflict1.first().clone());
        let left2 = Merge::resolved(conflict2.first().clone());
        let diff = |needle: &str| {
            let matcher = StringPattern::substring(needle).to_matcher();
            let options = MergeOptions {
                hunk_level: FileMergeHunkLevel::Line,
                same_change: SameChange::Accept,
            };
            diff_match_lines(&left1, &left2, &matcher, &options).unwrap()
        };

        assert!(diff(""));
        assert!(!diff("no match"));
        assert!(diff("line "));
        assert!(diff(" 1"));
        assert!(!diff(" 2"));
        assert!(diff(" 3"));
        assert!(!diff(" 3.1"));
        assert!(!diff(" 3.2"));
        assert!(diff(" 3.3"));
        assert!(!diff(" 4"));
        assert!(!diff(" 5"));
    }

    #[test]
    fn test_diff_match_lines_between_conflicts() {
        let (conflict1, conflict2) = diff_match_lines_samples();
        let diff = |needle: &str| {
            let matcher = StringPattern::substring(needle).to_matcher();
            let options = MergeOptions {
                hunk_level: FileMergeHunkLevel::Line,
                same_change: SameChange::Accept,
            };
            diff_match_lines(&conflict1, &conflict2, &matcher, &options).unwrap()
        };

        assert!(diff(""));
        assert!(!diff("no match"));
        assert!(diff("line "));
        assert!(diff(" 1"));
        assert!(!diff(" 2"));
        assert!(diff(" 3"));
        // " 3.1" and " 3.2" could be considered different because the hunk
        // includes a changed line " 3.3". However, we filters out unmatched
        // lines first, therefore the changed line is omitted from the hunk.
        assert!(!diff(" 3.1"));
        assert!(!diff(" 3.2"));
        assert!(diff(" 3.3"));
        assert!(!diff(" 4"));
        assert!(!diff(" 5")); // per A-B+A=A rule
    }

    #[test]
    fn test_diff_match_lines_between_resolved_and_conflict() {
        let (_conflict1, conflict2) = diff_match_lines_samples();
        let base = Merge::resolved(conflict2.get_remove(0).unwrap().clone());
        let diff = |needle: &str| {
            let matcher = StringPattern::substring(needle).to_matcher();
            let options = MergeOptions {
                hunk_level: FileMergeHunkLevel::Line,
                same_change: SameChange::Accept,
            };
            diff_match_lines(&base, &conflict2, &matcher, &options).unwrap()
        };

        assert!(diff(""));
        assert!(!diff("no match"));
        assert!(diff("line "));
        assert!(diff(" 1"));
        assert!(!diff(" 2"));
        assert!(diff(" 3"));
        assert!(diff(" 3.1"));
        assert!(diff(" 3.2"));
        assert!(!diff(" 4"));
        assert!(diff(" 5"));
    }
}
