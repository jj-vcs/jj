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
//! the Immediate Dominator for every node (using the standard iterative
//! algorithm), and then calculating the LCA for the set S. See:
//!
//! * <http://www.hipersoft.rice.edu/grads/publications/dom14.pdf>
//! * <https://en.wikipedia.org/wiki/Dominator_(graph_theory)>
//!
//! The running time is O(V+E+|S|*V)in the worst case, the space complexity is
//! O(V+E), where V is the number of nodes and E is the number of edges. In
//! practice the algorithm is fast and efficient for typical use cases because
//! the number of nodes that dominate any given node is typically small, and
//! the dominator tree is typically shallow.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::iter;
use std::rc::Rc;

use futures::future::try_join_all;
use indexmap::IndexMap;
use indexmap::IndexSet;
use itertools::Itertools as _;
use thiserror::Error;

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

    /// Returns a postorder traversal of the nodes in this graph starting from
    /// the given node.
    pub fn get_postorder<'a>(&'a self, start_node: &'a N) -> Vec<&'a N> {
        post_order(start_node, |&u| self.adjacent_nodes(u).unwrap()).collect_vec()
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
    N: Clone + Eq + Hash,
{
    /// The graph.
    pub graph: SimpleDirectedGraph<N>,
    /// The start node.
    pub start_node: N,
}

/// Calculates the dominators in a flow graph. Also has a method for finding the
/// closest common dominator of a set of nodes.
pub struct DominatorFinder<'a, N> {
    /// Map from nodes to integers in [0, N-1] range, in postorder (the start
    /// node has index N-1).
    node_to_id: HashMap<&'a N, InternalId>,
    /// The inverse of node_to_id.
    id_to_node: Vec<&'a N>,
    /// The immediate dominator for each node (by index). NOTE: the immediate
    /// dominator of the start node is itself.
    immediate_dominators: Vec<InternalId>,
}

/// Errors that can occur while finding dominators.
#[derive(Debug, Error, PartialEq)]
pub enum DominatorFinderError {
    /// The flow graph is invalid.
    #[error("The flow graph is invalid: some nodes are unreachable from the start node")]
    UnreachableNodesInFlowGraph,
    /// The target set is empty.
    #[error("Target set must not be empty")]
    EmptyTargetSet,
    /// The target set is invalid.
    #[error("Target set contains a node which is not in the flow graph")]
    UnknownNodeInTargetSet,
}

/// The dominator algorithm assigns consecutive numeric IDs to nodes, for
/// efficiency reasons. We use this type alias for clarity.
type InternalId = usize;

impl<'a, N> DominatorFinder<'a, N>
where
    N: Clone + Eq + Hash,
{
    /// Constructs a new DominatorFinder. Returns an error if the flow graph is
    /// invalid: e.g. if some node is unreachable from the start node.
    pub fn calculate(flow_graph: &'a FlowGraph<N>) -> Result<Self, DominatorFinderError> {
        // Get postorder traversal of the graph starting from the start node.
        let postorder = flow_graph.graph.get_postorder(&flow_graph.start_node);
        if postorder.len() != flow_graph.graph.num_nodes() {
            return Err(DominatorFinderError::UnreachableNodesInFlowGraph);
        }

        // Map generic types to integer IDs
        let mut node_to_id = HashMap::new();
        let mut id_to_node = Vec::new();
        for (index, &node) in postorder.iter().enumerate() {
            id_to_node.push(node);
            node_to_id.insert(node, index);
        }

        // Build graph using internal IDs.
        let num_nodes = node_to_id.len();
        let mut rev_adj = vec![vec![]; num_nodes];
        for (u, v) in flow_graph.graph.edges() {
            rev_adj[node_to_id[v]].push(node_to_id[u]);
        }

        // Find the immediate dominators for each node using the Cooper-Harvey-Kennedy
        // iterative algorithm.
        let immediate_dominators = Self::calculate_immediate_dominators(&rev_adj);

        Ok(Self {
            node_to_id,
            id_to_node,
            immediate_dominators,
        })
    }

    /// Returns a map from each node to its immediate dominator. NOTE: the
    /// immediate dominator of the start node is itself.
    pub fn get_immediate_dominators(&self) -> HashMap<N, N> {
        self.immediate_dominators
            .iter()
            .enumerate()
            .map(|(index, &idom)| {
                (
                    self.id_to_node[index].clone(),
                    self.id_to_node[idom].clone(),
                )
            })
            .collect()
    }

    /// Finds the closest common dominator for the given flow graph and set of
    /// nodes S (target_set).
    pub fn find_closest_common_dominator<NI>(
        &self,
        target_set: NI,
    ) -> Result<N, DominatorFinderError>
    where
        NI: IntoIterator<Item = N>,
    {
        // Convert generic target_set to internal IDs
        let target_ids: Vec<InternalId> = target_set
            .into_iter()
            .map(|node| match self.node_to_id.get(&node) {
                Some(id) => Ok(*id),
                None => Err(DominatorFinderError::UnknownNodeInTargetSet),
            })
            .try_collect()?;
        if target_ids.is_empty() {
            return Err(DominatorFinderError::EmptyTargetSet);
        }

        // The closest common dominator of a set of nodes is the lowest common ancestor
        // of those nodes in the dominator tree.
        let closest_common_dominator =
            Self::find_lowest_common_ancestor(&target_ids, &self.immediate_dominators);

        // Map the internal ID back to generic type N.
        Ok(self.id_to_node[closest_common_dominator].clone())
    }

    // Applies the Cooper-Harvey-Kennedy iterative algorithm to find the immediate
    // dominators for each node in the graph.
    // See http://www.hipersoft.rice.edu/grads/publications/dom14.pdf for details on how this function works.
    fn calculate_immediate_dominators(rev_adj: &[Vec<InternalId>]) -> Vec<InternalId> {
        // Step 1: Compute Dominators on Reverse Graph
        let num_nodes = rev_adj.len();
        let start_node_id = num_nodes - 1;

        // We hold the immediate dominator for each node in the following vector, in
        // index position (the kth entry is the immediate dominator of the node with ID
        // k). We initialize the immediate dominator of every node to usize::MAX to
        // represent that those nodes are not processed yet. Once a node is
        // processed, its immediate dominator is guaranteed to be a valid node
        // index.
        let mut immediate_dominators: Vec<InternalId> = vec![usize::MAX; num_nodes];
        // NOTE: technically speaking the immediate dominator is NOT defined for the
        // start node, but it is convenient for the algorithm to set it to itself; this
        // is consistent with the literature and specifically with
        // Cooper-Harvey-Kennedy.
        immediate_dominators[start_node_id] = start_node_id;

        loop {
            // Each iteration of the loop processes all nodes in reverse postorder, trying
            // to improve the immediate dominator for each node. The loop continues until we
            // have an iteration where no immediate dominator is changed. Note that the
            // entries in immediate_dominators are only guaranteed to be correct when the
            // loop terminates.
            let mut changed = false;

            // Iterate in reverse postorder, skipping the start node.
            for u in (0..start_node_id).rev() {
                let mut new_idom = usize::MAX;
                // Process predecessors (nodes that flow INTO u).
                let preds = &rev_adj[u];
                for &p in preds {
                    if immediate_dominators[p] == usize::MAX {
                        // Skip predecessors that have not been processed yet.
                        continue;
                    }
                    if new_idom == usize::MAX {
                        // This is the first predecessor of u that has been processed so far. We use
                        // it as the starting point for finding the new "improved" immediate
                        // dominator for u.
                        new_idom = p;
                    } else {
                        // "Intersect" the current new_idom with p's idom.
                        new_idom = Self::intersect(new_idom, p, &immediate_dominators);
                    }
                }
                if new_idom == usize::MAX {
                    // None of the predecessors of u have been processed yet. That's fine, we will
                    // try again of the next iteration of the outer loop.
                    continue;
                }
                if immediate_dominators[u] != new_idom {
                    // We "improved" the immediate dominator for u!
                    immediate_dominators[u] = new_idom;
                    changed = true;
                }
            }

            if !changed {
                // We reached the fixed point. We are done.
                break;
            }
        }

        // At this point we know the immediate dominator of every node, but we keep the
        // Option wrapper so that we can use the intersect function during
        // find_lowest_common_ancestor.
        immediate_dominators
    }

    // See http://www.hipersoft.rice.edu/grads/publications/dom14.pdf for details on how this function works.
    fn intersect(
        mut b1: InternalId,
        mut b2: InternalId,
        immediate_dominators: &[InternalId],
    ) -> InternalId {
        while b1 != b2 {
            while b1 < b2 {
                b1 = immediate_dominators[b1];
            }
            while b2 < b1 {
                b2 = immediate_dominators[b2];
            }
        }
        b1
    }

    // See http://www.hipersoft.rice.edu/grads/publications/dom14.pdf for details on how this function works.
    fn find_lowest_common_ancestor(
        targets: &[InternalId],
        immediate_dominators: &[InternalId],
    ) -> InternalId {
        targets
            .iter()
            .copied()
            .reduce(|a, b| Self::intersect(a, b, immediate_dominators))
            .expect("targets must not be empty")
    }
}

/// Errors that can occur while finding a dominator value (i.e. a dominator in a
/// value flow graph).
#[derive(Debug, Error, PartialEq)]
pub enum FindDominatorValueError<E> {
    /// An error occurred while computing the value of a node.
    #[error(transparent)]
    ValueFnError(E),
    /// An error occurred while finding the dominator.
    #[error(transparent)]
    DominatorFinderError(#[from] DominatorFinderError),
}

/// Helper struct for constructing a value flow graph. It caches the results
/// of applying value_fn to nodes, and also keeps track of the mapping from
/// values to nodes and nodes to values.
struct ValueCache<N, V, VF> {
    /// The function that emits values.
    value_fn: VF,
    /// Maps nodes to their corresponding values.
    node_values: HashMap<N, Rc<V>>,
    /// Maps values to the nodes that have that value.
    value_to_nodes: HashMap<Rc<V>, Vec<N>>,
}

impl<N, V, VF, E> ValueCache<N, V, VF>
where
    N: Hash + Eq + Clone,
    V: Hash + Eq,
    VF: AsyncFn(&N) -> Result<V, E>,
{
    /// Creates a new ValueCache that uses the given function to get values.
    fn new(value_fn: VF) -> Self {
        Self {
            value_fn,
            node_values: HashMap::new(),
            value_to_nodes: HashMap::new(),
        }
    }

    /// Gets all the values currently cached.
    fn get_values(&self) -> Vec<Rc<V>> {
        self.value_to_nodes.keys().cloned().collect_vec()
    }

    /// Returns the number of distinct values currently cached.
    fn num_distinct_values(&self) -> usize {
        self.value_to_nodes.len()
    }

    /// Returns the value for the given node, computing it if it is not already
    /// cached.
    async fn get_or_compute_value(&mut self, node: &N) -> Result<Rc<V>, E> {
        self.compute_values([node]).await?;
        Ok(self.node_values.get(node).expect("cached").clone())
    }

    /// Computes the value of each node, asynchronously and concurrently. Values
    /// for nodes which were previously evaluated are skipped.
    async fn compute_values<'a, NI>(&mut self, nodes: NI) -> Result<(), E>
    where
        N: 'a,
        NI: IntoIterator<Item = &'a N>,
    {
        // 1. Filter out nodes already in the map and create futures for new nodes.
        let futures = nodes
            .into_iter()
            .filter(|node| !self.node_values.contains_key(node))
            .map(|node| async {
                let value = (self.value_fn)(node).await?;
                Ok((node.clone(), Rc::new(value)))
            });
        // 2. Run all new futures concurrently
        let new_results: Vec<(N, Rc<V>)> = try_join_all(futures).await?;
        // 3. Insert the new entries into the maps.
        for (node, value) in new_results {
            self.node_values.insert(node.clone(), value.clone());
            self.value_to_nodes.entry(value).or_default().push(node);
        }
        Ok(())
    }
}

impl<N> FlowGraph<N>
where
    N: Clone + Eq + Hash,
{
    /// Constructs a new FlowGraph.
    pub fn new(graph: SimpleDirectedGraph<N>, start_node: N) -> Self {
        Self { graph, start_node }
    }

    /// Creates a flow graph of values from a flow graph of nodes.
    ///
    /// More precisely, let G be a FlowGraph of nodes with start node S. The
    /// value flow graph G' is a FlowGraph derived from G. Let v(g) be the
    /// result of applying value_fn to g. The nodes of G' are the set of values
    /// v(g), for all g in G. For each edge g1->g2 in G, there is a
    /// corresponding edge v(g1)->v(g2) in G'. The start node in G' is v(S).
    ///
    /// Returns an error if any value_fn invocation fails.
    pub fn create_value_flow_graph<'a, V>(&self, node_values: &'a HashMap<N, V>) -> FlowGraph<&'a V>
    where
        V: Eq + Hash,
    {
        let mut edges = vec![];
        let start_value = node_values.get(&self.start_node).expect("cached");
        for (parent, children) in &self.graph.adj {
            let parent_value = node_values.get(parent).expect("cached");
            for child in children {
                let child_value = node_values.get(child).expect("cached");
                edges.push((parent_value, child_value));
            }
        }
        FlowGraph::new(SimpleDirectedGraph::new(edges), start_value)
    }

    /// Constructs a value flow graph from the given flow graph and value
    /// function, and finds the closest common dominator value for the
    /// values of the final nodes. Returns an error if value_fn returns an
    /// error for any node in the flow graph. `final_nodes` must not be empty.
    pub async fn find_dominator_value<V, VF, E>(
        &self,
        final_nodes: &[N],
        value_fn: VF,
    ) -> Result<V, FindDominatorValueError<E>>
    where
        V: Hash + Eq + Clone,
        VF: AsyncFn(&N) -> Result<V, E>,
    {
        let mut value_cache = ValueCache::new(value_fn);
        // First compute the values of all final nodes asynchronously and concurrently.
        value_cache
            .compute_values(final_nodes)
            .await
            .map_err(|e| FindDominatorValueError::ValueFnError(e))?;

        let final_values = value_cache.get_values();
        match &*final_values {
            [] => {
                return Err(FindDominatorValueError::DominatorFinderError(
                    DominatorFinderError::EmptyTargetSet,
                ));
            }
            [final_value] => {
                // Optimization: if all final nodes have the same value, that value is the
                // closest common dominator. There is no need to build the value flow graph.
                return Ok(final_value.as_ref().clone());
            }
            _ => {}
        }

        let start_value = value_cache
            .get_or_compute_value(&self.start_node)
            .await
            .map_err(|err| FindDominatorValueError::ValueFnError(err))?;

        if value_cache.num_distinct_values() == final_values.len() {
            // Optimization: at this point we know that the value of the start node is one
            // of the final values (because the cardinality of the value set did not
            // increase when we calculated the value of the start node). In such cases we
            // know immediately that the closest common dominator is `start_value`.
            return Ok(start_value.as_ref().clone());
        }

        // Compute all remaining values.
        value_cache
            .compute_values(self.graph.nodes())
            .await
            .map_err(|err| FindDominatorValueError::ValueFnError(err))?;

        // NOTE: at this point we could compare the cardinality of the value set versus
        // the number of nodes: if equal then we know that every node has a
        // different value, and it is tempting to conclude that the result should be
        // `start_value` (because the shape of the value flow graph is identical
        // to the shape of the original flow graph). That is not always correct,
        // consider this example with start node A and final nodes C and D:
        //
        // A(1) -> B(2) -> C(3)
        //            \--> D(4)
        //
        // However, IF start node IS the closest common dominator of the original graph
        // (it is not in the example above) then the answer would be `start_value`;
        // so IF we knew that to be true we could skip building the value flow graph and
        // running the dominator algorithm in the value flow graph.

        let value_flow_graph = self.create_value_flow_graph(&value_cache.node_values);
        let dominator_finder = DominatorFinder::calculate(&value_flow_graph)?;
        let dominator_value =
            dominator_finder.find_closest_common_dominator(final_values.iter())?;

        Ok(dominator_value.as_ref().clone())
    }
}

/// Traverses nodes from `start_node` in post-order.
fn post_order<T, NI>(
    start_node: T,
    mut neighbors_fn: impl FnMut(&T) -> NI,
) -> impl Iterator<Item = T>
where
    T: Clone + Hash + Eq,
    NI: DoubleEndedIterator<Item = T>,
{
    let mut stack = vec![(start_node, false)];
    let mut visited: HashSet<T> = HashSet::new();
    iter::from_fn(move || {
        while let Some((node, processed)) = stack.pop() {
            if processed {
                // If we marked it as processed, it means its children
                // were already added to the stack and processed.
                return Some(node);
            }
            // Mark as visited so we don't start a new DFS from here
            if !visited.insert(node.clone()) {
                // The node is already visited, continue.
                continue;
            }
            let neighbors = neighbors_fn(&node);
            // Push the node back onto the stack with processed = true.
            // It will be popped and yielded AFTER its children.
            stack.push((node, true));
            // Push the neighbors onto the stack with processed = false. The neighbors are
            // added in reverse order, so they are processed in the
            // original order.
            for neighbor in neighbors.rev() {
                if !visited.contains(&neighbor) {
                    stack.push((neighbor, false));
                }
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use maplit::hashmap;
    use pollster::FutureExt as _;

    use super::*;

    #[test]
    fn test_closest_common_dominator_split() -> Result<(), DominatorFinderError> {
        //   /-> B \
        // A        -> D
        //   \-> C /
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_linear_chain() -> Result<(), DominatorFinderError> {
        // A -> B -> C -> D
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "D")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["A", "B"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "D"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "D"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["A", "B", "C", "D"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_classic_diamond() -> Result<(), DominatorFinderError> {
        //      /-> B -\
        //    A          -> D -> E
        //      \-> C -/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("A", "C"), ("B", "D"), ("C", "D"), ("D", "E")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["D"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["D", "E"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["A", "D"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_single_node() -> Result<(), DominatorFinderError> {
        // A
        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new([("A", "A")]), "A");
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        Ok(())
    }

    #[test]
    fn test_invalid_flowgraph() {
        //       /-> E
        // A -> B
        //       \-> F
        //           ^
        //           |
        // C --> D --/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "E"), ("B", "F"), ("C", "D"), ("D", "F")]),
            "A",
        );
        assert_eq!(
            DominatorFinder::calculate(&flow_graph).err(),
            Some(DominatorFinderError::UnreachableNodesInFlowGraph)
        );
    }

    #[test]
    fn test_closest_common_dominator_simple_cycle_with_entry() -> Result<(), DominatorFinderError> {
        //
        // A -> B -> C -> D
        //      ^         |
        //      |         |
        //      \--------/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "D"), ("D", "B")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A", "B"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "B", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D"])?, "D");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_figure_eight_with_bridge() -> Result<(), DominatorFinderError>
    {
        //
        //  A -> B -> C -> D -> E -> F -> G
        //       ^         |    ^         |
        //       |         |    |         |
        //        \_______/      \_______/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([
                ("A", "B"), // entry
                ("B", "C"),
                ("C", "D"),
                ("D", "B"), // Loop 1
                ("D", "E"), // Bridge
                ("E", "F"),
                ("F", "G"),
                ("G", "E"), // Loop 2
            ]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "E"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["C", "F"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D", "E"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["D", "F"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["E", "G"])?, "E");
        assert_eq!(df.find_closest_common_dominator(["F", "G"])?, "F");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_figure_eight() -> Result<(), DominatorFinderError> {
        //
        //  A -> B -> C --> D   -> E -> F
        //       ^         | ^          |
        //       |         | |          |
        //        \_______/  \_________/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([
                ("A", "B"), // entry
                ("B", "C"),
                ("C", "D"),
                ("D", "B"), // Loop 1
                ("D", "E"),
                ("E", "F"),
                ("F", "D"), // Loop 2
            ]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "D"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["C", "E"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["C", "F"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D", "E"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["D", "F"])?, "D");
        assert_eq!(df.find_closest_common_dominator(["E", "F"])?, "E");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_entry_cycle_dominance() -> Result<(), DominatorFinderError> {
        // A -> B -> C
        //      ^    |
        //      |----/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "B")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A", "B"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["A", "B", "C"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_nested_loops() -> Result<(), DominatorFinderError> {
        //           /---> E
        //           |     |
        //           |     |
        // A -> B -> C <--/
        //      ^    |
        //      |    V
        //      \----D
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([
                ("A", "B"),
                ("B", "C"),
                ("C", "D"),
                ("C", "E"),
                ("E", "C"),
                ("D", "B"),
            ]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A", "B"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["A", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "D"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["C", "E"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["D", "E"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "E"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "D", "E"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "D", "E"])?, "C");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D", "E"])?, "B");
        Ok(())
    }

    #[test]
    fn test_irreducible_graph_cooper_harvey_kennedy_fig2() -> Result<(), DominatorFinderError> {
        //        5
        //     /    \
        //    |      |
        //    V      V
        //    4      3
        //    |      |
        //    V      V
        //    1 <==> 2
        let graph = SimpleDirectedGraph::new([(1, 2), (2, 1), (3, 2), (4, 1), (5, 4), (5, 3)]);
        let flow_graph = FlowGraph::new(graph, 5);
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.get_immediate_dominators(),
            HashMap::from([(1, 5), (2, 5), (3, 5), (4, 5), (5, 5),])
        );
        Ok(())
    }

    #[test]
    fn test_irreducible_graph_cooper_harvey_kennedy_fig3() -> Result<(), DominatorFinderError> {
        //     6
        //   /   \
        //  |     |
        //  v     v
        //  5     4 --
        //  |     |    \
        //  v     v     v
        //  1 <=> 2 <=> 3
        let graph = SimpleDirectedGraph::new([
            (1, 2),
            (2, 1),
            (2, 3),
            (3, 2),
            (5, 1),
            (4, 2),
            (4, 3),
            (6, 5),
            (6, 4),
        ]);
        let flow_graph = FlowGraph::new(graph, 6);
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.get_immediate_dominators(),
            HashMap::from([(1, 6), (2, 6), (3, 6), (4, 6), (5, 6), (6, 6),])
        );
        assert_eq!(df.find_closest_common_dominator([2, 3])?, 6);
        Ok(())
    }

    #[test]
    fn test_dominator_tree_with_three_levels() -> Result<(), DominatorFinderError> {
        // Graph taken from https://en.wikipedia.org/wiki/Dominator_(graph_theory)
        //     1
        //     |  /---\
        //     v /     \
        //     2 <--\    \
        //    / \    \    \
        //   /   \    \    \
        //  |     |    |   |
        //  v     v    |   |
        //  3     4    |   |
        //  |     |    |   |
        //   \    v    |   v
        //    --> 5 --/    6
        //
        let graph =
            SimpleDirectedGraph::new([(1, 2), (2, 3), (2, 4), (2, 6), (3, 5), (4, 5), (5, 2)]);
        let flow_graph = FlowGraph::new(graph, 1);
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.get_immediate_dominators(),
            HashMap::from([(1, 1), (2, 1), (3, 2), (4, 2), (5, 2), (6, 2),])
        );
        assert_eq!(df.find_closest_common_dominator([1, 6])?, 1);
        assert_eq!(df.find_closest_common_dominator([2, 3])?, 2);
        assert_eq!(df.find_closest_common_dominator([2, 4])?, 2);
        assert_eq!(df.find_closest_common_dominator([2, 5])?, 2);
        assert_eq!(df.find_closest_common_dominator([2, 6])?, 2);
        assert_eq!(df.find_closest_common_dominator([3, 4])?, 2);
        assert_eq!(df.find_closest_common_dominator([3, 5])?, 2);
        assert_eq!(df.find_closest_common_dominator([3, 6])?, 2);
        assert_eq!(df.find_closest_common_dominator([4, 5])?, 2);
        assert_eq!(df.find_closest_common_dominator([4, 6])?, 2);
        assert_eq!(df.find_closest_common_dominator([5, 6])?, 2);
        assert_eq!(df.find_closest_common_dominator([2, 3, 5])?, 2);
        assert_eq!(df.find_closest_common_dominator([3, 4, 5])?, 2);
        assert_eq!(df.find_closest_common_dominator([3, 4, 5, 6])?, 2);
        Ok(())
    }

    #[test]
    fn test_big_graph_fig_18_3() -> Result<(), DominatorFinderError> {
        // Graph taken from Modern Compiler Implementation in Java,
        // by Appel and Palsberg, 2004
        //
        //           1
        //           |
        //           v
        //      /--> 2 <--\
        //      |   / \   |
        //      |  v   v  |
        //      \- 3   4 -/
        //             /\
        //            /  \
        //           v    v
        //     /-->  5    6
        //    /    /   \  /
        //   /    |     ||
        //  /     v     vv
        // |  /-> 8      7
        // |  |   |      |
        // |  |   v      v
        // |  \-- 9      11
        // |      |      |
        // |      v      v
        //  \--- 10 --> 12
        //
        let graph = SimpleDirectedGraph::new([
            (1, 2),
            (2, 3),
            (2, 4),
            (3, 2),
            (4, 2),
            (4, 5),
            (4, 6),
            (5, 7),
            (5, 8),
            (6, 7),
            (7, 11),
            (8, 9),
            (9, 8),
            (9, 10),
            (10, 5),
            (10, 12),
            (11, 12),
        ]);
        let flow_graph = FlowGraph::new(graph, 1);
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.get_immediate_dominators(),
            HashMap::from([
                (1, 1),
                (2, 1),
                (3, 2),
                (4, 2),
                (5, 4),
                (6, 4),
                (7, 4),
                (8, 5),
                (9, 8),
                (10, 9),
                (11, 7),
                (12, 4)
            ])
        );
        assert_eq!(df.find_closest_common_dominator([6, 3])?, 2);
        assert_eq!(df.find_closest_common_dominator([11, 9, 12])?, 4);
        assert_eq!(df.find_closest_common_dominator([11, 9, 5])?, 4);
        assert_eq!(df.find_closest_common_dominator([11, 10])?, 4);
        assert_eq!(df.find_closest_common_dominator([10, 11, 12, 3, 6])?, 2);
        Ok(())
    }

    #[test]
    fn test_big_graph_fig_19_8() -> Result<(), DominatorFinderError> {
        // Graph taken from Modern Compiler Implementation in Java,
        // by Appel and Palsberg, 2004
        //
        //          /----- A ----\
        //         |              |
        //         v              v
        //  /----> B ---\     /-> C --\
        //  |      |    |     |   |   |
        //  |      v    |     |   v   |
        //  |  /-- D    |     \-- E   |
        //  |  |   |    |         |   |
        //  |  |   |    |         |   |
        //  |  v   |    v         v   v
        //  |  F   \--> G           H
        //  |  |\       |          /
        //  |  | \      v         /
        //  |  |  \ /---J        /
        //  |  |   X            /
        //  |  | /  \          /
        //  |  vv    v         |
        //  |  I     K         |
        //  |  \    /          |
        //  |   \  /           |
        //  |    vv            v
        //  \---- L ---------> M
        //
        let graph = SimpleDirectedGraph::new([
            ("A", "B"),
            ("A", "C"),
            ("B", "D"),
            ("B", "G"),
            ("C", "E"),
            ("C", "H"),
            ("D", "F"),
            ("D", "G"),
            ("E", "C"),
            ("E", "H"),
            ("F", "I"),
            ("F", "K"),
            ("G", "J"),
            ("H", "M"),
            ("I", "L"),
            ("J", "I"),
            ("K", "L"),
            ("L", "B"),
            ("L", "M"),
        ]);
        let flow_graph = FlowGraph::new(graph, "A");
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.get_immediate_dominators(),
            HashMap::from([
                ("A", "A"),
                ("B", "A"),
                ("C", "A"),
                ("D", "B"),
                ("E", "C"),
                ("F", "D"),
                ("G", "B"),
                ("H", "C"),
                ("I", "B"),
                ("J", "G"),
                ("K", "F"),
                ("L", "B"),
                ("M", "A"),
            ])
        );
        assert_eq!(df.find_closest_common_dominator(["K", "L"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["K", "C"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "G", "J"])?, "B");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_tree() -> Result<(), DominatorFinderError> {
        // A -> B -> C
        // \     \-> D
        //  \------> E
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("B", "D"), ("A", "E")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["C", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["C", "D", "E"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_bypassing_path() -> Result<(), DominatorFinderError> {
        // A -> B -> C -> D
        // |              ^
        // v              |
        // E -------------/
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "D"), ("A", "E"), ("E", "D")]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        assert_eq!(df.find_closest_common_dominator(["B", "D"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["C", "D"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["C", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["D", "E"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C", "D"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["C", "D", "E"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_self_loop_handling() -> Result<(), DominatorFinderError> {
        // A->A (Self loop), A->B
        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new([("A", "A"), ("A", "B")]), "A");
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_multi_edge() -> Result<(), DominatorFinderError> {
        // Shape: A->B (x2), B->C.
        let flow_graph = FlowGraph::new(
            SimpleDirectedGraph::new([
                ("A", "B"),
                ("A", "B"), // Duplicate edge
                ("B", "C"),
            ]),
            "A",
        );
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A"])?, "A");
        assert_eq!(df.find_closest_common_dominator(["B", "C"])?, "B");
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_invalid_target_set() -> Result<(), DominatorFinderError> {
        // A -> B
        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new([("A", "B")]), "A");
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(
            df.find_closest_common_dominator([]),
            Err(DominatorFinderError::EmptyTargetSet)
        );
        Ok(())
    }

    #[test]
    fn test_closest_common_dominator_repeated_node() -> Result<(), DominatorFinderError> {
        // A -> B
        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new([("A", "B")]), "A");
        let df = DominatorFinder::calculate(&flow_graph)?;
        assert_eq!(df.find_closest_common_dominator(["A", "B", "A", "B"])?, "A");
        Ok(())
    }

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

    #[test]
    fn test_flow_graph_new() {
        let graph = SimpleDirectedGraph::new([("A", "B")]);
        let flow_graph = FlowGraph::new(graph.clone(), "A");
        assert_eq!(flow_graph.graph, graph);
        assert_eq!(flow_graph.start_node, "A");
        let flow_graph = FlowGraph::new(graph.clone(), "C");
        assert_eq!(flow_graph.graph, graph);
        assert_eq!(flow_graph.start_node, "C");
    }

    #[test]
    fn test_post_order() {
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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('F', neighbors_fn).collect_vec(),
            ['A', 'E', 'B', 'C', 'D', 'F']
        );
        assert_eq!(post_order('E', neighbors_fn).collect_vec(), ['A', 'E']);
        assert_eq!(
            post_order('D', neighbors_fn).collect_vec(),
            ['A', 'B', 'C', 'D']
        );
        assert_eq!(post_order('A', neighbors_fn).collect_vec(), ['A']);

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('I', neighbors_fn).collect_vec(),
            ['A', 'B', 'D', 'C', 'F', 'E', 'G', 'H', 'I']
        );

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('I', neighbors_fn).collect_vec(),
            ['A', 'C', 'b', 'e', 'D', 'G', 'f', 'h', 'I']
        );

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('G', neighbors_fn).collect_vec(),
            ['A', 'B', 'C', 'E', 'D', 'F', 'G']
        );

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('G', neighbors_fn).collect_vec(),
            ['A', 'B', 'c', 'E', 'F', 'D', 'G']
        );

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('F', neighbors_fn).collect_vec(),
            ['A', 'E', 'D', 'F']
        );
        assert_eq!(post_order('C', neighbors_fn).collect_vec(), ['A', 'B', 'C']);

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
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(
            post_order('D', neighbors_fn).collect_vec(),
            ['C', 'A', 'B', 'D']
        );

        // This graph:
        //  o C
        //  o B
        //  o A (to C)

        let neighbors = hashmap! {
            'A' => vec!['C'],
            'B' => vec!['A'],
            'C' => vec!['B'],
        };
        let neighbors_fn = |node: &char| neighbors[node].iter().copied();
        assert_eq!(post_order('C', neighbors_fn).collect_vec(), ['A', 'B', 'C']);
        assert_eq!(post_order('B', neighbors_fn).collect_vec(), ['C', 'A', 'B']);
        assert_eq!(post_order('A', neighbors_fn).collect_vec(), ['B', 'C', 'A']);
    }

    #[test]
    fn test_value_flow_graph_new() {
        // A(1) -> B(1) -> C(2)
        let simple_graph = SimpleDirectedGraph::new([("A", "B"), ("B", "C")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let node_values = HashMap::from([("A", 1), ("B", 1), ("C", 2)]);
        let value_flow_graph = flow_graph.create_value_flow_graph(&node_values);

        let expected_value_edges = [(&1, &1), (&1, &2)];
        let expected_flow_graph =
            FlowGraph::new(SimpleDirectedGraph::new(expected_value_edges), &1);
        assert_eq!(value_flow_graph, expected_flow_graph);
    }

    #[test]
    fn test_value_flow_graph_find_dominator_value() {
        // A(1) -> B(1) -> C(2) -> D(3)
        //          \------------> E(3)
        let simple_graph =
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "D"), ("B", "E")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let value_fn = async |node: &&str| match *node {
            "A" | "B" => Ok(1),
            "C" => Ok(2),
            "D" | "E" => Ok(3),
            _ => Err("Unknown node".to_string()),
        };

        // Value graph (* means node has a self-loop):
        //   1* -> 2 -> 3
        //    \         ^
        //     \--------|
        assert_eq!(
            flow_graph
                .find_dominator_value(&["D", "E"], value_fn)
                .block_on(),
            Ok(3)
        );
        assert_eq!(
            flow_graph
                .find_dominator_value(&["C", "D"], value_fn)
                .block_on(),
            Ok(1)
        );
        assert_eq!(
            flow_graph
                .find_dominator_value(&["B", "C"], value_fn)
                .block_on(),
            Ok(1)
        );
    }

    #[test]
    fn test_find_dominator_value_with_distinct_values() {
        // A(1) -> B(2) -> C(3) -> D(4)
        //          \------------> E(5)
        let simple_graph =
            SimpleDirectedGraph::new([("A", "B"), ("B", "C"), ("C", "D"), ("B", "E")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let value_fn = async |node: &&str| match *node {
            "A" => Ok(1),
            "B" => Ok(2),
            "C" => Ok(3),
            "D" => Ok(4),
            "E" => Ok(5),
            _ => Err("Unknown node".to_string()),
        };

        // Value graph:
        // 1 -> 2 -> 3 -> 4
        //       \------> 5
        assert_eq!(
            flow_graph
                .find_dominator_value(&["D", "E"], value_fn)
                .block_on(),
            Ok(2)
        );
        assert_eq!(
            flow_graph
                .find_dominator_value(&["C", "D"], value_fn)
                .block_on(),
            Ok(3)
        );
        assert_eq!(
            flow_graph
                .find_dominator_value(&["B", "C"], value_fn)
                .block_on(),
            Ok(2)
        );
    }

    #[test]
    fn test_find_dominator_value_with_invalid_flow_graph() {
        // Invalid flow graph: A(1) -> B(1), C(2) -> D(2) (C and D are not reachable
        // from A).
        let simple_graph = SimpleDirectedGraph::new([("A", "B"), ("C", "D")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let value_fn = async |node: &&str| match *node {
            "A" | "B" => Ok(1),
            "C" | "D" => Ok(2),
            _ => Err("Unknown node".to_string()),
        };
        // Todo: the flow_graph is invalid because C and D are not reachable from A, so
        // ideally find_dominator_value should return UnreachableNodesInFlowGraph, but
        // the optimizations in find_dominator_value currently cause it to
        // return the start value. The best way to fix this is to calculate (and store)
        // the post-order in FlowGraph::new, that way we could not possibly construct an
        // invalid flow graph. This is not a big concern in practice though.
        assert_eq!(
            flow_graph
                .find_dominator_value(&["B", "D"], value_fn)
                .block_on(),
            Ok(1)
        );
    }

    #[test]
    fn test_find_dominator_value_with_unknown_node_in_target_set() {
        // Flow graph: A(1) -> B(2).
        let simple_graph = SimpleDirectedGraph::new([("A", "B")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let value_fn = async |node: &&str| match *node {
            "A" => Ok(1),
            "B" => Ok(2),
            "X" => Ok(666),
            _ => Err("Unknown node".to_string()),
        };
        assert_eq!(
            flow_graph
                .find_dominator_value(&["B", "X"], value_fn)
                .block_on(),
            Err(FindDominatorValueError::DominatorFinderError(
                DominatorFinderError::UnknownNodeInTargetSet
            ))
        );
    }

    #[test]
    fn test_find_dominator_value_with_unknown_node() {
        // Flow graph: A(1) -> B(2).
        let simple_graph = SimpleDirectedGraph::new([("A", "B")]);
        let flow_graph = FlowGraph::new(simple_graph, "A");
        let value_fn = async |node: &&str| match *node {
            "A" => Ok(1),
            "B" => Ok(2),
            _ => Err("Unknown node".to_string()),
        };
        assert_eq!(
            flow_graph
                .find_dominator_value(&["B", "X"], value_fn)
                .block_on(),
            Err(FindDominatorValueError::ValueFnError(
                "Unknown node".to_string()
            ))
        );
    }
}
