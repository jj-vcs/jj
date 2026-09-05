// Copyright 2020-2023 The Jujutsu Authors
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

//! Utility for operation id resolution and traversal.

use std::collections::HashMap;
use std::slice;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::ready;
use futures::future::try_join_all;
use jj_core::object_id::HexPrefix;
use jj_core::object_id::PrefixResolution;
use jj_core::op_store::OpStore;
use jj_core::op_store::OpStoreError;
use jj_core::op_store::OpStoreResult;
use jj_core::op_store::OperationId;
pub use jj_core::op_walk::closest_common_ancestors;
pub use jj_core::op_walk::walk_ancestors;
pub use jj_core::op_walk::walk_ancestors_range;
use jj_core::operation::Operation;
use thiserror::Error;

use crate::op_heads_store;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RepoLoader;

/// Error that may occur during evaluation of operation set expression.
#[derive(Debug, Error)]
pub enum OpsetEvaluationError {
    /// Failed to resolve operation set expression.
    #[error(transparent)]
    OpsetResolution(#[from] OpsetResolutionError),
    /// Failed to read op heads.
    #[error(transparent)]
    OpHeadsStore(#[from] OpHeadsStoreError),
    /// Failed to access operation object.
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
}

/// Error that may occur during parsing and resolution of operation set
/// expression.
#[derive(Debug, Error)]
pub enum OpsetResolutionError {
    // TODO: Maybe empty/multiple operations should be allowed, and rejected by
    // caller as needed.
    /// Expression resolved to multiple operations.
    #[error(r#"The "{expr}" expression resolved to more than one operation"#)]
    MultipleOperations {
        /// Source expression.
        expr: String,
        /// Matched operation ids.
        candidates: Vec<OperationId>,
    },
    /// Expression resolved to no operations.
    #[error(r#"The "{0}" expression resolved to no operations"#)]
    EmptyOperations(String),
    /// Invalid symbol as an operation ID.
    #[error(r#"Operation ID "{0}" is not a valid hexadecimal prefix"#)]
    InvalidIdPrefix(String),
    /// Operation ID not found.
    #[error(r#"No operation ID matching "{0}""#)]
    NoSuchOperation(String),
    /// Operation ID prefix matches multiple operations.
    #[error(r#"Operation ID prefix "{0}" is ambiguous"#)]
    AmbiguousIdPrefix(String),
}

/// Resolves operation set expression without loading a repo.
pub async fn resolve_op_for_load(
    repo_loader: &RepoLoader,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let op_store = repo_loader.op_store();
    let op_heads_store = repo_loader.op_heads_store().as_ref();
    let get_current_op = async || {
        op_heads_store::resolve_op_heads(op_heads_store, op_store, async |op_heads| {
            Err(OpsetResolutionError::MultipleOperations {
                expr: "@".to_owned(),
                candidates: op_heads.iter().map(|op| op.id().clone()).collect(),
            }
            .into())
        })
        .await
    };
    let get_head_ops = async || get_current_head_ops(op_store, op_heads_store).await;
    resolve_single_op(op_store, get_current_op, get_head_ops, op_str).await
}

/// Resolves operation set expression against the loaded repo.
///
/// The "@" symbol will be resolved to the operation the repo was loaded at.
pub async fn resolve_op_with_repo(
    repo: &ReadonlyRepo,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    resolve_op_at(repo.op_store(), slice::from_ref(repo.operation()), op_str).await
}

/// Resolves operation set expression at the given head operations.
pub async fn resolve_op_at(
    op_store: &Arc<dyn OpStore>,
    head_ops: &[Operation],
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let get_current_op = async || match head_ops {
        [head_op] => Ok(head_op.clone()),
        [] => Err(OpsetResolutionError::EmptyOperations("@".to_owned()).into()),
        _ => Err(OpsetResolutionError::MultipleOperations {
            expr: "@".to_owned(),
            candidates: head_ops.iter().map(|op| op.id().clone()).collect(),
        }
        .into()),
    };
    let get_head_ops = async || Ok(head_ops.to_vec());
    resolve_single_op(op_store, get_current_op, get_head_ops, op_str).await
}

/// Resolves operation set expression with the given "@" symbol resolution
/// callbacks.
async fn resolve_single_op(
    op_store: &Arc<dyn OpStore>,
    get_current_op: impl AsyncFnOnce() -> Result<Operation, OpsetEvaluationError>,
    get_head_ops: impl AsyncFnOnce() -> Result<Vec<Operation>, OpsetEvaluationError>,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let op_symbol = op_str.trim_end_matches(['-', '+']);
    let op_postfix = &op_str[op_symbol.len()..];
    let head_ops = if op_postfix.contains('+') {
        Some(get_head_ops().await?)
    } else {
        None
    };
    let mut operation = match op_symbol {
        "@" => get_current_op().await,
        s => resolve_single_op_from_store(op_store, s).await,
    }?;
    for (i, c) in op_postfix.chars().enumerate() {
        let mut neighbor_ops = match c {
            '-' => operation.parents().await?,
            '+' => find_child_ops(head_ops.as_ref().unwrap(), operation.id()).await?,
            _ => unreachable!(),
        };
        operation = match neighbor_ops.len() {
            // Since there is no hint provided for `EmptyOperations` in
            // `opset_resolution_error_hint()` (there would be no useful hint for the
            // user to take action on anyway), we don't have to worry about op ids being
            // incoherent with the op set expression shown to the user, unlike for the
            // `MultipleOperations` variant.
            //
            // The full op set expression is guaranteed to be empty in this case,
            // because ancestors/descendants of an empty operation are empty.
            0 => Err(OpsetResolutionError::EmptyOperations(op_str.to_owned()))?,
            1 => neighbor_ops.pop().unwrap(),
            // Returns the exact subexpression that resolves to multiple operations,
            // rather than the full expression provided by the user.
            _ => Err(OpsetResolutionError::MultipleOperations {
                expr: op_str[..=op_symbol.len() + i].to_owned(),
                candidates: neighbor_ops.iter().map(|op| op.id().clone()).collect(),
            })?,
        };
    }
    Ok(operation)
}

async fn resolve_single_op_from_store(
    op_store: &Arc<dyn OpStore>,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    if op_str.is_empty() {
        return Err(OpsetResolutionError::InvalidIdPrefix(op_str.to_owned()).into());
    }
    let prefix = HexPrefix::try_from_hex(op_str)
        .ok_or_else(|| OpsetResolutionError::InvalidIdPrefix(op_str.to_owned()))?;
    match op_store.resolve_operation_id_prefix(&prefix).await? {
        PrefixResolution::NoMatch => {
            Err(OpsetResolutionError::NoSuchOperation(op_str.to_owned()).into())
        }
        PrefixResolution::SingleMatch(op_id) => {
            let data = op_store.read_operation(&op_id).await?;
            Ok(Operation::new(op_store.clone(), op_id, data))
        }
        PrefixResolution::AmbiguousMatch => {
            Err(OpsetResolutionError::AmbiguousIdPrefix(op_str.to_owned()).into())
        }
    }
}

/// Loads the current head operations. The returned operations may contain
/// redundant ones which are ancestors of the other heads.
pub async fn get_current_head_ops(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &dyn OpHeadsStore,
) -> Result<Vec<Operation>, OpsetEvaluationError> {
    let head_ops_futures = op_heads_store.get_op_heads().await?.into_iter().map(
        async |id| -> OpStoreResult<Operation> {
            let data = op_store.read_operation(&id).await?;
            Ok(Operation::new(op_store.clone(), id, data))
        },
    );
    let mut head_ops = try_join_all(head_ops_futures).await?;
    // To stabilize output, sort in the same order as resolve_op_heads()
    head_ops.sort_by_key(|op| op.metadata().time.end.timestamp);
    Ok(head_ops)
}

/// Looks up children of the `root_op_id` by traversing from the `head_ops`.
///
/// This will be slow if the `root_op_id` is far away (or unreachable) from the
/// `head_ops`.
async fn find_child_ops(
    head_ops: &[Operation],
    root_op_id: &OperationId,
) -> OpStoreResult<Vec<Operation>> {
    walk_ancestors(head_ops)
        .take_while(|res| ready(res.as_ref().map_or(true, |op| op.id() != root_op_id)))
        .try_filter(|op| ready(op.parent_ids().iter().any(|id| id == root_op_id)))
        .try_collect()
        .await
}

/// Stats about `reparent_range()`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReparentStats {
    /// New head operation ids in order of the old `head_ops`.
    pub new_head_ids: Vec<OperationId>,
    /// The number of rewritten operations.
    pub rewritten_count: usize,
    /// The number of ancestor operations that become unreachable from the
    /// rewritten heads.
    pub unreachable_count: usize,
}

/// Reparents the operation range `root_ops..head_ops` onto the `dest_op`.
///
/// Returns the new head operation ids as well as some stats. If the old
/// operation heads are remapped to the new heads, the operations within the
/// range `dest_op..root_ops` become unreachable.
///
/// If the source operation range `root_ops..head_ops` was empty, the
/// `new_head_ids` will be `[dest_op.id()]`, meaning the `dest_op` is the head.
// TODO: Find better place to host this function. It might be an OpStore method.
pub async fn reparent_range(
    op_store: &dyn OpStore,
    root_ops: &[Operation],
    head_ops: &[Operation],
    dest_op: &Operation,
) -> OpStoreResult<ReparentStats> {
    let ops_to_reparent: Vec<_> = walk_ancestors_range(head_ops, root_ops)
        .try_collect()
        .await?;
    let unreachable_count = walk_ancestors_range(root_ops, slice::from_ref(dest_op))
        .try_fold(0, |acc, _| async move { Ok(acc + 1) })
        .await?;

    assert!(
        ops_to_reparent
            .last()
            .is_none_or(|op| op.id() != op_store.root_operation_id()),
        "root operation cannot be rewritten"
    );
    let mut rewritten_ids = HashMap::new();
    for old_op in ops_to_reparent.into_iter().rev() {
        let mut data = old_op.store_operation().clone();
        let mut dest_once = Some(dest_op.id());
        data.parents = data
            .parents
            .iter()
            .filter_map(|id| rewritten_ids.get(id).or_else(|| dest_once.take()))
            .cloned()
            .collect();
        let new_id = op_store.write_operation(&data).await?;
        rewritten_ids.insert(old_op.id().clone(), new_id);
    }

    let mut dest_once = Some(dest_op.id());
    let new_head_ids = head_ops
        .iter()
        .filter_map(|op| rewritten_ids.get(op.id()).or_else(|| dest_once.take()))
        .cloned()
        .collect();
    Ok(ReparentStats {
        new_head_ids,
        rewritten_count: rewritten_ids.len(),
        unreachable_count,
    })
}
