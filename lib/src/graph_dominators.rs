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
//!
//! Generic implementation of the Common Dominator algorithm for directed
//! graphs, using the Cooper-Harvey-Kennedy iterative algorithm. Loosely
//! speaking the algorithm finds the "choke point" for a set of nodes S in a
//! directed graph (going from the "entry" node to nodes in S), closest to S.
//!
//! Dominance:
//!
//! * A flow graph is a directed graph with a designated entry node.
//! * A node z is said to dominate a node n if all paths from the entry node to
//!   n must go through z. Every node dominates itself, and the entry node
//!   dominates all nodes.
//! * A node can have one or more dominators.
//! * A node z strictly dominates n if z dominates n and z != n.
//! * The immediate dominator of a node n is the dominator of n that doesn't
//!   strictly dominate any other strict dominators of n. Informally it is the
//!   "closest" choke point on all paths from the entry node to n.
//! * Let S be a subset of the nodes in the graph. The intersection of the
//!   dominators of each node in S is the set of common dominators of S.
//! * The closest common dominator of S is the common dominator of S that
//!   doesn't strictly dominate any other common dominator of S. Informally, it
//!   is the choke point closest to S such that all paths from the entry node to
//!   S must go through it.
//!
//! Dominator Tree:
//!
//! For any flow graph G there is a corresponding dominator tree defined as
//! follows:
//! * The nodes of the dominator tree are the same as the nodes of G
//! * The root of the dominator tree is the entry node of G
//! * In the dominator tree, the children of a node are the nodes it immediately
//!   dominates
//!
//! The closest common dominator of S is the Lowest Common Ancestor (LCA)
//! of S in the graph's dominator tree.
//!
//! This implementation constructs the Dominator Tree by first determining
//! the Immediate Dominator (ipdom) for every node (using the standard iterative
//! algorithm), and then calculating the LCA for the set S. See:
//!
//! * <http://www.hipersoft.rice.edu/grads/publications/dom14.pdf>
//! * <https://en.wikipedia.org/wiki/Dominator_(graph_theory)>
//!
//! The running time is O(V+E+|S|*V)in the worst case, the space complexity is
//! O(V+E), where V is the number of nodes and E is the number of edges.
//!
//! For a DAG, the expensive iterative Dominator step converges in just two
//! passes, making the tree construction effectively linear, O(V+E). The primary
//! bottleneck becomes the naive "Lowest Common Ancestor" (LCA) lookup, which
//! scales with the size of the set and the depth of the tree. If you need
//! better running time for large graphs, you can optimize the LCA step.

use std::collections::HashMap;
use std::collections::HashSet;
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

    /// Returns a postorder traversal of the nodes in this graph starting from
    /// the given node.
    pub fn get_postorder<'a>(&'a self, start_node: &'a N) -> Vec<&'a N> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        self.dfs(start_node, &mut visited, &mut order);
        order
    }

    fn dfs<'a>(&'a self, u: &'a N, visited: &mut HashSet<&'a N>, order: &mut Vec<&'a N>) {
        visited.insert(u);
        if let Some(adj_set) = self.adj.get(u) {
            for v in adj_set {
                if !visited.contains(&v) {
                    self.dfs(v, visited, order);
                }
            }
        }
        order.push(u);
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

/// A FlowGraph is a directed graph with a designated start node.
///
/// Any node in the graph can be the start node. There are no reachability
/// requirements whatsoever: some nodes may be unreachable from the start node,
/// the start node could have incoming edges, the graph could be disconnected,
/// etc.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct FlowGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
{
    /// The graph.
    pub graph: SimpleDirectedGraph<N>,
    /// The start node.
    pub start_node: N,
}

/// Type alias for clarity.
type Index = usize;

impl<N> FlowGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
{
    /// Constructs a new FlowGraph.
    pub fn new(graph: SimpleDirectedGraph<N>, start_node: N) -> Self {
        Self { graph, start_node }
    }
}

// Below is the implementation of the closest common dominator algorithm for
// FlowGraphs.
impl<N> FlowGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
{
    /// Finds the closest common dominator for the given flow graph and set of
    /// nodes S (target_set).
    pub fn find_closest_common_dominator<NI>(&self, target_set: NI) -> Option<N>
    where
        NI: IntoIterator<Item = N>,
    {
        // 1. Get postorder traversal of the graph starting from the start node.
        let postorder = self.graph.get_postorder(&self.start_node);

        // 2. Map generic types to integer IDs
        let (node_to_id, id_to_node) = {
            let mut node_to_id = HashMap::new();
            let mut id_to_node = Vec::new();
            for (index, &node) in postorder.iter().enumerate() {
                id_to_node.push(node.clone());
                node_to_id.insert(node.clone(), index);
            }
            (node_to_id, id_to_node)
        };

        // 3. Convert generic target_set to internal IDs
        let target_ids: Option<Vec<Index>> = target_set
            .into_iter()
            .map(|node| node_to_id.get(&node).copied())
            .collect();
        if target_ids.is_none() || target_ids.as_ref().unwrap().is_empty() {
            return None;
        }
        let target_ids = target_ids.unwrap();

        // 4. Build Graph using internal IDs.
        let num_nodes = node_to_id.len();
        let start_node_id = num_nodes - 1;
        // start_node_is is always num_nodes-1 because of the way we construct the
        // postorder.
        assert_eq!(&start_node_id, node_to_id.get(&self.start_node).unwrap());

        let mut adj = vec![vec![]; num_nodes];
        let mut rev_adj = vec![vec![]; num_nodes];
        for (u, v) in self.graph.edges() {
            if let (Some(u_idx), Some(v_idx)) = (node_to_id.get(u), node_to_id.get(v)) {
                adj[*u_idx].push(*v_idx);
                rev_adj[*v_idx].push(*u_idx);
            }
        }

        // 5: Find the immediate dominators for each node using the
        // Cooper-Harvey-Kennedy iterative algorithm.
        let idom = Self::get_immediate_dominators(&adj, &rev_adj);

        // 6: Find LCA in Dominator Tree
        let mut current_lca = target_ids[0];
        for &node in target_ids.iter().skip(1) {
            current_lca = Self::find_lca(current_lca, node, &idom, start_node_id);
        }

        // 7: Map internal ID back to generic type T
        Some(id_to_node[current_lca].clone())
    }

    fn get_immediate_dominators(adj: &[Vec<Index>], rev_adj: &[Vec<Index>]) -> Vec<Option<Index>> {
        // Step 1: Compute Dominators on Reverse Graph
        let num_nodes = adj.len();
        let start_node_id = num_nodes - 1;

        // idom is the immediate dominator for each node.
        let mut idom: Vec<Option<Index>> = vec![None; num_nodes];
        idom[start_node_id] = Some(start_node_id);

        loop {
            let mut changed = false;
            // Iterate in reverse postorder, skipping the start node.
            for u in (0..start_node_id).rev() {
                let mut new_idom = None;

                // Process predecessors (nodes that flow INTO u).
                let preds = &rev_adj[u];

                // Find first processed predecessor.
                for &p in preds {
                    if idom[p].is_some() {
                        new_idom = Some(p);
                        break;
                    }
                }

                if let Some(mut candidate) = new_idom {
                    for &p in preds {
                        if p != candidate && idom[p].is_some() {
                            candidate = Self::intersect(candidate, p, &idom);
                        }
                    }

                    if idom[u] != Some(candidate) {
                        idom[u] = Some(candidate);
                        changed = true;
                    }
                }
            }

            if !changed {
                break;
            }
        }

        idom
    }

    fn intersect(mut b1: Index, mut b2: Index, idom: &[Option<Index>]) -> Index {
        while b1 != b2 {
            while b1 < b2 {
                b1 = idom[b1].unwrap();
            }
            while b2 < b1 {
                b2 = idom[b2].unwrap();
            }
        }
        b1
    }

    fn find_lca(u: Index, v: Index, idom: &[Option<Index>], root: Index) -> Index {
        let mut path_u = HashSet::new();
        let mut curr = u;
        loop {
            path_u.insert(curr);
            if curr == root {
                break;
            }
            match idom[curr] {
                Some(p) if p != curr => curr = p,
                _ => break,
            }
        }

        let mut curr = v;
        loop {
            if path_u.contains(&curr) {
                return curr;
            }
            if curr == root {
                break;
            }
            match idom[curr] {
                Some(p) if p != curr => curr = p,
                _ => break,
            }
        }
        root
    }

    /// Consumes this FlowGraph and returns the underlying graph and start node.
    pub fn consume(self) -> (SimpleDirectedGraph<N>, N) {
        (self.graph, self.start_node)
    }
}

#[cfg(test)]
mod tests {
    use indexmap::indexmap;
    use indexmap::indexset;

    use super::*;

    fn closest_common_dominator(
        edges: &[(&str, &str)],
        start_node: &str,
        target_set: Vec<&str>,
        expected: Option<&str>,
    ) {
        let graph = SimpleDirectedGraph::from_edge_list(
            edges.iter().map(|&(u, v)| (u.to_string(), v.to_string())),
        );
        let flow_graph = FlowGraph::new(graph, start_node.to_string());

        let target_set_string: Vec<_> = target_set.iter().map(|&n| n.to_string()).collect();
        let result = flow_graph.find_closest_common_dominator(target_set_string);
        assert_eq!(result, expected.map(|e| e.to_string()));
    }

    #[test]
    fn test_closest_common_dominator_split() {
        //   /-> B \
        // A        -> D
        //   \-> C /
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")];

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D"], Some("D"));

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "C", "D"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_linear_chain() {
        // A -> B -> C -> D
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D")];

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D"], Some("D"));

        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "D"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["A", "B", "C", "D"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_disjoint_no_common() {
        // A -> B
        // C -> D
        let edges = vec![("A", "B"), ("C", "D")];

        closest_common_dominator(&edges, "A", vec!["A", "C"], None);
        closest_common_dominator(&edges, "A", vec!["A", "D"], None);
        closest_common_dominator(&edges, "A", vec!["B", "D"], None);

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_classic_diamond() {
        //      /-> B -\
        //    A          -> D -> E
        //      \-> C -/
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D"), ("D", "E")];

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["D"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["D", "E"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["A", "D"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_basic_y_shape() {
        // A
        //  \
        //    --> C -> D
        //  /
        // B
        let edges = vec![("A", "C"), ("B", "C"), ("C", "D")];

        closest_common_dominator(&edges, "A", vec!["A", "B"], None);
        closest_common_dominator(&edges, "A", vec!["A", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("C"));

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], None);
        closest_common_dominator(&edges, "A", vec!["C"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D"], Some("D"));
    }

    #[test]
    fn test_closest_common_dominator_single_node() {
        // A
        let edges = vec![];
        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_complex_multi_source_multi_sink() {
        //       /-> E
        // A -> B
        //       \-> F
        //           ^
        //           |
        // C --> D --/
        let edges = vec![("A", "B"), ("B", "E"), ("B", "F"), ("C", "D"), ("D", "F")];

        closest_common_dominator(&edges, "A", vec!["E", "F"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["F"], Some("F"));
        closest_common_dominator(&edges, "A", vec!["B", "F"], Some("B"));
    }

    #[test]
    fn test_closest_common_dominator_simple_cycle_with_entry() {
        //
        // A -> B -> C -> D
        //      ^         |
        //      |         |
        //      \--------/
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("D", "B")];

        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "B", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "C", "D"], Some("B"));

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D"], Some("D"));
    }

    #[test]
    fn test_closest_common_dominator_figure_eight_with_bridge() {
        //
        //  A -> B -> C -> D -> E -> F -> G
        //       ^         |    ^         |
        //       |         |    |         |
        //        \_______/      \_______/
        let edges = vec![
            ("A", "B"), // entry
            ("B", "C"),
            ("C", "D"),
            ("D", "B"), // Loop 1
            ("D", "E"), // Bridge
            ("E", "F"),
            ("F", "G"),
            ("G", "E"), // Loop 2
        ];

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "E"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["C", "F"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D", "E"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["D", "F"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["E", "G"], Some("E"));
        closest_common_dominator(&edges, "A", vec!["F", "G"], Some("F"));
    }

    #[test]
    fn test_closest_common_dominator_figure_eight() {
        //
        //  A -> B -> C --> D   -> E -> F
        //       ^         | ^          |
        //       |         | |          |
        //        \_______/  \_________/
        let edges = vec![
            ("A", "B"), // entry
            ("B", "C"),
            ("C", "D"),
            ("D", "B"), // Loop 1
            ("D", "E"),
            ("E", "F"),
            ("F", "D"), // Loop 2
        ];

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["C", "E"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D", "E"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["D", "F"], Some("D"));
        closest_common_dominator(&edges, "A", vec!["E", "F"], Some("E"));
    }

    #[test]
    fn test_closest_common_dominator_entry_cycle_dominance() {
        // B -> C -> B (Loop)
        // A -> B
        let edges = vec![("A", "B"), ("B", "C"), ("C", "B")];

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C"], Some("C"));

        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["A", "B", "C"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_nested_loops() {
        //           /---> E
        //           |     |
        //           |     |
        // A -> B -> C <--/
        //      ^     \--> D
        //      |          |
        //      |----------|
        let edges = vec![
            ("A", "B"),
            ("B", "C"),
            ("C", "D"),
            ("C", "E"),
            ("E", "C"),
            ("D", "B"),
        ];

        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["A", "C"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["C", "E"], Some("C"));
        closest_common_dominator(&edges, "A", vec!["D", "E"], Some("C"));

        closest_common_dominator(&edges, "A", vec!["B", "C", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "C", "E"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "D", "E"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "D", "E"], Some("C"));

        closest_common_dominator(&edges, "A", vec!["B", "C", "D", "E"], Some("B"));
    }

    #[test]
    fn test_closest_common_dominator_tree() {
        // A -> B -> C
        // \     \-> D
        //  \------> E
        let edges = vec![("A", "B"), ("B", "C"), ("B", "D"), ("A", "E")];

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "E"], Some("A"));

        closest_common_dominator(&edges, "A", vec!["B", "C", "D"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["C", "D", "E"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_bypassing_path() {
        // A -> B -> C -> D
        // |              ^
        // v              |
        // E -------------/
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("A", "E"), ("E", "D")];

        closest_common_dominator(&edges, "A", vec!["B", "C"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["B", "D"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B", "E"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["C", "D"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["C", "E"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["D", "E"], Some("A"));

        closest_common_dominator(&edges, "A", vec!["B", "C", "D"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["C", "D", "E"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_infinite_loop_trap() {
        // A->B, C->D->C (Trap)
        let edges = vec![
            ("A", "B"), // Safe path
            ("C", "D"),
            ("D", "C"), // Trap
        ];

        closest_common_dominator(&edges, "A", vec!["A", "C"], None);
        closest_common_dominator(&edges, "A", vec!["B", "C"], None);
        closest_common_dominator(&edges, "A", vec!["C", "D"], None);
    }

    #[test]
    fn test_closest_common_dominator_self_loop_handling() {
        // A->A (Self loop), A->B
        let edges = vec![("A", "A"), ("A", "B")];
        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_multi_edge() {
        // Shape: A->B (x2), B->C.
        let edges = vec![
            ("A", "B"),
            ("A", "B"), // Duplicate edge
            ("B", "C"),
        ];
        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
    }

    #[test]
    fn test_closest_common_dominator_empty_target_set() {
        // A -> B
        let edges = vec![("A", "B")];
        closest_common_dominator(&edges, "A", vec![], None);
    }

    #[test]
    fn test_closest_common_dominator_repeated_node() {
        // A -> B
        let edges = vec![("A", "B")];

        closest_common_dominator(&edges, "A", vec!["A"], Some("A"));
        closest_common_dominator(&edges, "A", vec!["B"], Some("B"));
        closest_common_dominator(&edges, "A", vec!["A", "B"], Some("A"));
    }

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

    #[test]
    fn test_flow_graph_new() {
        let graph = SimpleDirectedGraph::from_edge_list(vec![("A", "B")]);
        let flow_graph = FlowGraph::new(graph.clone(), "A");
        assert_eq!(flow_graph.graph, graph);
        assert_eq!(flow_graph.start_node, "A");
        let flow_graph = FlowGraph::new(graph.clone(), "C");
        assert_eq!(flow_graph.graph, graph);
        assert_eq!(flow_graph.start_node, "C");
    }

    #[test]
    fn test_flow_graph_find_closest_common_dominator() {
        // A -> B -> C -> D
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D")];
        let simple_graph = SimpleDirectedGraph::from_edge_list(edges);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["C", "D"]),
            Some("C")
        );
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["B", "D"]),
            Some("B")
        );
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["A", "D"]),
            Some("A")
        );

        // Diamond: A -> {B, C} -> D
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")];
        let simple_graph = SimpleDirectedGraph::from_edge_list(edges);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["B", "C"]),
            Some("A")
        );
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["B", "D"]),
            Some("A")
        );
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["D"]),
            Some("D")
        );

        // Disjoint: A -> B, C -> D
        let edges = vec![("A", "B"), ("C", "D")];
        let simple_graph = SimpleDirectedGraph::from_edge_list(edges);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["B", "D"]),
            None
        );
        assert_eq!(
            flow_graph.find_closest_common_dominator(vec!["B", "C"]),
            None
        );
    }
}
