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

#![expect(missing_docs)]

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::Timestamp;
use crate::index::IndexStoreError;
use crate::index::ReadonlyIndex;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_store;
use crate::op_store::OpStoreError;
use crate::op_store::OperationId;
use crate::op_store::OperationMetadata;
use crate::op_store::TimestampRange;
use crate::op_walk;
use crate::operation::Operation;
use crate::ref_name::WorkspaceName;
use crate::repo::MutableRepo;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RepoLoader;
use crate::repo::RepoLoaderError;
use crate::settings::UserSettings;
use crate::view::View;

/// Error from attempts to write and publish transaction.
#[derive(Debug, Error)]
#[error("Failed to commit new operation")]
pub enum TransactionCommitError {
    IndexStore(#[from] IndexStoreError),
    OpHeadsStore(#[from] OpHeadsStoreError),
    OpStore(#[from] OpStoreError),
}

/// An in-memory representation of a repo and any changes being made to it.
///
/// Within the scope of a transaction, changes to the repository are made
/// in-memory to `mut_repo` and published to the repo backend when
/// [`Transaction::commit`] is called. When a transaction is committed, it
/// becomes atomically visible as an Operation in the op log that represents the
/// transaction itself, and as a View that represents the state of the repo
/// after the transaction. This is similar to how a Commit represents a change
/// to the contents of the repository and a Tree represents the repository's
/// contents after the change. See the documentation for [`op_store::Operation`]
/// and [`op_store::View`] for more information.
pub struct Transaction {
    mut_repo: MutableRepo,
    parent_ops: Vec<Operation>,
    op_metadata: OperationMetadata,
    end_time: Option<Timestamp>,
}

impl Transaction {
    pub fn new(mut_repo: MutableRepo, user_settings: &UserSettings) -> Self {
        let parent_ops = vec![mut_repo.base_repo().operation().clone()];
        let op_metadata = create_op_metadata(user_settings, "".to_string(), false);
        let end_time = user_settings.operation_timestamp();
        Self {
            mut_repo,
            parent_ops,
            op_metadata,
            end_time,
        }
    }

    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        self.mut_repo.base_repo()
    }

    pub fn set_attribute(&mut self, key: String, value: String) {
        self.op_metadata.attributes.insert(key, value);
    }

    pub fn repo(&self) -> &MutableRepo {
        &self.mut_repo
    }

    pub fn repo_mut(&mut self) -> &mut MutableRepo {
        &mut self.mut_repo
    }

    /// Merges the given `operations`. Returns the merged repo and the number of
    /// rebased commits. If `operations` is empty returns the root repo. If
    /// `operations` has a single entry, returns that entry's repo. Otherwise
    /// an actual merge happens. The new operation is not published.
    pub async fn merge_operations(
        repo_loader: &RepoLoader,
        operations: Vec<Operation>,
        workspace_name: Option<&WorkspaceName>,
        transaction_description: Option<&str>,
        transaction_attributes: impl IntoIterator<Item = (String, String)>,
    ) -> Result<(Arc<ReadonlyRepo>, usize), RepoLoaderError> {
        // IMPLEMENTATION NOTE: This used to be implemented as a much simple
        // recursive method, but unfortunately due to the async nature of the
        // method itself and its dependencies, that leads to stack-overflow in
        // some cases. See https://github.com/jj-vcs/jj/pull/9586 for more
        // details. Ideally the Rust compiler gets better at dealing with this,
        // then we could go back to the recursive implementation.
        match &operations[..] {
            [] => {
                let root_operation = repo_loader.root_operation().await;
                let root_repo = repo_loader.load_at(&root_operation).await?;
                return Ok((root_repo, 0));
            }
            [op] => {
                let repo = repo_loader.load_at(op).await?;
                return Ok((repo, 0));
            }
            _ => {}
        }

        let mut num_rebased = 0;
        let to_operation_ids =
            |ops: &[Operation]| ops.iter().map(|op| op.id().clone()).collect_vec();
        let operation_ids = to_operation_ids(&operations);

        // Caches the result of merging some operations.
        let mut merged_operations: HashMap<Vec<OperationId>, Operation> = HashMap::new();
        // Caches the result of op_walk::closest_common_ancestors invocations. Keyed by
        // the arguments to that method.
        let mut closest_common_ancestors: HashMap<_, Vec<Operation>> = HashMap::new();

        let transaction_attributes = transaction_attributes.into_iter().collect_vec();
        let tx = start_repo_transaction(
            &repo_loader.load_at(&operations[0]).await?,
            workspace_name,
            transaction_attributes.clone(),
        );
        let mut stack = vec![(1, operations, tx)];

        while let Some((index, operations, mut tx)) = stack.pop() {
            assert!(operations.len() > 1);
            assert!(index <= operations.len());
            if index == operations.len() {
                // We are done processing the operations, but there is more work on the stack.
                // Commit the transaction and cache the result.
                let tx_description = transaction_description.map_or_else(
                    || format!("merge {} operations", operations.len()),
                    |tx_description| tx_description.to_string(),
                );
                let merged_repo = tx.write(tx_description).await?.leave_unpublished();
                merged_operations.insert(
                    to_operation_ids(&operations),
                    merged_repo.operation().clone(),
                );
                continue;
            }

            let other_op = &operations[index];

            // Get the ancestor operations between the operations we have merged so far
            // (represented by `tx.parent_ops()`) and the next operation to merge
            // (`other_op`).
            let ancestor_ops = match closest_common_ancestors
                .entry((to_operation_ids(&tx.parent_ops), other_op.id().clone()))
            {
                Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
                Entry::Vacant(vacant_entry) => {
                    let ancestor_ops = op_walk::closest_common_ancestors(
                        tx.parent_ops.iter().cloned(),
                        [other_op.clone()],
                    )
                    .await?;
                    vacant_entry.insert(ancestor_ops.clone())
                }
            };
            assert!(!ancestor_ops.is_empty());

            let ancestor_op = if let [ancestor_op] = ancestor_ops.as_slice() {
                // There is a single common ancestor.
                Some(ancestor_op)
            } else {
                // There are multiple common ancestors, check to see if we have cached their
                // merge result.
                let ancestor_op_ids = ancestor_ops.iter().map(|op| op.id().clone()).collect_vec();
                merged_operations.get(&ancestor_op_ids)
            };

            if let Some(merged_ancestor_op) = ancestor_op {
                // We have the merge of the ancestor operations. We can proceed to merge with
                // other_op.
                num_rebased += tx.merge_operation(merged_ancestor_op, other_op).await?;
                // Push state on the stack to continue merging the rest of the operations.
                stack.push((index + 1, operations, tx));
                continue;
            }

            // We have to merge the ancestor ops.
            // We first push the current state to the stack so that after we merge the
            // ancestor ops, we can continue merging the rest of the operations.
            stack.push((index, operations, tx));
            // Then we push the ancestor ops to the stack so that we can merge them first.
            // We need to start a separate transaction for this.
            let new_tx = repo_loader
                .load_at(&ancestor_ops[0])
                .await?
                .start_transaction();
            stack.push((1, ancestor_ops.clone(), new_tx));
        }

        // We are all done! The result should be in the cache.
        let merged_operation = merged_operations.get(&operation_ids).cloned().unwrap();
        Ok((repo_loader.load_at(&merged_operation).await?, num_rebased))
    }

    /// Merges other_op into this transaction's base repo, using base_op as the
    /// merge base. Returns the number of rebased descendants.
    async fn merge_operation(
        &mut self,
        base_op: &Operation,
        other_op: &Operation,
    ) -> Result<usize, RepoLoaderError> {
        let repo_loader = self.base_repo().loader();
        let base_op_repo = repo_loader.load_at(base_op).await?;
        let other_repo = repo_loader.load_at(other_op).await?;
        self.parent_ops.push(other_op.clone());
        self.repo_mut().merge(&base_op_repo, &other_repo).await?;
        let num_rebased = self.repo_mut().rebase_descendants().await?;
        Ok(num_rebased)
    }

    pub fn set_is_snapshot(&mut self, is_snapshot: bool) {
        self.op_metadata.is_snapshot = is_snapshot;
    }

    pub fn set_workspace_name(&mut self, workspace_name: &WorkspaceName) {
        self.op_metadata.workspace_name = Some(workspace_name.to_owned());
    }

    /// Writes the transaction to the operation store and publishes it.
    pub async fn commit(
        self,
        description: impl Into<String>,
    ) -> Result<Arc<ReadonlyRepo>, TransactionCommitError> {
        self.write(description).await?.publish().await
    }

    /// Writes the transaction to the operation store, but does not publish it.
    /// That means that a repo can be loaded at the operation, but the
    /// operation will not be seen when loading the repo at head.
    pub async fn write(
        mut self,
        description: impl Into<String>,
    ) -> Result<UnpublishedOperation, TransactionCommitError> {
        let mut_repo = self.mut_repo;
        // TODO: Should we instead just do the rebasing here if necessary?
        assert!(
            !mut_repo.has_rewrites(),
            "BUG: Descendants have not been rebased after the last rewrites."
        );
        let base_repo = mut_repo.base_repo().clone();
        let (mut_index, view, predecessors) = mut_repo.consume();

        let operation = {
            let view_id = base_repo.op_store().write_view(view.store_view()).await?;
            self.op_metadata.description = description.into();
            self.op_metadata.time.end = self.end_time.unwrap_or_else(Timestamp::now);
            let parents = self.parent_ops.iter().map(|op| op.id().clone()).collect();
            let store_operation = op_store::Operation {
                view_id,
                parents,
                metadata: self.op_metadata,
                commit_predecessors: Some(predecessors),
            };
            let new_op_id = base_repo
                .op_store()
                .write_operation(&store_operation)
                .await?;
            Operation::new(base_repo.op_store().clone(), new_op_id, store_operation)
        };

        let index = base_repo.index_store().write_index(mut_index, &operation)?;
        let unpublished = UnpublishedOperation::new(base_repo.loader(), operation, view, index);
        Ok(unpublished)
    }
}

pub fn start_repo_transaction(
    repo: &Arc<ReadonlyRepo>,
    workspace_name: Option<&WorkspaceName>,
    transaction_attributes: impl IntoIterator<Item = (String, String)>,
) -> Transaction {
    let mut tx = repo.start_transaction();
    if let Some(workspace_name) = workspace_name {
        tx.set_workspace_name(workspace_name);
    }
    for (key, value) in transaction_attributes {
        tx.set_attribute(key, value);
    }
    tx
}

pub fn create_op_metadata(
    user_settings: &UserSettings,
    description: String,
    is_snapshot: bool,
) -> OperationMetadata {
    let timestamp = user_settings
        .operation_timestamp()
        .unwrap_or_else(Timestamp::now);
    let hostname = user_settings.operation_hostname().to_owned();
    let username = user_settings.operation_username().to_owned();
    OperationMetadata {
        time: TimestampRange {
            start: timestamp,
            end: timestamp,
        },
        description,
        hostname,
        username,
        is_snapshot,
        workspace_name: None,
        attributes: Default::default(),
    }
}

/// An unpublished operation in the store.
///
/// An Operation which has been written to the operation store but not
/// published. The repo can be loaded at an unpublished Operation, but the
/// Operation will not be visible in the op log if the repo is loaded at head.
///
/// Either [`Self::publish`] or [`Self::leave_unpublished`] must be called to
/// finish the operation.
#[must_use = "Either publish() or leave_unpublished() must be called to finish the operation."]
pub struct UnpublishedOperation {
    op_heads_store: Arc<dyn OpHeadsStore>,
    repo: Arc<ReadonlyRepo>,
}

impl UnpublishedOperation {
    fn new(
        repo_loader: &RepoLoader,
        operation: Operation,
        view: View,
        index: Box<dyn ReadonlyIndex>,
    ) -> Self {
        Self {
            op_heads_store: repo_loader.op_heads_store().clone(),
            repo: repo_loader.create_from(operation, view, index),
        }
    }

    pub fn operation(&self) -> &Operation {
        self.repo.operation()
    }

    pub async fn publish(self) -> Result<Arc<ReadonlyRepo>, TransactionCommitError> {
        let _lock = self.op_heads_store.lock().await?;
        self.op_heads_store
            .update_op_heads(self.operation().parent_ids(), self.operation().id())
            .await?;
        Ok(self.repo)
    }

    pub fn leave_unpublished(self) -> Arc<ReadonlyRepo> {
        self.repo
    }
}
