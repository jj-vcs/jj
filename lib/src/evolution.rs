// Copyright 2025 The Jujutsu Authors
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

//! Utility for commit evolution history.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::slice;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::dag_walk;
use crate::op_store::OpStoreError;
use crate::op_store::OpStoreResult;
use crate::op_walk;
use crate::operation::Operation;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;

/// Commit with predecessor information.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CommitEvolutionEntry {
    /// Commit id and metadata.
    pub commit: Commit,
    /// Operation where the commit was created or rewritten.
    pub operation: Option<Operation>,
    /// Reachable predecessor ids reconstructed from the commit metadata. This
    /// should be set if the associated `operation` is unknown.
    // TODO: remove with legacy commit.predecessors support
    #[serde(skip)]
    reachable_predecessors: Option<Vec<CommitId>>,
}

impl CommitEvolutionEntry {
    /// Predecessor ids of this commit.
    pub fn predecessor_ids(&self) -> &[CommitId] {
        match &self.operation {
            Some(op) => op.predecessors_for_commit(self.commit.id()).unwrap(),
            None => self.reachable_predecessors.as_ref().unwrap(),
        }
    }

    /// Predecessor commit objects of this commit.
    pub fn predecessors(&self) -> impl ExactSizeIterator<Item = BackendResult<Commit>> {
        let store = self.commit.store();
        self.predecessor_ids().iter().map(|id| store.get_commit(id))
    }
}

#[expect(missing_docs)]
#[derive(Debug, Error)]
pub enum WalkPredecessorsError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
    #[error("Predecessors cycle detected around commit {0}")]
    CycleDetected(CommitId),
}

/// Walks operations to emit commit predecessors in reverse topological order.
pub fn walk_predecessors<'repo>(
    repo: &'repo ReadonlyRepo,
    start_commits: &[CommitId],
) -> impl Iterator<Item = Result<CommitEvolutionEntry, WalkPredecessorsError>> + use<'repo> {
    WalkPredecessors {
        repo,
        op_ancestors: op_walk::walk_ancestors(slice::from_ref(repo.operation())),
        to_visit: start_commits.to_vec(),
        queued: VecDeque::new(),
    }
}

struct WalkPredecessors<'repo, I> {
    repo: &'repo ReadonlyRepo,
    op_ancestors: I,
    to_visit: Vec<CommitId>,
    queued: VecDeque<CommitEvolutionEntry>,
}

impl<I> WalkPredecessors<'_, I>
where
    I: Iterator<Item = OpStoreResult<Operation>>,
{
    fn try_next(&mut self) -> Result<Option<CommitEvolutionEntry>, WalkPredecessorsError> {
        while !self.to_visit.is_empty() && self.queued.is_empty() {
            let Some(op) = self.op_ancestors.next().transpose()? else {
                // Scanned all operations, no fallback needed.
                self.flush_commits()?;
                break;
            };
            if !op.stores_commit_predecessors() {
                // There may be concurrent ops, but let's simply switch to the
                // legacy commit traversal. Operation history should be mostly
                // linear.
                self.scan_commits()?;
                break;
            }
            self.visit_op(&op)?;
        }
        Ok(self.queued.pop_front())
    }

    /// Looks for predecessors within the given operation.
    fn visit_op(&mut self, op: &Operation) -> Result<(), WalkPredecessorsError> {
        let mut to_emit = Vec::new(); // transitive edges should be short
        let mut has_dup = false;
        let mut i = 0;
        while let Some(cur_id) = self.to_visit.get(i) {
            if let Some(next_ids) = op.predecessors_for_commit(cur_id) {
                if to_emit.contains(cur_id) {
                    self.to_visit.remove(i);
                    has_dup = true;
                    continue;
                }
                to_emit.extend(self.to_visit.splice(i..=i, next_ids.iter().cloned()));
            } else {
                i += 1;
            }
        }

        let store = self.repo.store();
        let mut emit = |id: &CommitId| -> BackendResult<()> {
            let commit = store.get_commit(id)?;
            self.queued.push_back(CommitEvolutionEntry {
                commit,
                operation: Some(op.clone()),
                reachable_predecessors: None,
            });
            Ok(())
        };
        match &*to_emit {
            [] => {}
            [id] if !has_dup => emit(id)?,
            _ => {
                let sorted_ids = dag_walk::topo_order_reverse_ok(
                    to_emit.iter().map(Ok),
                    |&id| id,
                    |&id| op.predecessors_for_commit(id).into_iter().flatten().map(Ok),
                    |id| id, // Err(&CommitId) if graph has cycle
                )
                .map_err(|id| WalkPredecessorsError::CycleDetected(id.clone()))?;
                for &id in &sorted_ids {
                    if op.predecessors_for_commit(id).is_some() {
                        emit(id)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Traverses predecessors from remainder commits.
    fn scan_commits(&mut self) -> BackendResult<()> {
        let store = self.repo.store();
        let index = self.repo.index();
        let mut commit_predecessors: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        let commits = dag_walk::topo_order_reverse_ok(
            self.to_visit.drain(..).map(|id| store.get_commit(&id)),
            |commit: &Commit| commit.id().clone(),
            |commit: &Commit| {
                let ids = commit_predecessors
                    .entry(commit.id().clone())
                    .or_insert_with(|| {
                        commit
                            .store_commit()
                            .predecessors
                            .iter()
                            // exclude unreachable predecessors
                            .filter(|id| index.has_id(id))
                            .cloned()
                            .collect()
                    });
                ids.iter().map(|id| store.get_commit(id)).collect_vec()
            },
            |_| panic!("graph has cycle"),
        )?;
        self.queued.extend(commits.into_iter().map(|commit| {
            let predecessors = commit_predecessors
                .remove(commit.id())
                .expect("commit must be visited once");
            CommitEvolutionEntry {
                commit,
                operation: None,
                reachable_predecessors: Some(predecessors),
            }
        }));
        Ok(())
    }

    /// Moves remainder commits to output queue.
    fn flush_commits(&mut self) -> BackendResult<()> {
        self.queued.reserve(self.to_visit.len());
        for id in self.to_visit.drain(..) {
            let commit = self.repo.store().get_commit(&id)?;
            self.queued.push_back(CommitEvolutionEntry {
                commit,
                operation: None,
                // There were no legacy operations, so the commit should have no
                // predecessors.
                reachable_predecessors: Some(vec![]),
            });
        }
        Ok(())
    }
}

impl<I> Iterator for WalkPredecessors<'_, I>
where
    I: Iterator<Item = OpStoreResult<Operation>>,
{
    type Item = Result<CommitEvolutionEntry, WalkPredecessorsError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.try_next().transpose()
    }
}

/// Collects predecessor records from `new_ops` to `old_ops`, and resolves
/// transitive entries.
///
/// This function assumes that there exists a single greatest common ancestors
/// between `old_ops` and `new_ops`. If `old_ops` and `new_ops` have ancestors
/// and descendants each other, or if criss-crossed merges exist between these
/// operations, the returned mapping would be lossy.
pub fn accumulate_predecessors(
    new_ops: &[Operation],
    old_ops: &[Operation],
) -> Result<BTreeMap<CommitId, Vec<CommitId>>, WalkPredecessorsError> {
    if new_ops.is_empty() || old_ops.is_empty() {
        return Ok(BTreeMap::new()); // No common ancestor exists
    }

    // Fast path for the single forward operation case.
    if let [op] = new_ops
        && op.parent_ids().iter().eq(old_ops.iter().map(|op| op.id()))
    {
        let Some(map) = &op.store_operation().commit_predecessors else {
            return Ok(BTreeMap::new());
        };
        return resolve_transitive_edges(map, map.keys())
            .map_err(|id| WalkPredecessorsError::CycleDetected(id.clone()));
    }

    // Follow reverse edges from the common ancestor to old_ops. Here we use
    // BTreeMap to stabilize order of the reversed edges.
    let mut accumulated = BTreeMap::new();
    let reverse_ops = op_walk::walk_ancestors_range(old_ops, new_ops);
    if !try_collect_predecessors_into(&mut accumulated, reverse_ops)? {
        return Ok(BTreeMap::new());
    }
    let mut accumulated = reverse_edges(accumulated);
    // Follow forward edges from new_ops to the common ancestor.
    let forward_ops = op_walk::walk_ancestors_range(new_ops, old_ops);
    if !try_collect_predecessors_into(&mut accumulated, forward_ops)? {
        return Ok(BTreeMap::new());
    }
    let new_commit_ids = new_ops
        .iter()
        .filter_map(|op| op.store_operation().commit_predecessors.as_ref())
        .flat_map(|map| map.keys());
    resolve_transitive_edges(&accumulated, new_commit_ids)
        .map_err(|id| WalkPredecessorsError::CycleDetected(id.clone()))
}

fn try_collect_predecessors_into(
    collected: &mut BTreeMap<CommitId, Vec<CommitId>>,
    ops: impl IntoIterator<Item = OpStoreResult<Operation>>,
) -> OpStoreResult<bool> {
    for op in ops {
        let op = op?;
        let Some(map) = &op.store_operation().commit_predecessors else {
            return Ok(false);
        };
        // Just insert. There should be no duplicate entries.
        collected.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    Ok(true)
}

/// Resolves transitive edges in `graph` starting from the `start` nodes,
/// returns new DAG. The returned DAG only includes edges reachable from the
/// `start` nodes.
fn resolve_transitive_edges<'a: 'b, 'b>(
    graph: &'a BTreeMap<CommitId, Vec<CommitId>>,
    start: impl IntoIterator<Item = &'b CommitId>,
) -> Result<BTreeMap<CommitId, Vec<CommitId>>, &'b CommitId> {
    let mut new_graph: BTreeMap<CommitId, Vec<CommitId>> = BTreeMap::new();
    let sorted_ids = dag_walk::topo_order_forward_ok(
        start.into_iter().map(Ok),
        |&id| id,
        |&id| graph.get(id).into_iter().flatten().map(Ok),
        |id| id, // Err(&CommitId) if graph has cycle
    )?;
    for cur_id in sorted_ids {
        let Some(neighbors) = graph.get(cur_id) else {
            continue;
        };
        let lookup = |id| new_graph.get(id).map_or(slice::from_ref(id), Vec::as_slice);
        let new_neighbors = match &neighbors[..] {
            [id] => lookup(id).to_vec(), // unique() not needed
            ids => ids.iter().flat_map(lookup).unique().cloned().collect(),
        };
        new_graph.insert(cur_id.clone(), new_neighbors);
    }
    Ok(new_graph)
}

fn reverse_edges(graph: BTreeMap<CommitId, Vec<CommitId>>) -> BTreeMap<CommitId, Vec<CommitId>> {
    let mut new_graph: BTreeMap<CommitId, Vec<CommitId>> = BTreeMap::new();
    for (node1, neighbors) in graph {
        for node2 in neighbors {
            new_graph.entry(node2).or_default().push(node1.clone());
        }
    }
    new_graph
}
