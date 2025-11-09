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
use std::collections::VecDeque;
use std::hash::Hash;
use std::ops::Deref as _;
use std::sync::Arc;

use futures::future::try_join_all;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::commit::Commit;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::evolution::walk_predecessors;
use jj_lib::merge::Merge;
use jj_lib::merge::MergeBuilder;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::rewrite::merge_commit_trees;
use pollster::FutureExt as _;
use thiserror::Error;

/// Maps change-ids to commits with that change-id.
pub type CommitsByChangeId = HashMap<ChangeId, Vec<Commit>>;

/// The proposed solution for converging a change.
pub struct ConvergeSolution {
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
    /// The proposed tree.
    pub tree: MergedTree,
}

#[expect(missing_docs)]
#[derive(Debug, Error)]
pub enum ConvergeError {
    #[error("There are no divergent changes")]
    NoDivergentChanges(),
    #[error("Need user input")]
    NeedUserInput(),
    #[error("User aborted")]
    UserAborted(),
    #[error("Too many commits in change evolution history, unable to converge")]
    TooManyCommitsInChangeEvolution(),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
    #[error(transparent)]
    WalkPredecessors(#[from] WalkPredecessorsError),
    #[error(transparent)]
    IO(#[from] std::io::Error),
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
    ) -> Result<&'a ChangeId, ConvergeError>;

    /// Prompts the user to choose the author for the solution commit.
    fn choose_author(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Signature, ConvergeError>;

    /// Prompts the user to choose the parents for the solution commit.
    fn choose_parents(&self, divergent_commits: &[Commit]) -> Result<Vec<CommitId>, ConvergeError>;

    /// Prompts the user to merge the description, given the divergent commits
    /// and evolution_fork_point.
    fn merge_description(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<String, ConvergeError>;
}

/// A non-interactive UI that always returns ConvergeError::NeedUserInput.
pub struct NonInteractiveUI {}

/// Attempts to solve divergence, if present in the commits given by the revset
/// expression, using the given UI interface for interacting with the user.
///
/// When traversing the evolution graph of a divergent change, at most
/// `max_evolution_nodes` commits are traversed.
pub async fn propose_divergence_solution(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: &dyn ConvergeUI,
    divergent_change_search_space: Arc<ResolvedRevsetExpression>,
    max_evolution_nodes: usize,
) -> Result<ConvergeSolution, ConvergeError> {
    let divergent_changes = find_divergent_changes(repo, divergent_change_search_space)?;
    if divergent_changes.is_empty() {
        return Err(ConvergeError::NoDivergentChanges());
    }
    let change_id = choose_change(converge_ui, &divergent_changes)?;
    // Note: change_id is one of the keys in divergent_changes, so this unwrap
    // should never fail.
    let divergent_commits = divergent_changes.get(change_id).unwrap();
    converge_change(
        repo,
        converge_ui,
        change_id.clone(),
        divergent_commits,
        max_evolution_nodes,
    )
    .await
}

/// Adds a new commit for the proposed solution, succeeding the divergent
/// commits, and rebases all their descendants on top of it.
pub fn apply_solution(
    solution: ConvergeSolution,
    repo_mut: &mut MutableRepo,
) -> Result<(Commit, usize), ConvergeError> {
    let new_commit = repo_mut
        .new_commit(solution.parents.clone(), solution.tree.clone())
        .set_change_id(solution.change_id.clone())
        .set_description(solution.description)
        .set_author(solution.author)
        .set_predecessors(solution.divergent_commit_ids)
        .write()
        .block_on()?;
    let num_rebased = repo_mut.rebase_descendants().block_on()?;
    Ok((new_commit, num_rebased))
}

/// Evaluates the provided revset expression and returns those commits that are
/// part of a divergent change, in the sense that the expression matches two or
/// more commits in the result with the same change-id.
///
/// The commits are keyed by their change-id.
fn find_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    revset_expression: Arc<ResolvedRevsetExpression>,
) -> Result<CommitsByChangeId, ConvergeError> {
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
            .push(commit);
    }

    // Remove entries that have only a single commit — we only care about
    // changes with multiple divergent commits.
    result.retain(|_, commits| commits.len() > 1);

    Ok(result)
}

/// A node in the truncated evolution graph.
struct TruncatedEvolutionNode {
    /// The evolution entry for the commit represented by this node. Note: if
    /// reachable_predecessors is present, it contains *all* reachable
    /// predecessors, even those belonging to unrelated change-ids.
    entry: CommitEvolutionEntry,
    /// The predecessors of this node's commit in the truncated evolution graph.
    /// These are those predecessors of the commit that have the same
    /// change-id as the commit, other predecessors are not included here.
    predecessors: Vec<CommitId>,
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id.
struct TruncatedEvolutionGraph {
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    divergent_commit_ids: Vec<CommitId>,
    /// The nodes, keyed by commit id. The graph is not necessarily a tree,
    /// since commits may have multiple predecessors.
    nodes: HashMap<CommitId, TruncatedEvolutionNode>,
    /// The closest choke point of the divergent commits in the truncated
    /// evolution graph.
    evolution_fork_point: CommitId,
}

impl TruncatedEvolutionGraph {
    /// Builds a truncated evolution graph for the given divergent commits,
    /// which are expected to all have the same change-id.
    fn new(
        repo: &ReadonlyRepo,
        divergent_commits: &[Commit],
        max_evolution_nodes: usize,
    ) -> Result<Self, ConvergeError> {
        todo!()
    }

    fn get_evolution_fork_point(&self) -> Result<&Commit, ConvergeError> {
        // Note: evolution_fork_point is guaranteed to be a key in nodes, so this unwrap
        // should never fail.
        let fork_point = self.nodes.get(&self.evolution_fork_point).unwrap();
        Ok(&fork_point.entry.commit)
    }
}

fn choose_change<'a>(
    converge_ui: &dyn ConvergeUI,
    divergent_changes: &'a CommitsByChangeId,
) -> Result<&'a ChangeId, ConvergeError> {
    if divergent_changes.len() == 1 {
        // Note: divergent_changes is not empty, so it is ok to unwrap.
        let (change_id, _divergent_commits) = divergent_changes.iter().next().unwrap();
        return Ok(change_id);
    }
    // TODO: consider using heuristics to automatically choose a "good" change-id to
    // converge, falling back to prompting the user only if the heuristics are
    // inconclusive. This is specially important in non-interactive mode.
    converge_ui.choose_change(divergent_changes)
}

async fn converge_change(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: &dyn ConvergeUI,
    change_id: ChangeId,
    divergent_commits: &[Commit],
    max_evolution_nodes: usize,
) -> Result<ConvergeSolution, ConvergeError> {
    let truncated_evolution_graph =
        TruncatedEvolutionGraph::new(repo, divergent_commits, max_evolution_nodes)?;
    let fork_point = truncated_evolution_graph.get_evolution_fork_point()?;

    let author = solve_author(converge_ui, divergent_commits, &truncated_evolution_graph)?;
    let description =
        solve_description(converge_ui, divergent_commits, &truncated_evolution_graph)?;
    let parents = solve_parents(repo, converge_ui, &truncated_evolution_graph)?;

    let tree = converge_trees(
        repo,
        &truncated_evolution_graph.divergent_commit_ids,
        fork_point,
        &parents,
    )
    .await?;

    Ok(ConvergeSolution {
        change_id,
        divergent_commit_ids: truncated_evolution_graph.divergent_commit_ids.clone(),
        author,
        description,
        parents,
        tree,
    })
}

fn solve<T, F>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: fn(&Commit) -> T,
    ui_chooser: F,
) -> Result<T, ConvergeError>
where
    T: Eq + Hash + Clone,
    F: FnOnce(
        /* divergent_commits: */ &[Commit],
        &TruncatedEvolutionGraph,
    ) -> Result<T, ConvergeError>,
{
    let divergent_values: HashSet<T> = divergent_commits.iter().map(value_fn).collect();
    if divergent_values.len() == 1 {
        return Ok(divergent_values.into_iter().next().unwrap()); // Note: divergent_values is not empty.
    }
    let merge = create_merge(graph, value_fn)?;
    match merge.try_resolve_deduplicating_same_diffs() {
        Some(value) => Ok(value.clone()),
        None => ui_chooser(divergent_commits, graph),
    }
}

fn solve_author(
    converge_ui: &dyn ConvergeUI,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<Signature, ConvergeError> {
    let value_fn = |c: &Commit| c.author().clone();
    let ui_chooser = |commits: &[Commit], graph: &TruncatedEvolutionGraph| {
        converge_ui.choose_author(commits, graph.get_evolution_fork_point()?)
    };
    solve(divergent_commits, graph, value_fn, ui_chooser)
}

fn solve_description(
    converge_ui: &dyn ConvergeUI,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<String, ConvergeError> {
    let value_fn = |c: &Commit| c.description().to_string();
    let ui_chooser = |commits: &[Commit], graph: &TruncatedEvolutionGraph| {
        converge_ui.merge_description(commits, graph.get_evolution_fork_point()?)
    };
    solve(divergent_commits, graph, value_fn, ui_chooser)
}

fn solve_parents(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: &dyn ConvergeUI,
    graph: &TruncatedEvolutionGraph,
) -> Result<Vec<CommitId>, ConvergeError> {
    todo!()
}

async fn converge_trees(
    repo: &Arc<ReadonlyRepo>,
    divergent_commit_ids: &Vec<CommitId>,
    fork_point: &Commit,
    parents: &[CommitId],
) -> Result<MergedTree, ConvergeError> {
    todo!()
}

fn create_merge<T>(
    graph: &TruncatedEvolutionGraph,
    value_fn: fn(&Commit) -> T,
) -> Result<Merge<T>, ConvergeError> {
    let fork_point = graph.get_evolution_fork_point()?;

    // Add the base value.
    let mut merge_builder: MergeBuilder<T> = Default::default();
    merge_builder.extend([value_fn(fork_point)]);

    let mut to_visit = VecDeque::from(graph.divergent_commit_ids.clone());
    while !to_visit.is_empty() {
        let commit_id = to_visit.pop_front().unwrap();
        let node = graph.nodes.get(&commit_id).unwrap();
        for predecessor_commit_id in &node.predecessors {
            let predecessor_node = graph.nodes.get(predecessor_commit_id).unwrap();
            let node_value = value_fn(&node.entry.commit);
            let predecessor_value = value_fn(&predecessor_node.entry.commit);
            // Add a term corresponding to the predecessor->node edge (first the remove,
            // then the add).
            merge_builder.extend([predecessor_value, node_value]);
            if predecessor_commit_id != &graph.evolution_fork_point {
                to_visit.push_back(predecessor_commit_id.clone());
            }
        }
    }
    Ok(merge_builder.build())
}

impl ConvergeUI for NonInteractiveUI {
    fn choose_change<'a>(
        &self,
        _divergent_changes: &'a CommitsByChangeId,
    ) -> Result<&'a ChangeId, ConvergeError> {
        Err(ConvergeError::NeedUserInput())
    }

    fn choose_author(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Signature, ConvergeError> {
        Err(ConvergeError::NeedUserInput())
    }

    fn choose_parents(
        &self,
        _divergent_commits: &[Commit],
    ) -> Result<Vec<CommitId>, ConvergeError> {
        Err(ConvergeError::NeedUserInput())
    }

    fn merge_description(
        &self,
        _divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<String, ConvergeError> {
        Err(ConvergeError::NeedUserInput())
    }
}
