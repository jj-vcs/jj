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

//! Utility for solving divergence. See
//! <https://github.com/jj-vcs/jj/blob/main/docs/design/jj-converge-command.md>
//! for more details.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::executor::block_on_stream;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::TreeId;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::evolution::walk_predecessors;
use jj_lib::graph_dominators::FlowGraph;
use jj_lib::graph_dominators::SimpleDirectedGraph;
use jj_lib::graph_dominators::ValueCache;
use jj_lib::index::IndexError;
use jj_lib::merge::Merge;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::store::Store;
use thiserror::Error;

/// Maps change-ids to commits with that change-id.
pub type CommitsByChangeId = HashMap<ChangeId, HashMap<CommitId, Commit>>;

/// The result of attempting to converge a particular attribute (description,
/// author, parents, tree) of a set of divergent commits.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ConvergedAttribute<T> {
    /// The attribute was successfully merged.
    Solved(T),
    /// The attribute could not be merged automatically.
    Unsolved {
        /// This is a hint to the caller: merge the attribute of the divergent
        /// commits (minus the excluded_divergent_commits), using this
        /// commit as the base.
        base_commit: CommitId,
        /// This is a hint to the caller: these divergent commits should be
        /// excluded from the merge.
        excluded_divergent_commits: HashSet<CommitId>,
    },
}

/// The proposed solution for converging a change.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ConvergeResult {
    /// The proposed author.
    pub author: ConvergedAttribute<Signature>,
    /// The proposed description.
    pub description: ConvergedAttribute<String>,
    /// The proposed parents.
    pub parents: ConvergedAttribute<Vec<CommitId>>,
    /// The proposed tree. Not set if we haven't converged the parents yet.
    pub tree: Option<TreeIdsAndLabels>,
}

/// Errors that can occur during converge.
#[derive(Debug, Error)]
pub enum ConvergeError {
    /// A backend error occurred.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// An index error occurred.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// An error occurred while evaluating the revset expression for finding
    /// divergent commits.
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
    /// An error occurred while traversing the evolution graph of the divergent
    /// commits.
    #[error(transparent)]
    WalkPredecessors(#[from] WalkPredecessorsError),
    /// An IO error occurred.
    #[error(transparent)]
    IO(#[from] std::io::Error),
    /// An unexpected error occurred.
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// Evaluates the revset expression and returns those commits that are
/// divergent, in the sense that the expression matches two or more commits in
/// the result with the same change-id.
///
/// The commits are keyed by their change-id.
pub async fn find_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    revset_expression: Arc<ResolvedRevsetExpression>,
) -> Result<CommitsByChangeId, RevsetEvaluationError> {
    let mut result = CommitsByChangeId::new();
    let mut stream = revset_expression.evaluate(repo.as_ref())?.stream();
    while let Some(commit_id) = stream.try_next().await? {
        let commit = repo.store().get_commit_async(&commit_id).await?;
        result
            .entry(commit.change_id().clone())
            .or_default()
            .insert(commit.id().clone(), commit);
    }
    // Remove entries that have only a single commit — we only care about
    // changes with multiple divergent commits.
    result.retain(|_, commits| commits.len() > 1);
    Ok(result)
}

/// Attempts to solve divergence in the divergent commits given by the
/// TruncatedEvolutionGraph. The caller can provide any subset of author,
/// description, parents, and tree, to be used in the solution commit. An
/// attempt is made to automatically produce a value for those attributes not
/// given by the user.
pub async fn converge_change(
    truncated_evolution_graph: &TruncatedEvolutionGraph,
    author: Option<Signature>,
    description: Option<String>,
    parents: Option<Vec<CommitId>>,
    tree: Option<TreeIdsAndLabels>,
) -> Result<ConvergeResult, ConvergeError> {
    let author = if let Some(author) = author {
        ConvergedAttribute::Solved(author)
    } else {
        converge_author(truncated_evolution_graph).await?
    };
    let description = if let Some(description) = description {
        ConvergedAttribute::Solved(description)
    } else {
        converge_description(truncated_evolution_graph).await?
    };
    let parents = if let Some(parents) = parents {
        ConvergedAttribute::Solved(parents)
    } else {
        converge_parents(truncated_evolution_graph).await?
    };

    let tree = if let Some(tree) = tree {
        Some(tree)
    } else if let ConvergedAttribute::Solved(parents) = &parents {
        let tree = converge_trees(truncated_evolution_graph, parents).await?;
        Some(TreeIdsAndLabels::new(tree))
    } else {
        None
    };

    Ok(ConvergeResult {
        author,
        description,
        parents,
        tree,
    })
}

/// Adds a new commit for the proposed solution, as a successor of the divergent
/// commits.
pub async fn apply_solution(
    author: Signature,
    description: String,
    parents: Vec<CommitId>,
    tree: TreeIdsAndLabels,
    change_id: ChangeId,
    divergent_commit_ids: &Vec<CommitId>,
    repo_mut: &mut MutableRepo,
) -> Result<(Commit, usize), ConvergeError> {
    let merged_tree = tree.to_merged_tree(repo_mut.store());
    let solution = repo_mut
        .new_commit(parents, merged_tree)
        .set_change_id(change_id.clone())
        .set_description(description)
        .set_author(author)
        .set_predecessors(divergent_commit_ids.clone())
        .write()
        .await?;
    for divergent_commit_id in divergent_commit_ids {
        repo_mut.set_rewritten_commit(divergent_commit_id.clone(), solution.id().clone());
    }
    let num_rebased = repo_mut.rebase_descendants().await?;
    Ok((solution, num_rebased))
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id, and only goes as far as
/// the closest common dominator of the divergent commits.
pub struct TruncatedEvolutionGraph {
    // The repo.
    repo: Arc<ReadonlyRepo>,
    // The commits to converge.
    divergent_commits: Vec<Commit>,
    // The ids of the commits to converge.
    divergent_commit_ids: Vec<CommitId>,
    /// The evolution graph of the divergent commits, with edges X->Y if commit
    /// X is a predecessor of commit Y and both X and Y have the same
    /// divergent change-id. The graph is not necessarily a tree (commits
    /// may have multiple predecessors). The start node is the evolution
    /// fork point.
    pub flow_graph: FlowGraph<CommitId>,
}

impl TruncatedEvolutionGraph {
    /// Builds a truncated evolution graph for the given divergent commits,
    /// which are expected to all have the same change-id.
    pub async fn new(
        repo: Arc<ReadonlyRepo>,
        divergent_commits: Vec<Commit>,
    ) -> Result<Self, ConvergeError> {
        validate(
            divergent_commits.len() > 1,
            &format!(
                "Expected multiple divergent commits, got {}",
                divergent_commits.len()
            ),
        )?;

        let divergent_commit_ids = divergent_commits
            .iter()
            .map(|c| c.id().clone())
            .collect_vec();

        // Ensure all provided divergent commits belong to the same change-id.
        // Note: divergent_commits is not empty, so it is ok to unwrap.
        let divergent_change_id = if divergent_commits.iter().map(|c| c.change_id()).all_equal() {
            divergent_commits.first().unwrap().change_id().clone()
        } else {
            return Err(ConvergeError::Other(
                "all divergent commits must have the same change-id".into(),
            ));
        };

        // The list of edges, with commits pointing to their successors.
        let mut edges = vec![];
        let mut seen = HashSet::new();
        let mut to_visit = HashSet::with_capacity(divergent_commit_ids.len());
        to_visit.extend(divergent_commit_ids.iter().cloned());

        let evolution_nodes = block_on_stream(
            walk_predecessors(&repo, divergent_commit_ids.as_slice()).boxed_local(),
        );

        // These are the commits in the graph that have no predecessors. Typically
        // there is exactly one entry in initial_nodes (the first commit for the
        // change-id).
        let mut initial_nodes = vec![];

        for node in evolution_nodes {
            let entry = node?;
            let commit_id = entry.commit.id();
            if *entry.commit.change_id() != divergent_change_id {
                // Skip commits with unrelated change ids.
                continue;
            }
            to_visit.remove(commit_id);
            if !seen.insert(commit_id.clone()) {
                // TODO: think about this some more. Can 2 different operations result in the
                // same commit? Maybe the key should be (commit-id, operation-id).

                // Note: currently walk_predecessors returns an error if the graph is cyclic, so
                // we shouldn't encounter the same commit twice. But in the future we could
                // allow cyclic evolution, and if we do there is no reason to disallow it here.
                // By continuing we future proof this.
                continue;
            }
            let predecessors = entry
                .predecessors()
                .await?
                .iter()
                .filter_map(|commit| {
                    if *commit.change_id() == divergent_change_id {
                        Some(commit.id().clone())
                    } else {
                        None
                    }
                })
                .collect_vec();
            for predecessor in &predecessors {
                edges.push((predecessor.clone(), commit_id.clone()));
            }
            if predecessors.is_empty() {
                initial_nodes.push(commit_id.clone());
                if to_visit.is_empty() {
                    break;
                }
            } else {
                to_visit.extend(predecessors);
            }
        }

        validate(
            !initial_nodes.is_empty(),
            "Unexpected error: initial_nodes should not be empty",
        )?;

        // By definition the flow graph must have a single initial node.
        let initial_node = if initial_nodes.len() == 1 {
            initial_nodes[0].clone()
        } else {
            // In graphs with multiple "real" initial nodes we introduce a virtual initial
            // node (the root commit) and pretend the two or more "real" initial nodes are
            // successors of the root commit.
            let root_commit_id = repo.store().root_commit_id().clone();
            for initial_node in initial_nodes {
                edges.push((root_commit_id.clone(), initial_node));
            }
            root_commit_id
        };

        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new(edges), initial_node);
        Ok(Self {
            repo,
            divergent_commits,
            divergent_commit_ids,
            flow_graph,
        })
    }

    /// Returns the repo.
    pub fn repo(&self) -> &Arc<ReadonlyRepo> {
        &self.repo
    }

    /// Returns the divergent commits.
    pub fn divergent_commits(&self) -> &Vec<Commit> {
        &self.divergent_commits
    }

    /// Returns the commit ids of the divergent commits.
    pub fn divergent_commit_ids(&self) -> &Vec<CommitId> {
        &self.divergent_commit_ids
    }

    /// Returns the change-id of the divergent commits. All divergent commits
    /// are expected to have the same change-id.
    pub fn change_id(&self) -> &ChangeId {
        self.divergent_commits[0].change_id()
    }
}

async fn converge_author(
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergedAttribute<Signature>, ConvergeError> {
    let value_fn = async |c: &Commit| Ok(c.author().clone());
    let excluded_divergent_commits = HashSet::default();
    let (value_merge, base_commit) =
        create_value_merge(graph, &excluded_divergent_commits, value_fn).await?;
    if let Some(value) = value_merge.resolve_trivial(SameChange::Accept) {
        Ok(ConvergedAttribute::Solved(value.clone()))
    } else {
        Ok(ConvergedAttribute::Unsolved {
            base_commit,
            excluded_divergent_commits: HashSet::default(),
        })
    }
}

async fn converge_description(
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergedAttribute<String>, ConvergeError> {
    let value_fn = async |c: &Commit| Ok(c.description().to_string());
    let excluded_divergent_commits = HashSet::default();
    let (value_merge, base_commit) =
        create_value_merge(graph, &excluded_divergent_commits, value_fn).await?;
    if let Some(value) = value_merge.resolve_trivial(SameChange::Accept) {
        Ok(ConvergedAttribute::Solved(value.clone()))
    } else {
        Ok(ConvergedAttribute::Unsolved {
            base_commit,
            excluded_divergent_commits: HashSet::default(),
        })
    }
}

async fn converge_parents(
    _graph: &TruncatedEvolutionGraph,
) -> Result<ConvergedAttribute<Vec<CommitId>>, ConvergeError> {
    todo!()
}

/// A MergedTree, without the `Arc<Store>`. That allows us to derive Eq and Hash
/// for it, which we need in some algorithms.
#[derive(Eq, Hash, PartialEq, Clone, Debug)]
pub struct TreeIdsAndLabels {
    /// The tree IDs of the merged tree.
    pub tree_ids: Merge<TreeId>,
    /// Conflict labels of the merged tree.
    pub labels: ConflictLabels,
}

impl TreeIdsAndLabels {
    /// Creates a new TreeIdsAndLabels.
    pub fn new(merged_tree: MergedTree) -> Self {
        let (tree_ids, labels) = merged_tree.into_tree_ids_and_labels();
        Self { tree_ids, labels }
    }

    /// Converts the TreeIdsAndLabels into a MergedTree.
    pub fn to_merged_tree(&self, store: &Arc<Store>) -> MergedTree {
        MergedTree::new(store.clone(), self.tree_ids.clone(), self.labels.clone())
    }
}

// Assume A, B, C are the divergent commits, P is the solution parents (i.e. the
// parents chosen by converge_parents), and F is a commit chosen as a "good base
// for converging trees" as explained below.
//
// Notation:
// * MCTNR: merge_commit_trees_no_resolve
// * F^: MCTNR(F.parents()), i.e. the unresolved MergedTree of the parents of F.
// * F': the resolved MergedTree of F rebased on top of the tree of P
// * A': the resolved MergedTree of A rebased on top of the tree of P
// * B': the resolved MergedTree of B rebased on top of the tree of P
// * C': the resolved MergedTree of C rebased on top of the tree of P
//
// Let X be an arbitrary commit. X' is given by:
// X' = MergedTree::merge{ MCTNR(P) + (X.tree - X^) } =
//    = MergedTree::merge{ MCTNR(P) + (X.tree - MCTNR(X.parents())) }
//
// converge_trees returns:
// Solution = MergedTree::merge{ F' + (A' - F') + (B' - F') + (C' - F') }
//
// What is F? What is a "good base for converging trees"? F is calculated as
// follows:
// 1. For each commit X in the truncated evolution graph, we calculate
//    X'.tree_ids()
// 2. We build the "Value Transition Graph" of the values from step 1, with
//    edges between values corresponding to edges in the truncated evolution
//    graph: if commit X is a predecessor of commit Y, then the value transition
//    graph has an edge from X'.tree_ids() to Y'.tree_ids()
// 3. We find the dominator value of this Value Transition Graph
// 4. The dominator value is "produced" from one or more commits in the
//    truncated evolution graph
// 5. F is any of those producer commits (we pick the first one)
async fn converge_trees(
    _truncated_evolution_graph: &TruncatedEvolutionGraph,
    _parents: &[CommitId],
) -> Result<MergedTree, ConvergeError> {
    todo!()
}

// Creates a merge of values, using as terms the values of the divergent
// commits, and as base the dominator value. Returns the merge together with the
// commit id of one of the commits that produces the dominator value. Commits in
// excluded_divergent_commits are not used in the merge.
async fn create_value_merge<T, VF>(
    graph: &TruncatedEvolutionGraph,
    excluded_divergent_commits: &HashSet<CommitId>,
    value_fn: VF,
) -> Result<(Merge<T>, CommitId), ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: AsyncFn(&Commit) -> Result<T, ConvergeError>,
{
    let mut value_cache = ValueCache::new(async |commit_id: &CommitId| {
        let commit = graph.repo().store().get_commit_async(commit_id).await?;
        value_fn(&commit).await
    });

    let divergent_commits = graph
        .divergent_commit_ids()
        .iter()
        .filter(|id| !excluded_divergent_commits.contains(*id));

    // Calculate the dominator value on the value flow graph, and record which
    // commits produce which values.
    let dominator_value = graph
        .flow_graph
        .find_dominator_value_with_value_cache(divergent_commits.clone(), &mut value_cache)
        .await
        .map_err(|e| ConvergeError::Other(e.into()))?;
    let dominator_producer = get_value_producer(graph, &dominator_value, &value_cache)?;

    let mut merge_builder = MergeBuilder::default();
    // ADD
    merge_builder.extend([(*dominator_value).clone()]);
    for divergent_commit in divergent_commits {
        let commit_value = value_cache.get_value(divergent_commit).await?;
        // REMOVE, ADD
        merge_builder.extend([(*dominator_value).clone(), (*commit_value).clone()]);
    }
    Ok((merge_builder.build(), dominator_producer))
}

/// Returns a commit that produces a given value (e.g. finds a commit that
/// produces a given description). The value must be present in value_cache.
fn get_value_producer<T, VF>(
    truncated_evolution_graph: &TruncatedEvolutionGraph,
    value: &Rc<T>,
    value_cache: &ValueCache<CommitId, T, VF>,
) -> Result<CommitId, ConvergeError>
where
    T: Eq + Hash,
    VF: AsyncFn(&CommitId) -> Result<T, ConvergeError>,
{
    let producers = value_cache.get_nodes_for_value(value).unwrap();
    match producers.len() {
        0 => unreachable!(), // If it is present in ValueCache, it comes from some commit.
        1 => return Ok(producers[0].clone()),
        _ => {}
    }

    // If there is more than one producer we choose the one of minimum rank, where
    // rank is defined as lowest change-offset. Because some backends may not
    // provide change-offsets for hidden commits, we consider those as having
    // maximum change-offset and use input-order as the secondary sorting criterion.
    // By input-order we refer to the order of commits passed to converge_change.
    // But some commits are not given as input, so we use CommitId as tertiary
    // sorting criterion.

    let resolved_change_targets = truncated_evolution_graph
        .repo()
        .resolve_change_id(truncated_evolution_graph.change_id())?;
    let input_position: HashMap<&CommitId, usize> = truncated_evolution_graph
        .divergent_commit_ids()
        .iter()
        .enumerate()
        .map(|(position, commit_id)| (commit_id, position))
        .collect();
    let producer = producers
        .iter()
        .min_by_key(|commit_id: &&CommitId| {
            let change_offset = match &resolved_change_targets {
                Some(change_targets) => change_targets.find_offset(commit_id).unwrap_or(usize::MAX),
                None => usize::MAX,
            };
            let input_position = *input_position.get(commit_id).unwrap_or(&usize::MAX);
            (change_offset, input_position, *commit_id)
        })
        .unwrap()
        .clone();
    Ok(producer)
}

fn validate(predicate: bool, msg: &str) -> Result<(), ConvergeError> {
    if !predicate {
        Err(ConvergeError::Other(msg.into()))
    } else {
        Ok(())
    }
}
