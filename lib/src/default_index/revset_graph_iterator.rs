// Copyright 2021 The Jujutsu Authors
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

use std::cmp::Ordering;
use std::cmp::min;
use std::collections::BTreeMap;
use std::collections::VecDeque;

use itertools::Itertools as _;

use super::bit_set::PositionsBitSet;
use super::composite::CompositeCommitIndex;
use super::composite::CompositeIndex;
use super::entry::CommitIndexEntry;
use super::entry::GlobalCommitPosition;
use super::rev_walk::RevWalk;
use super::revset_engine::BoxedRevWalk;
use crate::backend::CommitId;
use crate::graph::GraphEdge;
use crate::graph::GraphNode;
use crate::revset::RevsetEvaluationError;

// This can be cheaply allocated and hashed compared to `CommitId`-based type.
type CommitGraphEdge = GraphEdge<GlobalCommitPosition>;

/// Given a `RevWalk` over some set of revisions, yields the same revisions with
/// associated edge types.
///
/// If a revision's parent is in the input set, then the edge will be "direct".
/// Otherwise, there will be one "indirect" edge for each closest ancestor in
/// the set, and one "missing" edge for each edge leading outside the set.
///
/// Example (uppercase characters are in the input set):
///
/// A          A
/// |\         |\
/// B c        B :
/// |\|     => |\:
/// d E        ~ E
/// |/          ~
/// root
///
/// The implementation works by walking the input set one commit at a time. It
/// then considers all parents of the commit. It looks ahead in the input set
/// far enough that all the parents will have been consumed if they are in the
/// input (and puts them away so we can emit them later). If a parent of the
/// current commit is not in the input set (i.e. it was not in the look-ahead),
/// we walk these external commits until we end up back back in the input set.
/// That walk may result in consuming more elements from the input `RevWalk`.
/// In the example above, when we consider "A", we will initially look ahead to
/// "B" and "c". When we consider edges from the external commit "c", we will
/// further consume the input `RevWalk` to "E".
///
/// Missing edges are those that don't lead back into the input set. If all
/// edges from an external commit are missing, we consider the edge to that
/// commit to also be missing. In the example above, that means that "B" will
/// have a missing edge to "d" rather than to the root.
///
/// `RevsetGraphWalk` can be configured to skip transitive edges that it would
/// otherwise return. In this mode (which is the default), the edge from "A" to
/// "E" in the example above would be excluded because there's also a transitive
/// path from "A" to "E" via "B". The implementation of that mode
/// adds a filtering step just before yielding the edges for a commit. The
/// filtering works by doing a DFS in the simplified graph. That may require
/// even more look-ahead. Consider this example (uppercase characters are in the
/// input set):
///
///   J
///  /|
/// | i
/// | |\
/// | | H
/// G | |
/// | e f
/// |  \|\
/// |   D |
///  \ /  c
///   b  /
///   |/
///   A
///   |
///  root
///
/// When walking from "J", we'll find indirect edges to "H", "G", and "D". This
/// is our unfiltered set of edges, before removing transitive edges. In order
/// to know that "D" is an ancestor of "H", we need to also walk from "H". We
/// use the same search for finding edges from "H" as we used from "J". That
/// results in looking ahead all the way to "A". We could reduce the amount of
/// look-ahead by stopping at "c" since we're only interested in edges that
/// could lead to "D", but that would require extra book-keeping to remember for
/// later that the edges from "f" and "H" are only partially computed.
pub(super) struct RevsetGraphWalk<'a> {
    input_set_walk: BoxedRevWalk<'a>,
    /// Commits in the input set we had to take out of the `RevWalk` while
    /// walking external edges. Does not necessarily include the commit
    /// we're currently about to emit.
    look_ahead: VecDeque<GlobalCommitPosition>,
    /// The last consumed position. This is always the smallest key in the
    /// look_ahead set, but it's faster to keep a separate field for it.
    min_position: GlobalCommitPosition,
    /// Edges for commits not in the input set.
    edges: BTreeMap<GlobalCommitPosition, Vec<CommitGraphEdge>>,
    skip_transitive_edges: bool,
}

impl<'a> RevsetGraphWalk<'a> {
    pub fn new(input_set_walk: BoxedRevWalk<'a>, skip_transitive_edges: bool) -> Self {
        Self {
            input_set_walk,
            look_ahead: VecDeque::new(),
            min_position: GlobalCommitPosition::MAX,
            edges: Default::default(),
            skip_transitive_edges,
        }
    }

    fn next_index_position(
        &mut self,
        index: &CompositeIndex,
    ) -> Result<Option<GlobalCommitPosition>, RevsetEvaluationError> {
        match self.look_ahead.pop_back() {
            Some(position) => Ok(Some(position)),
            None => self.input_set_walk.next(index).transpose(),
        }
    }

    fn pop_edges_from_internal_commit(
        &mut self,
        index: &CompositeIndex,
        index_entry: &CommitIndexEntry,
    ) -> Result<Vec<CommitGraphEdge>, RevsetEvaluationError> {
        let position = index_entry.position();
        while let Some(entry) = self.edges.last_entry() {
            match entry.key().cmp(&position) {
                Ordering::Less => break, // no cached edges found
                Ordering::Equal => return Ok(entry.remove()),
                Ordering::Greater => entry.remove(),
            };
        }
        self.new_edges_from_internal_commit(index, index_entry)
    }

    fn new_edges_from_internal_commit(
        &mut self,
        index: &CompositeIndex,
        index_entry: &CommitIndexEntry,
    ) -> Result<Vec<CommitGraphEdge>, RevsetEvaluationError> {
        let mut parent_entries = index_entry.parents();
        if parent_entries.len() == 1 {
            let parent = parent_entries.next().unwrap();
            let parent_position = parent.position();
            self.consume_to(index, parent_position)?;
            if self.look_ahead.binary_search(&parent_position).is_ok() {
                Ok(vec![CommitGraphEdge::direct(parent_position)])
            } else {
                let parent_edges = self.edges_from_external_commit(index, parent)?;
                if parent_edges.iter().all(|edge| edge.is_missing()) {
                    Ok(vec![CommitGraphEdge::missing(parent_position)])
                } else {
                    Ok(parent_edges.to_vec())
                }
            }
        } else {
            let mut edges = Vec::new();
            let mut known_ancestors = PositionsBitSet::with_max_pos(index_entry.position());
            for parent in parent_entries {
                let parent_position = parent.position();
                self.consume_to(index, parent_position)?;
                if self.look_ahead.binary_search(&parent_position).is_ok() {
                    edges.push(CommitGraphEdge::direct(parent_position));
                } else {
                    let parent_edges = self.edges_from_external_commit(index, parent)?;
                    if parent_edges.iter().all(|edge| edge.is_missing()) {
                        edges.push(CommitGraphEdge::missing(parent_position));
                    } else {
                        edges.extend(
                            parent_edges
                                .iter()
                                .filter(|edge| !known_ancestors.get_set(edge.target)),
                        );
                    }
                }
            }
            if self.skip_transitive_edges {
                self.remove_transitive_edges(index.commits(), &mut edges);
            }
            Ok(edges)
        }
    }

    fn edges_from_external_commit(
        &mut self,
        index: &CompositeIndex,
        index_entry: CommitIndexEntry<'_>,
    ) -> Result<&[CommitGraphEdge], RevsetEvaluationError> {
        let position = index_entry.position();
        let mut stack = vec![index_entry];
        while let Some(entry) = stack.last() {
            let position = entry.position();
            if self.edges.contains_key(&position) {
                stack.pop().unwrap();
                continue;
            }
            let mut parent_entries = entry.parents();
            let complete_parent_edges = if parent_entries.len() == 1 {
                let parent = parent_entries.next().unwrap();
                let parent_position = parent.position();
                self.consume_to(index, parent_position)?;
                if self.look_ahead.binary_search(&parent_position).is_ok() {
                    // We have found a path back into the input set
                    Some(vec![CommitGraphEdge::indirect(parent_position)])
                } else if let Some(parent_edges) = self.edges.get(&parent_position) {
                    if parent_edges.iter().all(|edge| edge.is_missing()) {
                        Some(vec![CommitGraphEdge::missing(parent_position)])
                    } else {
                        Some(parent_edges.clone())
                    }
                } else if parent_position < self.min_position {
                    // The parent is not in the input set
                    Some(vec![CommitGraphEdge::missing(parent_position)])
                } else {
                    // The parent is not in the input set but it's somewhere in the range
                    // where we have commits in the input set, so continue searching.
                    stack.push(parent);
                    None
                }
            } else {
                let mut edges = Vec::new();
                let mut known_ancestors = PositionsBitSet::with_max_pos(position);
                let mut parents_complete = true;
                for parent in parent_entries {
                    let parent_position = parent.position();
                    self.consume_to(index, parent_position)?;
                    if self.look_ahead.binary_search(&parent_position).is_ok() {
                        // We have found a path back into the input set
                        edges.push(CommitGraphEdge::indirect(parent_position));
                    } else if let Some(parent_edges) = self.edges.get(&parent_position) {
                        if parent_edges.iter().all(|edge| edge.is_missing()) {
                            edges.push(CommitGraphEdge::missing(parent_position));
                        } else {
                            edges.extend(
                                parent_edges
                                    .iter()
                                    .filter(|edge| !known_ancestors.get_set(edge.target)),
                            );
                        }
                    } else if parent_position < self.min_position {
                        // The parent is not in the input set
                        edges.push(CommitGraphEdge::missing(parent_position));
                    } else {
                        // The parent is not in the input set but it's somewhere in the range
                        // where we have commits in the input set, so continue searching.
                        stack.push(parent);
                        parents_complete = false;
                    }
                }
                parents_complete.then(|| {
                    if self.skip_transitive_edges {
                        self.remove_transitive_edges(index.commits(), &mut edges);
                    }
                    edges
                })
            };
            if let Some(edges) = complete_parent_edges {
                stack.pop().unwrap();
                self.edges.insert(position, edges);
            }
        }
        Ok(self.edges.get(&position).unwrap())
    }

    fn remove_transitive_edges(
        &self,
        index: &CompositeCommitIndex,
        edges: &mut Vec<CommitGraphEdge>,
    ) {
        if !edges.iter().any(|edge| edge.is_indirect()) {
            return;
        }
        let Some((min_pos, max_pos)) = reachable_positions(edges).minmax().into_option() else {
            return;
        };

        let enqueue_parents = |work: &mut Vec<GlobalCommitPosition>, entry: &CommitIndexEntry| {
            if let Some(edges) = self.edges.get(&entry.position()) {
                // Edges to internal commits are known. Skip external commits
                // which should never be in the input edges.
                work.extend(reachable_positions(edges).filter(|&pos| pos >= min_pos));
            } else {
                // The commit isn't visited yet. Cannot skip external commits.
                let positions = entry.parent_positions();
                work.extend(positions.into_iter().filter(|&pos| pos >= min_pos));
            }
        };

        let mut min_generation = u32::MAX;
        let mut initial_targets = PositionsBitSet::with_max_pos(max_pos);
        let mut work = vec![];
        // To start with, add the edges one step after the input edges.
        for pos in reachable_positions(edges) {
            initial_targets.set(pos);
            let entry = index.entry_by_pos(pos);
            min_generation = min(min_generation, entry.generation_number());
            enqueue_parents(&mut work, &entry);
        }
        // Find commits reachable transitively and add them to the `unwanted` set.
        let mut unwanted = PositionsBitSet::with_max_pos(max_pos);
        while let Some(pos) = work.pop() {
            if unwanted.get_set(pos) {
                // Already visited
                continue;
            }
            if initial_targets.get(pos) {
                // Already visited
                continue;
            }
            let entry = index.entry_by_pos(pos);
            if entry.generation_number() < min_generation {
                continue;
            }
            enqueue_parents(&mut work, &entry);
        }

        edges.retain(|edge| !unwanted.get(edge.target));
    }

    fn consume_to(
        &mut self,
        index: &CompositeIndex,
        pos: GlobalCommitPosition,
    ) -> Result<(), RevsetEvaluationError> {
        while pos < self.min_position {
            if let Some(next_position) = self.input_set_walk.next(index).transpose()? {
                self.look_ahead.push_front(next_position);
                self.min_position = next_position;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn try_next(
        &mut self,
        index: &CompositeIndex,
    ) -> Result<Option<GraphNode<CommitId>>, RevsetEvaluationError> {
        let Some(position) = self.next_index_position(index)? else {
            return Ok(None);
        };
        let entry = index.commits().entry_by_pos(position);
        let edges = self.pop_edges_from_internal_commit(index, &entry)?;
        let edges = edges
            .iter()
            .map(|edge| edge.map(|pos| index.commits().entry_by_pos(pos).commit_id()))
            .collect();
        Ok(Some((entry.commit_id(), edges)))
    }
}

impl RevWalk<CompositeIndex> for RevsetGraphWalk<'_> {
    type Item = Result<GraphNode<CommitId>, RevsetEvaluationError>;

    fn next(&mut self, index: &CompositeIndex) -> Option<Self::Item> {
        self.try_next(index).transpose()
    }
}

fn reachable_positions(
    edges: &[CommitGraphEdge],
) -> impl DoubleEndedIterator<Item = GlobalCommitPosition> {
    edges
        .iter()
        .filter(|edge| !edge.is_missing())
        .map(|edge| edge.target)
}
