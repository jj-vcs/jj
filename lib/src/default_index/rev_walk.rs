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

use std::cmp::Reverse;
use std::cmp::max;
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::Fuse;
use std::iter::FusedIterator;
use std::ops::Range;

use smallvec::SmallVec;

use super::composite::CompositeCommitIndex;
use super::composite::CompositeIndex;
use super::entry::GlobalCommitPosition;
use super::entry::SmallGlobalCommitPositionsVec;
use super::rev_walk_queue::RevWalkQueue;
use super::rev_walk_queue::RevWalkWorkItem;

/// Like `Iterator`, but doesn't borrow the `index` internally.
pub(super) trait RevWalk<I: ?Sized> {
    type Item;

    /// Advances the iteration and returns the next item.
    ///
    /// The caller must provide the same `index` instance.
    ///
    /// Returns `None` when the iteration is finished. Once `None` is returned,
    /// this will never resume. In other words, a `RevWalk` is fused.
    fn next(&mut self, index: &I) -> Option<Self::Item>;

    // The following methods are provided for convenience. They are not supposed
    // to be reimplemented.

    /// Wraps in adapter that will filter and transform items by the given
    /// function.
    fn filter_map<B, F>(self, f: F) -> FilterMapRevWalk<Self, F>
    where
        Self: Sized,
        F: FnMut(&I, Self::Item) -> Option<B>,
    {
        FilterMapRevWalk { walk: self, f }
    }

    /// Wraps in adapter that will transform items by the given function.
    fn map<B, F>(self, f: F) -> MapRevWalk<Self, F>
    where
        Self: Sized,
        F: FnMut(&I, Self::Item) -> B,
    {
        MapRevWalk { walk: self, f }
    }

    /// Wraps in adapter that can peek one more item without consuming.
    fn peekable(self) -> PeekableRevWalk<I, Self>
    where
        Self: Sized,
    {
        PeekableRevWalk {
            walk: self,
            peeked: None,
        }
    }

    /// Reattaches the underlying `index`.
    fn attach(self, index: &I) -> RevWalkBorrowedIndexIter<'_, I, Self>
    where
        Self: Sized,
    {
        RevWalkBorrowedIndexIter { index, walk: self }
    }
}

impl<I: ?Sized, W: RevWalk<I> + ?Sized> RevWalk<I> for Box<W> {
    type Item = W::Item;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        <W as RevWalk<I>>::next(self, index)
    }
}

/// Adapter that turns `Iterator` into `RevWalk` by dropping index argument.
///
/// As the name suggests, the source object is usually a slice or `Vec`.
#[derive(Clone, Debug)]
pub(super) struct EagerRevWalk<T> {
    iter: Fuse<T>,
}

impl<T: Iterator> EagerRevWalk<T> {
    pub fn new(iter: T) -> Self {
        Self { iter: iter.fuse() }
    }
}

impl<I: ?Sized, T: Iterator> RevWalk<I> for EagerRevWalk<T> {
    type Item = T::Item;

    fn next(&mut self, _index: &I) -> Option<Self::Item> {
        self.iter.next()
    }
}

#[derive(Clone, Debug)]
#[must_use]
pub(super) struct FilterMapRevWalk<W, F> {
    walk: W,
    f: F,
}

impl<B, I, W, F> RevWalk<I> for FilterMapRevWalk<W, F>
where
    I: ?Sized,
    W: RevWalk<I>,
    F: FnMut(&I, W::Item) -> Option<B>,
{
    type Item = B;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        while let Some(item) = self.walk.next(index) {
            if let Some(new_item) = (self.f)(index, item) {
                return Some(new_item);
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
#[must_use]
pub(super) struct MapRevWalk<W, F> {
    walk: W,
    f: F,
}

impl<B, I, W, F> RevWalk<I> for MapRevWalk<W, F>
where
    I: ?Sized,
    W: RevWalk<I>,
    F: FnMut(&I, W::Item) -> B,
{
    type Item = B;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        self.walk.next(index).map(|item| (self.f)(index, item))
    }
}

#[derive(Clone, Debug)]
#[must_use]
pub(super) struct PeekableRevWalk<I: ?Sized, W: RevWalk<I>> {
    walk: W,
    // Since RevWalk is fused, we don't need a nested Option<Option<_>>.
    peeked: Option<W::Item>,
}

impl<I: ?Sized, W: RevWalk<I>> PeekableRevWalk<I, W> {
    pub fn peek(&mut self, index: &I) -> Option<&W::Item> {
        if self.peeked.is_none() {
            self.peeked = self.walk.next(index);
        }
        self.peeked.as_ref()
    }

    pub fn next_if(
        &mut self,
        index: &I,
        predicate: impl FnOnce(&W::Item) -> bool,
    ) -> Option<W::Item> {
        match self.next(index) {
            Some(item) if predicate(&item) => Some(item),
            other => {
                assert!(self.peeked.is_none());
                self.peeked = other;
                None
            }
        }
    }
}

impl<I: ?Sized, W: RevWalk<I>> RevWalk<I> for PeekableRevWalk<I, W> {
    type Item = W::Item;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        self.peeked.take().or_else(|| self.walk.next(index))
    }
}

/// Adapter that turns `RevWalk` into `Iterator` by attaching borrowed `index`.
#[derive(Clone, Debug)]
#[must_use]
pub(super) struct RevWalkBorrowedIndexIter<'a, I: ?Sized, W> {
    index: &'a I,
    walk: W,
}

impl<I: ?Sized, W> RevWalkBorrowedIndexIter<'_, I, W> {
    /// Turns into `'static`-lifetime walk object by detaching the index.
    pub fn detach(self) -> W {
        self.walk
    }
}

impl<I: ?Sized, W: RevWalk<I>> Iterator for RevWalkBorrowedIndexIter<'_, I, W> {
    type Item = W::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.walk.next(self.index)
    }
}

impl<I: ?Sized, W: RevWalk<I>> FusedIterator for RevWalkBorrowedIndexIter<'_, I, W> {}

/// Adapter that turns `RevWalk` into `Iterator` by attaching owned `index`.
#[derive(Clone, Debug)]
#[must_use]
pub(super) struct RevWalkOwnedIndexIter<I, W> {
    index: I,
    walk: W,
}

impl<I, W: RevWalk<I>> Iterator for RevWalkOwnedIndexIter<I, W> {
    type Item = W::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.walk.next(&self.index)
    }
}

impl<I, W: RevWalk<I>> FusedIterator for RevWalkOwnedIndexIter<I, W> {}

pub(super) trait RevWalkIndex {
    type Position: Copy + Ord;
    type AdjacentPositions: IntoIterator<Item = Self::Position>;

    fn adjacent_positions(&self, pos: Self::Position) -> Self::AdjacentPositions;
}

impl RevWalkIndex for CompositeIndex {
    type Position = GlobalCommitPosition;
    type AdjacentPositions = SmallGlobalCommitPositionsVec;

    fn adjacent_positions(&self, pos: Self::Position) -> Self::AdjacentPositions {
        self.commits().entry_by_pos(pos).parent_positions()
    }
}

#[derive(Clone)]
pub(super) struct RevWalkDescendantsIndex {
    children_map: HashMap<GlobalCommitPosition, DescendantIndexPositionsVec>,
}

// See SmallGlobalCommitPositionsVec for the array size.
type DescendantIndexPositionsVec = SmallVec<[Reverse<GlobalCommitPosition>; 4]>;

impl RevWalkDescendantsIndex {
    fn build(
        index: &CompositeCommitIndex,
        positions: impl IntoIterator<Item = GlobalCommitPosition>,
    ) -> Self {
        // For dense set, it's probably cheaper to use `Vec` instead of `HashMap`.
        let mut children_map: HashMap<GlobalCommitPosition, DescendantIndexPositionsVec> =
            HashMap::new();
        for pos in positions {
            children_map.entry(pos).or_default(); // mark head node
            for parent_pos in index.entry_by_pos(pos).parent_positions() {
                let parent = children_map.entry(parent_pos).or_default();
                parent.push(Reverse(pos));
            }
        }

        Self { children_map }
    }

    fn contains_pos(&self, pos: GlobalCommitPosition) -> bool {
        self.children_map.contains_key(&pos)
    }
}

impl RevWalkIndex for RevWalkDescendantsIndex {
    type Position = Reverse<GlobalCommitPosition>;
    type AdjacentPositions = DescendantIndexPositionsVec;

    fn adjacent_positions(&self, pos: Self::Position) -> Self::AdjacentPositions {
        self.children_map[&pos.0].clone()
    }
}

#[derive(Clone)]
#[must_use]
pub(super) struct RevWalkBuilder<'a> {
    index: &'a CompositeIndex,
    wanted: Vec<GlobalCommitPosition>,
    unwanted: Vec<GlobalCommitPosition>,
}

impl<'a> RevWalkBuilder<'a> {
    pub fn new(index: &'a CompositeIndex) -> Self {
        Self {
            index,
            wanted: Vec::new(),
            unwanted: Vec::new(),
        }
    }

    /// Sets head positions to be included.
    pub fn wanted_heads(mut self, positions: Vec<GlobalCommitPosition>) -> Self {
        self.wanted = positions;
        self
    }

    /// Sets root positions to be excluded. The roots precede the heads.
    pub fn unwanted_roots(mut self, positions: Vec<GlobalCommitPosition>) -> Self {
        self.unwanted = positions;
        self
    }

    /// Walks ancestors.
    pub fn ancestors(self) -> RevWalkAncestors<'a> {
        self.ancestors_with_min_pos(GlobalCommitPosition::MIN)
    }

    fn ancestors_with_min_pos(self, min_pos: GlobalCommitPosition) -> RevWalkAncestors<'a> {
        let index = self.index;
        let mut wanted_queue = RevWalkQueue::with_min_pos(min_pos);
        let mut unwanted_queue = RevWalkQueue::with_min_pos(min_pos);
        wanted_queue.extend(self.wanted, ());
        unwanted_queue.extend(self.unwanted, ());
        RevWalkBorrowedIndexIter {
            index,
            walk: RevWalkImpl {
                wanted_queue,
                unwanted_queue,
            },
        }
    }

    /// Walks ancestors within the `generation_range`.
    ///
    /// A generation number counts from the heads.
    pub fn ancestors_filtered_by_generation(
        self,
        generation_range: Range<u32>,
    ) -> RevWalkAncestorsGenerationRange<'a> {
        let index = self.index;
        let mut wanted_queue = RevWalkQueue::with_min_pos(GlobalCommitPosition::MIN);
        let mut unwanted_queue = RevWalkQueue::with_min_pos(GlobalCommitPosition::MIN);
        let item_range = RevWalkItemGenerationRange::from_filter_range(generation_range.clone());
        wanted_queue.extend(self.wanted, Reverse(item_range));
        unwanted_queue.extend(self.unwanted, ());
        RevWalkBorrowedIndexIter {
            index,
            walk: RevWalkGenerationRangeImpl {
                wanted_queue,
                unwanted_queue,
                generation_end: generation_range.end,
            },
        }
    }

    /// Walks ancestors until all of the reachable roots in `root_positions` get
    /// visited.
    ///
    /// Use this if you are only interested in descendants of the given roots.
    /// The caller still needs to filter out unwanted entries.
    pub fn ancestors_until_roots(
        self,
        root_positions: impl IntoIterator<Item = GlobalCommitPosition>,
    ) -> RevWalkAncestors<'a> {
        // We can also make it stop visiting based on the generation number. Maybe
        // it will perform better for unbalanced branchy history.
        // https://github.com/jj-vcs/jj/pull/1492#discussion_r1160678325
        let min_pos = root_positions
            .into_iter()
            .min()
            .unwrap_or(GlobalCommitPosition::MAX);
        self.ancestors_with_min_pos(min_pos)
    }

    /// Fully consumes ancestors and walks back from the `root_positions`.
    ///
    /// The returned iterator yields entries in order of ascending index
    /// position.
    pub fn descendants(
        self,
        root_positions: HashSet<GlobalCommitPosition>,
    ) -> RevWalkDescendants<'a> {
        let index = self.index;
        let candidate_positions = self
            .ancestors_until_roots(root_positions.iter().copied())
            .collect();
        RevWalkBorrowedIndexIter {
            index,
            walk: RevWalkDescendantsImpl {
                candidate_positions,
                root_positions,
                reachable_positions: HashSet::new(),
            },
        }
    }

    /// Fully consumes ancestors and walks back from the `root_positions` within
    /// the `generation_range`.
    ///
    /// A generation number counts from the roots.
    ///
    /// The returned iterator yields entries in order of ascending index
    /// position.
    pub fn descendants_filtered_by_generation(
        self,
        root_positions: Vec<GlobalCommitPosition>,
        generation_range: Range<u32>,
    ) -> RevWalkDescendantsGenerationRange {
        let index = self.index;
        let positions = self.ancestors_until_roots(root_positions.iter().copied());
        let descendants_index = RevWalkDescendantsIndex::build(index.commits(), positions);

        let mut wanted_queue = RevWalkQueue::with_min_pos(Reverse(GlobalCommitPosition::MAX));
        let unwanted_queue = RevWalkQueue::with_min_pos(Reverse(GlobalCommitPosition::MAX));
        let item_range = RevWalkItemGenerationRange::from_filter_range(generation_range.clone());
        for pos in root_positions {
            // Do not add unreachable roots which shouldn't be visited
            if descendants_index.contains_pos(pos) {
                wanted_queue.push(Reverse(pos), Reverse(item_range));
            }
        }
        RevWalkOwnedIndexIter {
            index: descendants_index,
            walk: RevWalkGenerationRangeImpl {
                wanted_queue,
                unwanted_queue,
                generation_end: generation_range.end,
            },
        }
    }
}

pub(super) type RevWalkAncestors<'a> =
    RevWalkBorrowedIndexIter<'a, CompositeIndex, RevWalkImpl<GlobalCommitPosition>>;

#[derive(Clone)]
#[must_use]
pub(super) struct RevWalkImpl<P> {
    wanted_queue: RevWalkQueue<P, ()>,
    unwanted_queue: RevWalkQueue<P, ()>,
}

impl<I: RevWalkIndex + ?Sized> RevWalk<I> for RevWalkImpl<I::Position> {
    type Item = I::Position;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        while let Some(item) = self.wanted_queue.pop() {
            self.wanted_queue.skip_while_eq(&item.pos);
            if flush_queue_until(&mut self.unwanted_queue, index, item.pos).is_some() {
                continue;
            }
            self.wanted_queue
                .extend(index.adjacent_positions(item.pos), ());
            return Some(item.pos);
        }
        None
    }
}

pub(super) type RevWalkAncestorsGenerationRange<'a> =
    RevWalkBorrowedIndexIter<'a, CompositeIndex, RevWalkGenerationRangeImpl<GlobalCommitPosition>>;
pub(super) type RevWalkDescendantsGenerationRange = RevWalkOwnedIndexIter<
    RevWalkDescendantsIndex,
    RevWalkGenerationRangeImpl<Reverse<GlobalCommitPosition>>,
>;

#[derive(Clone)]
#[must_use]
pub(super) struct RevWalkGenerationRangeImpl<P> {
    // Sort item generations in ascending order
    wanted_queue: RevWalkQueue<P, Reverse<RevWalkItemGenerationRange>>,
    unwanted_queue: RevWalkQueue<P, ()>,
    generation_end: u32,
}

impl<P: Ord> RevWalkGenerationRangeImpl<P> {
    fn enqueue_wanted_adjacents<I>(
        &mut self,
        index: &I,
        pos: P,
        generation: RevWalkItemGenerationRange,
    ) where
        I: RevWalkIndex<Position = P> + ?Sized,
    {
        // `gen.start` is incremented from 0, which should never overflow
        if generation.start + 1 >= self.generation_end {
            return;
        }
        let succ_generation = RevWalkItemGenerationRange {
            start: generation.start + 1,
            end: generation.end.saturating_add(1),
        };
        self.wanted_queue
            .extend(index.adjacent_positions(pos), Reverse(succ_generation));
    }
}

impl<I: RevWalkIndex + ?Sized> RevWalk<I> for RevWalkGenerationRangeImpl<I::Position> {
    type Item = I::Position;

    fn next(&mut self, index: &I) -> Option<Self::Item> {
        while let Some(item) = self.wanted_queue.pop() {
            if flush_queue_until(&mut self.unwanted_queue, index, item.pos).is_some() {
                self.wanted_queue.skip_while_eq(&item.pos);
                continue;
            }
            let Reverse(mut pending_gen) = item.value;
            let mut some_in_range = pending_gen.contains_end(self.generation_end);
            while let Some(x) = self.wanted_queue.pop_eq(&item.pos) {
                // Merge overlapped ranges to reduce number of the queued items.
                // For queries like `:(heads-)`, `gen.end` is close to `u32::MAX`, so
                // ranges can be merged into one. If this is still slow, maybe we can add
                // special case for upper/lower bounded ranges.
                let Reverse(generation) = x.value;
                some_in_range |= generation.contains_end(self.generation_end);
                pending_gen = if let Some(merged) = pending_gen.try_merge_end(generation) {
                    merged
                } else {
                    self.enqueue_wanted_adjacents(index, item.pos, pending_gen);
                    generation
                };
            }
            self.enqueue_wanted_adjacents(index, item.pos, pending_gen);
            if some_in_range {
                return Some(item.pos);
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RevWalkItemGenerationRange {
    start: u32,
    end: u32,
}

impl RevWalkItemGenerationRange {
    /// Translates filter range to item range so that overlapped ranges can be
    /// merged later.
    ///
    /// Example: `generation_range = 1..4`
    /// ```text
    ///     (original)                       (translated)
    ///     0 1 2 3 4                        0 1 2 3 4
    ///       *=====o  generation_range              +  generation_end
    ///     + :     :  item's generation     o=====* :  item's range
    /// ```
    fn from_filter_range(range: Range<u32>) -> Self {
        Self {
            start: 0,
            end: u32::saturating_sub(range.end, range.start),
        }
    }

    /// Suppose sorted ranges `self, other`, merges them if overlapped.
    #[must_use]
    fn try_merge_end(self, other: Self) -> Option<Self> {
        (other.start <= self.end).then(|| Self {
            start: self.start,
            end: max(self.end, other.end),
        })
    }

    #[must_use]
    fn contains_end(self, end: u32) -> bool {
        self.start < end && end <= self.end
    }
}

/// Walks queue items until `bottom_pos`. Returns item if found at `bottom_pos`.
fn flush_queue_until<I: RevWalkIndex + ?Sized>(
    queue: &mut RevWalkQueue<I::Position, ()>,
    index: &I,
    bottom_pos: I::Position,
) -> Option<RevWalkWorkItem<I::Position, ()>> {
    while let Some(item) = queue.pop_if(|x| x.pos >= bottom_pos) {
        queue.skip_while_eq(&item.pos);
        queue.extend(index.adjacent_positions(item.pos), ());
        if item.pos == bottom_pos {
            return Some(item);
        }
    }
    None
}

/// Walks descendants from the roots, in order of ascending index position.
pub(super) type RevWalkDescendants<'a> =
    RevWalkBorrowedIndexIter<'a, CompositeIndex, RevWalkDescendantsImpl>;

#[derive(Clone)]
#[must_use]
pub(super) struct RevWalkDescendantsImpl {
    candidate_positions: Vec<GlobalCommitPosition>,
    root_positions: HashSet<GlobalCommitPosition>,
    reachable_positions: HashSet<GlobalCommitPosition>,
}

impl RevWalkDescendants<'_> {
    /// Builds a set of index positions reachable from the roots.
    ///
    /// This is equivalent to `.collect()` on the new iterator, but returns the
    /// internal buffer instead.
    pub fn collect_positions_set(mut self) -> HashSet<GlobalCommitPosition> {
        self.by_ref().for_each(drop);
        self.walk.reachable_positions
    }
}

impl RevWalk<CompositeIndex> for RevWalkDescendantsImpl {
    type Item = GlobalCommitPosition;

    fn next(&mut self, index: &CompositeIndex) -> Option<Self::Item> {
        let index = index.commits();
        while let Some(candidate_pos) = self.candidate_positions.pop() {
            if self.root_positions.contains(&candidate_pos)
                || index
                    .entry_by_pos(candidate_pos)
                    .parent_positions()
                    .iter()
                    .any(|parent_pos| self.reachable_positions.contains(parent_pos))
            {
                self.reachable_positions.insert(candidate_pos);
                return Some(candidate_pos);
            }
        }
        None
    }
}

#[cfg(test)]
#[rustversion::attr(
    since(1.89),
    expect(clippy::cloned_ref_to_slice_refs, reason = "makes tests more readable")
)]
mod tests {
    use itertools::Itertools as _;

    use super::super::composite::AsCompositeIndex as _;
    use super::super::mutable::DefaultMutableIndex;
    use super::*;
    use crate::backend::ChangeId;
    use crate::backend::CommitId;
    use crate::default_index::readonly::FieldLengths;

    const TEST_FIELD_LENGTHS: FieldLengths = FieldLengths {
        // TODO: align with commit_id_generator()?
        commit_id: 3,
        change_id: 16,
    };

    /// Generator of unique 16-byte ChangeId excluding root id
    fn change_id_generator() -> impl FnMut() -> ChangeId {
        let mut iter = (1_u128..).map(|n| ChangeId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    fn to_positions_vec(
        index: &CompositeIndex,
        commit_ids: &[CommitId],
    ) -> Vec<GlobalCommitPosition> {
        commit_ids
            .iter()
            .map(|id| index.commits().commit_id_to_pos(id).unwrap())
            .collect()
    }

    #[test]
    fn test_filter_map_rev_walk() {
        let source = EagerRevWalk::new(vec![0, 1, 2, 3, 4].into_iter());
        let mut filtered = source.filter_map(|_, v| (v & 1 == 0).then_some(v + 5));
        assert_eq!(filtered.next(&()), Some(5));
        assert_eq!(filtered.next(&()), Some(7));
        assert_eq!(filtered.next(&()), Some(9));
        assert_eq!(filtered.next(&()), None);
        assert_eq!(filtered.next(&()), None);
    }

    #[test]
    fn test_map_rev_walk() {
        let source = EagerRevWalk::new(vec![0, 1, 2].into_iter());
        let mut mapped = source.map(|_, v| v + 5);
        assert_eq!(mapped.next(&()), Some(5));
        assert_eq!(mapped.next(&()), Some(6));
        assert_eq!(mapped.next(&()), Some(7));
        assert_eq!(mapped.next(&()), None);
        assert_eq!(mapped.next(&()), None);
    }

    #[test]
    fn test_peekable_rev_walk() {
        let source = EagerRevWalk::new(vec![0, 1, 2, 3].into_iter());
        let mut peekable = source.peekable();
        assert_eq!(peekable.peek(&()), Some(&0));
        assert_eq!(peekable.peek(&()), Some(&0));
        assert_eq!(peekable.next(&()), Some(0));
        assert_eq!(peekable.peeked, None);
        assert_eq!(peekable.next_if(&(), |&v| v == 2), None);
        assert_eq!(peekable.next(&()), Some(1));
        assert_eq!(peekable.next_if(&(), |&v| v == 2), Some(2));
        assert_eq!(peekable.peeked, None);
        assert_eq!(peekable.peek(&()), Some(&3));
        assert_eq!(peekable.next_if(&(), |&v| v == 3), Some(3));
        assert_eq!(peekable.peeked, None);
        assert_eq!(peekable.next(&()), None);
        assert_eq!(peekable.next(&()), None);

        let source = EagerRevWalk::new((vec![] as Vec<i32>).into_iter());
        let mut peekable = source.peekable();
        assert_eq!(peekable.peek(&()), None);
        assert_eq!(peekable.next(&()), None);
    }

    #[test]
    fn test_walk_ancestors() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        // 5
        // |\
        // 4 | 3
        // | |/
        // 1 2
        // |/
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_2.clone()]);

        let walk_commit_ids = |wanted: &[CommitId], unwanted: &[CommitId]| {
            let index = index.as_composite();
            RevWalkBuilder::new(index)
                .wanted_heads(to_positions_vec(index, wanted))
                .unwanted_roots(to_positions_vec(index, unwanted))
                .ancestors()
                .map(|pos| index.commits().entry_by_pos(pos).commit_id())
                .collect_vec()
        };

        // No wanted commits
        assert!(walk_commit_ids(&[], &[]).is_empty());
        // Simple linear walk to roo
        assert_eq!(
            walk_commit_ids(&[id_4.clone()], &[]),
            vec![id_4.clone(), id_1.clone(), id_0.clone()]
        );
        // Commits that are both wanted and unwanted are not walked
        assert_eq!(walk_commit_ids(&[id_0.clone()], &[id_0.clone()]), vec![]);
        // Commits that are listed twice are only walked once
        assert_eq!(
            walk_commit_ids(&[id_0.clone(), id_0.clone()], &[]),
            vec![id_0.clone()]
        );
        // If a commit and its ancestor are both wanted, the ancestor still gets walked
        // only once
        assert_eq!(
            walk_commit_ids(&[id_0.clone(), id_1.clone()], &[]),
            vec![id_1.clone(), id_0.clone()]
        );
        // Ancestors of both wanted and unwanted commits are not walked
        assert_eq!(
            walk_commit_ids(&[id_2.clone()], &[id_1.clone()]),
            vec![id_2.clone()]
        );
        // Same as above, but the opposite order, to make sure that order in index
        // doesn't matter
        assert_eq!(
            walk_commit_ids(&[id_1.clone()], &[id_2.clone()]),
            vec![id_1.clone()]
        );
        // Two wanted nodes
        assert_eq!(
            walk_commit_ids(&[id_1.clone(), id_2.clone()], &[]),
            vec![id_2.clone(), id_1.clone(), id_0.clone()]
        );
        // Order of output doesn't depend on order of input
        assert_eq!(
            walk_commit_ids(&[id_2.clone(), id_1.clone()], &[]),
            vec![id_2.clone(), id_1.clone(), id_0]
        );
        // Two wanted nodes that share an unwanted ancestor
        assert_eq!(
            walk_commit_ids(&[id_5.clone(), id_3.clone()], &[id_2]),
            vec![id_5, id_4, id_3, id_1]
        );
    }

    #[test]
    fn test_walk_ancestors_until_roots() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        //   7
        // 6 |
        // 5 |
        // 4 |
        // | 3
        // | 2
        // |/
        // 1
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        let id_6 = CommitId::from_hex("666666");
        let id_7 = CommitId::from_hex("777777");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone()]);
        index.add_commit_data(id_6.clone(), new_change_id(), &[id_5.clone()]);
        index.add_commit_data(id_7.clone(), new_change_id(), &[id_3.clone()]);

        let index = index.as_composite();
        let make_iter = |heads: &[CommitId], roots: &[CommitId]| {
            RevWalkBuilder::new(index)
                .wanted_heads(to_positions_vec(index, heads))
                .ancestors_until_roots(to_positions_vec(index, roots))
        };
        let to_commit_id = |pos| index.commits().entry_by_pos(pos).commit_id();

        let mut iter = make_iter(&[id_6.clone(), id_7.clone()], &[id_3.clone()]);
        assert_eq!(iter.walk.wanted_queue.len(), 2);
        assert_eq!(iter.next().map(to_commit_id), Some(id_7.clone()));
        assert_eq!(iter.next().map(to_commit_id), Some(id_6.clone()));
        assert_eq!(iter.next().map(to_commit_id), Some(id_5.clone()));
        assert_eq!(iter.walk.wanted_queue.len(), 2);
        assert_eq!(iter.next().map(to_commit_id), Some(id_4.clone()));
        assert_eq!(iter.walk.wanted_queue.len(), 1); // id_1 shouldn't be queued
        assert_eq!(iter.next().map(to_commit_id), Some(id_3.clone()));
        assert_eq!(iter.walk.wanted_queue.len(), 0); // id_2 shouldn't be queued
        assert!(iter.next().is_none());

        let iter = make_iter(&[id_6.clone(), id_7.clone(), id_2.clone()], &[id_3.clone()]);
        assert_eq!(iter.walk.wanted_queue.len(), 2); // id_2 shouldn't be queued

        let iter = make_iter(&[id_6.clone(), id_7.clone()], &[]);
        assert_eq!(iter.walk.wanted_queue.len(), 0); // no ids should be queued
    }

    #[test]
    fn test_walk_ancestors_filtered_by_generation() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        // 8 6
        // | |
        // 7 5
        // |/|
        // 4 |
        // | 3
        // 2 |
        // |/
        // 1
        // |
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        let id_6 = CommitId::from_hex("666666");
        let id_7 = CommitId::from_hex("777777");
        let id_8 = CommitId::from_hex("888888");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_3.clone()]);
        index.add_commit_data(id_6.clone(), new_change_id(), &[id_5.clone()]);
        index.add_commit_data(id_7.clone(), new_change_id(), &[id_4.clone()]);
        index.add_commit_data(id_8.clone(), new_change_id(), &[id_7.clone()]);

        let walk_commit_ids = |wanted: &[CommitId], unwanted: &[CommitId], range: Range<u32>| {
            let index = index.as_composite();
            RevWalkBuilder::new(index)
                .wanted_heads(to_positions_vec(index, wanted))
                .unwanted_roots(to_positions_vec(index, unwanted))
                .ancestors_filtered_by_generation(range)
                .map(|pos| index.commits().entry_by_pos(pos).commit_id())
                .collect_vec()
        };

        // Empty generation bounds
        assert_eq!(walk_commit_ids(&[&id_8].map(Clone::clone), &[], 0..0), []);
        assert_eq!(
            walk_commit_ids(&[&id_8].map(Clone::clone), &[], Range { start: 2, end: 1 }),
            []
        );

        // Simple generation bounds
        assert_eq!(
            walk_commit_ids(&[&id_2].map(Clone::clone), &[], 0..3),
            [&id_2, &id_1, &id_0].map(Clone::clone)
        );

        // Ancestors may be walked with different generations
        assert_eq!(
            walk_commit_ids(&[&id_6].map(Clone::clone), &[], 2..4),
            [&id_4, &id_3, &id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_5].map(Clone::clone), &[], 2..3),
            [&id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_5, &id_7].map(Clone::clone), &[], 2..3),
            [&id_2, &id_1].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_7, &id_8].map(Clone::clone), &[], 0..2),
            [&id_8, &id_7, &id_4].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_6, &id_7].map(Clone::clone), &[], 0..3),
            [&id_7, &id_6, &id_5, &id_4, &id_3, &id_2].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_6, &id_7].map(Clone::clone), &[], 2..3),
            [&id_4, &id_3, &id_2].map(Clone::clone)
        );

        // Ancestors of both wanted and unwanted commits are not walked
        assert_eq!(
            walk_commit_ids(&[&id_5].map(Clone::clone), &[&id_2].map(Clone::clone), 1..5),
            [&id_4, &id_3].map(Clone::clone)
        );
    }

    #[test]
    #[expect(clippy::redundant_clone)] // allow id_n.clone()
    fn test_walk_ancestors_filtered_by_generation_range_merging() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        // Long linear history with some short branches
        let ids = (0..11)
            .map(|n| CommitId::try_from_hex(format!("{n:06x}")).unwrap())
            .collect_vec();
        index.add_commit_data(ids[0].clone(), new_change_id(), &[]);
        for i in 1..ids.len() {
            index.add_commit_data(ids[i].clone(), new_change_id(), &[ids[i - 1].clone()]);
        }
        let id_branch5_0 = CommitId::from_hex("050000");
        let id_branch5_1 = CommitId::from_hex("050001");
        index.add_commit_data(id_branch5_0.clone(), new_change_id(), &[ids[5].clone()]);
        index.add_commit_data(
            id_branch5_1.clone(),
            new_change_id(),
            &[id_branch5_0.clone()],
        );

        let walk_commit_ids = |wanted: &[CommitId], range: Range<u32>| {
            let index = index.as_composite();
            RevWalkBuilder::new(index)
                .wanted_heads(to_positions_vec(index, wanted))
                .ancestors_filtered_by_generation(range)
                .map(|pos| index.commits().entry_by_pos(pos).commit_id())
                .collect_vec()
        };

        // Multiple non-overlapping generation ranges to track:
        // 9->6: 3..5, 6: 0..2
        assert_eq!(
            walk_commit_ids(&[&ids[9], &ids[6]].map(Clone::clone), 4..6),
            [&ids[5], &ids[4], &ids[2], &ids[1]].map(Clone::clone)
        );

        // Multiple non-overlapping generation ranges to track, and merged later:
        // 10->7: 3..5, 7: 0..2
        // 10->6: 4..6, 7->6, 1..3, 6: 0..2
        assert_eq!(
            walk_commit_ids(&[&ids[10], &ids[7], &ids[6]].map(Clone::clone), 5..7),
            [&ids[5], &ids[4], &ids[2], &ids[1], &ids[0]].map(Clone::clone)
        );

        // Merge range with sub-range (1..4 + 2..3 should be 1..4, not 1..3):
        // 8,7,6->5::1..4, B5_1->5::2..3
        assert_eq!(
            walk_commit_ids(
                &[&ids[8], &ids[7], &ids[6], &id_branch5_1].map(Clone::clone),
                5..6
            ),
            [&ids[3], &ids[2], &ids[1]].map(Clone::clone)
        );
    }

    #[test]
    fn test_walk_descendants_filtered_by_generation() {
        let mut new_change_id = change_id_generator();
        let mut index = DefaultMutableIndex::full(TEST_FIELD_LENGTHS);
        // 8 6
        // | |
        // 7 5
        // |/|
        // 4 |
        // | 3
        // 2 |
        // |/
        // 1
        // |
        // 0
        let id_0 = CommitId::from_hex("000000");
        let id_1 = CommitId::from_hex("111111");
        let id_2 = CommitId::from_hex("222222");
        let id_3 = CommitId::from_hex("333333");
        let id_4 = CommitId::from_hex("444444");
        let id_5 = CommitId::from_hex("555555");
        let id_6 = CommitId::from_hex("666666");
        let id_7 = CommitId::from_hex("777777");
        let id_8 = CommitId::from_hex("888888");
        index.add_commit_data(id_0.clone(), new_change_id(), &[]);
        index.add_commit_data(id_1.clone(), new_change_id(), &[id_0.clone()]);
        index.add_commit_data(id_2.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_3.clone(), new_change_id(), &[id_1.clone()]);
        index.add_commit_data(id_4.clone(), new_change_id(), &[id_2.clone()]);
        index.add_commit_data(id_5.clone(), new_change_id(), &[id_4.clone(), id_3.clone()]);
        index.add_commit_data(id_6.clone(), new_change_id(), &[id_5.clone()]);
        index.add_commit_data(id_7.clone(), new_change_id(), &[id_4.clone()]);
        index.add_commit_data(id_8.clone(), new_change_id(), &[id_7.clone()]);

        let visible_heads = [&id_6, &id_8].map(Clone::clone);
        let walk_commit_ids = |roots: &[CommitId], heads: &[CommitId], range: Range<u32>| {
            let index = index.as_composite();
            RevWalkBuilder::new(index)
                .wanted_heads(to_positions_vec(index, heads))
                .descendants_filtered_by_generation(to_positions_vec(index, roots), range)
                .map(|Reverse(pos)| index.commits().entry_by_pos(pos).commit_id())
                .collect_vec()
        };

        // Empty generation bounds
        assert_eq!(
            walk_commit_ids(&[&id_0].map(Clone::clone), &visible_heads, 0..0),
            []
        );
        assert_eq!(
            walk_commit_ids(
                &[&id_8].map(Clone::clone),
                &visible_heads,
                Range { start: 2, end: 1 }
            ),
            []
        );

        // Full generation bounds
        assert_eq!(
            walk_commit_ids(&[&id_0].map(Clone::clone), &visible_heads, 0..u32::MAX),
            [
                &id_0, &id_1, &id_2, &id_3, &id_4, &id_5, &id_6, &id_7, &id_8
            ]
            .map(Clone::clone)
        );

        // Simple generation bounds
        assert_eq!(
            walk_commit_ids(&[&id_3].map(Clone::clone), &visible_heads, 0..3),
            [&id_3, &id_5, &id_6].map(Clone::clone)
        );

        // Descendants may be walked with different generations
        assert_eq!(
            walk_commit_ids(&[&id_0].map(Clone::clone), &visible_heads, 2..4),
            [&id_2, &id_3, &id_4, &id_5].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_1].map(Clone::clone), &visible_heads, 2..3),
            [&id_4, &id_5].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_2, &id_3].map(Clone::clone), &visible_heads, 2..3),
            [&id_5, &id_6, &id_7].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_2, &id_4].map(Clone::clone), &visible_heads, 0..2),
            [&id_2, &id_4, &id_5, &id_7].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_2, &id_3].map(Clone::clone), &visible_heads, 0..3),
            [&id_2, &id_3, &id_4, &id_5, &id_6, &id_7].map(Clone::clone)
        );
        assert_eq!(
            walk_commit_ids(&[&id_2, &id_3].map(Clone::clone), &visible_heads, 2..3),
            [&id_5, &id_6, &id_7].map(Clone::clone)
        );

        // Roots set contains entries unreachable from heads
        assert_eq!(
            walk_commit_ids(
                &[&id_2, &id_3].map(Clone::clone),
                &[&id_8].map(Clone::clone),
                0..3
            ),
            [&id_2, &id_4, &id_7].map(Clone::clone)
        );
    }
}
