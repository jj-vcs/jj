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

//! Running operations on a workspace: snapshotting, importing from and
//! exporting to Git, finishing transactions, and updating the working copy.

use std::sync::Arc;

use jj_lib::op_store::OperationId;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::transaction::Transaction;
use jj_lib::transaction::TransactionCommitError;

/// Options controlling how operations on a workspace interact with the
/// working copy and the operation log.
#[derive(Clone, Debug)]
pub struct RunnerOptions {
    /// Don't snapshot or update the working copy.
    pub ignore_working_copy: bool,
    /// Publish new operations to the operation log. If false, operations are
    /// written but left unpublished, and the working copy isn't updated.
    pub integrate_operation: bool,
    /// Allow rewriting immutable commits.
    pub ignore_immutable: bool,
    /// Attributes to record in the metadata of new operations.
    pub transaction_attributes: Vec<(String, String)>,
}

impl RunnerOptions {
    /// Starts a transaction on `repo` for the workspace `workspace_name`, with
    /// the configured operation attributes.
    pub fn start_transaction(
        &self,
        repo: &Arc<ReadonlyRepo>,
        workspace_name: &WorkspaceName,
    ) -> Transaction {
        let mut tx = repo.start_transaction();
        tx.set_workspace_name(workspace_name);
        for (key, value) in &self.transaction_attributes {
            tx.set_attribute(key.clone(), value.clone());
        }
        tx
    }

    /// Writes the operation of `tx`, and publishes it unless
    /// [`Self::integrate_operation`] is false.
    pub async fn commit_transaction(
        &self,
        tx: Transaction,
        description: impl Into<String>,
    ) -> Result<Arc<ReadonlyRepo>, TransactionCommitError> {
        let unpublished_op = tx.write(description).await?;
        if self.integrate_operation {
            unpublished_op.publish().await
        } else {
            Ok(unpublished_op.leave_unpublished())
        }
    }
}

/// Something noteworthy that happened while snapshotting the working copy,
/// importing from or exporting to Git, or finishing a transaction.
#[derive(Debug)]
pub enum RunnerEvent<'a> {
    /// Descendants of rewritten commits were rebased.
    DescendantsRebased {
        /// Number of rebased commits.
        num_rebased: usize,
        /// Why the commits were rebased.
        reason: RebaseReason,
    },
    /// A new operation was written but not published because
    /// [`RunnerOptions::integrate_operation`] is false.
    OperationLeftUnpublished {
        /// ID of the unpublished operation.
        op_id: &'a OperationId,
        /// Whether the operation snapshotted the working copy.
        is_snapshot: bool,
    },
}

/// Why descendant commits were rebased.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RebaseReason {
    /// Commits were rewritten by a transaction.
    RewrittenCommits,
    /// Commits were rewritten in the Git repo.
    RewrittenInGit,
    /// The working-copy commit was rewritten by a snapshot.
    WorkingCopySnapshot,
}
