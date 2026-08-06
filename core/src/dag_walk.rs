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

//! General-purpose DAG algorithms.

use std::collections::HashSet;
use std::convert::Infallible;
use std::hash::Hash;
use std::iter;

use itertools::Itertools as _;

/// Traverses nodes from `start` in depth-first order.
pub fn dfs<T, ID, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
) -> impl Iterator<Item = T>
where
    ID: Hash + Eq,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let neighbors_fn = move |node: &T| to_infallible_iter(neighbors_fn(node));
    dfs_ok(to_infallible_iter(start), id_fn, neighbors_fn).map(|Ok(node)| node)
}

/// Traverses nodes from `start` in depth-first order.
///
/// An `Err` is emitted as a node with no neighbors. Caller may decide to
/// short-circuit on it.
pub fn dfs_ok<T, ID, E, II, NI>(
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

/// Builds a list of nodes reachable from the `start` where neighbors come
/// before the node itself.
///
/// If the graph has cycle, `cycle_fn()` is called with one of the nodes
/// involved in the cycle.
pub fn topo_order_forward<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let neighbors_fn = move |node: &T| to_ok_iter(neighbors_fn(node));
    topo_order_forward_ok(to_ok_iter(start), id_fn, neighbors_fn, cycle_fn)
}

/// Builds a list of `Ok` nodes reachable from the `start` where neighbors come
/// before the node itself.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error. If the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub fn topo_order_forward_ok<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
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
            let neighbors_iter = neighbors_fn(&node).into_iter();
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

/// Builds a list of nodes reachable from the `start` where neighbors come after
/// the node itself.
///
/// If the graph has cycle, `cycle_fn()` is called with one of the nodes
/// involved in the cycle.
pub fn topo_order_reverse<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let neighbors_fn = move |node: &T| to_ok_iter(neighbors_fn(node));
    topo_order_reverse_ok(to_ok_iter(start), id_fn, neighbors_fn, cycle_fn)
}

/// Builds a list of `Ok` nodes reachable from the `start` where neighbors come
/// after the node itself.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error. If the graph has cycle, `cycle_fn()` is called with one
/// of the nodes involved in the cycle.
pub fn topo_order_reverse_ok<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    neighbors_fn: impl FnMut(&T) -> NI,
    cycle_fn: impl FnOnce(T) -> E,
) -> Result<Vec<T>, E>
where
    ID: Hash + Eq + Clone,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut result = topo_order_forward_ok(start, id_fn, neighbors_fn, cycle_fn)?;
    result.reverse();
    Ok(result)
}

/// Find nodes in the start set that are not reachable from other nodes in the
/// start set.
pub fn heads<T, ID, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
) -> HashSet<T>
where
    T: Hash + Eq + Clone,
    ID: Hash + Eq,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    let neighbors_fn = move |node: &T| to_infallible_iter(neighbors_fn(node));
    let Ok(node) = heads_ok(to_infallible_iter(start), id_fn, neighbors_fn);
    node
}

/// Finds `Ok` nodes in the start set that are not reachable from other nodes in
/// the start set.
///
/// If `start` or `neighbors_fn()` yields an `Err`, this function terminates and
/// returns the error.
pub fn heads_ok<T, ID, E, II, NI>(
    start: II,
    id_fn: impl Fn(&T) -> ID,
    mut neighbors_fn: impl FnMut(&T) -> NI,
) -> Result<HashSet<T>, E>
where
    T: Hash + Eq + Clone,
    ID: Hash + Eq,
    II: IntoIterator<Item = Result<T, E>>,
    NI: IntoIterator<Item = Result<T, E>>,
{
    let mut heads: HashSet<T> = start.into_iter().try_collect()?;
    // Do a BFS until we have only one item left in the frontier. That frontier must
    // have originated from one of the heads, and since there can't be cycles,
    // it won't be able to eliminate any other heads.
    let mut frontier: Vec<T> = heads.iter().cloned().collect();
    let mut visited: HashSet<ID> = heads.iter().map(&id_fn).collect();
    let mut root_reached = false;
    while frontier.len() > 1 || (!frontier.is_empty() && root_reached) {
        frontier = frontier
            .iter()
            .flat_map(|node| {
                let neighbors = neighbors_fn(node).into_iter().collect_vec();
                if neighbors.is_empty() {
                    root_reached = true;
                }
                neighbors
            })
            .try_collect()?;
        for node in &frontier {
            heads.remove(node);
        }
        frontier.retain(|node| visited.insert(id_fn(node)));
    }
    Ok(heads)
}

fn to_ok_iter<T, E>(iter: impl IntoIterator<Item = T>) -> impl Iterator<Item = Result<T, E>> {
    iter.into_iter().map(Ok)
}

fn to_infallible_iter<T>(
    iter: impl IntoIterator<Item = T>,
) -> impl Iterator<Item = Result<T, Infallible>> {
    to_ok_iter(iter)
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use maplit::hashmap;
    use maplit::hashset;

    use super::*;

    #[test]
    fn test_dfs_ok() {
        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec![Ok('A'), Err('X')],
            'C' => vec![Ok('B')],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = |node: &char| neighbors[node].clone();

        // Self and neighbor nodes shouldn't be lost at the error.
        let nodes = dfs_ok([Ok('C')], id_fn, neighbors_fn).collect_vec();
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['C'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse(vec!['C', 'B'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['C', 'B', 'A']);
        let common = topo_order_reverse(vec!['B', 'C'], id_fn, neighbors_fn, cycle_fn).unwrap();
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['F'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['F', 'E', 'D', 'C', 'B', 'A']);
        let common =
            topo_order_reverse(vec!['F', 'E', 'C'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['F', 'D', 'E', 'C', 'B', 'A']);
        let common =
            topo_order_reverse(vec!['F', 'D', 'E'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['F', 'D', 'C', 'B', 'E', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['I'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['I', 'D', 'B', 'H', 'F', 'G', 'E', 'C', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['I'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['I', 'h', 'G', 'e', 'C', 'f', 'D', 'b', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['E'], id_fn, neighbors_fn, cycle_fn).unwrap();
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['G'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['G', 'E', 'F', 'D', 'C', 'B', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['G'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['G', 'F', 'E', 'D', 'c', 'B', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['G'], id_fn, neighbors_fn, cycle_fn).unwrap();
        assert_eq!(common, vec!['G', 'f', 'e', 'd', 'C', 'B', 'A']);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['F', 'C'], id_fn, neighbors_fn, cycle_fn).unwrap();
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let common = topo_order_reverse(vec!['D'], id_fn, neighbors_fn, cycle_fn).unwrap();
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let result: Result<Vec<_>, _> =
            topo_order_reverse(vec!['C'], id_fn, neighbors_fn, cycle_fn);
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
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        let result = topo_order_reverse(vec!['D'], id_fn, neighbors_fn, cycle_fn);
        assert_matches!(result, Err('C' | 'B' | 'A'));
    }

    #[test]
    fn test_topo_order_ok() {
        let neighbors = hashmap! {
            'A' => vec![Err('Y')],
            'B' => vec![Ok('A'), Err('X')],
            'C' => vec![Ok('B')],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = |node: &char| neighbors[node].clone();
        let cycle_fn = |id| id;

        // Terminates at Err('X') no matter if the sorting order is forward or
        // reverse. The visiting order matters.
        let result = topo_order_forward_ok([Ok('C')], id_fn, neighbors_fn, cycle_fn);
        assert_eq!(result, Err('X'));
        let result = topo_order_reverse_ok([Ok('C')], id_fn, neighbors_fn, cycle_fn);
        assert_eq!(result, Err('X'));
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

        let actual = heads(
            vec!['A', 'C', 'D', 'F'],
            |node| *node,
            |node| neighbors[node].clone(),
        );
        assert_eq!(actual, hashset!['D', 'F']);

        // Check with a different order in the start set
        let actual = heads(
            vec!['F', 'D', 'C', 'A'],
            |node| *node,
            |node| neighbors[node].clone(),
        );
        assert_eq!(actual, hashset!['D', 'F']);
    }

    #[test]
    fn test_heads_ok() {
        let neighbors = hashmap! {
            'A' => vec![],
            'B' => vec![Ok('A'), Err('X')],
            'C' => vec![Ok('B')],
        };
        let id_fn = |node: &char| *node;
        let neighbors_fn = |node: &char| neighbors[node].clone();

        let result = heads_ok([Ok('C')], id_fn, neighbors_fn);
        assert_eq!(result, Ok(hashset! {'C'}));
        let result = heads_ok([Ok('B')], id_fn, neighbors_fn);
        assert_eq!(result, Ok(hashset! {'B'}));
        let result = heads_ok([Ok('A')], id_fn, neighbors_fn);
        assert_eq!(result, Ok(hashset! {'A'}));
        let result = heads_ok([Ok('C'), Ok('B')], id_fn, neighbors_fn);
        assert_eq!(result, Err('X'));
        let result = heads_ok([Ok('C'), Ok('A')], id_fn, neighbors_fn);
        assert_eq!(result, Err('X'));
    }
}
