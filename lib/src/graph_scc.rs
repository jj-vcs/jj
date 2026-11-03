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

//! Algorithms for strongly connected components of directed graphs.
//!
//! * <https://en.wikipedia.org/wiki/Strongly_connected_component>

use std::collections::HashMap;
use std::hash::Hash;

/// Finds the strongly connected components reachable from `nodes`.
///
/// `neighbors` returns neighbor nodes, which need not be included in `nodes`.
///
/// Components are returned in topological order. The order within a component
/// is unspecified.
///
/// This uses Kosaraju's algorithm:
///
/// * <https://en.wikipedia.org/wiki/Kosaraju's_algorithm>
/// * <https://www.cis.upenn.edu/~cis1210/current/lectures/notes.pdf>
pub fn strongly_connected_components<T, II, NI>(
    nodes: II,
    mut neighbors: impl FnMut(&T) -> NI,
) -> Vec<Vec<T>>
where
    T: Clone + Eq + Hash,
    II: IntoIterator<Item = T>,
    NI: IntoIterator<Item = T>,
{
    // Build dense maps from nodes and neighbors
    let mut node_indices = HashMap::new();
    let mut node_values = Vec::new();
    for node in nodes {
        node_indices.entry(node).or_insert_with_key(|node| {
            let index = node_values.len();
            node_values.push(node.clone());
            index
        });
    }
    let mut forward_edges: Vec<Vec<usize>> = Vec::with_capacity(node_values.len());
    let mut reverse_edges: Vec<Vec<usize>> = vec![vec![]; node_values.len()];
    let mut node_index = 0;
    while node_index < node_values.len() {
        let mut neighbor_indices = Vec::new();
        for neighbor in neighbors(&node_values[node_index]) {
            let neighbor_index = *node_indices.entry(neighbor).or_insert_with_key(|neighbor| {
                // New neighbor node found
                let index = node_values.len();
                node_values.push(neighbor.clone());
                reverse_edges.push(vec![]);
                index
            });
            neighbor_indices.push(neighbor_index);
            reverse_edges[neighbor_index].push(node_index);
        }
        forward_edges.push(neighbor_indices);
        node_index += 1;
    }

    // For each node of the graph do Visit(node):
    //   If node is unvisited:
    //     1. Mark node as visited.
    //     2. For each out-neighbour next of node, do Visit(next).
    //     3. Prepend node to postorder. (Append here and iterate in reverse)
    let mut visited = vec![false; node_values.len()];
    let mut postorder = Vec::with_capacity(node_values.len());
    for start in 0..node_values.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((node, i)) = stack.last_mut() {
            if let Some(&next) = forward_edges[*node].get(*i) {
                *i += 1;
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, 0));
                }
            } else {
                postorder.push(*node);
                stack.pop();
            }
        }
    }

    // For each node of postorder, do Assign(node, root=node):
    //   If node has not been assigned to a component then:
    //     1. Assign node as belonging to the component of root.
    //     2. For each in-neighbour next of node, do Assign(next, root).
    visited.fill(false);
    let mut components = Vec::new();
    for &start in postorder.iter().rev() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(node) = stack.pop() {
            component.push(node_values[node].clone());
            for &next in &reverse_edges[node] {
                if !visited[next] {
                    visited[next] = true;
                    stack.push(next);
                }
            }
        }
        components.push(component);
    }

    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strongly_connected_components() {
        let edges: &[&[usize]] = &[
            // 0 <-> 1 -> 2 <-> 3 -> 4
            &[1],
            &[0, 2],
            &[3],
            &[2, 4],
            &[],
            // 5 is isolated
            &[],
            // 6 has a self-loop
            &[6],
        ];
        let mut components =
            strongly_connected_components([0, 5, 6, 0], |&n| edges[n].iter().copied());
        for c in &mut components {
            c.sort_unstable();
        }
        let id = |node| components.iter().position(|c| c.contains(&node)).unwrap();
        assert_eq!(components.len(), 5);
        assert_eq!(components[id(0)], [0, 1]);
        assert_eq!(components[id(1)], [0, 1]);
        assert_eq!(components[id(2)], [2, 3]);
        assert_eq!(components[id(3)], [2, 3]);
        assert_eq!(components[id(4)], [4]);
        assert_eq!(components[id(5)], [5]);
        assert_eq!(components[id(6)], [6]);
        assert!(id(0) < id(2));
        assert!(id(2) < id(4));
    }

    #[test]
    fn test_strongly_connected_components_empty_graph() {
        let components = strongly_connected_components([0; 0], |_| [0; 0]);
        assert!(components.is_empty());
    }

    #[test]
    fn test_strongly_connected_components_deep_graphs() {
        // 0 -> 1 -> 2 -> ... -> 20_000
        let components = strongly_connected_components([0], |&n| (n < 20_000).then_some(n + 1));
        assert_eq!(components.len(), 20_001);
        assert_eq!(components.first().unwrap(), &[0]);
        assert_eq!(components.last().unwrap(), &[20_000]);
    }

    #[test]
    fn test_strongly_connected_components_converging_paths() {
        //  /-> 1 -\
        // 0        -> 3
        //  \-> 2 -/
        // The middle nodes can appear in either order.
        let edges: &[&[usize]] = &[&[1, 2], &[3], &[3], &[]];
        let components = strongly_connected_components([0], |&n| edges[n].iter().copied());
        assert_eq!(components.len(), 4);
        assert!(components.iter().all(|c| c.len() == 1));
        let id = |node| components.iter().position(|c| c.contains(&node)).unwrap();
        assert!(id(0) < id(1));
        assert!(id(0) < id(2));
        assert!(id(1) < id(3));
        assert!(id(2) < id(3));
    }

    #[test]
    fn test_strongly_connected_components_unreachable_cycle() {
        let edges: &[&[usize]] = &[
            // 0 -> 1 -> 2 -> 0
            &[1],
            &[2],
            &[0],
            // 3 <-> 4 (unreachable)
            &[4],
            &[3],
        ];
        let mut components =
            strongly_connected_components([0, 0, 1], |&n| edges[n].iter().copied());
        for c in &mut components {
            c.sort_unstable();
        }
        assert_eq!(components.len(), 1);
        assert_eq!(components[0], [0, 1, 2]);
    }

    #[test]
    fn test_strongly_connected_components_repeated_neighbors() {
        // 0 -> 1 -> 0 -> 2, with repeated edges.
        let edges: &[&[usize]] = &[&[1, 1, 2, 2], &[0, 0], &[]];
        let mut components = strongly_connected_components([0], |&n| edges[n].iter().copied());
        assert_eq!(components.len(), 2);
        components[0].sort_unstable();
        assert_eq!(components[0], [0, 1]);
        assert_eq!(components[1], [2]);
    }
}
