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

//! General-purpose async DAG algorithms.

use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::iter;
use std::mem;

use futures::Stream;
use futures::future::try_join_all;
use futures::stream;
use itertools::Itertools as _;
use smallvec::SmallVec;
use smallvec::smallvec_inline;

/// Traverses nodes from `start` in depth-first order.
///
/// An `Err` is emitted as a node with no neighbors. Caller may decide to
/// short-circuit on it.
pub fn dfs<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
) -> impl Iterator<Item = Result<T, E>>
where
    ID: Hash + Eq,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut work: Vec<Result<T, E>> = start.into_iter().collect();
    let mut visited: HashSet<ID> = HashSet::new();
    iter::from_fn(move || {
        loop {
            let c = match work.pop() {
                Some(Ok(c)) => c,
                r @ (Some(Err(_)) | None) => return r,
            };
            let id = id_fn(&c);
            if visited.contains(&id) {
                continue;
            }
            for p in neighbors_fn(&c) {
                work.push(p);
            }
            visited.insert(id);
            return Some(Ok(c));
        }
    })
}

/// Builds a list of `Ok` nodes reachable from the `start` where neighbors come
/// before the node itself.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error. If the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub async fn topo_order_forward<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl AsyncFnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut stack: Vec<(T, bool)> = start.into_iter().map(|r| Ok((r?, false))).try_collect()?;
    let mut visiting = HashSet::new();
    let mut emitted = HashSet::new();
    let mut result = vec![];
    while let Some((node, neighbors_visited)) = stack.pop() {
        let id = id_fn(&node);
        if emitted.contains(&id) {
            continue;
        }
        if !neighbors_visited {
            if !visiting.insert(id.clone()) {
                return Err(cycle_fn(node));
            }
            let neighbors_iter = neighbors_fn(&node).await.into_iter();
            stack.reserve(neighbors_iter.size_hint().0 + 1);
            stack.push((node, true));
            for neighbor in neighbors_iter {
                stack.push((neighbor?, false));
            }
        } else {
            visiting.remove(&id);
            emitted.insert(id);
            result.push(node);
        }
    }
    Ok(result)
}

/// Builds a list of `Ok` nodes reachable from the `start` where neighbors come
/// after the node itself.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error. If the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub async fn topo_order_reverse<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    neighbors_fn: impl AsyncFnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut result = topo_order_forward(start, id_fn, neighbors_fn, cycle_fn).await?;
    result.reverse();
    Ok(result)
}

/// Like `topo_order_reverse()`, but can iterate linear DAG lazily.
///
/// The DAG is supposed to be (mostly) topologically ordered by `T: Ord`.
/// For example, topological order of chronological data should respect
/// timestamp (except a few outliers caused by clock skew.)
///
/// Use `topo_order_reverse()` if the DAG is heavily branched. This can
/// only process linear part lazily.
///
/// The returned iterator short-circuits at an `Err`. Pending non-linear nodes
/// before the `Err` will be discarded. If the graph has cycle, `cycle_fn()` is
/// called with one of the nodes involved in the cycle.
pub fn topo_order_reverse_lazy<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    neighbors_fn: impl AsyncFnMut(&T) -> NI,
    cycle_fn: impl FnMut(T) -> E,
) -> impl Stream<Item = Result<T, E>>
where
    T: Ord,
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut inner = TopoOrderReverseLazyInner::empty();
    inner.extend(start);
    stream::unfold(
        (inner, id_fn, neighbors_fn, cycle_fn),
        |(mut inner, id_fn, mut neighbors_fn, mut cycle_fn)| async move {
            inner
                .next(&id_fn, &mut neighbors_fn, &mut cycle_fn)
                .await
                .map(|result| (result, (inner, id_fn, neighbors_fn, cycle_fn)))
        },
    )
}

#[derive(Clone, Debug)]
struct TopoOrderReverseLazyInner<T, ID, E> {
    start: Vec<T>,
    result: Vec<Result<T, E>>,
    emitted: HashSet<ID>,
}

impl<T: Ord, ID: Hash + Eq + Clone, E> TopoOrderReverseLazyInner<T, ID, E> {
    fn empty() -> Self {
        Self {
            start: Vec::new(),
            result: Vec::new(),
            emitted: HashSet::new(),
        }
    }

    fn extend(&mut self, iter: impl IntoIterator<Item = Result<T, E>>) {
        let iter = iter.into_iter();
        self.start.reserve(iter.size_hint().0);
        for res in iter {
            if let Ok(node) = res {
                self.start.push(node);
            } else {
                // Emit the error and terminate
                self.start.clear();
                self.result.insert(0, res);
                return;
            }
        }
    }

    async fn next<NI: IntoIterator<Item = Result<T, E>>>(
        &mut self,
        id_fn: impl Fn(&T) -> ID,
        mut neighbors_fn: impl AsyncFnMut(&T) -> NI,
        mut cycle_fn: impl FnMut(T) -> E,
    ) -> Option<Result<T, E>> {
        if let Some(res) = self.result.pop() {
            return Some(res);
        }

        // Fast path for linear DAG
        if self.start.len() <= 1 {
            let node = self.start.pop()?;
            self.extend(neighbors_fn(&node).await);
            if self.emitted.insert(id_fn(&node)) {
                return Some(Ok(node));
            } else {
                return Some(Err(cycle_fn(node)));
            }
        }

        // Extract graph nodes based on T's order, and sort them by using ids
        // (because we wouldn't want to clone T itself)
        let start_ids = self.start.iter().map(&id_fn).collect_vec();
        match look_ahead_sub_graph(mem::take(&mut self.start), &id_fn, &mut neighbors_fn).await {
            Ok((mut node_map, neighbor_ids_map, remainder)) => {
                self.start = remainder;
                let sorted_ids = match topo_order_forward(
                    start_ids.iter().map(Ok),
                    |id| *id,
                    async |id| neighbor_ids_map[id].iter().map(Ok),
                    |id| cycle_fn(node_map.remove(id).unwrap()),
                )
                .await
                {
                    Ok(ids) => ids,
                    Err(err) => return Some(Err(err)),
                };
                self.result.reserve(sorted_ids.len());
                for id in sorted_ids {
                    let (id, node) = node_map.remove_entry(id).unwrap();
                    if self.emitted.insert(id) {
                        self.result.push(Ok(node));
                    } else {
                        self.result.push(Err(cycle_fn(node)));
                    }
                }
                self.result.pop()
            }
            Err(err) => Some(Err(err)),
        }
    }
}

/// Splits DAG at the first single fork point, and builds a list of nodes
/// reachable from the `start` where neighbors come after the node itself.
///
/// This is a building block for lazy DAG iterators similar to
/// [`topo_order_reverse_lazy()`]. The `start` list will be updated to
/// include the next nodes to visit.
///
/// If the split chunk of the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub async fn topo_order_reverse_chunked<T, ID, E, NI>(
    start: &mut Vec<T>,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl AsyncFnMut(&T) -> NI,
    mut cycle_fn: impl FnMut(T) -> E,
) -> Result<SmallVec<[T; 1]>, E>
where
    T: Ord,
    ID: Hash + Eq + Clone,
    NI: IntoIterator<Item = Result<T, E>>,
{
    // Fast path for linear DAG
    if start.len() <= 1 {
        let Some(node) = start.pop() else {
            return Ok(SmallVec::new());
        };
        let neighbors_iter = neighbors_fn(&node).await.into_iter();
        start.reserve(neighbors_iter.size_hint().0);
        for neighbor in neighbors_iter {
            start.push(neighbor?);
        }
        return Ok(smallvec_inline![node]);
    }

    // Extract graph nodes based on T's order, and sort them by using ids
    // (because we wouldn't want to clone T itself)
    let start_ids = start.iter().map(&id_fn).collect_vec();
    let (mut node_map, neighbor_ids_map, remainder) =
        look_ahead_sub_graph(mem::take(start), &id_fn, &mut neighbors_fn).await?;
    *start = remainder;
    let sorted_ids = topo_order_forward(
        start_ids.iter().map(Ok),
        |id| *id,
        async |id| neighbor_ids_map[id].iter().map(Ok),
        |id| cycle_fn(node_map.remove(id).unwrap()),
    )
    .await?;
    let sorted_nodes = sorted_ids
        .iter()
        .rev()
        .map(|&id| node_map.remove(id).unwrap())
        .collect();
    Ok(sorted_nodes)
}

/// Splits DAG at single fork point, and extracts branchy part as sub graph.
///
/// ```text
///  o | C
///  | o B
///  |/ <---- split here (A->B or A->C would create cycle)
///  o A
/// ```
///
/// If a branch reached to root (empty neighbors), the graph can't be split
/// anymore because the other branch may be connected to a descendant of
/// the rooted branch.
///
/// ```text
///  o | C
///  | o B
///  |  <---- can't split here (there may be edge A->B)
///  o A
/// ```
///
/// We assume the graph is (mostly) topologically ordered by `T: Ord`.
async fn look_ahead_sub_graph<T, ID, E, NI>(
    start: Vec<T>,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl AsyncFnMut(&T) -> NI,
) -> Result<(HashMap<ID, T>, HashMap<ID, Vec<ID>>, Vec<T>), E>
where
    T: Ord,
    ID: Hash + Eq + Clone,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut queue: BinaryHeap<T> = start.into();
    // Build separate node/neighbors maps since lifetime is different at caller
    let mut node_map: HashMap<ID, T> = HashMap::new();
    let mut neighbor_ids_map: HashMap<ID, Vec<ID>> = HashMap::new();
    let mut has_reached_root = false;
    while queue.len() > 1 || node_map.is_empty() || has_reached_root {
        let Some(node) = queue.pop() else {
            break;
        };
        let node_id = id_fn(&node);
        if node_map.contains_key(&node_id) {
            continue;
        }

        let mut neighbor_ids = Vec::new();
        let mut neighbors_iter = neighbors_fn(&node).await.into_iter().peekable();
        has_reached_root |= neighbors_iter.peek().is_none();
        for neighbor in neighbors_iter {
            let neighbor = neighbor?;
            neighbor_ids.push(id_fn(&neighbor));
            queue.push(neighbor);
        }
        node_map.insert(node_id.clone(), node);
        neighbor_ids_map.insert(node_id, neighbor_ids);
    }

    assert!(queue.len() <= 1, "order of remainder shouldn't matter");
    let remainder = queue.into_vec();

    // Omit unvisited neighbors
    if let Some(unvisited_id) = remainder.first().map(&id_fn) {
        for neighbor_ids in neighbor_ids_map.values_mut() {
            neighbor_ids.retain(|id| *id != unvisited_id);
        }
    }

    Ok((node_map, neighbor_ids_map, remainder))
}

/// Builds a list of `Ok` nodes reachable from the `start` where neighbors come
/// after the node itself.
///
/// Unlike `topo_order_reverse()`, nodes are sorted in reverse `T: Ord` order
/// so long as they can respect the topological requirement.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error. If the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub async fn topo_order_reverse_ord<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl AsyncFnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    T: Ord,
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    struct InnerNode<T> {
        node: Option<T>,
        indegree: usize,
    }

    // DFS to accumulate incoming edges
    let mut stack: Vec<T> = start.into_iter().try_collect()?;
    let mut head_node_map: HashMap<ID, T> = HashMap::new();
    let mut inner_node_map: HashMap<ID, InnerNode<T>> = HashMap::new();
    let mut neighbor_ids_map: HashMap<ID, Vec<ID>> = HashMap::new();
    while let Some(node) = stack.pop() {
        let node_id = id_fn(&node);
        if neighbor_ids_map.contains_key(&node_id) {
            continue; // Already visited
        }

        let neighbors_iter = neighbors_fn(&node).await.into_iter();
        let pos = stack.len();
        stack.reserve(neighbors_iter.size_hint().0);
        for neighbor in neighbors_iter {
            stack.push(neighbor?);
        }
        let neighbor_ids = stack[pos..].iter().map(&id_fn).collect_vec();
        if let Some(inner) = inner_node_map.get_mut(&node_id) {
            inner.node = Some(node);
        } else {
            head_node_map.insert(node_id.clone(), node);
        }

        for neighbor_id in &neighbor_ids {
            if let Some(inner) = inner_node_map.get_mut(neighbor_id) {
                inner.indegree += 1;
            } else {
                let inner = InnerNode {
                    node: head_node_map.remove(neighbor_id),
                    indegree: 1,
                };
                inner_node_map.insert(neighbor_id.clone(), inner);
            }
        }
        neighbor_ids_map.insert(node_id, neighbor_ids);
    }

    debug_assert!(
        head_node_map
            .keys()
            .all(|id| !inner_node_map.contains_key(id))
    );
    debug_assert!(inner_node_map.values().all(|inner| inner.node.is_some()));
    debug_assert!(inner_node_map.values().all(|inner| inner.indegree > 0));

    // Using Kahn's algorithm
    let mut queue: BinaryHeap<T> = head_node_map.into_values().collect();
    let mut result = Vec::new();
    while let Some(node) = queue.pop() {
        let node_id = id_fn(&node);
        result.push(node);
        for neighbor_id in neighbor_ids_map.remove(&node_id).unwrap() {
            let inner = inner_node_map.get_mut(&neighbor_id).unwrap();
            inner.indegree -= 1;
            if inner.indegree == 0 {
                queue.push(inner.node.take().unwrap());
                inner_node_map.remove(&neighbor_id);
            }
        }
    }

    if let Some(inner) = inner_node_map.into_values().next() {
        Err(cycle_fn(inner.node.unwrap()))
    } else {
        Ok(result)
    }
}

/// Finds `Ok` nodes in the start set that are not reachable from other nodes in
/// the start set.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error.
pub async fn heads<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    neighbors_fn: impl AsyncFn(&T) -> Result<NI, E>,
) -> Result<HashSet<T>, E>
where
    T: Hash + Eq + Clone,
    ID: Hash + Eq,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let mut heads: HashSet<T> = start.into_iter().collect();
    // Do a BFS until we have only one item left in the frontier. That frontier must
    // have originated from one of the heads, and since there can't be cycles,
    // it won't be able to eliminate any other heads.
    let mut frontier: Vec<T> = heads.iter().cloned().collect();
    let mut visited: HashSet<ID> = heads.iter().map(&id_fn).collect();
    let mut root_reached = false;
    while frontier.len() > 1 || (!frontier.is_empty() && root_reached) {
        let neighbors_lists = try_join_all(frontier.iter().map(|node| neighbors_fn(node))).await?;
        let mut new_frontier = vec![];
        for neighbors in neighbors_lists {
            let length_before = new_frontier.len();
            new_frontier.extend(neighbors);
            root_reached |= new_frontier.len() == length_before;
        }
        frontier = new_frontier;
        for node in &frontier {
            heads.remove(node);
        }
        frontier.retain(|node| visited.insert(id_fn(node)));
    }
    Ok(heads)
}

/// Finds the closest common `Ok` neighbor among the `set1` and `set2`. Uses
/// `T`'s `Ord` implementation as a heuristic for determining which neighbor to
/// visit next. The neighbor that compares greater will be visited first.
///
/// If the traverse reached to an `Err`, this function terminates and returns
/// the error.
pub async fn closest_common_node<T, ID, E, II1, II2, NI>(
    set1: II1,
    set2: II2,
    id_fn: impl Fn(&T) -> ID,
    neighbors_fn: impl AsyncFn(&T) -> Result<NI, E>,
) -> Result<Option<T>, E>
where
    T: Ord,
    ID: Hash + Eq,
    II1: IntoIterator<Item = T>,
    II2: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let mut encountered1 = HashSet::new();
    let mut encountered2 = HashSet::new();

    // Contains (node, is_set1) items
    let mut work = BinaryHeap::new();

    for node in set1 {
        encountered1.insert(id_fn(&node));
        work.push((node, true));
    }
    for node in set2 {
        encountered2.insert(id_fn(&node));
        work.push((node, false));
    }

    while let Some((node, is_set1)) = work.pop() {
        let (this_encountered, other_encountered) = if is_set1 {
            (&mut encountered1, &mut encountered2)
        } else {
            (&mut encountered2, &mut encountered1)
        };
        if other_encountered.contains(&id_fn(&node)) {
            return Ok(Some(node));
        }
        let neighbors = neighbors_fn(&node).await?;
        for neighbor in neighbors {
            let neighbor_id = id_fn(&neighbor);
            if this_encountered.insert(neighbor_id) {
                work.push((neighbor, is_set1));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use assert_matches::assert_matches;
    use futures::StreamExt as _;
    use futures::TryStreamExt as _;
    use maplit::hashmap;
    use maplit::hashset;
    use pollster::FutureExt as _;

    use super::*;

    fn to_ok_iter(
        iter: impl IntoIterator<Item = char>,
    ) -> impl Iterator<Item = Result<char, char>> {
        iter.into_iter().map(Ok)
    }

    #[test]
    fn test_dfs() {
        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec![Ok('A'), Err('X')],
            'C' => vec![Ok('B')],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = |node: &char| neighbors[node].clone();

        // Self and neighbor nodes shouldn't be lost at the error.
        let nodes = dfs([Ok('C')], id_fn, neighbors_fn).collect_vec();
        assert_eq!(nodes, [Ok('C'), Ok('B'), Err('X'), Ok('A')]);
    }

    #[test]
    fn test_topo_order_reverse_linear() {
        // This graph:
        //  o C
        //  o B
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse(vec![Ok('C'), Ok('B')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse(vec![Ok('B'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common: Vec<_> =
            topo_order_reverse_lazy(vec![Ok('C'), Ok('B')], id_fn, neighbors_fn, cycle_fn)
                .try_collect()
                .block_on()
                .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common: Vec<_> =
            topo_order_reverse_lazy(vec![Ok('B'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
                .try_collect()
                .block_on()
                .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse_ord(vec![Ok('C'), Ok('B')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse_ord(vec![Ok('B'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_merge() {
        // This graph:
        //  o F
        //  |\
        //  o | E
        //  | o D
        //  | o C
        //  | o B
        //  |/
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['C'],
            'E' => vec!['A'],
            'F' => vec!['E', 'D'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('F')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
        let common = topo_order_reverse(
            vec![Ok('F'), Ok('E'), Ok('C')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'D', 'E', 'C', 'B', 'A']);
        let common = topo_order_reverse(
            vec![Ok('F'), Ok('D'), Ok('E')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'D', 'C', 'B', 'E', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('F')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
        let common: Vec<_> = topo_order_reverse_lazy(
            vec![Ok('F'), Ok('E'), Ok('C')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .try_collect()
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'D', 'E', 'C', 'B', 'A']);
        let common: Vec<_> = topo_order_reverse_lazy(
            vec![Ok('F'), Ok('D'), Ok('E')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .try_collect()
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'D', 'C', 'B', 'E', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('F')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
        let common = topo_order_reverse_ord(
            vec![Ok('F'), Ok('E'), Ok('C')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
        let common = topo_order_reverse_ord(
            vec![Ok('F'), Ok('D'), Ok('E')],
            id_fn,
            neighbors_fn,
            cycle_fn,
        )
        .block_on()
        .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_nested_merges() {
        // This graph:
        //  o I
        //  |\
        //  | o H
        //  | |\
        //  | | o G
        //  | o | F
        //  | | o E
        //  o |/ D
        //  | o C
        //  o | B
        //  |/
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['A'],
            'D' => vec!['B'],
            'E' => vec!['C'],
            'F' => vec!['C'],
            'G' => vec!['E'],
            'H' => vec!['F', 'G'],
            'I' => vec!['D', 'H'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'D', 'B', 'H', 'F', 'G', 'E', 'C', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'D', 'B', 'H', 'F', 'G', 'E', 'C', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'H', 'G', 'F', 'E', 'D', 'C', 'B', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_nested_merges_bad_order() {
        // This graph:
        //  o I
        //  |\
        //  | |\
        //  | | |\
        //  | | | o h (h > I)
        //  | | |/|
        //  | | o | G
        //  | |/| o f
        //  | o |/ e (e > I, G)
        //  |/| o D
        //  o |/ C
        //  | o b (b > D)
        //  |/
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'b' => vec!['A'],
            'C' => vec!['A'],
            'D' => vec!['b'],
            'e' => vec!['C', 'b'],
            'f' => vec!['D'],
            'G' => vec!['e', 'D'],
            'h' => vec!['G', 'f'],
            'I' => vec!['C', 'e', 'G', 'h'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'h', 'G', 'e', 'C', 'f', 'D', 'b', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'h', 'G', 'e', 'C', 'f', 'D', 'b', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('I')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['I', 'h', 'f', 'G', 'e', 'D', 'b', 'C', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_merge_bad_fork_order_at_root() {
        // This graph:
        //  o E
        //  |\
        //  o | D
        //  | o C
        //  | o B
        //  |/
        //  o a (a > D, B)

        let neighbors = hashmap! {
            'a' => vec![],
            'B' => vec!['a'],
            'C' => vec!['B'],
            'D' => vec!['a'],
            'E' => vec!['D', 'C'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('E')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['E', 'D', 'C', 'B', 'a']);

        // The root node 'a' is visited before 'C'. If the graph were split there,
        // the branch 'C->B->a' would be orphaned.
        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('E')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['E', 'D', 'C', 'B', 'a']);

        let common = topo_order_reverse_ord(vec![Ok('E')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['E', 'D', 'C', 'B', 'a']);
    }

    #[test]
    fn test_topo_order_reverse_merge_and_linear() {
        // This graph:
        //  o G
        //  |\
        //  | o F
        //  o | E
        //  | o D
        //  |/
        //  o C
        //  o B
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['C'],
            'E' => vec!['C'],
            'F' => vec!['D'],
            'G' => vec!['E', 'F'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'E', 'F', 'D', 'C', 'B', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'E', 'F', 'D', 'C', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'F', 'E', 'D', 'C', 'B', 'A']);

        // Iterator can be lazy for linear chunks.
        let mut inner_iter = TopoOrderReverseLazyInner::empty();
        inner_iter.extend([Ok('G')]);
        assert_eq!(
            inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on(),
            Some(Ok('G'))
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());
        assert_eq!(
            iter::from_fn(|| inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on())
                .take(4)
                .collect_vec(),
            ['E', 'F', 'D', 'C'].map(Ok),
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());

        // Run each step of lazy iterator by using low-level function.
        let mut start = vec!['G'];
        let next = |start: &mut Vec<char>| {
            topo_order_reverse_chunked(start, id_fn, neighbors_fn, cycle_fn)
                .block_on()
                .unwrap()
        };
        assert_eq!(next(&mut start), ['G'].into());
        assert_eq!(start, ['E', 'F']);
        assert_eq!(next(&mut start), ['E', 'F', 'D', 'C'].into());
        assert_eq!(start, ['B']);
        assert_eq!(next(&mut start), ['B'].into());
        assert_eq!(start, ['A']);
        assert_eq!(next(&mut start), ['A'].into());
        assert!(start.is_empty());
        assert!(next(&mut start).is_empty());
        assert!(start.is_empty());
    }

    #[test]
    fn test_topo_order_reverse_merge_and_linear_bad_fork_order() {
        // This graph:
        //  o G
        //  |\
        //  o | F
        //  o | E
        //  | o D
        //  |/
        //  o c (c > E, D)
        //  o B
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'c' => vec!['B'],
            'D' => vec!['c'],
            'E' => vec!['c'],
            'F' => vec!['E'],
            'G' => vec!['F', 'D'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'F', 'E', 'D', 'c', 'B', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'F', 'E', 'D', 'c', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'F', 'E', 'D', 'c', 'B', 'A']);

        // Iterator can be lazy for linear chunks. The node 'c' is visited before 'D',
        // but it will be processed lazily.
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let mut inner_iter = TopoOrderReverseLazyInner::empty();
        inner_iter.extend([Ok('G')]);
        assert_eq!(
            inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on(),
            Some(Ok('G'))
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());
        assert_eq!(
            iter::from_fn(|| inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on())
                .take(4)
                .collect_vec(),
            ['F', 'E', 'D', 'c'].map(Ok),
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());

        // Run each step of lazy iterator by using low-level function.
        let mut start = vec!['G'];
        let next = |start: &mut Vec<char>| {
            topo_order_reverse_chunked(start, id_fn, neighbors_fn, cycle_fn)
                .block_on()
                .unwrap()
        };
        assert_eq!(next(&mut start), ['G'].into());
        assert_eq!(start, ['F', 'D']);
        assert_eq!(next(&mut start), ['F', 'E', 'D', 'c'].into());
        assert_eq!(start, ['B']);
        assert_eq!(next(&mut start), ['B'].into());
        assert_eq!(start, ['A']);
        assert_eq!(next(&mut start), ['A'].into());
        assert!(start.is_empty());
        assert!(next(&mut start).is_empty());
        assert!(start.is_empty());
    }

    #[test]
    fn test_topo_order_reverse_merge_and_linear_bad_merge_order() {
        // This graph:
        //  o G
        //  |\
        //  o | f (f > G)
        //  o | e
        //  | o d (d > G)
        //  |/
        //  o C
        //  o B
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'd' => vec!['C'],
            'e' => vec!['C'],
            'f' => vec!['e'],
            'G' => vec!['f', 'd'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'f', 'e', 'd', 'C', 'B', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'f', 'e', 'd', 'C', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('G')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['G', 'f', 'e', 'd', 'C', 'B', 'A']);

        // Iterator can be lazy for linear chunks.
        let mut inner_iter = TopoOrderReverseLazyInner::empty();
        inner_iter.extend([Ok('G')]);
        assert_eq!(
            inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on(),
            Some(Ok('G'))
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());
        assert_eq!(
            iter::from_fn(|| inner_iter.next(id_fn, neighbors_fn, cycle_fn).block_on())
                .take(4)
                .collect_vec(),
            ['f', 'e', 'd', 'C'].map(Ok),
        );
        assert!(!inner_iter.start.is_empty());
        assert!(inner_iter.result.is_empty());

        // Run each step of lazy iterator by using low-level function.
        let mut start = vec!['G'];
        let next = |start: &mut Vec<char>| {
            topo_order_reverse_chunked(start, id_fn, neighbors_fn, cycle_fn)
                .block_on()
                .unwrap()
        };
        assert_eq!(next(&mut start), ['G'].into());
        assert_eq!(start, ['f', 'd']);
        assert_eq!(next(&mut start), ['f', 'e', 'd', 'C'].into());
        assert_eq!(start, ['B']);
        assert_eq!(next(&mut start), ['B'].into());
        assert_eq!(start, ['A']);
        assert_eq!(next(&mut start), ['A'].into());
        assert!(start.is_empty());
        assert!(next(&mut start).is_empty());
        assert!(start.is_empty());
    }

    #[test]
    fn test_topo_order_reverse_multiple_heads() {
        // This graph:
        //  o F
        //  |\
        //  o | E
        //  | o D
        //  | | o C
        //  | | |
        //  | | o B
        //  | |/
        //  |/
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['A'],
            'E' => vec!['A'],
            'F' => vec!['E', 'D'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('F'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);

        let common: Vec<_> =
            topo_order_reverse_lazy(vec![Ok('F'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
                .try_collect()
                .block_on()
                .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('F'), Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_multiple_roots() {
        // This graph:
        //  o D
        //  | \
        //  o | C
        //    o B
        //    o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec![],
            'D' => vec!['C', 'B'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['D', 'C', 'B', 'A']);

        let common: Vec<_> = topo_order_reverse_lazy(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn)
            .try_collect()
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['D', 'C', 'B', 'A']);

        let common = topo_order_reverse_ord(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_eq!(common, vec!['D', 'C', 'B', 'A']);
    }

    #[test]
    fn test_topo_order_reverse_cycle_linear() {
        // This graph:
        //  o C
        //  o B
        //  o A (to C)

        let neighbors = hashmap! {
            'A' => vec!['C'],
            'B' => vec!['A'],
            'C' => vec!['B'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let result = topo_order_reverse(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_matches!(result, Err('C' | 'B' | 'A'));

        let result: Vec<_> = topo_order_reverse_lazy(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .take(4)
            .collect()
            .block_on();
        assert_matches!(
            result[..],
            [Ok('C'), Ok('B'), Ok('A'), Err('C' | 'B' | 'A')]
        );

        let result =
            topo_order_reverse_ord(vec![Ok('C')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_matches!(result, Err('C' | 'B' | 'A'));
    }

    #[test]
    fn test_topo_order_reverse_cycle_to_branchy_sub_graph() {
        // This graph:
        //  o D
        //  |\
        //  | o C
        //  |/
        //  o B
        //  o A (to C)

        let neighbors = hashmap! {
            'A' => vec!['C'],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['B', 'C'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let result = topo_order_reverse(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_matches!(result, Err('C' | 'B' | 'A'));

        let result: Vec<_> = topo_order_reverse_lazy(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn)
            .take(5)
            .collect()
            .block_on();
        assert_matches!(
            result[..],
            [Ok('D'), Ok('C'), Ok('B'), Ok('A'), Err('C' | 'B' | 'A')]
        );

        let result =
            topo_order_reverse_ord(vec![Ok('D')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_matches!(result, Err('C' | 'B' | 'A'));
    }

    #[test]
    fn test_topo_order_reverse_lazy_cycle_within_branchy_sub_graph() {
        // This graph:
        //  o D
        //  |\
        //  | o C
        //  |/
        //  o B (to C)
        //  o A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A', 'C'],
            'C' => vec!['B'],
            'D' => vec!['B', 'C'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| to_ok_iter(neighbors[node].iter().copied());
        let cycle_fn = |id| id;

        let mut stream = pin!(topo_order_reverse_lazy(
            [Ok('D')],
            id_fn,
            neighbors_fn,
            cycle_fn
        ));
        stream.next().block_on().unwrap().unwrap(); // Skip first item (D).
        let result = stream.next().block_on().unwrap();
        assert_matches!(result, Err('C' | 'B'));

        // Try again with low-level function
        let mut start = vec!['D'];
        topo_order_reverse_chunked(&mut start, id_fn, neighbors_fn, cycle_fn)
            .block_on()
            .unwrap();
        assert_matches!(
            topo_order_reverse_chunked(&mut start, id_fn, neighbors_fn, cycle_fn).block_on(),
            Err('C' | 'B')
        );
    }

    #[test]
    fn test_topo_order() {
        let neighbors = hashmap! {
            'A' => vec![Err('Y')],
            'B' => vec![Ok('A'), Err('X')],
            'C' => vec![Ok('B')],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        // Terminates at Err('X') no matter if the sorting order is forward or
        // reverse. The visiting order matters.
        let result = topo_order_forward([Ok('C')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_eq!(result, Err('X'));
        let result = topo_order_reverse([Ok('C')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_eq!(result, Err('X'));
        let nodes: Vec<_> = topo_order_reverse_lazy([Ok('C')], id_fn, neighbors_fn, cycle_fn)
            .collect()
            .block_on();
        assert_eq!(nodes, [Ok('C'), Ok('B'), Err('X')]);
        let result = topo_order_reverse_ord([Ok('C')], id_fn, neighbors_fn, cycle_fn).block_on();
        assert_eq!(result, Err('X'));
    }

    #[test]
    fn test_closest_common_node_tricky() {
        // Test this case where A is the shortest distance away, but we still want the
        // result to be B because A is an ancestor of B. In other words, we want
        // to minimize the longest distance.
        //
        //  E       H
        //  |\     /|
        //  | D   G |
        //  | C   F |
        //   \ \ / /
        //    \ B /
        //     \|/
        //      A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['C'],
            'E' => vec!['A','D'],
            'F' => vec!['B'],
            'G' => vec!['F'],
            'H' => vec!['A', 'G'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let common = closest_common_node(vec!['E'], vec!['H'], id_fn, neighbors_fn).block_on();
        assert_eq!(common, Ok(Some('B')));
    }

    #[test]
    fn test_closest_common_node_tricky_ancestor() {
        // Find the clostest common ancestor between B and F. It should be B because
        // it's even an ancestor of F, but we used to find A because the path to it
        // from F is shorter (via E).
        //
        //  F
        //  |\
        //  D |
        //  | |
        //  C E
        //  | |
        //  B |
        //  |/
        //  A

        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec!['A'],
            'C' => vec!['B'],
            'D' => vec!['C'],
            'E' => vec!['A'],
            'F' => vec!['D', 'E'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let common = closest_common_node(vec!['B'], vec!['F'], id_fn, neighbors_fn).block_on();
        assert_eq!(common, Ok(Some('B')));
    }

    #[test]
    fn test_closest_common_node_tricky_ancestor_with_bad_heuristic() {
        // Find the clostest common ancestor between D and A. It should be D because
        // it's even an ancestor of A, but we currently find F when using a bad
        // heuristic that reaches F sooner.
        //
        //  A
        //  |\
        //  B |
        //  | |
        //  C E
        //  | |
        //  D |
        //  |/
        //  F

        let neighbors = hashmap! {
            'F' => vec![],
            'E' => vec!['F'],
            'D' => vec!['F'],
            'C' => vec!['D'],
            'B' => vec!['C'],
            'A' => vec!['B', 'E'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let common = closest_common_node(vec!['D'], vec!['A'], id_fn, neighbors_fn).block_on();
        assert_eq!(common, Ok(Some('F')));
    }

    #[test]
    fn test_closest_common_node() {
        let neighbors = hashmap! {
            'A' => Err('Y'),
            'B' => Ok(vec!['A']),
            'C' => Ok(vec!['A']),
            'D' => Err('X'),
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| neighbors[node].clone();

        let result = closest_common_node(['B'], ['C'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Ok(Some('A')));
        let result = closest_common_node(['C'], ['D'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Err('X'));
    }

    #[test]
    fn test_closest_common_node_simple_with_bad_heuristic() {
        // We still find the right common ancestor between C and A when given a bad
        // heuristic.
        //
        //  C A
        //  |/
        //  B
        //  |
        //  D

        let neighbors = hashmap! {
            'D' => vec![],
            'B' => vec!['D'],
            'C' => vec!['B'],
            'A' => vec!['B'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let common = closest_common_node(vec!['C'], vec!['A'], id_fn, neighbors_fn).block_on();
        assert_eq!(common, Ok(Some('B')));
    }

    #[test]
    fn test_closest_common_node_ancestor_with_bad_heuristic() {
        // We still find the right common ancestor between A and B when given a bad
        // heuristic.
        //
        //  A
        //  |
        //  C
        //  |
        //  B
        //  |
        //  D

        let neighbors = hashmap! {
            'D' => vec![],
            'B' => vec!['D'],
            'C' => vec!['B'],
            'A' => vec!['C'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let common = closest_common_node(vec!['A'], vec!['B'], id_fn, neighbors_fn).block_on();
        assert_eq!(common, Ok(Some('B')));
    }

    #[test]
    fn test_heads_mixed() {
        // Test the uppercase letters are in the start set
        //
        //  D F
        //  |/|
        //  C e
        //  |/
        //  b
        //  |
        //  A

        let neighbors = hashmap! {
            'A' => vec![],
            'b' => vec!['A'],
            'C' => vec!['b'],
            'D' => vec!['C'],
            'e' => vec!['b'],
            'F' => vec!['C', 'e'],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| Ok::<_, char>(neighbors[node].clone());

        let actual = heads(vec!['A', 'C', 'D', 'F'], id_fn, neighbors_fn).block_on();
        assert_eq!(actual, Ok(hashset!['D', 'F']));

        // Check with a different order in the start set
        let actual = heads(vec!['F', 'D', 'C', 'A'], id_fn, neighbors_fn).block_on();
        assert_eq!(actual, Ok(hashset!['D', 'F']));
    }

    #[test]
    fn test_heads() {
        let neighbors = hashmap! {
            'A' => Ok(vec![]),
            'B' => Err('X'),
            'C' => Ok(vec!['B']),
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = async |node: &char| neighbors[node].clone();

        let result = heads(['C'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Ok(hashset! {'C'}));
        let result = heads(['B'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Ok(hashset! {'B'}));
        let result = heads(['A'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Ok(hashset! {'A'}));
        let result = heads(['C', 'B'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Err('X'));
        let result = heads(['C', 'A'], id_fn, neighbors_fn).block_on();
        assert_eq!(result, Err('X'));
    }
}
