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
/// Note: multi-edges cannot be represented by this data structure.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct SimpleDirectedGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
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
    N: Clone + Eq + Hash + PartialEq,
{
    /// Constructs a new SimpleDirectedGraph from an adjacency map.
    /// Note: if necessary, the input map is canonicalized, preserving iteration
    /// order.
    pub fn new(mut adj: IndexMap<N, IndexSet<N>>) -> Self {
        let mut missing_nodes = IndexSet::new();
        for (_, children) in &adj {
            for child in children {
                if !adj.contains_key(child) {
                    missing_nodes.insert(child.clone());
                }
            }
        }
        for node in missing_nodes {
            adj.entry(node).or_default();
        }
        Self { adj }
    }

    /// Returns the nodes in this graph.
    pub fn nodes(&self) -> impl Iterator<Item = &N> {
        self.adj.keys()
    }

    /// Returns the edges in this graph.
    pub fn edges(&self) -> impl Iterator<Item = (&N, &N)> {
        self.adj
            .iter()
            .flat_map(|(parent, adj_set)| adj_set.iter().map(move |child| (parent, child)))
    }

    /// Returns the adjacent nodes for the given node, or None if the node is
    /// not in the graph.
    pub fn adjacent_nodes(&self, node: &N) -> Option<impl Iterator<Item = &N>> {
        self.adj.get(node).map(|adj_set| adj_set.iter())
    }

    /// Returns true if this graph contains the given node.
    pub fn contains_node(&self, node: &N) -> bool {
        self.adj.contains_key(node)
    }

    /// Constructs a new graph from a list of edges. Iteration order is
    /// preserved from the input. Multi-edges in the input are removed.
    pub fn from_edge_list<EI>(edges: EI) -> Self
    where
        EI: IntoIterator<Item = (N, N)>,
    {
        let mut nodes: IndexSet<N> = IndexSet::new();
        let mut adj: IndexMap<N, IndexSet<N>> = IndexMap::new();
        for (parent, child) in edges {
            let values = match adj.entry(parent.clone()) {
                indexmap::map::Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
                indexmap::map::Entry::Vacant(vacant_entry) => {
                    nodes.insert(parent.clone());
                    vacant_entry.insert(IndexSet::new())
                }
            };
            if values.insert(child.clone()) {
                nodes.insert(child.clone());
            }
        }
        if nodes.len() > adj.len() {
            // Some nodes only appear as children, so we need to add them to the adj map
            // with empty adjacency sets.
            for node in nodes {
                adj.entry(node.clone()).or_default();
            }
        }
        Self { adj }
    }

    /// Returns a new graph with the same nodes as this graph and all edges
    /// reversed.
    pub fn reverse(&self) -> Self {
        let mut rev_adj: IndexMap<N, IndexSet<N>> = IndexMap::new();
        for (parent, children) in &self.adj {
            // Ensure parent is in rev_adj even if it has no children.
            rev_adj.entry(parent.clone()).or_default();
            for child in children {
                rev_adj
                    .entry(child.clone())
                    .or_default()
                    .insert(parent.clone());
            }
        }
        Self { adj: rev_adj }
    }
}

#[cfg(test)]
mod tests {
    use indexmap::indexmap;
    use indexmap::indexset;

    use super::*;

    #[test]
    fn test_simple_directed_graph_new() {
        let adj = indexmap! {
            "A" => indexset! {"B"},
            "B" => indexset!{},
        };
        let graph = SimpleDirectedGraph::new(adj.clone());
        assert_eq!(graph.adj, adj);

        // adj does not have entries for "B" or "D".
        let adj = indexmap! {
            "A" => indexset! {"B", "C", "D"},
            "C" => indexset!{},
        };
        let graph = SimpleDirectedGraph::new(adj);
        assert_eq!(
            graph.adj,
            indexmap! {
                "A" => indexset! {"B", "C", "D"},
                "C" => indexset!{},
                "B" => indexset!{},
                "D" => indexset!{},
            }
        );
    }

    #[test]
    fn test_simple_directed_graph_nodes() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![("A", "B"), ("B", "C")]);
        let nodes: Vec<_> = graph.nodes().copied().collect();
        assert_eq!(nodes, vec!["A", "B", "C"]);

        let graph = SimpleDirectedGraph::<String>::from_edge_list(vec![]);
        let nodes: Vec<_> = graph.nodes().cloned().collect();
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_simple_directed_graph_edges() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![("A", "B"), ("B", "C"), ("A", "C")]);
        let edges: Vec<_> = graph.edges().map(|(&u, &v)| (u, v)).collect();
        assert_eq!(edges, vec![("A", "B"), ("A", "C"), ("B", "C")]);

        let graph = SimpleDirectedGraph::<String>::from_edge_list(vec![]);
        let edges: Vec<_> = graph.edges().collect();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_simple_directed_graph_adjacent_nodes() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![("A", "B"), ("A", "C"), ("B", "D")]);
        assert_eq!(
            graph
                .adjacent_nodes(&"A")
                .unwrap()
                .copied()
                .collect::<Vec<_>>(),
            vec!["B", "C"]
        );
        assert_eq!(
            graph
                .adjacent_nodes(&"B")
                .unwrap()
                .copied()
                .collect::<Vec<_>>(),
            vec!["D"]
        );
        assert!(graph.adjacent_nodes(&"C").unwrap().next().is_none());
        assert!(graph.adjacent_nodes(&"Z").is_none());
    }

    #[test]
    fn test_simple_directed_graph_contains_node() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![("A", "B"), ("B", "C")]);
        assert!(graph.contains_node(&"A"));
        assert!(graph.contains_node(&"B"));
        assert!(graph.contains_node(&"C"));
        assert!(!graph.contains_node(&"D"));
    }

    #[test]
    fn test_simple_directed_graph_from_edge_list() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![
            ("A", "B"),
            ("A", "C"),
            ("B", "C"),
            ("A", "B"),
        ]);
        let nodes: Vec<_> = graph.nodes().copied().collect();
        assert_eq!(nodes, vec!["A", "B", "C"]);
        let edges: Vec<_> = graph.edges().map(|(&u, &v)| (u, v)).collect();
        assert_eq!(edges, vec![("A", "B"), ("A", "C"), ("B", "C")]);

        let graph = SimpleDirectedGraph::from_edge_list(vec![("B", "C"), ("A", "B")]);
        let nodes: Vec<_> = graph.nodes().copied().collect();
        assert_eq!(nodes, vec!["B", "A", "C"]);
        let edges: Vec<_> = graph.edges().map(|(&u, &v)| (u, v)).collect();
        assert_eq!(edges, vec![("B", "C"), ("A", "B")]);
    }
}
