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
use std::hash::Hash;
use std::sync::Arc;

use indexmap::IndexMap;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::TreeId;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::evolution::walk_predecessors;
use jj_lib::graph_dominators::FlowGraph;
use jj_lib::graph_dominators::SimpleDirectedGraph;
use jj_lib::merge::Merge;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetIteratorExt as _;
use pollster::FutureExt as _;
use thiserror::Error;

/// Maps change-ids to commits with that change-id.
pub type CommitsByChangeId = HashMap<ChangeId, HashMap<CommitId, Commit>>;

/// Encapsulates the solution to a problem, where the problem may be divergence
/// as a whole, or determining a specific aspect of the solution such
/// as the author, description, parents or tree of the converge commit.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ConvergeResult<T> {
    /// The proposed solution.
    Solution(T),
    /// Need user input to find a solution, but there is no ConvergeUI available
    /// to provide that input.
    NeedUserInput(String),
    /// The user aborted the operation.
    Aborted,
}

/// The proposed solution for converging a change.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ConvergeCommit {
    /// The change-id of the change being converged.
    pub change_id: ChangeId,
    /// The divergent commits that are being converged.
    pub divergent_commit_ids: Vec<CommitId>,
    /// The proposed author.
    pub author: Signature,
    /// The proposed description.
    pub description: String,
    /// The proposed parents.
    pub parents: Vec<CommitId>,
    /// The proposed tree IDs.
    pub tree_ids: Merge<TreeId>,
    /// Conflict labels.
    pub conflict_labels: ConflictLabels,
}

/// Errors that can occur during converge.
#[derive(Debug, Error)]
pub enum ConvergeError {
    /// A backend error occurred.
    #[error(transparent)]
    Backend(#[from] BackendError),
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

/// Interface for user interactions during converge. This is only available
/// during interactive converge, to communicate with the user whenever input is
/// required.
pub trait ConvergeUI {
    /// Prompts the user to choose a change-id to converge.
    ///
    /// Converge returns immediately if this method returns None. This method is
    /// only invoked if there are multiple divergent change-ids.
    fn choose_change<'a>(
        &self,
        divergent_changes: &'a CommitsByChangeId,
    ) -> Result<Option<&'a ChangeId>, ConvergeError>;

    /// Prompts the user to choose the author for the solution commit.
    fn choose_author(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Option<Signature>, ConvergeError>;

    /// Prompts the user to choose the parents for the solution commit.
    fn choose_parents(
        &self,
        divergent_commits: &[Commit],
    ) -> Result<Option<Vec<CommitId>>, ConvergeError>;

    /// Prompts the user to merge the description.
    fn merge_description(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Option<String>, ConvergeError>;
}

/// Evaluates the revset expression and returns those commits that are
/// divergent, in the sense that the expression matches two or more commits in
/// the result with the same change-id.
///
/// The commits are keyed by their change-id.
pub fn find_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    revset_expression: Arc<ResolvedRevsetExpression>,
) -> Result<CommitsByChangeId, RevsetEvaluationError> {
    let divergent_commits: Vec<Commit> = revset_expression
        .evaluate(repo.as_ref())?
        .iter()
        .commits(repo.store())
        .try_collect()?;

    let mut result = CommitsByChangeId::new();
    for commit in divergent_commits {
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

/// Prompts the user to choose a change-id to converge, if there are multiple
/// divergent change-ids.
pub fn choose_change<'a>(
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_changes: &'a CommitsByChangeId,
) -> Result<Option<&'a ChangeId>, ConvergeError> {
    match divergent_changes.len() {
        0 => return Ok(None),
        1 => return Ok(Some(divergent_changes.keys().next().unwrap())),
        _ => (),
    }
    // TODO: consider using heuristics to automatically choose a "good" change-id to
    // converge, falling back to prompting the user only if the heuristics are
    // inconclusive. This is specially important in non-interactive mode.
    match converge_ui {
        Some(converge_ui) => converge_ui.choose_change(divergent_changes),
        None => Ok(None),
    }
}

/// Attempts to solve divergence in the given divergent commits.
///
/// divergent_commits is expected to have two or more commits, all with the same
/// change-id, otherwise an error is returned. When traversing the evolution
/// graph of a divergent change, at most `max_evolution_nodes` commits are
/// traversed.
// TODO: consider also reporting some summary information about what's done.
// TODO: consider keeping track of stats and reporting those somehow.
// TODO: consider adding some logging???
pub async fn converge_change(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    max_evolution_nodes: usize,
) -> Result<ConvergeResult<Box<ConvergeCommit>>, ConvergeError> {
    if divergent_commits.len() <= 1 {
        return Err(ConvergeError::Other(
            format!(
                "Expected multiple divergent commits, got {}",
                divergent_commits.len()
            )
            .into(),
        ));
    }

    let truncated_evolution_graph =
        TruncatedEvolutionGraph::new(repo, divergent_commits, max_evolution_nodes)?;

    let author = match converge_author(converge_ui, divergent_commits, &truncated_evolution_graph)?
    {
        ConvergeResult::Solution(author) => author,
        ConvergeResult::NeedUserInput(msg) => return Ok(ConvergeResult::NeedUserInput(msg)),
        ConvergeResult::Aborted => return Ok(ConvergeResult::Aborted),
    };

    let description =
        match converge_description(converge_ui, divergent_commits, &truncated_evolution_graph)? {
            ConvergeResult::Solution(description) => description,
            ConvergeResult::NeedUserInput(msg) => return Ok(ConvergeResult::NeedUserInput(msg)),
            ConvergeResult::Aborted => return Ok(ConvergeResult::Aborted),
        };

    let parents = match converge_parents(repo, converge_ui, &truncated_evolution_graph)? {
        ConvergeResult::Solution(parents) => parents,
        ConvergeResult::NeedUserInput(msg) => return Ok(ConvergeResult::NeedUserInput(msg)),
        ConvergeResult::Aborted => return Ok(ConvergeResult::Aborted),
    };

    let tree = converge_trees(
        repo,
        divergent_commits,
        &truncated_evolution_graph,
        &parents,
    )
    .await?;

    Ok(ConvergeResult::Solution(Box::new(ConvergeCommit {
        change_id: truncated_evolution_graph.change_id().clone(),
        divergent_commit_ids: truncated_evolution_graph.divergent_commit_ids.clone(),
        author,
        description,
        parents,
        tree_ids: tree.tree_ids().clone(),
        conflict_labels: tree.labels().clone(),
    })))
}

/// Adds a new commit for the proposed solution, as a successor of the divergent
/// commits.
///
/// If rewrite_divergent_commits is true, the divergent commits are rewritten
/// and their descendants are rebased on top of it. Otherwise the new
/// commit gets a new change-id, and the divergent commits are left unchanged.
pub fn apply_solution(
    solution: Box<ConvergeCommit>,
    rewrite_divergent_commits: bool,
    repo_mut: &mut MutableRepo,
) -> Result<(Commit, usize), ConvergeError> {
    let merged_tree = MergedTree::new(
        repo_mut.store().clone(),
        solution.tree_ids.clone(),
        solution.conflict_labels.clone(),
    );
    let commit_builder = repo_mut
        .new_commit(solution.parents, merged_tree)
        .set_description(solution.description)
        .set_author(solution.author)
        .set_predecessors(solution.divergent_commit_ids.clone());
    let new_commit = if rewrite_divergent_commits {
        let commit = commit_builder
            .set_change_id(solution.change_id.clone())
            .write()
            .block_on()?;
        for divergent_commit_id in solution.divergent_commit_ids {
            repo_mut.set_rewritten_commit(divergent_commit_id, commit.id().clone());
        }
        commit
    } else {
        commit_builder.write().block_on()?
    };
    let num_rebased = repo_mut.rebase_descendants().block_on()?;
    Ok((new_commit, num_rebased))
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id, and only goes as far as
/// the closest common dominator of the divergent commits.
pub struct TruncatedEvolutionGraph {
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    pub divergent_commit_ids: Vec<CommitId>,
    /// The evolution graph of the divergent commits, with edges X->Y if commit
    /// X is a predecessor of commit Y and both X and Y have the same
    /// divergent change-id. The graph is not necessarily a tree (commits
    /// may have multiple predecessors). The start node is the evolution
    /// fork point.
    pub flow_graph: FlowGraph<CommitId>,
    /// The evolution entries for the commits in the graph.
    pub commits: HashMap<CommitId, CommitEvolutionEntry>,
}

impl TruncatedEvolutionGraph {
    /// Builds a truncated evolution graph for the given divergent commits,
    /// which are expected to all have the same change-id.
    pub fn new(
        repo: &ReadonlyRepo,
        divergent_commits: &[Commit],
        max_evolution_nodes: usize,
    ) -> Result<Self, ConvergeError> {
        validate(
            !divergent_commits.is_empty(),
            "divergent_commits must not be empty",
        )?;
        let max_evolution_nodes = max_evolution_nodes.max(divergent_commits.len());

        let divergent_commit_ids = divergent_commits
            .iter()
            .map(|c| c.id().clone())
            .collect_vec();

        // Ensure all provided divergent commits belong to the same change-id.
        // Note: divergent_commits is not empty, so it is ok to unwrap.
        let divergent_change_id = divergent_commits.iter().next().unwrap().change_id().clone();
        for c in divergent_commits.iter().skip(1) {
            validate(
                *c.change_id() == divergent_change_id,
                "all divergent commits must have the same change-id",
            )?;
        }

        // The adjacency list, with commits pointing to their predecessors.
        let mut adj: IndexMap<CommitId, IndexSet<CommitId>> = IndexMap::new();
        let mut commits = HashMap::new();
        let evolution_nodes = walk_predecessors(repo, divergent_commit_ids.as_slice());

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
            match commits.entry(commit_id.clone()) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    // TODO: think about this some more. Can 2 different operations result in the
                    // same commit? Maybe the key should be (commit-id, operation-id).

                    // Note: currently walk_predecessors returns an error if the graph is cyclic, so
                    // we shouldn't encounter the same commit twice. But in the future we could
                    // allow cyclic evolution, and if we do there is no reason to disallow it here.
                    // By continuing we future proof this.
                    continue;
                }
                std::collections::hash_map::Entry::Vacant(e) => e.insert(entry.clone()),
            };
            if commits.len() >= max_evolution_nodes {
                adj.entry(commit_id.clone()).or_default();
                initial_nodes.push(commit_id.clone());
                break;
            }
            let predecessors: Vec<CommitId> = entry
                .predecessors()
                .filter_map_ok(|commit| {
                    if *commit.change_id() == divergent_change_id {
                        Some(commit.id().clone())
                    } else {
                        None
                    }
                })
                .try_collect()?;
            adj.entry(commit_id.clone())
                .or_default()
                .extend(predecessors.iter().cloned());
            if predecessors.is_empty() {
                initial_nodes.push(commit_id.clone());
            }
        }

        validate(
            !initial_nodes.is_empty(),
            "Unexpected error: initial_nodes should not be empty",
        )?;

        // To compute the evolution fork point (see below) there must be a single
        // "initial node". In graphs with multiple "real" initial nodes we introduce a
        // virtual initial node (the root commit) and pretend the two or more "real"
        // initial nodes are successors of the root commit.
        let initial_node = if initial_nodes.len() == 1 {
            initial_nodes[0].clone()
        } else {
            let root_commit_id = repo.store().root_commit_id().clone();
            commits.insert(
                root_commit_id.clone(),
                CommitEvolutionEntry::for_root_commit(repo.store()),
            );
            adj.entry(root_commit_id.clone()).or_default();
            for initial_node in &initial_nodes {
                adj.entry(initial_node.clone())
                    .or_default()
                    .insert(root_commit_id.clone());
            }
            root_commit_id
        };

        let flow_graph = FlowGraph::new(SimpleDirectedGraph::new(adj).reverse(), initial_node);

        // The evolution fork point is the "closest common dominator" of the set of
        // divergent commits in the reverse truncated evolution graph (with edge U->V
        // when commit V is a successor of commit U). To compute it, there must be a
        // single "entry node" in the (reverse) graph. The logic above ensures this
        // condition is satisfied, thus the closest common dominator is
        // guaranteed to exist (although it could happen to be the virtual
        // initial node).
        let evolution_fork_point =
            match flow_graph.find_closest_common_dominator(divergent_commit_ids.clone()) {
                Some(dominator) => dominator.clone(),
                None => {
                    // Should not happen since we added a virtual initial node.
                    return Err(ConvergeError::Other("Unexpected error".into()));
                }
            };

        // Remove irrelevant commits.
        let (graph, _) = flow_graph.consume();
        let flow_graph =
            FlowGraph::new(graph, evolution_fork_point.clone()).trim(&divergent_commit_ids);
        commits.retain(|commit_id, _| flow_graph.graph.contains_node(commit_id));

        Ok(Self {
            divergent_commit_ids,
            flow_graph,
            commits,
        })
    }

    /// Returns the change-id of the commits in the graph.
    pub fn change_id(&self) -> &ChangeId {
        self.commits.values().next().unwrap().commit.change_id()
    }

    /// Returns the commit for the given commit id.
    pub fn get_commit(&self, commit_id: &CommitId) -> Result<&Commit, ConvergeError> {
        let node = self.commits.get(commit_id).ok_or(ConvergeError::Other(
            format!("Unexpected commit id: {commit_id}").into(),
        ))?;
        Ok(&node.commit)
    }

    /// Returns the evolution fork point of the graph, which is the closest
    /// common dominator of the commits (in the reverse graph). In other words,
    /// this is the commit from which all the commits in the graph evolved, and
    /// that is closest to those commits.
    pub fn get_evolution_fork_point(&self) -> &Commit {
        // Note: evolution_fork_point is guaranteed to be in the graph, so this unwrap
        // should never fail.
        self.get_commit(&self.flow_graph.start_node).unwrap()
    }
}

fn converge_author(
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<Signature>, ConvergeError> {
    let value_fn = |c: &Commit| Ok(c.author().clone());
    if let Some(value) = converge(divergent_commits, graph, value_fn)? {
        return Ok(ConvergeResult::Solution(value));
    }
    let ui_chooser = |converge_ui: &dyn ConvergeUI| {
        converge_ui.choose_author(divergent_commits, graph.get_evolution_fork_point())
    };
    converge_interactively(converge_ui, ui_chooser, "author")
}

fn converge_description(
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<String>, ConvergeError> {
    let value_fn = |c: &Commit| Ok(c.description().to_string());
    if let Some(value) = converge(divergent_commits, graph, value_fn)? {
        return Ok(ConvergeResult::Solution(value));
    }
    let ui_chooser = |converge_ui: &dyn ConvergeUI| {
        converge_ui.merge_description(divergent_commits, graph.get_evolution_fork_point())
    };
    converge_interactively(converge_ui, ui_chooser, "description")
}

fn converge_parents(
    _repo: &Arc<ReadonlyRepo>,
    _converge_ui: Option<&dyn ConvergeUI>,
    _graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<Vec<CommitId>>, ConvergeError> {
    todo!()
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
    _repo: &Arc<ReadonlyRepo>,
    _divergent_commits: &[Commit],
    _truncated_evolution_graph: &TruncatedEvolutionGraph,
    _parents: &[CommitId],
) -> Result<MergedTree, ConvergeError> {
    todo!()
}

fn converge<T, VF>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: VF,
) -> Result<Option<T>, ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    let dominator_value = find_dominator_value(graph, &value_fn)?;
    // Create a merge of values, using as terms the values of the divergent commits,
    // and as base the value of the closest common dominator in the value
    // history graph (closest to the values of the divergent commits).
    let mut merge_builder = MergeBuilder::default();
    // ADD
    merge_builder.extend([dominator_value.clone()]);
    for divergent_commit in divergent_commits {
        let commit_value = value_fn(divergent_commit)?;
        // REMOVE, ADD
        merge_builder.extend([dominator_value.clone(), commit_value]);
    }
    let merge = merge_builder.build();
    Ok(merge.resolve_trivial(SameChange::Accept).cloned())
}

fn find_dominator_value<T, VF>(
    graph: &TruncatedEvolutionGraph,
    value_fn: &VF,
) -> Result<T, ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    let dominator_value = graph
        .flow_graph
        .find_dominator_value(&graph.divergent_commit_ids, |commit_id: &CommitId| {
            value_fn(graph.get_commit(commit_id)?)
        })?;
    // By construction the dominator value always exists, so it is safe to unwrap
    // here.
    Ok(dominator_value.unwrap().clone())
}

fn converge_interactively<T, F>(
    converge_ui: Option<&dyn ConvergeUI>,
    ui_chooser: F,
    attribute: &str,
) -> Result<ConvergeResult<T>, ConvergeError>
where
    T: Eq + Hash + Clone,
    F: FnOnce(&dyn ConvergeUI) -> Result<Option<T>, ConvergeError>,
{
    let Some(converge_ui) = converge_ui else {
        return Ok(ConvergeResult::NeedUserInput(format!(
            "cannot converge {attribute} automatically"
        )));
    };
    match ui_chooser(converge_ui)? {
        Some(value) => Ok(ConvergeResult::Solution(value)),
        None => Ok(ConvergeResult::Aborted),
    }
}

fn validate(predicate: bool, msg: &str) -> Result<(), ConvergeError> {
    if !predicate {
        Err(ConvergeError::Other(msg.into()))
    } else {
        Ok(())
    }
}
