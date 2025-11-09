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
use std::sync::Arc;

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
    /// The evolution history of the divergent commits is too large.
    #[error("Too many commits in change evolution history, unable to converge")]
    TooManyCommitsInChangeEvolution(),
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
            "expected multiple divergent commits for the change-id".into(),
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

/// A node in the truncated evolution graph.
pub struct TruncatedEvolutionNode {
    /// The evolution entry for the commit represented by this node. Note: if
    /// reachable_predecessors is present, it contains *all* reachable
    /// predecessors, even those belonging to unrelated change-ids.
    pub entry: CommitEvolutionEntry,
    /// The predecessors of this node's commit in the truncated evolution graph.
    /// These are those predecessors of the commit that have the same
    /// change-id as the commit, other predecessors are not included here.
    pub predecessors: Vec<CommitId>,
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id.
pub struct TruncatedEvolutionGraph {
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    pub divergent_commit_ids: Vec<CommitId>,
    /// The nodes, keyed by commit id. The graph is not necessarily a tree,
    /// since commits may have multiple predecessors.
    pub nodes: HashMap<CommitId, TruncatedEvolutionNode>,
    /// The closest choke point of the divergent commits in the truncated
    /// evolution graph.
    pub evolution_fork_point: CommitId,
}

impl TruncatedEvolutionGraph {
    /// Builds a truncated evolution graph for the given divergent commits,
    /// which are expected to all have the same change-id.
    pub fn new(
        _repo: &ReadonlyRepo,
        _divergent_commits: &[Commit],
        _max_evolution_nodes: usize,
    ) -> Result<Self, ConvergeError> {
        todo!()
    }

    /// Returns the change-id of the commits in the graph.
    pub fn change_id(&self) -> &ChangeId {
        self.nodes.values().next().unwrap().entry.commit.change_id()
    }

    /// Returns the commit for the given commit id.
    pub fn get_commit(&self, commit_id: &CommitId) -> Result<&Commit, ConvergeError> {
        let node = self.nodes.get(commit_id).ok_or(ConvergeError::Other(
            format!("Unexpected commit id: {commit_id}").into(),
        ))?;
        Ok(&node.entry.commit)
    }

    /// Returns the evolution fork point of the graph, which is the closest
    /// common dominator of the commits (in the reverse graph). In other words,
    /// this is the commit from which all the commits in the graph evolved, and
    /// that is closest to those commits.
    pub fn get_evolution_fork_point(&self) -> Result<&Commit, ConvergeError> {
        // TODO: change return type to &Commit
        // Note: evolution_fork_point is guaranteed to be a key in nodes, so this unwrap
        // should never fail.
        self.get_commit(&self.evolution_fork_point)
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
    let ui_chooser = converge_ui.map(|converge_ui| {
        |commits: &[Commit], graph: &TruncatedEvolutionGraph| {
            converge_ui.choose_author(commits, graph.get_evolution_fork_point()?)
        }
    });
    converge_interactively(divergent_commits, graph, ui_chooser, "author")
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
    let ui_chooser = converge_ui.map(|converge_ui| {
        |commits: &[Commit], graph: &TruncatedEvolutionGraph| {
            converge_ui.merge_description(commits, graph.get_evolution_fork_point()?)
        }
    });
    converge_interactively(divergent_commits, graph, ui_chooser, "description")
}

fn converge_parents(
    _repo: &Arc<ReadonlyRepo>,
    _converge_ui: Option<&dyn ConvergeUI>,
    _graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<Vec<CommitId>>, ConvergeError> {
    todo!()
}

// Assume A, B, C are the divergent commits, P is the solution parents (i.e. the
// parents chosen by converge_parents), and F is the evolution fork point.
//
// Notation:
// * N': commit N rebased on top of P
// * N^: the merged parents of commit N.
//
// converge_trees returns:

#[allow(clippy::empty_line_after_outer_attr)]
#[rustfmt::skip]
// T =    F'    +    (A'    -   F')     +   (B'     -   F')     +   (C'     -   F')
//   = (P+F-F^) + ((P+A-A^) - (P+F-F^)) + ((P+B-B^) - (P+F-F^)) + ((P+C-C^) - (P+F-F^))
//   = (P+F-F^) +   ((A-A^) -   (F-F^)) +   ((B-B^) -   (F-F^)) +   ((C-C^) -   (F-F^))
//   = (P+F-F^) +    (A-A^  +    F^-F)  +    (B-B^  +    F^-F)  +    (C-C^  +    F^-F)
//   = (P-F^+F) +   (-A^+A      -F+F^)  +   (-B^+B      -F+F^)  +   (-C^+C      -F+F^)

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
    let divergent_values: HashSet<T> = divergent_commits.iter().map(&value_fn).try_collect()?;
    if divergent_values.len() == 1 {
        return Ok(Some(divergent_values.into_iter().next().unwrap()));
    }
    let dominator_value = find_dominator_value::<T, VF>(divergent_commits, graph, &value_fn)?;
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

#[allow(unused)]
fn find_dominator_value<T, VF>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: &VF,
) -> Result<T, ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    todo!();
}

fn converge_interactively<T, F>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    ui_chooser: Option<F>,
    msg: &str,
) -> Result<ConvergeResult<T>, ConvergeError>
where
    T: Eq + Hash + Clone,
    F: FnOnce(&[Commit], &TruncatedEvolutionGraph) -> Result<Option<T>, ConvergeError>,
{
    match ui_chooser {
        Some(ui_chooser) => match ui_chooser(divergent_commits, graph)? {
            Some(value) => Ok(ConvergeResult::Solution(value)),
            None => Ok(ConvergeResult::Aborted),
        },
        None => Ok(ConvergeResult::NeedUserInput(format!(
            "cannot converge {msg} automatically"
        ))),
    }
}
