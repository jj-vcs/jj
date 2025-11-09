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
use std::rc::Rc;
use std::sync::Arc;

use futures::TryStreamExt as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::TreeId;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::graph_dominators::FlowGraph;
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
        base_commit: &Commit,
    ) -> Result<Option<String>, ConvergeError>;
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
/// change-id, otherwise an error is returned.
// TODO: consider also reporting some summary information about what's done.
// TODO: consider keeping track of stats and reporting those somehow.
// TODO: consider adding some logging???
pub async fn converge_change(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
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

    let truncated_evolution_graph = TruncatedEvolutionGraph::new(repo, divergent_commits).await?;

    let author = match converge_author(
        repo,
        converge_ui,
        divergent_commits,
        &truncated_evolution_graph,
    )
    .await?
    {
        ConvergeResult::Solution(author) => author,
        ConvergeResult::NeedUserInput(msg) => return Ok(ConvergeResult::NeedUserInput(msg)),
        ConvergeResult::Aborted => return Ok(ConvergeResult::Aborted),
    };

    let description = match converge_description(
        repo,
        converge_ui,
        divergent_commits,
        &truncated_evolution_graph,
    )
    .await?
    {
        ConvergeResult::Solution(description) => description,
        ConvergeResult::NeedUserInput(msg) => return Ok(ConvergeResult::NeedUserInput(msg)),
        ConvergeResult::Aborted => return Ok(ConvergeResult::Aborted),
    };

    let parents = match converge_parents(repo, converge_ui, &truncated_evolution_graph).await? {
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
pub async fn apply_solution(
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
            .await?;
        for divergent_commit_id in solution.divergent_commit_ids {
            repo_mut.set_rewritten_commit(divergent_commit_id, commit.id().clone());
        }
        commit
    } else {
        commit_builder.write().await?
    };
    let num_rebased = repo_mut.rebase_descendants().await?;
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
    pub async fn new(
        _repo: &ReadonlyRepo,
        _divergent_commits: &[Commit],
    ) -> Result<Self, ConvergeError> {
        todo!()
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
}

async fn converge_author(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<Signature>, ConvergeError> {
    let value_fn = async |c: &Commit| Ok(c.author().clone());
    let (value_merge, _base_commit) =
        create_value_merge(repo, divergent_commits, graph, value_fn).await?;
    if let Some(value) = value_merge.resolve_trivial(SameChange::Accept) {
        return Ok(ConvergeResult::Solution(value.clone()));
    }
    let ui_chooser = |converge_ui: &dyn ConvergeUI| converge_ui.choose_author(divergent_commits);
    converge_interactively(converge_ui, ui_chooser, "author")
}

async fn converge_description(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<String>, ConvergeError> {
    let value_fn = async |c: &Commit| Ok(c.description().to_string());
    let (value_merge, base_commit) =
        create_value_merge(repo, divergent_commits, graph, value_fn).await?;
    if let Some(value) = value_merge.resolve_trivial(SameChange::Accept) {
        return Ok(ConvergeResult::Solution(value.clone()));
    }
    let ui_chooser = |converge_ui: &dyn ConvergeUI| {
        let base_commit = graph.get_commit(&base_commit)?;
        converge_ui.merge_description(divergent_commits, base_commit)
    };
    converge_interactively(converge_ui, ui_chooser, "description")
}

async fn converge_parents(
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

// Creates a merge of values, using as terms the values of the divergent
// commits, and as base the value the dominator value. Returns the merge
// together with the commit id of one of the commits that produces the dominator
// value.
async fn create_value_merge<T, VF>(
    repo: &Arc<ReadonlyRepo>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: VF,
) -> Result<(Merge<T>, CommitId), ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: AsyncFn(&Commit) -> Result<T, ConvergeError>,
{
    let mut value_cache =
        ValueCache::new(async |commit_id: &CommitId| value_fn(graph.get_commit(commit_id)?).await);
    let dominator_value = find_dominator_value(graph, divergent_commits, &mut value_cache).await?;
    let dominator_producer = get_value_producer(repo, graph, &dominator_value, &value_cache)?;

    let mut merge_builder = MergeBuilder::default();
    // ADD
    merge_builder.extend([(*dominator_value).clone()]);
    for divergent_commit in divergent_commits {
        let commit_value = value_cache.get_value(divergent_commit.id()).unwrap();
        // REMOVE, ADD
        merge_builder.extend([(*dominator_value).clone(), (**commit_value).clone()]);
    }
    Ok((merge_builder.build(), dominator_producer))
}

async fn find_dominator_value<T, VF>(
    _graph: &TruncatedEvolutionGraph,
    _divergent_commits: &[Commit],
    _value_cache: &mut ValueCache<CommitId, T, VF>,
) -> Result<Rc<T>, ConvergeError>
where
    T: Eq + Hash,
    VF: AsyncFn(&CommitId) -> Result<T, ConvergeError>,
{
    todo!();
}

/// Returns a commit that produces a given value (e.g. finds a commit that
/// produces a given description). The value must be present in value_cache.
fn get_value_producer<T, VF>(
    _repo: &Arc<ReadonlyRepo>,
    _truncated_evolution_graph: &TruncatedEvolutionGraph,
    _value: &Rc<T>,
    _value_cache: &ValueCache<CommitId, T, VF>,
) -> Result<CommitId, IndexError>
where
    T: Eq + Hash,
    VF: AsyncFn(&CommitId) -> Result<T, ConvergeError>,
{
    todo!();
}

fn converge_interactively<T, F>(
    converge_ui: Option<&dyn ConvergeUI>,
    ui_chooser: F,
    attribute: &str,
) -> Result<ConvergeResult<T>, ConvergeError>
where
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
