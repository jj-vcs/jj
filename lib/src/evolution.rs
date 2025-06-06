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

use std::collections::VecDeque;
use std::slice;
use std::sync::Arc;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::dag_walk;
use crate::op_store::OpStoreError;
use crate::op_store::OpStoreResult;
use crate::op_store::OperationId;
use crate::op_walk;
use crate::operation::Operation;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::store::Store;

/// Commit with predecessor information.
#[derive(Clone, Debug)]
pub struct CommitEvolutionEntry {
    /// Commit id and metadata.
    pub commit: Commit,
    /// Operation where the commit was created or rewritten.
    pub operation: Option<Operation>,
}

impl CommitEvolutionEntry {
    /// Predecessor ids of this commit.
    pub fn predecessor_ids(&self) -> &[CommitId] {
        match &self.operation {
            Some(op) => op.predecessors_for_commit(self.commit.id()).unwrap(),
            None => &self.commit.store_commit().predecessors,
        }
    }

    /// Predecessor commit objects of this commit.
    pub fn predecessors(&self) -> impl ExactSizeIterator<Item = BackendResult<Commit>> + use<'_> {
        let store = self.commit.store();
        self.predecessor_ids().iter().map(|id| store.get_commit(id))
    }
}

#[allow(missing_docs)]
#[derive(Debug, Error)]
pub enum WalkPredecessorsError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
    #[error("Predecessors cycle detected at operation {0}")]
    CycleDetected(OperationId),
}

/// Walks operations to emit commit predecessors in reverse topological order.
pub fn walk_predecessors(
    repo: &ReadonlyRepo,
    start_commits: &[CommitId],
) -> impl Iterator<Item = Result<CommitEvolutionEntry, WalkPredecessorsError>> {
    WalkPredecessors {
        store: repo.store().clone(),
        op_ancestors: op_walk::walk_ancestors(slice::from_ref(repo.operation())),
        to_visit: start_commits.to_vec(),
        queued: VecDeque::new(),
    }
}

struct WalkPredecessors<I> {
    store: Arc<Store>,
    op_ancestors: I,
    to_visit: Vec<CommitId>,
    queued: VecDeque<CommitEvolutionEntry>,
}

impl<I> WalkPredecessors<I>
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
                // Predecessors will be visited in reverse order if they appear
                // in the same operation. See scan_commits() for why.
                to_emit.extend(self.to_visit.splice(i..=i, next_ids.iter().rev().cloned()));
            } else {
                i += 1;
            }
        }

        let mut emit = |id: &CommitId| -> BackendResult<()> {
            let commit = self.store.get_commit(id)?;
            self.queued.push_back(CommitEvolutionEntry {
                commit,
                operation: Some(op.clone()),
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
                    |_| (),
                )
                .map_err(|()| WalkPredecessorsError::CycleDetected(op.id().clone()))?;
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
        // TODO: commits to visit might be gc-ed if we make index not preserve
        // commit.predecessor_ids.
        let commits = dag_walk::topo_order_reverse_ok(
            self.to_visit.drain(..).map(|id| self.store.get_commit(&id)),
            |commit: &Commit| commit.id().clone(),
            |commit: &Commit| {
                // Predecessors don't need to follow any defined order. However
                // in practice, if there are multiple predecessors, then usually
                // the first predecessor is the previous version of the same
                // change, and the other predecessors are commits that were
                // squashed into it. If multiple commits are squashed at once,
                // then they are usually recorded in chronological order. We
                // want to show squashed commits in reverse chronological order,
                // and we also want to show squashed commits before the squash
                // destination (since the destination's subgraph may contain
                // earlier squashed commits as well), so we visit the
                // predecessors in reverse order.
                let ids = &commit.store_commit().predecessors;
                ids.iter()
                    .rev()
                    .map(|id| self.store.get_commit(id))
                    .collect_vec()
            },
            |_| panic!("graph has cycle"),
        )?;
        self.queued
            .extend(commits.into_iter().map(|commit| CommitEvolutionEntry {
                commit,
                operation: None,
            }));
        Ok(())
    }

    /// Moves remainder commits to output queue.
    fn flush_commits(&mut self) -> BackendResult<()> {
        // TODO: commits to visit might be gc-ed if we make index not preserve
        // commit.predecessor_ids.
        self.queued.reserve(self.to_visit.len());
        for id in self.to_visit.drain(..) {
            let commit = self.store.get_commit(&id)?;
            self.queued.push_back(CommitEvolutionEntry {
                commit,
                operation: None,
            });
        }
        Ok(())
    }
}

impl<I> Iterator for WalkPredecessors<I>
where
    I: Iterator<Item = OpStoreResult<Operation>>,
{
    type Item = Result<CommitEvolutionEntry, WalkPredecessorsError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.try_next().transpose()
    }
}
