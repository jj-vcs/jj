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

#![allow(missing_docs)]

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeSet, BinaryHeap, HashSet};
use std::fmt;
use std::iter::Peekable;
use std::ops::Range;
use std::sync::Arc;

use itertools::Itertools;

use crate::backend::{ChangeId, CommitId, MillisSinceEpoch};
use crate::default_index::{CompositeIndex, IndexEntry, IndexPosition};
use crate::default_revset_graph_iterator::RevsetGraphIterator;
use crate::id_prefix::{IdIndex, IdIndexSource, IdIndexSourceEntry};
use crate::index::{HexPrefix, PrefixResolution};
use crate::matchers::{EverythingMatcher, Matcher, PrefixMatcher, Visit};
use crate::repo_path::RepoPath;
use crate::revset::{
    ChangeIdIndex, ResolvedExpression, ResolvedPredicateExpression, Revset, RevsetEvaluationError,
    RevsetFilterPredicate, GENERATION_RANGE_FULL,
};
use crate::revset_graph::RevsetGraphEdge;
use crate::rewrite;
use crate::store::Store;

trait ToPredicateFn: fmt::Debug {
    /// Creates function that tests if the given entry is included in the set.
    ///
    /// The predicate function is evaluated in order of `RevsetIterator`.
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a>;
}

impl<T: ToPredicateFn + ?Sized> ToPredicateFn for Box<T> {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        <T as ToPredicateFn>::to_predicate_fn(self, index)
    }
}

trait InternalRevset: fmt::Debug + ToPredicateFn {
    // All revsets currently iterate in order of descending index position
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a>;

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a;
}

pub struct RevsetImpl<'index> {
    inner: Box<dyn InternalRevset>,
    index: CompositeIndex<'index>,
}

impl<'index> RevsetImpl<'index> {
    fn new(revset: Box<dyn InternalRevset>, index: CompositeIndex<'index>) -> Self {
        Self {
            inner: revset,
            index,
        }
    }

    fn entries(&self) -> Box<dyn Iterator<Item = IndexEntry<'index>> + '_> {
        self.inner.iter(self.index)
    }

    pub fn iter_graph_impl(&self) -> RevsetGraphIterator<'_, 'index> {
        RevsetGraphIterator::new(self.index, self.entries())
    }
}

impl fmt::Debug for RevsetImpl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevsetImpl")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<'index> Revset<'index> for RevsetImpl<'index> {
    fn iter(&self) -> Box<dyn Iterator<Item = CommitId> + '_> {
        Box::new(self.entries().map(|index_entry| index_entry.commit_id()))
    }

    fn commit_change_ids(&self) -> Box<dyn Iterator<Item = (CommitId, ChangeId)> + '_> {
        Box::new(
            self.entries()
                .map(|index_entry| (index_entry.commit_id(), index_entry.change_id())),
        )
    }

    fn iter_graph(&self) -> Box<dyn Iterator<Item = (CommitId, Vec<RevsetGraphEdge>)> + '_> {
        Box::new(self.iter_graph_impl())
    }

    fn change_id_index(&self) -> Box<dyn ChangeIdIndex + 'index> {
        // TODO: Create a persistent lookup from change id to commit ids.
        let mut pos_by_change = IdIndex::builder();
        for entry in self.entries() {
            pos_by_change.insert(&entry.change_id(), entry.position());
        }
        Box::new(ChangeIdIndexImpl {
            index: self.index,
            pos_by_change: pos_by_change.build(),
        })
    }

    fn is_empty(&self) -> bool {
        self.entries().next().is_none()
    }

    fn count(&self) -> usize {
        self.entries().count()
    }
}

struct ChangeIdIndexImpl<'index> {
    index: CompositeIndex<'index>,
    pos_by_change: IdIndex<ChangeId, IndexPosition, 4>,
}

impl ChangeIdIndex for ChangeIdIndexImpl<'_> {
    fn resolve_prefix(&self, prefix: &HexPrefix) -> PrefixResolution<Vec<CommitId>> {
        self.pos_by_change
            .resolve_prefix_with(self.index, prefix, |entry| entry.commit_id())
            .map(|(_, commit_ids)| commit_ids)
    }

    fn shortest_unique_prefix_len(&self, change_id: &ChangeId) -> usize {
        self.pos_by_change
            .shortest_unique_prefix_len(self.index, change_id)
    }
}

impl<'index> IdIndexSource<IndexPosition> for CompositeIndex<'index> {
    type Entry = IndexEntry<'index>;

    fn entry_at(&self, pointer: &IndexPosition) -> Self::Entry {
        self.entry_by_pos(*pointer)
    }
}

impl IdIndexSourceEntry<ChangeId> for IndexEntry<'_> {
    fn to_key(&self) -> ChangeId {
        self.change_id()
    }
}

#[derive(Debug)]
struct EagerRevset {
    positions: Vec<IndexPosition>,
}

impl EagerRevset {
    pub const fn empty() -> Self {
        EagerRevset {
            positions: Vec::new(),
        }
    }
}

impl InternalRevset for EagerRevset {
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        let entries = self
            .positions
            .iter()
            .map(move |&pos| index.entry_by_pos(pos));
        Box::new(entries)
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl ToPredicateFn for EagerRevset {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        _index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        predicate_fn_from_positions(self.positions.iter().copied())
    }
}

struct RevWalkRevset<F> {
    walk: F,
}

impl<F> RevWalkRevset<F>
where
    // Returns trait object because we can't express the following constraints
    // without using named lifetime and type parameter:
    //
    //     for<'index>
    //         F: Fn(CompositeIndex<'index>) -> _,
    //         F::Output: Iterator<Item = IndexEntry<'index>> + 'index
    //
    // There's a workaround, but it doesn't help infer closure types.
    // https://github.com/rust-lang/rust/issues/47815
    // https://users.rust-lang.org/t/hrtb-on-multiple-generics/34255
    F: Fn(CompositeIndex<'_>) -> Box<dyn Iterator<Item = IndexEntry<'_>> + '_>,
{
    fn new(walk: F) -> Self {
        RevWalkRevset { walk }
    }
}

impl<F> fmt::Debug for RevWalkRevset<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevWalkRevset").finish_non_exhaustive()
    }
}

impl<F> InternalRevset for RevWalkRevset<F>
where
    F: Fn(CompositeIndex<'_>) -> Box<dyn Iterator<Item = IndexEntry<'_>> + '_>,
{
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        (self.walk)(index)
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl<F> ToPredicateFn for RevWalkRevset<F>
where
    F: Fn(CompositeIndex<'_>) -> Box<dyn Iterator<Item = IndexEntry<'_>> + '_>,
{
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        predicate_fn_from_entries(self.iter(index))
    }
}

fn predicate_fn_from_entries<'index, 'iter>(
    iter: impl Iterator<Item = IndexEntry<'index>> + 'iter,
) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'iter> {
    predicate_fn_from_positions(iter.map(|entry| entry.position()))
}

fn predicate_fn_from_positions<'iter>(
    iter: impl Iterator<Item = IndexPosition> + 'iter,
) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'iter> {
    let mut iter = iter.fuse().peekable();
    Box::new(move |entry| {
        while iter.next_if(|&pos| pos > entry.position()).is_some() {
            continue;
        }
        iter.next_if(|&pos| pos == entry.position()).is_some()
    })
}

#[derive(Debug)]
struct FilterRevset<P> {
    candidates: Box<dyn InternalRevset>,
    predicate: P,
}

impl<P: ToPredicateFn> InternalRevset for FilterRevset<P> {
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        let p = self.predicate.to_predicate_fn(index);
        Box::new(self.candidates.iter(index).filter(p))
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl<P: ToPredicateFn> ToPredicateFn for FilterRevset<P> {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p1 = self.candidates.to_predicate_fn(index);
        let mut p2 = self.predicate.to_predicate_fn(index);
        Box::new(move |entry| p1(entry) && p2(entry))
    }
}

#[derive(Debug)]
struct NotInPredicate<S>(S);

impl<S: ToPredicateFn> ToPredicateFn for NotInPredicate<S> {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p = self.0.to_predicate_fn(index);
        Box::new(move |entry| !p(entry))
    }
}

#[derive(Debug)]
struct UnionRevset {
    set1: Box<dyn InternalRevset>,
    set2: Box<dyn InternalRevset>,
}

impl InternalRevset for UnionRevset {
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        Box::new(UnionRevsetIterator {
            iter1: self.set1.iter(index).peekable(),
            iter2: self.set2.iter(index).peekable(),
        })
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl ToPredicateFn for UnionRevset {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p1 = self.set1.to_predicate_fn(index);
        let mut p2 = self.set2.to_predicate_fn(index);
        Box::new(move |entry| p1(entry) || p2(entry))
    }
}

#[derive(Debug)]
struct UnionPredicate<S1, S2> {
    set1: S1,
    set2: S2,
}

impl<S1, S2> ToPredicateFn for UnionPredicate<S1, S2>
where
    S1: ToPredicateFn,
    S2: ToPredicateFn,
{
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p1 = self.set1.to_predicate_fn(index);
        let mut p2 = self.set2.to_predicate_fn(index);
        Box::new(move |entry| p1(entry) || p2(entry))
    }
}

struct UnionRevsetIterator<
    'index,
    I1: Iterator<Item = IndexEntry<'index>>,
    I2: Iterator<Item = IndexEntry<'index>>,
> {
    iter1: Peekable<I1>,
    iter2: Peekable<I2>,
}

impl<'index, I1: Iterator<Item = IndexEntry<'index>>, I2: Iterator<Item = IndexEntry<'index>>>
    Iterator for UnionRevsetIterator<'index, I1, I2>
{
    type Item = IndexEntry<'index>;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.iter1.peek(), self.iter2.peek()) {
            (None, _) => self.iter2.next(),
            (_, None) => self.iter1.next(),
            (Some(entry1), Some(entry2)) => match entry1.position().cmp(&entry2.position()) {
                Ordering::Less => self.iter2.next(),
                Ordering::Equal => {
                    self.iter1.next();
                    self.iter2.next()
                }
                Ordering::Greater => self.iter1.next(),
            },
        }
    }
}

#[derive(Debug)]
struct IntersectionRevset {
    set1: Box<dyn InternalRevset>,
    set2: Box<dyn InternalRevset>,
}

impl InternalRevset for IntersectionRevset {
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        Box::new(IntersectionRevsetIterator {
            iter1: self.set1.iter(index).peekable(),
            iter2: self.set2.iter(index).peekable(),
        })
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl ToPredicateFn for IntersectionRevset {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p1 = self.set1.to_predicate_fn(index);
        let mut p2 = self.set2.to_predicate_fn(index);
        Box::new(move |entry| p1(entry) && p2(entry))
    }
}

struct IntersectionRevsetIterator<
    'index,
    I1: Iterator<Item = IndexEntry<'index>>,
    I2: Iterator<Item = IndexEntry<'index>>,
> {
    iter1: Peekable<I1>,
    iter2: Peekable<I2>,
}

impl<'index, I1: Iterator<Item = IndexEntry<'index>>, I2: Iterator<Item = IndexEntry<'index>>>
    Iterator for IntersectionRevsetIterator<'index, I1, I2>
{
    type Item = IndexEntry<'index>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match (self.iter1.peek(), self.iter2.peek()) {
                (None, _) => {
                    return None;
                }
                (_, None) => {
                    return None;
                }
                (Some(entry1), Some(entry2)) => match entry1.position().cmp(&entry2.position()) {
                    Ordering::Less => {
                        self.iter2.next();
                    }
                    Ordering::Equal => {
                        self.iter1.next();
                        return self.iter2.next();
                    }
                    Ordering::Greater => {
                        self.iter1.next();
                    }
                },
            }
        }
    }
}

#[derive(Debug)]
struct DifferenceRevset {
    // The minuend (what to subtract from)
    set1: Box<dyn InternalRevset>,
    // The subtrahend (what to subtract)
    set2: Box<dyn InternalRevset>,
}

impl InternalRevset for DifferenceRevset {
    fn iter<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn Iterator<Item = IndexEntry<'index>> + 'a> {
        Box::new(DifferenceRevsetIterator {
            iter1: self.set1.iter(index).peekable(),
            iter2: self.set2.iter(index).peekable(),
        })
    }

    fn into_predicate<'a>(self: Box<Self>) -> Box<dyn ToPredicateFn + 'a>
    where
        Self: 'a,
    {
        self
    }
}

impl ToPredicateFn for DifferenceRevset {
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let mut p1 = self.set1.to_predicate_fn(index);
        let mut p2 = self.set2.to_predicate_fn(index);
        Box::new(move |entry| p1(entry) && !p2(entry))
    }
}

struct DifferenceRevsetIterator<
    'index,
    I1: Iterator<Item = IndexEntry<'index>>,
    I2: Iterator<Item = IndexEntry<'index>>,
> {
    iter1: Peekable<I1>,
    iter2: Peekable<I2>,
}

impl<'index, I1: Iterator<Item = IndexEntry<'index>>, I2: Iterator<Item = IndexEntry<'index>>>
    Iterator for DifferenceRevsetIterator<'index, I1, I2>
{
    type Item = IndexEntry<'index>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match (self.iter1.peek(), self.iter2.peek()) {
                (None, _) => {
                    return None;
                }
                (_, None) => {
                    return self.iter1.next();
                }
                (Some(entry1), Some(entry2)) => match entry1.position().cmp(&entry2.position()) {
                    Ordering::Less => {
                        self.iter2.next();
                    }
                    Ordering::Equal => {
                        self.iter2.next();
                        self.iter1.next();
                    }
                    Ordering::Greater => {
                        return self.iter1.next();
                    }
                },
            }
        }
    }
}

pub fn evaluate<'index>(
    expression: &ResolvedExpression,
    store: &Arc<Store>,
    index: CompositeIndex<'index>,
) -> Result<RevsetImpl<'index>, RevsetEvaluationError> {
    let context = EvaluationContext {
        store: store.clone(),
        index,
    };
    let internal_revset = context.evaluate(expression)?;
    Ok(RevsetImpl::new(internal_revset, index))
}

struct EvaluationContext<'index> {
    store: Arc<Store>,
    index: CompositeIndex<'index>,
}

fn to_u32_generation_range(range: &Range<u64>) -> Result<Range<u32>, RevsetEvaluationError> {
    let start = range.start.try_into().map_err(|_| {
        RevsetEvaluationError::Other(format!(
            "Lower bound of generation ({}) is too large",
            range.start
        ))
    })?;
    let end = range.end.try_into().unwrap_or(u32::MAX);
    Ok(start..end)
}

impl<'index> EvaluationContext<'index> {
    fn evaluate(
        &self,
        expression: &ResolvedExpression,
    ) -> Result<Box<dyn InternalRevset>, RevsetEvaluationError> {
        let index = self.index;
        match expression {
            ResolvedExpression::Commits(commit_ids) => {
                Ok(Box::new(self.revset_for_commit_ids(commit_ids)))
            }
            ResolvedExpression::Ancestors { heads, generation } => {
                let head_set = self.evaluate(heads)?;
                let head_positions = head_set
                    .iter(index)
                    .map(|entry| entry.position())
                    .collect_vec();
                if generation == &GENERATION_RANGE_FULL {
                    Ok(Box::new(RevWalkRevset::new(move |index| {
                        Box::new(index.walk_revs(&head_positions, &[]))
                    })))
                } else {
                    let generation = to_u32_generation_range(generation)?;
                    Ok(Box::new(RevWalkRevset::new(move |index| {
                        Box::new(
                            index
                                .walk_revs(&head_positions, &[])
                                .filter_by_generation(generation.clone()),
                        )
                    })))
                }
            }
            ResolvedExpression::Range {
                roots,
                heads,
                generation,
            } => {
                let root_set = self.evaluate(roots)?;
                let root_positions = root_set
                    .iter(index)
                    .map(|entry| entry.position())
                    .collect_vec();
                let head_set = self.evaluate(heads)?;
                let head_positions = head_set
                    .iter(index)
                    .map(|entry| entry.position())
                    .collect_vec();
                if generation == &GENERATION_RANGE_FULL {
                    Ok(Box::new(RevWalkRevset::new(move |index| {
                        Box::new(index.walk_revs(&head_positions, &root_positions))
                    })))
                } else {
                    let generation = to_u32_generation_range(generation)?;
                    Ok(Box::new(RevWalkRevset::new(move |index| {
                        Box::new(
                            index
                                .walk_revs(&head_positions, &root_positions)
                                .filter_by_generation(generation.clone()),
                        )
                    })))
                }
            }
            ResolvedExpression::DagRange {
                roots,
                heads,
                generation_from_roots,
            } => {
                let root_set = self.evaluate(roots)?;
                let root_positions = root_set
                    .iter(index)
                    .map(|entry| entry.position())
                    .collect_vec();
                let head_set = self.evaluate(heads)?;
                let head_positions = head_set
                    .iter(index)
                    .map(|entry| entry.position())
                    .collect_vec();
                if generation_from_roots == &(1..2) {
                    let root_positions_set: HashSet<_> = root_positions.iter().copied().collect();
                    let candidates = Box::new(RevWalkRevset::new(move |index| {
                        Box::new(
                            index
                                .walk_revs(&head_positions, &[])
                                .take_until_roots(&root_positions),
                        )
                    }));
                    let predicate = as_pure_predicate_fn(move |_index, entry| {
                        entry
                            .parent_positions()
                            .iter()
                            .any(|parent_pos| root_positions_set.contains(parent_pos))
                    });
                    // TODO: Suppose heads include all visible heads, ToPredicateFn version can be
                    // optimized to only test the predicate()
                    Ok(Box::new(FilterRevset {
                        candidates,
                        predicate,
                    }))
                } else if generation_from_roots == &GENERATION_RANGE_FULL {
                    let mut positions = index
                        .walk_revs(&head_positions, &[])
                        .descendants(&root_positions)
                        .map(|entry| entry.position())
                        .collect_vec();
                    positions.reverse();
                    Ok(Box::new(EagerRevset { positions }))
                } else {
                    // For small generation range, it might be better to build a reachable map
                    // with generation bit set, which can be calculated incrementally from roots:
                    //   reachable[pos] = (reachable[parent_pos] | ...) << 1
                    let mut positions = index
                        .walk_revs(&head_positions, &[])
                        .descendants_filtered_by_generation(
                            &root_positions,
                            to_u32_generation_range(generation_from_roots)?,
                        )
                        .map(|entry| entry.position())
                        .collect_vec();
                    positions.reverse();
                    Ok(Box::new(EagerRevset { positions }))
                }
            }
            ResolvedExpression::Heads(candidates) => {
                let candidate_set = self.evaluate(candidates)?;
                let head_positions: BTreeSet<_> = index.heads_pos(
                    candidate_set
                        .iter(index)
                        .map(|entry| entry.position())
                        .collect(),
                );
                let positions = head_positions.into_iter().rev().collect();
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Roots(candidates) => {
                let candidate_entries = self.evaluate(candidates)?.iter(index).collect_vec();
                let candidate_positions = candidate_entries
                    .iter()
                    .map(|entry| entry.position())
                    .collect_vec();
                let filled = index
                    .walk_revs(&candidate_positions, &[])
                    .descendants(&candidate_positions)
                    .collect_positions_set();
                let mut positions = vec![];
                for candidate in candidate_entries {
                    if !candidate
                        .parent_positions()
                        .iter()
                        .any(|parent| filled.contains(parent))
                    {
                        positions.push(candidate.position());
                    }
                }
                Ok(Box::new(EagerRevset { positions }))
            }
            ResolvedExpression::Latest { candidates, count } => {
                let candidate_set = self.evaluate(candidates)?;
                Ok(Box::new(
                    self.take_latest_revset(candidate_set.as_ref(), *count),
                ))
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
            ResolvedPredicateExpression::Set(expression) => {
                Ok(self.evaluate(expression)?.into_predicate())
            }
            ResolvedPredicateExpression::NotIn(complement) => {
                let set = self.evaluate_predicate(complement)?;
                Ok(Box::new(NotInPredicate(set)))
            }
            ResolvedPredicateExpression::Union(expression1, expression2) => {
                let set1 = self.evaluate_predicate(expression1)?;
                let set2 = self.evaluate_predicate(expression2)?;
                Ok(Box::new(UnionPredicate { set1, set2 }))
            }
        }
    }

    fn revset_for_commit_ids(&self, commit_ids: &[CommitId]) -> EagerRevset {
        let mut positions = commit_ids
            .iter()
            .map(|id| self.index.commit_id_to_pos(id).unwrap())
            .collect_vec();
        positions.sort_unstable_by_key(|&pos| Reverse(pos));
        positions.dedup();
        EagerRevset { positions }
    }

    fn take_latest_revset(&self, candidate_set: &dyn InternalRevset, count: usize) -> EagerRevset {
        if count == 0 {
            return EagerRevset::empty();
        }

        #[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
        struct Item {
            timestamp: MillisSinceEpoch,
            pos: IndexPosition, // tie-breaker
        }

        let make_rev_item = |entry: IndexEntry<'_>| {
            let commit = self.store.get_commit(&entry.commit_id()).unwrap();
            Reverse(Item {
                timestamp: commit.committer().timestamp.timestamp.clone(),
                pos: entry.position(),
            })
        };

        // Maintain min-heap containing the latest (greatest) count items. For small
        // count and large candidate set, this is probably cheaper than building vec
        // and applying selection algorithm.
        let mut candidate_iter = candidate_set.iter(self.index).map(make_rev_item).fuse();
        let mut latest_items = BinaryHeap::from_iter(candidate_iter.by_ref().take(count));
        for item in candidate_iter {
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
        EagerRevset { positions }
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
    F: Fn(CompositeIndex<'_>, &IndexEntry<'_>) -> bool,
{
    fn to_predicate_fn<'a, 'index: 'a>(
        &'a self,
        index: CompositeIndex<'index>,
    ) -> Box<dyn FnMut(&IndexEntry<'_>) -> bool + 'a> {
        let f = &self.0;
        Box::new(move |entry| f(index, entry))
    }
}

fn as_pure_predicate_fn<F>(f: F) -> PurePredicateFn<F>
where
    F: Fn(CompositeIndex<'_>, &IndexEntry<'_>) -> bool,
{
    PurePredicateFn(f)
}

fn box_pure_predicate_fn<'a>(
    f: impl Fn(CompositeIndex<'_>, &IndexEntry<'_>) -> bool + 'a,
) -> Box<dyn ToPredicateFn + 'a> {
    Box::new(PurePredicateFn(f))
}

fn build_predicate_fn(
    store: Arc<Store>,
    predicate: &RevsetFilterPredicate,
) -> Box<dyn ToPredicateFn> {
    match predicate {
        RevsetFilterPredicate::ParentCount(parent_count_range) => {
            let parent_count_range = parent_count_range.clone();
            box_pure_predicate_fn(move |_index, entry| {
                parent_count_range.contains(&entry.num_parents())
            })
        }
        RevsetFilterPredicate::Description(pattern) => {
            let pattern = pattern.clone();
            box_pure_predicate_fn(move |_index, entry| {
                let commit = store.get_commit(&entry.commit_id()).unwrap();
                pattern.matches(commit.description())
            })
        }
        RevsetFilterPredicate::Author(pattern) => {
            let pattern = pattern.clone();
            // TODO: Make these functions that take a needle to search for accept some
            // syntax for specifying whether it's a regex and whether it's
            // case-sensitive.
            box_pure_predicate_fn(move |_index, entry| {
                let commit = store.get_commit(&entry.commit_id()).unwrap();
                pattern.matches(&commit.author().name) || pattern.matches(&commit.author().email)
            })
        }
        RevsetFilterPredicate::Committer(pattern) => {
            let pattern = pattern.clone();
            box_pure_predicate_fn(move |_index, entry| {
                let commit = store.get_commit(&entry.commit_id()).unwrap();
                pattern.matches(&commit.committer().name)
                    || pattern.matches(&commit.committer().email)
            })
        }
        RevsetFilterPredicate::File(paths) => {
            // TODO: Add support for globs and other formats
            let matcher: Box<dyn Matcher> = if let Some(paths) = paths {
                Box::new(PrefixMatcher::new(paths))
            } else {
                Box::new(EverythingMatcher)
            };
            box_pure_predicate_fn(move |index, entry| {
                has_diff_from_parent(&store, index, entry, matcher.as_ref())
            })
        }
        RevsetFilterPredicate::HasConflict => box_pure_predicate_fn(move |_index, entry| {
            let commit = store.get_commit(&entry.commit_id()).unwrap();
            commit.has_conflict().unwrap()
        }),
    }
}

fn has_diff_from_parent(
    store: &Arc<Store>,
    index: CompositeIndex<'_>,
    entry: &IndexEntry<'_>,
    matcher: &dyn Matcher,
) -> bool {
    let commit = store.get_commit(&entry.commit_id()).unwrap();
    let parents = commit.parents();
    if let [parent] = parents.as_slice() {
        // Fast path: no need to load the root tree
        let unchanged = commit.tree_id() == parent.tree_id();
        if matcher.visit(RepoPath::root()) == Visit::AllRecursively {
            return !unchanged;
        } else if unchanged {
            return false;
        }
    }
    let from_tree = rewrite::merge_commit_trees_without_repo(store, &index, &parents).unwrap();
    let to_tree = commit.tree().unwrap();
    from_tree.diff(&to_tree, matcher).next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ChangeId, CommitId, ObjectId};
    use crate::default_index::{AsCompositeIndex as _, DefaultMutableIndex};

    /// Generator of unique 16-byte ChangeId excluding root id
    fn change_id_generator() -> impl FnMut() -> ChangeId {
        let mut iter = (1_u128..).map(|n| ChangeId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    #[test]
    fn test_revset_combinator() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(3, 16);
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

        let get_pos = |id: &CommitId| index.as_composite().commit_id_to_pos(id).unwrap();
        let get_entry = |id: &CommitId| index.as_composite().entry_by_id(id).unwrap();
        let make_entries = |ids: &[&CommitId]| ids.iter().map(|id| get_entry(id)).collect_vec();
        let make_set = |ids: &[&CommitId]| -> Box<dyn InternalRevset> {
            let positions = ids.iter().copied().map(get_pos).collect();
            Box::new(EagerRevset { positions })
        };

        let set = make_set(&[&id_4, &id_3, &id_2, &id_0]);
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(p(&get_entry(&id_4)));
        assert!(p(&get_entry(&id_3)));
        assert!(p(&get_entry(&id_2)));
        assert!(!p(&get_entry(&id_1)));
        assert!(p(&get_entry(&id_0)));
        // Uninteresting entries can be skipped
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(p(&get_entry(&id_3)));
        assert!(!p(&get_entry(&id_1)));
        assert!(p(&get_entry(&id_0)));

        let set = FilterRevset {
            candidates: make_set(&[&id_4, &id_2, &id_0]),
            predicate: as_pure_predicate_fn(|_index, entry| entry.commit_id() != id_4),
        };
        assert_eq!(
            set.iter(index.as_composite()).collect_vec(),
            make_entries(&[&id_2, &id_0])
        );
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(!p(&get_entry(&id_4)));
        assert!(!p(&get_entry(&id_3)));
        assert!(p(&get_entry(&id_2)));
        assert!(!p(&get_entry(&id_1)));
        assert!(p(&get_entry(&id_0)));

        // Intersection by FilterRevset
        let set = FilterRevset {
            candidates: make_set(&[&id_4, &id_2, &id_0]),
            predicate: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            set.iter(index.as_composite()).collect_vec(),
            make_entries(&[&id_2])
        );
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(!p(&get_entry(&id_4)));
        assert!(!p(&get_entry(&id_3)));
        assert!(p(&get_entry(&id_2)));
        assert!(!p(&get_entry(&id_1)));
        assert!(!p(&get_entry(&id_0)));

        let set = UnionRevset {
            set1: make_set(&[&id_4, &id_2]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            set.iter(index.as_composite()).collect_vec(),
            make_entries(&[&id_4, &id_3, &id_2, &id_1])
        );
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(p(&get_entry(&id_4)));
        assert!(p(&get_entry(&id_3)));
        assert!(p(&get_entry(&id_2)));
        assert!(p(&get_entry(&id_1)));
        assert!(!p(&get_entry(&id_0)));

        let set = IntersectionRevset {
            set1: make_set(&[&id_4, &id_2, &id_0]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            set.iter(index.as_composite()).collect_vec(),
            make_entries(&[&id_2])
        );
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(!p(&get_entry(&id_4)));
        assert!(!p(&get_entry(&id_3)));
        assert!(p(&get_entry(&id_2)));
        assert!(!p(&get_entry(&id_1)));
        assert!(!p(&get_entry(&id_0)));

        let set = DifferenceRevset {
            set1: make_set(&[&id_4, &id_2, &id_0]),
            set2: make_set(&[&id_3, &id_2, &id_1]),
        };
        assert_eq!(
            set.iter(index.as_composite()).collect_vec(),
            make_entries(&[&id_4, &id_0])
        );
        let mut p = set.to_predicate_fn(index.as_composite());
        assert!(p(&get_entry(&id_4)));
        assert!(!p(&get_entry(&id_3)));
        assert!(!p(&get_entry(&id_2)));
        assert!(!p(&get_entry(&id_1)));
        assert!(p(&get_entry(&id_0)));
    }
}
