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

//! Transactions for making atomic changes to a repo.

use std::sync::Arc;

use jj_core::backend::BackendError;
use jj_core::backend::Timestamp;
use jj_core::index::IndexError;
use jj_core::index::ReadonlyIndex;
use jj_core::index_store::IndexStoreError;
use jj_core::op_store;
use jj_core::op_store::OpStoreError;
use jj_core::op_store::OperationMetadata;
use jj_core::op_store::TimestampRange;
use jj_core::operation::Operation;
use jj_core::ref_name::WorkspaceName;
use jj_core::view::View;
use thiserror::Error;

use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::repo::MutableRepo;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RepoLoader;
use crate::repo::RepoLoaderError;
use crate::settings::UserSettings;

/// Error from attempts to write and publish transaction.
#[derive(Debug, Error)]
#[error("Failed to commit new operation")]
pub enum TransactionCommitError {
    /// Error from the backend.
    Backend(#[from] BackendError),
    /// Error from the index.
    Index(#[from] IndexError),
    /// Error from the index store.
    IndexStore(#[from] IndexStoreError),
    /// Error from the operation heads store.
    OpHeadsStore(#[from] OpHeadsStoreError),
    /// Error from the operation store.
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
    /// Creates a transaction on top of `repo`.
    pub fn new(repo: &Arc<ReadonlyRepo>) -> Self {
        let mut_repo = MutableRepo::new(repo.clone(), repo.readonly_index(), repo.view());
        let parent_ops = vec![repo.operation().clone()];
        let user_settings = repo.settings();
        let op_metadata = create_op_metadata(user_settings, "".to_string(), false);
        let end_time = user_settings.operation_timestamp();
        Self {
            mut_repo,
            parent_ops,
            op_metadata,
            end_time,
        }
    }

    /// Returns the repo the transaction started from.
    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        self.mut_repo.base_repo()
    }

    /// Returns the parent operations of the operation to be created.
    pub fn parent_ops(&self) -> &[Operation] {
        &self.parent_ops
    }

    /// Sets an attribute in the operation metadata.
    pub fn set_attribute(&mut self, key: String, value: String) {
        self.op_metadata.attributes.insert(key, value);
    }

    /// Returns the repo being modified by this transaction.
    pub fn repo(&self) -> &MutableRepo {
        &self.mut_repo
    }

    /// Returns the repo being modified by this transaction, mutably.
    pub fn repo_mut(&mut self) -> &mut MutableRepo {
        &mut self.mut_repo
    }

    /// Merges other_op into this transaction, using base_op as the merge base.
    pub async fn merge_operation(
        &mut self,
        base_op: &Operation,
        other_op: &Operation,
    ) -> Result<(), RepoLoaderError> {
        let repo_loader = self.base_repo().loader();
        let base_op_repo = repo_loader.load_at(base_op).await?;
        let other_repo = repo_loader.load_at(other_op).await?;
        self.parent_ops.push(other_op.clone());
        self.repo_mut().merge(&base_op_repo, &other_repo).await?;
        Ok(())
    }

    /// Sets whether the operation records a working-copy snapshot.
    pub fn set_is_snapshot(&mut self, is_snapshot: bool) {
        self.op_metadata.is_snapshot = is_snapshot;
    }

    /// Sets the name of the workspace the operation is attributed to.
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
        let (mut_index, view, predecessors) = mut_repo.consume().await?;
        assert!(
            view.is_heads_normalized(),
            "BUG: View heads must be normalized before persisting in the database"
        );

        // Make sure the commits referenced by the new view have been persisted
        // before we write the view and the operation.
        base_repo.store().flush()?;

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

/// Creates operation metadata from the user settings, with the start and
/// end times set to the configured operation timestamp or the current time.
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

    /// Returns the unpublished operation.
    pub fn operation(&self) -> &Operation {
        self.repo.operation()
    }

    /// Publishes the operation by updating the operation heads, and returns the
    /// repo loaded at the operation.
    pub async fn publish(self) -> Result<Arc<ReadonlyRepo>, TransactionCommitError> {
        let _lock = self.op_heads_store.lock().await?;
        self.op_heads_store
            .update_op_heads(self.operation().parent_ids(), self.operation().id())
            .await?;
        Ok(self.repo)
    }

    /// Returns the repo loaded at the operation without publishing it.
    pub fn leave_unpublished(self) -> Arc<ReadonlyRepo> {
        self.repo
    }
}
