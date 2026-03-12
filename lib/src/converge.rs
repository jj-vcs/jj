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
use jj_lib::backend::BackendResult;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::TreeId;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::evolution::walk_predecessors;
use jj_lib::graph_dominators::find_closest_common_dominator;
use jj_lib::merge::Merge;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::rewrite::merge_commit_trees_no_resolve;
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
    match divergent_commits.len() {
        0 => {
            return Err(ConvergeError::Other(
                "divergent_commits must not be empty".into(),
            ));
        }
        1 => {
            return Err(ConvergeError::Other(
                format!(
                    "divergent_commits must have multiple commits, change-id: {}",
                    divergent_commits[0].change_id()
                )
                .into(),
            ));
        }
        _ => (),
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

impl TruncatedEvolutionNode {
    /// Returns the commit id represented by this node.
    pub fn commit_id(&self) -> &CommitId {
        self.entry.commit.id()
    }
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

        let mut nodes = HashMap::new();
        let evolution_nodes = walk_predecessors(repo, divergent_commit_ids.as_slice());

        // These are the commits in the graph that have no predecessors. Typically
        // there is exactly one entry in initial_nodes (the first commit for the
        // change-id).
        let mut initial_nodes = vec![];

        for node in evolution_nodes {
            let entry = node?;
            if *entry.commit.change_id() != divergent_change_id {
                continue;
            }
            if nodes.contains_key(entry.commit.id()) {
                // TODO: think about this some more. Can 2 different operations result in the
                // same commit? Maybe the key should be (commit-id, operation-id).

                // Note: currently walk_predecessors returns an error if the graph is cyclic, so
                // we shouldn't encounter the same commit twice. But in the future we could
                // allow cyclic evolution, and if we do there is no reason to disallow it here.
                // By continuing we future proof this.
                continue;
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

            if nodes.len() >= max_evolution_nodes {
                return Err(ConvergeError::TooManyCommitsInChangeEvolution());
            }

            if predecessors.is_empty() {
                initial_nodes.push(entry.commit.id().clone());
            }
            nodes.insert(
                entry.commit.id().clone(),
                TruncatedEvolutionNode {
                    entry,
                    predecessors,
                },
            );
        }

        validate(
            !initial_nodes.is_empty(),
            "Unexpected error: initial_nodes should not be empty",
        )?;

        // To compute the evolution fork point (see below) there must be a single
        // "initial node". In graphs with multiple "real" initial nodes we introduce a
        // virtual initial node (the root commit) and pretend the two or more "real"
        // initial nodes are successors of the root commit.
        if initial_nodes.len() > 1 {
            let root_commit_id = repo.store().root_commit_id().clone();
            nodes
                .entry(root_commit_id.clone())
                .or_insert(TruncatedEvolutionNode {
                    entry: CommitEvolutionEntry::for_root_commit(repo.store()),
                    predecessors: vec![],
                });
            for initial_node in &initial_nodes {
                let predecessors = &mut nodes.get_mut(initial_node).unwrap().predecessors;
                if predecessors.contains(&root_commit_id) {
                    continue;
                }
                predecessors.push(root_commit_id.clone());
            }
        }

        let evolution_fork_point =
            Self::compute_evolution_fork_point(&divergent_commit_ids, &nodes)?;
        Ok(Self {
            divergent_commit_ids,
            nodes,
            evolution_fork_point,
        })
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

    fn compute_evolution_fork_point(
        divergent_commit_ids: &[CommitId],
        nodes: &HashMap<CommitId, TruncatedEvolutionNode>,
    ) -> Result<CommitId, ConvergeError> {
        // The evolution fork point is the "closest common dominator" of the set of
        // divergent commits in the reverse truncated evolution graph (with edge U->V
        // when commit V is a successor of commit U). To compute it, there must be a
        // single "entry node" in the (reverse) graph. The logic above ensures this
        // condition is satisfied, thus the closest common dominator is
        // guaranteed to exist (although it could happen to be the virtual
        // initial node).

        let edges: Vec<_> = nodes
            .iter()
            .flat_map(|(commit_id, node)| {
                node.predecessors
                    .iter()
                    .map(move |predecessor_id| (predecessor_id.clone(), commit_id.clone()))
            })
            .collect();

        let dominator = find_closest_common_dominator(
            nodes.keys().cloned(),
            edges,
            divergent_commit_ids.iter().cloned(),
        );
        match dominator {
            Ok(Some(dominator)) => Ok(dominator),
            Ok(None) => {
                // Should not happen since we added a virtual initial node.
                Err(ConvergeError::Other("Unexpected error".into()))
            }
            Err(e) => {
                // Should not happen since our nodes2 and edges are well-formed.
                Err(ConvergeError::Other(
                    format!("Unexpected error: {e}").into(),
                ))
            }
        }
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
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<Vec<CommitId>>, ConvergeError> {
    // Filter out divergent commits that are descendants of other divergent commits
    // (we cannot use the parents of those commits because that would introduce
    // cycles when we rebase everything on top of the parents).
    let viable_commits = remove_descendants(repo, &graph.divergent_commit_ids)?;
    let get_parents_fn = |c: &Commit| Ok(c.parent_ids().to_vec());
    let viable_parents: HashSet<_> = viable_commits.iter().map(get_parents_fn).try_collect()?;
    if viable_parents.len() == 1 {
        return Ok(ConvergeResult::Solution(
            viable_parents.into_iter().next().unwrap(),
        ));
    }
    if let Some(value) = converge(&viable_commits, graph, get_parents_fn)? {
        return Ok(ConvergeResult::Solution(value));
    }
    // TODO: need to think about the best way to present the parent choices to the
    // user. There may be 10 divergent commits, 9 of them with parents {A, B} and 1
    // with parents {C, D}. Showing a list of 10 commit ids may not be the best way
    // to do this.
    let ui_chooser = converge_ui.map(|converge_ui| {
        |commits: &[Commit], _graph: &TruncatedEvolutionGraph| converge_ui.choose_parents(commits)
    });
    converge_interactively(&viable_commits, graph, ui_chooser, "parents")
}

// Assume A, B, C are the divergent commits, P is the solution parents (i.e. the
// parents chosen by converge_parents), and F is a commit producing the
// dominator value (of the trees).
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
    repo: &Arc<ReadonlyRepo>,
    divergent_commits: &[Commit],
    truncated_evolution_graph: &TruncatedEvolutionGraph,
    parents: &[CommitId],
) -> Result<MergedTree, ConvergeError> {
    let parent_commits: Vec<Commit> =
        try_join_all(parents.iter().map(|id| repo.store().get_commit_async(id))).await?;
    let parents_merged_tree = merge_commit_trees_no_resolve(repo.as_ref(), &parent_commits).await?;

    // We first compute the dominator value of the trees (in the value history graph of the trees), together
    // with the commit(s) that produce that tree. Any such commit is a good candidate to be used as the base
    // of the merge.

    let tree_ids_fn = |c: &Commit| {
        Ok(
            rebase_tree_onto_solution_parents(c, &parents_merged_tree, repo)
                .block_on()?
                .into_tree_ids(),
        )
    };
    let divergent_trees: HashSet<_> = divergent_commits.iter().map(&tree_ids_fn).try_collect()?;
    if divergent_trees.len() == 1 {
        return Ok(rebase_tree_onto_solution_parents(
            &divergent_commits[0],
            &parents_merged_tree,
            repo,
        )
        .block_on()?);
    }

    let (_dominator, producers) =
        find_dominator_value_and_producers(divergent_commits, truncated_evolution_graph, &tree_ids_fn)?;
    if producers.is_empty() {
        return Err(ConvergeError::Other(
            "Unexpected error: no producer commits found for the dominator tree".into(),
        ));
    }

    // The first "producer" is the base of our merge of trees.
    let base_commit = truncated_evolution_graph.get_commit(&producers[0])?;
    let base_commit_tree_labels = format!("converge base: {}", base_commit.conflict_label());
    let base_commit_parent_tree = base_commit.parent_tree_no_resolve(repo.as_ref()).await?;
    let base_commit_parent_tree_labels = format!(
        "converge base parent(s): {}",
        base_commit.parents_conflict_label().await?
    );

    let mut terms: Vec<(MergedTree, String)> = Vec::new();

    // First add the tree of the base commit "rebased" on top of the solution's parent(s).
    {
        // Add
        terms.push((
            parents_merged_tree.clone(),
            "converge solution parent(s)".to_string(),
        ));
        // Remove
        terms.push((
            base_commit_parent_tree.clone(),
            base_commit_parent_tree_labels.clone(),
        ));
        // Add
        terms.push((base_commit.tree(), base_commit_tree_labels.clone()));
    }

    for divergent_commit in divergent_commits {
        // Add the tree of each divergent commit "rebased" on top of the solution's
        // parent(s), minus the tree of the base commit "rebased" on top of the
        // solution's parent(s).

        // Remove
        terms.push((
            divergent_commit
                .parent_tree_no_resolve(repo.as_ref())
                .await?,
            format!(
                "divergent commit parents: {}",
                divergent_commit.parents_conflict_label().await?
            ),
        ));
        // Add
        terms.push((
            divergent_commit.tree(),
            format!("divergent commit: {}", divergent_commit.conflict_label()),
        ));
        // Remove
        terms.push((base_commit.tree(), base_commit_tree_labels.clone()));
        // Add
        terms.push((
            base_commit_parent_tree.clone(),
            base_commit_parent_tree_labels.clone(),
        ));
    }

    Ok(MergedTree::merge(MergeBuilder::from_iter(terms).build()).await?)
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

// A node in the value history graph. It can be either a "real" value (i.e. an
// attribute of a commit, for example the commit description), or a "virtual"
// value that we introduce to represent the single entry point of the graph.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
enum ValueTag<T> {
    Real(T),
    Virtual,
}

fn find_dominator_value<T, VF>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: &VF,
) -> Result<T, ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    let fork_point = graph.get_evolution_fork_point()?;
    let fork_point_value = value_fn(fork_point)?;

    let mut nodes: HashSet<ValueTag<T>> = HashSet::new();
    let mut edges: HashSet<(ValueTag<T>, ValueTag<T>)> = HashSet::new();
    let mut targets: HashSet<ValueTag<T>> = HashSet::new();
    let mut to_visit = VecDeque::with_capacity(divergent_commits.len());

    nodes.insert(ValueTag::Virtual);
    nodes.insert(ValueTag::Real(fork_point_value.clone()));

    // Add an edge from the virtual entry value to the fork point value, to ensure
    // the graph has a single entry node.
    edges.insert((ValueTag::Virtual, ValueTag::Real(fork_point_value.clone())));

    for divergent_commit in divergent_commits {
        let commit_value = value_fn(divergent_commit)?;
        targets.insert(ValueTag::Real(commit_value));
        to_visit.push_back(divergent_commit.id().clone());
    }

    while let Some(commit_id) = to_visit.pop_front() {
        let node = graph.nodes.get(&commit_id).unwrap();
        let node_value = value_fn(&node.entry.commit)?;
        nodes.insert(ValueTag::Real(node_value.clone()));
        for predecessor_commit_id in &node.predecessors {
            let predecessor_commit = graph.get_commit(predecessor_commit_id)?;
            let predecessor_value = value_fn(predecessor_commit)?;
            edges.insert((
                ValueTag::Real(predecessor_value),
                ValueTag::Real(node_value.clone()),
            ));
            if predecessor_commit_id != &graph.evolution_fork_point {
                to_visit.push_back(predecessor_commit_id.clone());
            }
        }
    }

    match find_closest_common_dominator(nodes, edges, targets.iter().cloned()) {
        Ok(Some(ValueTag::Real(value))) => Ok(value.clone()),
        Ok(Some(ValueTag::Virtual)) => Err(ConvergeError::Other(
            "Unexpected error: the common dominator should not be the virtual node".into(),
        )),
        Ok(None) => Err(ConvergeError::Other(
            "Unexpected error: no common dominator found".into(),
        )),
        Err(e) => Err(ConvergeError::Other(
            format!("Unexpected error: {e}").into(),
        )),
    }
}

// Similar to find_dominator_value, but also returns the commits that "produce"
// the dominator value (there may be multiple commits that produce the same
// value).
fn find_dominator_value_and_producers<T, VF>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: &VF,
) -> Result<(T, Vec<CommitId>), ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    // The nodes of the value history graph, along with the commits that produced
    // each value. Includes the virtual entry node.
    let mut nodes: HashMap<ValueTag<T>, Vec<CommitId>> =
        HashMap::with_capacity(divergent_commits.len());

    // The edges in the value history graph, including edges from the virtual entry
    // node.
    let mut edges: HashSet<(ValueTag<T>, ValueTag<T>)> =
        HashSet::with_capacity(divergent_commits.len());

    // The values of the divergent commits (we want to find the closest common
    // dominator to these values in the value history graph).
    let mut targets: HashSet<ValueTag<T>> = HashSet::with_capacity(divergent_commits.len());

    let mut to_visit = VecDeque::with_capacity(divergent_commits.len());

    for divergent_commit in divergent_commits {
        let commit_value = ValueTag::Real(value_fn(divergent_commit)?);
        nodes
            .entry(commit_value.clone())
            .or_default()
            .push(divergent_commit.id().clone());
        targets.insert(commit_value.clone());
        to_visit.push_back((divergent_commit.id().clone(), commit_value));
    }

    if nodes.len() == 1 {
        // All divergent commits have the same value for this aspect, so we can just use
        // that value in the solution.
        match nodes.iter().next().unwrap() {
            (ValueTag::Real(value), commit_ids) => {
                return Ok((value.clone(), commit_ids.clone()));
            }
            _ => unreachable!(),
        }
    }

    nodes.insert(ValueTag::Virtual, vec![]);

    let fork_point = graph.get_evolution_fork_point()?;
    let fork_point_value = ValueTag::Real(value_fn(fork_point)?);
    nodes
        .entry(fork_point_value.clone())
        .or_default()
        .push(fork_point.id().clone());
    // Add an edge from the virtual entry value to the fork point value, to ensure
    // the graph has a single entry node.
    edges.insert((ValueTag::Virtual, fork_point_value.clone()));

    while let Some((commit_id, node_value)) = to_visit.pop_front() {
        let node = graph.nodes.get(&commit_id).unwrap();
        for predecessor_commit_id in &node.predecessors {
            let predecessor_commit = graph.get_commit(predecessor_commit_id)?;
            let predecessor_value = ValueTag::Real(value_fn(predecessor_commit)?);
            nodes
                .entry(predecessor_value.clone())
                .or_default()
                .push(predecessor_commit_id.clone());
            edges.insert((predecessor_value.clone(), node_value.clone()));
            if predecessor_commit_id != &graph.evolution_fork_point {
                to_visit.push_back((predecessor_commit_id.clone(), predecessor_value));
            }
        }
    }

    let dominator = match find_closest_common_dominator(
        nodes.keys().cloned(),
        edges.iter().cloned(),
        targets.iter().cloned(),
    ) {
        Ok(Some(dominator)) => Ok(dominator),
        Ok(None) => Err(ConvergeError::Other(
            "Unexpected error: no common dominator found".into(),
        )),
        Err(e) => Err(ConvergeError::Other(
            format!("Unexpected error: {e}").into(),
        )),
    }?;
    let dominator_producers = nodes.get(&dominator).ok_or_else(|| {
        ConvergeError::Other("Unexpected error: dominator value not found in nodes".into())
    })?;
    let dominator_value = match dominator {
        ValueTag::Real(value) => Ok(value),
        ValueTag::Virtual => Err(ConvergeError::Other(
            "Unexpected error: the common dominator should not be the virtual node".into(),
        )),
    }?;
    Ok((dominator_value.clone(), dominator_producers.clone()))
}

async fn rebase_tree_onto_solution_parents(
    c: &Commit,
    parents_merged_tree: &MergedTree,
    repo: &Arc<ReadonlyRepo>,
) -> BackendResult<MergedTree> {
    rebase_tree_onto_solution_parents_no_resolve(c, parents_merged_tree, repo)
        .await?
        .resolve()
        .await
}

async fn rebase_tree_onto_solution_parents_no_resolve(
    c: &Commit,
    parents_merged_tree: &MergedTree,
    repo: &Arc<ReadonlyRepo>,
) -> BackendResult<MergedTree> {
    let mut terms: Vec<(MergedTree, String)> = Vec::new();
    // Add
    terms.push((
        parents_merged_tree.clone(),
        "converge solution parent(s)".to_string(),
    ));
    // Remove
    terms.push((
        c.parent_tree_no_resolve(repo.as_ref()).await?,
        c.parents_conflict_label().await?,
    ));
    // Add
    terms.push((c.tree(), c.conflict_label()));
    Ok(MergedTree::merge_no_resolve(
        MergeBuilder::from_iter(terms).build(),
    ))
}

/// Returns those commits in commit_ids that are not descendants of any other
/// commit in commit_ids.
fn remove_descendants(
    repo: &Arc<ReadonlyRepo>,
    commit_ids: &[CommitId],
) -> Result<Vec<Commit>, ConvergeError> {
    if commit_ids.is_empty() {
        return Ok(vec![]);
    }
    let commits = Arc::new(RevsetExpression::Commits(commit_ids.to_vec()));
    let revset_expression = commits.minus(&commits.children().descendants());
    let result: Vec<_> = revset_expression
        .evaluate(repo.deref())?
        .iter()
        .commits(repo.store())
        .try_collect()?;
    validate(
        !result.is_empty(),
        &format!("the result of remove_descendants should never be empty; commits: {commit_ids:?}"),
    )?;
    Ok(result)
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

fn validate(predicate: bool, msg: &str) -> Result<(), ConvergeError> {
    if !predicate {
        Err(ConvergeError::Other(msg.into()))
    } else {
        Ok(())
    }
}
