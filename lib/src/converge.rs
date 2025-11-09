// Copyright 2020 The Jujutsu Authors
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
pub struct MergedState {
    /// A merge of the author.
    pub author: Merge<Signature>,
    /// A merge of the description.
    pub description: Merge<String>,
    /// A merge of the parents.
    pub parents: Merge<Vec<CommitId>>,
    /// A merge of the commit trees.
    pub tree: Merge<MergedTree>,
}

/// A node in the truncated evolution graph.
pub struct TruncatedEvolutionNode {
    /// The evolution entry for the commit represented by this node. Note: if
    /// reachable_predecessors is present, it contains *all* reachable
    /// predecessors, even those belonging to unrelated change-ids.
    pub entry: CommitEvolutionEntry,
    /// The predecessors of this node in the truncated evolution graph. These
    /// are those predecessors of the commit that have the same change-id as the
    /// commit.
    pub predecessors: Vec<CommitId>,
}

/// The truncated evolution graph for a divergent change.
///
/// This is similar to the evolog graph, but truncated in the sense that it only
/// contains commits that are for the given change-id. It may also be incomplete
/// if there are more than `max_nodes` commits in the evolution history with the
/// given change-id.
pub struct TruncatedEvolutionGraph {
    /// The change-id of the change being converged.
    pub divergent_change_id: ChangeId,
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    pub divergent_commit_ids: Vec<CommitId>,
    /// The nodes, keyed by commit id. The graph is not necessarily a tree,
    /// since commits may have multiple predecessors.
    pub nodes: HashMap<CommitId, TruncatedEvolutionNode>,
    /// The roots of the graph. If complete is true, then these are the commits
    /// with no predecessors. If complete is false then this also includes
    /// commits whose predecessors were not included in the graph due to the
    /// max_nodes limit. If complete is true then typically (pretty much always)
    /// there will be a single root, but if complete is false then there may be
    /// multiple roots.
    pub roots: HashSet<CommitId>,
    /// Whether the graph is complete. Tis is false there are more than
    /// `max_nodes` commits in the evolution history with the given change-id,
    /// and true otherwise.
    pub complete: bool,
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
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// Evaluates the provided revset expression and returns those commits that are
/// part of a divergent change, in the sense that the expression matches two or
/// more commits in the result with the same change-id.
///
/// The commits are keyed by their change-id.
pub fn get_divergent_commits(
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
pub fn build_truncated_evolution_graph(
    repo: &ReadonlyRepo,
    divergent_commits: &[Commit],
    max_nodes: usize,
) -> Result<TruncatedEvolutionGraph, ConvergeError> {
    if divergent_commits.is_empty() {
        return Err(ConvergeError::Other("no divergent commits provided".into()));
    }
    let divergent_commit_ids = divergent_commits
        .iter()
        .map(|c| c.id().clone())
        .collect_vec();

    let divergent_change_id = divergent_commits[0].change_id().clone();
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
