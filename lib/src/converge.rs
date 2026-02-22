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
use std::sync::Arc;

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
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetIteratorExt as _;
use thiserror::Error;

#[expect(missing_docs)]
pub type CommitsByChangeId = HashMap<ChangeId, HashSet<Commit>>;

/// Represents the state of a change that is being converged. Each field is a
/// merge of the corresponding field from the commits in the truncated evolution
/// graph. The merge is used to determine the values to use for the new commit
/// that will replace the commits being converged.
struct MergedState {
    /// A merge of the author.
    author: Merge<Signature>,
    /// A merge of the description.
    // TODO: add a separate field for description tags.
    description_text: Merge<String>,
    /// A merge of the parents.
    parents: Merge<Vec<CommitId>>,
}

/// A node in the truncated evolution graph.
struct TruncatedEvolutionNode {
    /// The evolution entry for the commit represented by this node. Note: if
    /// reachable_predecessors is present, it contains *all* reachable
    /// predecessors, even those belonging to unrelated change-ids.
    entry: CommitEvolutionEntry,
    /// The predecessors of this node in the truncated evolution graph. These
    /// are those predecessors of the commit that have the same change-id as the
    /// commit.
    predecessors: Vec<CommitId>,
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id. It may also be incomplete
/// if there are more than `max_nodes` commits in the evolution history with the
/// given change-id.
struct TruncatedEvolutionGraph {
    /// The change-id of the change being converged.
    divergent_change_id: ChangeId,
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    divergent_commit_ids: Vec<CommitId>,
    /// The nodes, keyed by commit id. The graph is not necessarily a tree,
    /// since commits may have multiple predecessors.
    nodes: HashMap<CommitId, TruncatedEvolutionNode>,
    /// The roots of the graph. If complete is true, then these are the commits
    /// with no predecessors. If complete is false then this also includes
    /// commits whose predecessors were not included in the graph due to the
    /// max_nodes limit. If complete is true then typically (pretty much always)
    /// there will be a single root, but if complete is false then there may be
    /// multiple roots.
    roots: HashSet<CommitId>,
    /// Whether the graph is complete. Tis is false there are more than
    /// `max_nodes` commits in the evolution history with the given change-id,
    /// and true otherwise.
    complete: bool,
}

#[expect(missing_docs)]
#[derive(Debug, Error)]
pub enum ConvergeError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
    #[error(transparent)]
    WalkPredecessors(#[from] WalkPredecessorsError),
    #[error("There are no divergent changes")]
    NoDivergentChanges(),
    #[error("Need user input")]
    NeedUserInput(),
    #[error("User aborted")]
    UserAborted(),
    #[error(transparent)]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// Interface for user interactions during converge. This is only available
/// during interactive converge, to communicate with the user whenever input is
/// required.
pub trait ConvergeUI {
    /// Prompts the user to choose a change-id to converge. Converge returns
    /// immediately if this method returns None. This method is only invoked
    /// if there are multiple divergent change-ids. If a ChangeId is returned,
    /// it must be one of the keys in divergent_commits.
    fn choose_change_id(
        &self,
        divergent_commits: &CommitsByChangeId,
    ) -> Result<Option<ChangeId>, ConvergeError>;

    /// Prompts the user to choose an author for the solution commit. The
    /// implemention must return the CommitId of either one of the divergent
    /// commits or the evolution fork point (if present). The author of the
    /// solution is the author of the chosen commit. divergent_commits is
    /// guaranteed to have two or more commits.
    ///
    /// Note: in the future we could allow the user to choose an arbitrary
    /// author, if we find a good reason to do that.
    fn choose_author(
        &self,
        divergent_commits: &HashSet<Commit>,
        evolution_fork_point: Option<&Commit>,
    ) -> Result<CommitId, ConvergeError>;

    /// Prompts the user to merge the description text (without any tags), given
    /// the divergent commits and evolution_fork_point.
    fn merge_description_text(
        &self,
        divergent_commits: &HashSet<Commit>,
        evolution_fork_point: Option<&Commit>,
    ) -> Result<String, ConvergeError>;

    /// Prompts the user to choose the parents for the solution commit. The
    /// implemention must return the CommitId of one of the divergent commits.
    /// The parents of the solution are the parents of the chosen commit.
    ///
    /// Note: in the future we could make this more flexible, if we find a good
    /// reason to do that.
    fn choose_parents(
        &self,
        divergent_commits: &HashSet<Commit>,
    ) -> Result<CommitId, ConvergeError>;
}

/// The proposed solution for converging a change.
pub struct ConvergeSolution {
    /// The change-id of the change being converged.
    pub change_id: ChangeId,
    /// The proposed author.
    pub author: Signature,
    /// The proposed description.
    pub description_text: String,
    /// The proposed parents.
    pub parents: Vec<CommitId>,
    /// The proposed tree.
    pub tree: MergedTree,
}

/// The result of an attempt to converge a divergent change.
pub enum ConvergeResult {
    ProposedSolution(Box<ConvergeSolution>),
}

pub fn converge(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&mut impl ConvergeUI>,
    target_expr: Arc<ResolvedRevsetExpression>,
    max_nodes: usize,
) -> Result<ConvergeResult, ConvergeError> {
    let divergent_commits = get_divergent_commits(repo, target_expr)?;
    if divergent_commits.is_empty() {
        return Err(ConvergeError::NoDivergentChanges());
    }
    let change_and_commits = choose_change(&converge_ui, &divergent_commits)?;
    if let Some((change_id, divergent_commits)) = change_and_commits {
        converge_change(repo, converge_ui, change_id, divergent_commits, max_nodes)
    } else {
        Err(ConvergeError::UserAborted())
    }
}

fn choose_change<'a>(
    converge_ui: &Option<&mut impl ConvergeUI>,
    divergent_commits: &'a CommitsByChangeId,
) -> Result<Option<(ChangeId, &'a HashSet<Commit>)>, ConvergeError> {
    if divergent_commits.len() == 1 {
        let (change_id, divergent_commits) = divergent_commits.iter().next().unwrap();
        return Ok(Some((change_id.clone(), divergent_commits)));
    }
    if converge_ui.is_none() {
        return Err(ConvergeError::NeedUserInput());
    }

    if let Some(change_id) = converge_ui
        .as_ref()
        .unwrap()
        .choose_change_id(divergent_commits)?
    {
        if let Some(divergent_commits) = divergent_commits.get(&change_id) {
            Ok(Some((change_id, divergent_commits)))
        } else {
            Err(ConvergeError::Other("invalid change-id chosen".into()))
        }
    } else {
        Ok(None)
    }
}

fn converge_change(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&mut impl ConvergeUI>,
    change_id: ChangeId,
    divergent_commits: &HashSet<Commit>,
    max_nodes: usize,
) -> Result<ConvergeResult, ConvergeError> {
    let truncated_evolution_graph =
        build_truncated_evolution_graph(repo, divergent_commits, max_nodes)?;
    let merged_state = merge_truncated_evolution_graph(&truncated_evolution_graph)?;

    let author = match merged_state.author.try_resolve_deduplicating_same_diffs() {
        Some(author) => Ok(author.clone()),
        None => match converge_ui {
            Some(ref ui) => {
                let commit_id = ui.choose_author(divergent_commits, None)?;
                let author = truncated_evolution_graph
                    .nodes
                    .get(&commit_id)
                    .ok_or_else(|| {
                        ConvergeError::Other("invalid commit id chosen for author".into())
                    })
                    .map(|node| node.entry.commit.author().clone())?;
                Ok(author)
            }
            None => Err(ConvergeError::NeedUserInput()),
        },
    }?;

    let description_text = match merged_state
        .description_text
        .try_resolve_deduplicating_same_diffs()
    {
        Some(description_text) => Ok(description_text.clone()),
        None => match converge_ui {
            Some(ref ui) => Ok(ui.merge_description_text(divergent_commits, None)?),
            None => Err(ConvergeError::NeedUserInput()),
        },
    }?;

    let parents = match merged_state.parents.try_resolve_deduplicating_same_diffs() {
        Some(parents) => Ok(parents.clone()),
        None => match converge_ui {
            Some(ref ui) => {
                let commit_id = ui.choose_parents(divergent_commits)?;
                let parents = truncated_evolution_graph
                    .nodes
                    .get(&commit_id)
                    .ok_or_else(|| {
                        ConvergeError::Other("invalid commit id chosen for parents".into())
                    })
                    .map(|node| node.entry.commit.parent_ids().to_vec())?;
                Ok(parents)
            }
            None => Err(ConvergeError::NeedUserInput()),
        },
    }?;

    let tree: MergedTree = converge_trees(repo, converge_ui, &truncated_evolution_graph, &parents)?;
    Ok(ConvergeResult::ProposedSolution(Box::new(
        ConvergeSolution {
            change_id,
            author,
            description_text,
            parents,
            tree,
        },
    )))
}

fn converge_trees(
    _repo: &Arc<ReadonlyRepo>,
    _converge_ui: Option<&mut impl ConvergeUI>,
    _truncated_evolution_graph: &TruncatedEvolutionGraph,
    _parents: &[CommitId],
) -> Result<MergedTree, ConvergeError> {
    todo!()
}

/// Evaluates the provided revset expression and returns those commits that are
/// part of a divergent change, in the sense that the expression matches two or
/// more commits in the result with the same change-id.
///
/// The commits are keyed by their change-id.
fn get_divergent_commits(
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
            .insert(commit);
    }

    // Remove entries that have only a single commit — we only care about
    // changes with multiple divergent commits.
    result.retain(|_, commits| commits.len() > 1);

    Ok(result)
}

/// Builds a truncated evolution graph for the given divergent commits, which
/// are expected to all have the same change-id.
fn build_truncated_evolution_graph(
    repo: &ReadonlyRepo,
    divergent_commits: &HashSet<Commit>,
    max_nodes: usize,
) -> Result<TruncatedEvolutionGraph, ConvergeError> {
    if divergent_commits.is_empty() {
        return Err(ConvergeError::Other("no divergent commits provided".into()));
    }
    let divergent_commit_ids = divergent_commits
        .iter()
        .map(|c| c.id().clone())
        .collect_vec();

    let divergent_change_id = divergent_commits.iter().next().unwrap().change_id().clone();
    // Ensure all provided divergent commits belong to the same change-id.
    for c in divergent_commits.iter().skip(1) {
        if c.change_id() != &divergent_change_id {
            return Err(ConvergeError::Other(
                "divergent commits have differing change-ids".into(),
            ));
        }
    }

    let mut nodes = HashMap::new();
    let mut roots = HashSet::new();
    let mut complete = true;

    let evolution_nodes = walk_predecessors(repo, divergent_commit_ids.as_slice());
    for node in evolution_nodes {
        let entry = node?;
        if *entry.commit.change_id() != divergent_change_id {
            continue;
        }
        if nodes.contains_key(entry.commit.id()) {
            // Note: currently walk_predecessors returns an error if the graph is cyclic, so
            // we shouldn't encounter the same commit twice. But in the future we could
            // allow cyclic evolution and if we do there is no reason to disallow it here.
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

        if predecessors.is_empty() {
            roots.insert(entry.commit.id().clone());
        }
        if nodes.len() >= max_nodes {
            complete = false;
            roots.insert(entry.commit.id().clone());
            break;
        }

        nodes.insert(
            entry.commit.id().clone(),
            TruncatedEvolutionNode {
                entry,
                predecessors,
            },
        );
    }

    Ok(TruncatedEvolutionGraph {
        divergent_change_id,
        divergent_commit_ids,
        nodes,
        roots,
        complete,
    })
}

fn merge_truncated_evolution_graph(
    graph: &TruncatedEvolutionGraph,
) -> Result<MergedState, ConvergeError> {
    let author: Merge<Signature> = create_merge(graph, |c| c.author().clone())?;
    let description_text: Merge<String> = create_merge(graph, |c| c.description().to_string())?;
    let parents: Merge<Vec<CommitId>> = create_merge(graph, |c| c.parent_ids().to_vec())?;
    Ok(MergedState {
        author,
        description_text,
        parents,
    })
}

fn create_merge<T>(
    graph: &TruncatedEvolutionGraph,
    attr_fn: fn(&Commit) -> T,
) -> Result<Merge<T>, ConvergeError> {
    if graph.roots.is_empty() {
        return Err(ConvergeError::Other(
            "no roots in truncated evolution graph".into(),
        ));
    }
    if graph.roots.len() > 1 {
        // TODO: implement this by adding a "virtual root".
        return Err(ConvergeError::Other(
            "multiple roots in truncated evolution graph is not yet supported".into(),
        ));
    }
    let root_id = graph.roots.iter().next().unwrap();
    let root_node = graph.nodes.get(root_id).ok_or_else(|| {
        ConvergeError::Other(format!("root commit {root_id} not found in graph").into())
    })?;

    let mut merge_builder: MergeBuilder<T> = Default::default();
    merge_builder.extend([attr_fn(&root_node.entry.commit)]);
    for node in graph.nodes.values() {
        for predecessor in &node.predecessors {
            let predecessor_node = graph.nodes.get(predecessor).ok_or_else(|| {
                ConvergeError::Other(
                    format!("predecessor commit {predecessor} not found in graph").into(),
                )
            })?;
            let node_attr = attr_fn(&node.entry.commit);
            let predecessor_attr = attr_fn(&predecessor_node.entry.commit);
            merge_builder.extend([predecessor_attr, node_attr]);
        }
    }

    Ok(merge_builder.build())
}
