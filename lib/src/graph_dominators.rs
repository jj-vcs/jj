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

//! Generic implementation of the "closest common dominator" algorithm for
//! directed graphs.

use std::hash::Hash;

use indexmap::IndexMap;
use indexmap::IndexSet;

/// An immutable directed graph with nodes of type N and a minimal interface for
/// iterating over nodes and their adjacent nodes.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct SimpleDirectedGraph<N>
where
    N: Clone + Eq + Hash,
{
    /// The adjacency map of the graph. Each key is a node, and the
    /// corresponding value is the set of adjacent nodes (i.e., the children of
    /// the key node). The adjacency map is in canonical form: for every
    /// u->v edge, there is an entry in adj with key v (even if v has no
    /// outgoing edges).
    adj: IndexMap<N, IndexSet<N>>,
}

impl<N> SimpleDirectedGraph<N>
where
    N: Clone + Eq + Hash,
{
    /// Constructs a new SimpleDirectedGraph from a list of edges.
    pub fn new<EI>(edges: EI) -> Self
    where
        EI: IntoIterator<Item = (N, N)>,
    {
        let mut adj: IndexMap<N, IndexSet<N>> = IndexMap::new();
        for (parent, child) in edges {
            adj.entry(parent).or_default().insert(child.clone());
            adj.entry(child).or_default();
        }
        Self { adj }
    }

    /// Returns the nodes in this graph.
    pub fn nodes(&self) -> impl Iterator<Item = &N> {
        self.adj.keys()
    }

    /// Returns the nodes in this graph.
    pub fn num_nodes(&self) -> usize {
        self.adj.len()
    }

    /// Returns the edges in this graph.
    pub fn edges(&self) -> impl Iterator<Item = (&N, &N)> {
        self.adj
            .iter()
            .flat_map(|(parent, adj_set)| adj_set.iter().map(move |child| (parent, child)))
    }

    /// Returns the adjacent nodes for the given node, or None if the node is
    /// not in the graph.
    pub fn adjacent_nodes(&self, node: &N) -> Option<impl DoubleEndedIterator<Item = &N>> {
        self.adj.get(node).map(|adj_set| adj_set.iter())
    }

    /// Returns true if this graph contains the given node.
    pub fn contains_node(&self, node: &N) -> bool {
        self.adj.contains_key(node)
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools as _;

    use super::*;

    #[test]
    fn test_simple_directed_graph_nodes() {
        let graph = SimpleDirectedGraph::new([("A", "B"), ("B", "C")]);
        let nodes = graph.nodes().copied().collect_vec();
        assert_eq!(nodes, ["A", "B", "C"]);

        let graph = SimpleDirectedGraph::<String>::new([]);
        let nodes = graph.nodes().cloned().collect_vec();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_simple_directed_graph_edges() {
        let graph = SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("A", "C")]);
        let edges = graph.edges().map(|(&u, &v)| (u, v)).collect_vec();
        assert_eq!(edges, [("A", "B"), ("A", "C"), ("B", "C")]);

        let graph = SimpleDirectedGraph::<String>::new([]);
        let edges = graph.edges().collect_vec();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_simple_directed_graph_adjacent_nodes() {
        let graph = SimpleDirectedGraph::new([("A", "B"), ("A", "C"), ("B", "D")]);
        assert_eq!(
            graph.adjacent_nodes(&"A").unwrap().copied().collect_vec(),
            ["B", "C"]
        );
        assert_eq!(
            graph.adjacent_nodes(&"B").unwrap().copied().collect_vec(),
            ["D"]
        );
        assert!(graph.adjacent_nodes(&"C").unwrap().next().is_none());
        assert!(graph.adjacent_nodes(&"Z").is_none());
    }

    #[test]
    fn test_simple_directed_graph_contains_node() {
        let graph = SimpleDirectedGraph::new([("A", "B"), ("B", "C")]);
        assert!(graph.contains_node(&"A"));
        assert!(graph.contains_node(&"B"));
        assert!(graph.contains_node(&"C"));
        assert!(!graph.contains_node(&"D"));
    }

    #[test]
    fn test_simple_directed_graph_new() {
        let graph = SimpleDirectedGraph::new([("A", "B"), ("A", "C"), ("B", "C"), ("A", "B")]);
        let nodes = graph.nodes().copied().collect_vec();
        assert_eq!(nodes, ["A", "B", "C"]);
        let edges = graph.edges().map(|(&u, &v)| (u, v)).collect_vec();
        assert_eq!(edges, [("A", "B"), ("A", "C"), ("B", "C")]);

        let graph = SimpleDirectedGraph::new([("B", "C"), ("A", "B")]);
        let nodes = graph.nodes().copied().collect_vec();
        assert_eq!(nodes, ["B", "C", "A"]);
        let edges = graph.edges().map(|(&u, &v)| (u, v)).collect_vec();
        assert_eq!(edges, [("B", "C"), ("A", "B")]);
    }
}
