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

use futures::TryStreamExt as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::backend::TreeId;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::evolution::WalkPredecessorsError;
use jj_lib::graph_dominators::FlowGraph;
use jj_lib::index::IndexError;
use jj_lib::merge::Merge;
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
        _repo: Arc<ReadonlyRepo>,
        _divergent_commits: Vec<Commit>,
    ) -> Result<Self, ConvergeError> {
        todo!()
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
    _graph: &TruncatedEvolutionGraph,
    _excluded_divergent_commits: &HashSet<CommitId>,
    _value_fn: VF,
) -> Result<(Merge<T>, CommitId), ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: AsyncFn(&Commit) -> Result<T, ConvergeError>,
{
    todo!();
}
