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

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;

use itertools::Itertools as _;

/// Generic implementation of the Common Dominator algorithm for directed
/// graphs, using the Cooper-Harvey-Kennedy iterative algorithm. Loosely
/// speaking the algorithm finds the "choke point" for a set of nodes S in a
/// directed graph (going from the "entry" node to nodes in S), closest to S.
///
/// Dominance:
///
/// * An entry node is a node with no incoming edges.
/// * A graph may have zero or more entry nodes. In any case, a virtual node can
///   be added to a graph to make it have a unique entry node.
/// * A node z is said to dominate a node n if all paths from the unique entry
///   node to n must go through z. Every node dominates itself, and the (unique)
///   entry node dominates all nodes.
/// * A node can have one or more dominators.
/// * A node z strictly dominates n if z dominates n and z != n.
/// * The immediate dominator of a node n is the dominator of n that doesn't
///   strictly dominate any other strict dominators of n. Informally it is the
///   "closest" choke point on all paths from the entry node to n.
/// * Let S be a subset of the nodes in the graph. The intersection of the
///   dominators of each node in S is the set of common dominators of S.
/// * The closest common dominator of S is the common dominator of S that
///   doesn't strictly dominate any other common dominator of S. Informally, it
///   is the choke point closest to S such that all paths from the entry node to
///   S must go through it.
///
/// Dominator Tree:
///
/// For any directed graph G with a single entry node there is a corresponding
/// dominator tree defined as follows:
/// * The nodes of the dominator tree are the same as the nodes of G
/// * The root of the dominator tree is the entry node of G
/// * In the dominator tree, the children of a node are the nodes it immediately
///   dominates
///
/// The closest common dominator of S is the Lowest Common Ancestor (LCA)
/// of S in the graph's dominator tree.
///
/// This implementation constructs the Dominator Tree by first determining
/// the Immediate Dominator (ipdom) for every node (using the standard iterative
/// algorithm), and then calculating the LCA for the set S. See:
///
/// * <http://www.hipersoft.rice.edu/grads/publications/dom14.pdf>
/// * <https://en.wikipedia.org/wiki/Dominator_(graph_theory)>
///
/// The running time is O(V+E+|S|*V)in the worst case, the space complexity is
/// O(V+E), where V is the number of nodes and E is the number of edges.
///
/// For a DAG, the expensive iterative Dominator step converges in just two
/// passes, making the tree construction effectively linear, O(V+E). The primary
/// bottleneck becomes the naive "Lowest Common Ancestor" (LCA) lookup, which
/// scales with the size of the set and the depth of the tree. If you need
/// better running time for large graphs, you can optimize the LCA step.
///
/// T must be Hash + Eq to be used as a key, and Clone to be returned.
pub struct DominatorFinder<T> {
    // Nodes are given consecutive integer IDs internally for efficient graph algorithms.
    node_to_id: HashMap<T, Index>,
    // Maps internal IDs back to the original generic type T for output.
    id_to_node: Vec<T>,
    // Forward adjacency list: adj[u] = [v1, v2, ...] means there are edges u->v1, u->v2, ...
    // Includes a virtual entry node that points to all natural entry nodes (in the original
    // graph), ensuring a single entry for the graph.
    adj: Vec<Vec<Index>>,
    // Reverse adjacency list.
    // Includes the virtual entry node.
    rev_adj: Vec<Vec<Index>>,
}

/// Type alias for clarity.
type Index = usize;

/// Finds the closest common dominator for the given graph and set of nodes S
/// (target_set).
pub fn find_closest_common_dominator<T, NI, EI>(
    nodes: NI,
    edges: EI,
    target_set: &[T],
) -> Result<Option<T>, String>
where
    T: Hash + Eq + Clone,
    NI: IntoIterator<Item = T>,
    EI: IntoIterator<Item = (T, T)>,
{
    DominatorFinder::new(nodes, edges)?.closest_common_dominator(target_set)
}

/// Finds the closest common post-dominator for the given graph and set of nodes
/// S (target_set).
pub fn find_closest_common_post_dominator<T, NI, EI>(
    nodes: NI,
    edges: EI,
    target_set: &[T],
) -> Result<Option<T>, String>
where
    T: Hash + Eq + Clone,
    NI: IntoIterator<Item = T>,
    EI: IntoIterator<Item = (T, T)>,
{
    let reverse_edges = edges.into_iter().map(|(u, v)| (v, u));
    DominatorFinder::new(nodes, reverse_edges)?.closest_common_dominator(target_set)
}

impl<T> DominatorFinder<T>
where
    T: Hash + Eq + Clone,
{
    /// Constructs a new DominatorFinder from a list of nodes and edges.
    /// The edges must correspond to the nodes provided; (u, v) means u->v.
    ///
    /// Returns an error if edges contain unknown nodes, or if the graph does
    /// not have any entry nodes (i.e., every node has at least one incoming
    /// edge). In the latter case, clients can add a virtual entry node
    /// themselves before calling this constructor. Clearly this is never an
    /// issue for DAGs.
    pub fn new<NI, EI>(nodes: NI, edges: EI) -> Result<Self, String>
    where
        NI: IntoIterator<Item = T>,
        EI: IntoIterator<Item = (T, T)>,
    {
        let mut node_to_id = HashMap::new();
        let mut id_to_node = Vec::new();

        // 1. Map generic types to integer IDs
        let mut i = 0;
        for node in nodes {
            match node_to_id.entry(node.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(i);
                    id_to_node.push(node);
                    i += 1;
                }
                _ => {
                    // Skip duplicates.
                }
            }
        }
        let n_original = i; // The number of unique nodes (excluding the virtual entry).
        let virtual_entry = n_original;

        let mut adj = vec![vec![]; n_original + 1]; // Reserve space for virtual entry
        let mut rev_adj = vec![vec![]; n_original + 1]; // Reserve space for virtual entry

        // 2. Build Graph using internal IDs
        for (u, v) in edges {
            if let (Some(&u), Some(&v)) = (node_to_id.get(&u), node_to_id.get(&v)) {
                if u == v {
                    continue; // Ignore self loops, they don't affect dominance.
                }
                adj[u].push(v);
                rev_adj[v].push(u);
            } else {
                return Err("Edge contains unknown node".to_string());
            }
        }

        // 3: Augment Graph with a unique virtual entry node. Note that we do this even
        // if the input graph already has a single entry.

        // Connect sources to the Virtual Entry
        let mut has_entry_node = false;
        for (i, predecessors) in rev_adj.iter_mut().enumerate().take(n_original) {
            if predecessors.is_empty() {
                predecessors.push(virtual_entry);
                adj[virtual_entry].push(i);
                has_entry_node = true;
            }
        }

        if !has_entry_node {
            return Err("Graph has no entry node".to_string());
        }

        Ok(Self {
            node_to_id,
            id_to_node,
            adj,
            rev_adj,
        })
    }

    /// Finds the closest common dominator for the given set of nodes S
    /// (target_set). Returns None if the closest common dominator in the
    /// augmented graph is the virtual entry. Returns an error if any node in
    /// target_set is unknown.
    pub fn closest_common_dominator(&self, target_set: &[T]) -> Result<Option<T>, String> {
        // Convert generic inputs to internal IDs
        let target_set: Vec<Index> = target_set
            .iter()
            .map(|node| match self.node_to_id.get(node) {
                Some(&id) => Ok(id),
                None => Err("Target set contains unknown node".to_string()),
            })
            .try_collect()?;

        if target_set.is_empty() {
            return Ok(None);
        }

        // Step 1: Compute Dominators on Reverse Graph
        let n_original = self.adj.len() - 1;
        let virtual_entry = n_original; // The last node is the virtual entry
        let execution_order = Self::get_reverse_post_order(&self.adj, virtual_entry);

        // idom is the immediate dominator for each node.
        let mut idom: Vec<Option<Index>> = vec![None; n_original + 1];
        idom[virtual_entry] = Some(virtual_entry);

        loop {
            let mut changed = false;
            for &u in &execution_order {
                if u == virtual_entry {
                    continue;
                }

                // Process predecessors (nodes that flow INTO u).
                let preds = &self.rev_adj[u];
                if preds.is_empty() {
                    continue;
                }

                let mut new_idom = None;
                // Find first processed predecessor.
                for &p in preds {
                    if idom[p].is_some() {
                        new_idom = Some(p);
                        break;
                    }
                }

                if let Some(mut candidate) = new_idom {
                    for &p in preds {
                        if idom[p].is_some() {
                            candidate = Self::intersect(candidate, p, &idom, &execution_order);
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

        // Step 3: Find LCA in Dominator Tree
        let mut current_lca = target_set[0];
        for &node in target_set.iter().skip(1) {
            current_lca = Self::find_lca(current_lca, node, &idom, virtual_entry);
        }

        if current_lca == virtual_entry {
            return Ok(None); // Only common dominator is the artificial root
        }

        // Map internal ID back to generic type T
        Ok(Some(self.id_to_node[current_lca].clone()))
    }

    fn intersect(mut b1: Index, mut b2: Index, idom: &[Option<Index>], order: &[Index]) -> Index {
        let get_order_idx =
            |node: Index| -> Index { order.iter().position(|&x| x == node).unwrap() };

        while b1 != b2 {
            while get_order_idx(b1) > get_order_idx(b2) {
                b1 = idom[b1].unwrap();
            }
            while get_order_idx(b2) > get_order_idx(b1) {
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

    fn get_reverse_post_order(graph: &[Vec<Index>], root: Index) -> Vec<Index> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        Self::dfs(root, graph, &mut visited, &mut order);
        order.reverse();
        order
    }

    fn dfs(u: Index, graph: &[Vec<Index>], visited: &mut HashSet<Index>, order: &mut Vec<Index>) {
        visited.insert(u);
        for &v in &graph[u] {
            if !visited.contains(&v) {
                Self::dfs(v, graph, visited, order);
            }
        }
        order.push(u);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(nodes: Vec<&str>, edges: Vec<(&str, &str)>) -> DominatorFinder<String> {
        let nodes_string: Vec<String> = nodes.iter().map(|&n| n.to_string()).collect();
        let edges_string: Vec<(String, String)> = edges
            .iter()
            .map(|&(u, v)| (u.to_string(), v.to_string()))
            .collect();
        DominatorFinder::new(nodes_string, edges_string).unwrap()
    }

    fn run_test(nodes: &[&str], edges: &[(&str, &str)], s: Vec<&str>, expected: Option<&str>) {
        let finder = setup(nodes.to_owned(), edges.to_owned());
        let s_string: Vec<String> = s.iter().map(|&n| n.to_string()).collect();
        let result = finder.closest_common_dominator(&s_string);
        assert_eq!(result, Ok(expected.map(|e| e.to_string())));
    }

    fn run_test_expect_error(
        nodes: &[&str],
        edges: &[(&str, &str)],
        s: Vec<&str>,
        expected_error: &str,
    ) {
        let finder = setup(nodes.to_owned(), edges.to_owned());
        let s_string: Vec<String> = s.iter().map(|&n| n.to_string()).collect();
        let result = finder.closest_common_dominator(&s_string);
        assert_eq!(result, Err(expected_error.to_string()));
    }

    #[test]
    fn test_split() {
        //   /-> B -> D
        // A
        //   \-> C -> D
        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")];

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["C"], Some("C"));
        run_test(&nodes, &edges, vec!["D"], Some("D"));

        run_test(&nodes, &edges, vec!["B", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "C", "D"], Some("A"));
    }

    #[test]
    fn test_linear_chain() {
        // A -> B -> C -> D
        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D")];

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["C"], Some("C"));
        run_test(&nodes, &edges, vec!["D"], Some("D"));

        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "D"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "D"], Some("C"));
        run_test(&nodes, &edges, vec!["A", "B", "C", "D"], Some("A"));
    }

    #[test]
    fn test_disjoint_no_common() {
        // A -> B
        // C -> D
        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![("A", "B"), ("C", "D")];

        run_test(&nodes, &edges, vec!["A", "C"], None);
        run_test(&nodes, &edges, vec!["A", "D"], None);
        run_test(&nodes, &edges, vec!["B", "D"], None);

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
    }

    #[test]
    fn test_classic_diamond() {
        //      /-> B -\
        //    A          -> D -> E
        //      \-> C -/
        let nodes = vec!["A", "B", "C", "D", "E"];
        let edges = vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D"), ("D", "E")];

        run_test(&nodes, &edges, vec!["B", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("A"));
        run_test(&nodes, &edges, vec!["D"], Some("D"));
        run_test(&nodes, &edges, vec!["D", "E"], Some("D"));
        run_test(&nodes, &edges, vec!["A", "D"], Some("A"));
    }

    #[test]
    fn test_basic_y_shape() {
        // A
        //  \
        //    --> C -> D
        //  /
        // B
        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![("A", "C"), ("B", "C"), ("C", "D")];

        run_test(&nodes, &edges, vec!["A", "B"], None);
        run_test(&nodes, &edges, vec!["A", "C"], None);
        run_test(&nodes, &edges, vec!["C", "D"], Some("C"));

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["C"], Some("C"));
        run_test(&nodes, &edges, vec!["D"], Some("D"));
    }

    #[test]
    fn test_single_node() {
        // A
        let nodes = vec!["A"];
        let edges = vec![];

        run_test(&nodes, &edges, vec!["A"], Some("A"));
    }

    #[test]
    fn test_generic_integers() {
        // Using Integers instead of Strings
        // 1 -> 2
        // 1 -> 3
        let nodes = vec![1, 2, 3];
        let edges = vec![(1, 2), (1, 3)];

        let finder = DominatorFinder::new(nodes, edges).unwrap();
        let result = finder.closest_common_dominator(&[2, 3]);
        assert_eq!(result, Ok(Some(1)));
    }

    #[test]
    fn test_complex_multi_source_multi_sink() {
        //       /-> E
        // A -> B
        //       \-> F
        //           ^
        //           |
        // C --> D --/
        let nodes = vec!["A", "B", "C", "D", "E", "F"];
        let edges = vec![("A", "B"), ("B", "E"), ("B", "F"), ("C", "D"), ("D", "F")];

        run_test(&nodes, &edges, vec!["E", "F"], None);
        run_test(&nodes, &edges, vec!["F"], Some("F"));
        run_test(&nodes, &edges, vec!["B", "F"], None);
    }

    #[test]
    fn test_simple_cycle_with_entry() {
        //
        // A -> B -> C -> D
        //      ^         |
        //      |         |
        //      \--------/

        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("D", "B")];

        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "B", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "C", "D"], Some("B"));

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["C"], Some("C"));
        run_test(&nodes, &edges, vec!["D"], Some("D"));
    }

    #[test]
    fn test_figure_eight_with_bridge() {
        //
        //  A -> B -> C -> D -> E -> F -> G
        //       ^         |    ^         |
        //       |         |    |         |
        //        \_______/      \_______/
        let nodes = vec!["A", "B", "C", "D", "E", "F", "G"];
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

        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "E"], Some("C"));
        run_test(&nodes, &edges, vec!["C", "F"], Some("C"));
        run_test(&nodes, &edges, vec!["D", "E"], Some("D"));
        run_test(&nodes, &edges, vec!["D", "F"], Some("D"));
        run_test(&nodes, &edges, vec!["E", "G"], Some("E"));
        run_test(&nodes, &edges, vec!["F", "G"], Some("F"));
    }

    #[test]
    fn test_figure_eight() {
        //
        //  A -> B -> C --> D   -> E -> F
        //       ^         | ^          |
        //       |         | |          |
        //        \_______/  \_________/
        let nodes = vec!["A", "B", "C", "D", "E", "F"];
        let edges = vec![
            ("A", "B"), // entry
            ("B", "C"),
            ("C", "D"),
            ("D", "B"), // Loop 1
            ("D", "E"),
            ("E", "F"),
            ("F", "D"), // Loop 2
        ];

        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "D"], Some("C"));
        run_test(&nodes, &edges, vec!["C", "E"], Some("C"));
        run_test(&nodes, &edges, vec!["D", "E"], Some("D"));
        run_test(&nodes, &edges, vec!["D", "F"], Some("D"));
        run_test(&nodes, &edges, vec!["E", "F"], Some("E"));
    }

    #[test]
    fn test_entry_cycle_dominance() {
        // B -> C -> B (Loop)
        // A -> B
        let nodes = vec!["A", "B", "C"];
        let edges = vec![("A", "B"), ("B", "C"), ("C", "B")];

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["C"], Some("C"));

        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["A", "B", "C"], Some("A"));
    }

    #[test]
    fn test_nested_loops() {
        //           /---> E
        //           |     |
        //           |     |
        // A -> B -> C <--/
        //      ^     \--> D
        //      |          |
        //      |----------|
        let nodes = vec!["A", "B", "C", "D", "E"];
        let edges = vec![
            ("A", "B"),
            ("B", "C"),
            ("C", "D"),
            ("C", "E"),
            ("E", "C"),
            ("D", "B"),
        ];

        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
        run_test(&nodes, &edges, vec!["A", "C"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "D"], Some("C"));
        run_test(&nodes, &edges, vec!["C", "E"], Some("C"));
        run_test(&nodes, &edges, vec!["D", "E"], Some("C"));

        run_test(&nodes, &edges, vec!["B", "C", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "C", "E"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "D", "E"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "D", "E"], Some("C"));

        run_test(&nodes, &edges, vec!["B", "C", "D", "E"], Some("B"));
    }

    #[test]
    fn test_tree() {
        // A -> B -> C
        // \     \-> D
        //  \------> E
        let nodes = vec!["A", "B", "C", "D", "E"];
        let edges = vec![("A", "B"), ("B", "C"), ("B", "D"), ("A", "E")];

        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("A"));
        run_test(&nodes, &edges, vec!["C", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "E"], Some("A"));

        run_test(&nodes, &edges, vec!["B", "C", "D"], Some("B"));
        run_test(&nodes, &edges, vec!["C", "D", "E"], Some("A"));
    }

    #[test]
    fn test_bypassing_path() {
        // A -> B -> C -> D
        // |              ^
        // v              |
        // E -------------/
        let nodes = vec!["A", "B", "C", "D", "E"];
        let edges = vec![("A", "B"), ("B", "C"), ("C", "D"), ("A", "E"), ("E", "D")];

        run_test(&nodes, &edges, vec!["B", "C"], Some("B"));
        run_test(&nodes, &edges, vec!["B", "D"], Some("A"));
        run_test(&nodes, &edges, vec!["B", "E"], Some("A"));
        run_test(&nodes, &edges, vec!["C", "D"], Some("A"));
        run_test(&nodes, &edges, vec!["C", "E"], Some("A"));
        run_test(&nodes, &edges, vec!["D", "E"], Some("A"));

        run_test(&nodes, &edges, vec!["B", "C", "D"], Some("A"));
        run_test(&nodes, &edges, vec!["C", "D", "E"], Some("A"));
    }

    #[test]
    fn test_infinite_loop_trap() {
        // A->B, C->D->C (Trap)
        let nodes = vec!["A", "B", "C", "D"];
        let edges = vec![
            ("A", "B"), // Safe path
            ("C", "D"),
            ("D", "C"), // Trap
        ];

        run_test(&nodes, &edges, vec!["A", "C"], None);
        run_test(&nodes, &edges, vec!["B", "C"], None);
        run_test(&nodes, &edges, vec!["C", "D"], None);
    }

    #[test]
    fn test_self_loop_handling() {
        // A->A (Self loop), A->B
        let nodes = vec!["A", "B"];
        let edges = vec![("A", "A"), ("A", "B")];
        run_test(&nodes, &edges, vec!["A"], Some("A"));
    }

    #[test]
    fn test_multi_edge() {
        // Shape: A->B (x2), B->C.
        let nodes = vec!["A", "B", "C"];
        let edges = vec![
            ("A", "B"),
            ("A", "B"), // Duplicate edge
            ("B", "C"),
        ];
        run_test(&nodes, &edges, vec!["A"], Some("A"));
    }

    #[test]
    fn test_empty_target_set() {
        // A -> B
        let nodes = vec!["A", "B"];
        let edges = vec![("A", "B")];
        run_test(&nodes, &edges, vec![], None);
    }

    #[test]
    fn test_empty_graph() {
        let nodes: Vec<String> = vec![];
        let edges: Vec<(String, String)> = vec![];
        let finder = DominatorFinder::new(nodes, edges);
        assert_eq!(finder.err(), Some("Graph has no entry node".to_string()));
    }

    #[test]
    fn test_repeated_node() {
        // A -> B
        let nodes = vec!["A", "B", "A", "B"];
        let edges = vec![("A", "B")];

        run_test(&nodes, &edges, vec!["A"], Some("A"));
        run_test(&nodes, &edges, vec!["B"], Some("B"));
        run_test(&nodes, &edges, vec!["A", "B"], Some("A"));
    }

    #[test]
    fn test_invalid_edge() {
        let nodes = vec!["A", "B"];
        {
            let edges = vec![("A", "C")];
            let finder = DominatorFinder::new(nodes.clone(), edges);
            assert_eq!(finder.err(), Some("Edge contains unknown node".to_string()));
        }
        {
            let edges = vec![("C", "A")];
            let finder = DominatorFinder::new(nodes.clone(), edges);
            assert_eq!(finder.err(), Some("Edge contains unknown node".to_string()));
        }
        {
            let edges = vec![("C", "D")];
            let finder = DominatorFinder::new(nodes.clone(), edges);
            assert_eq!(finder.err(), Some("Edge contains unknown node".to_string()));
        }
    }

    #[test]
    fn test_invalid_node_in_target() {
        // A -> B
        let nodes = vec!["A", "B"];
        let edges = vec![("A", "B")];

        run_test_expect_error(
            &nodes,
            &edges,
            vec!["C"],
            "Target set contains unknown node",
        );
        run_test_expect_error(
            &nodes,
            &edges,
            vec!["A", "C"],
            "Target set contains unknown node",
        );
        run_test_expect_error(
            &nodes,
            &edges,
            vec!["B", "C"],
            "Target set contains unknown node",
        );
    }
}
