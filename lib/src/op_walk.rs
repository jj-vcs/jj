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

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::slice;
use std::sync::Arc;

use itertools::Itertools as _;
use pollster::FutureExt as _;
use thiserror::Error;

use crate::dag_walk;
use crate::object_id::HexPrefix;
use crate::object_id::PrefixResolution;
use crate::op_heads_store;
use crate::op_heads_store::OpHeadResolutionError;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_store::OpStore;
use crate::op_store::OpStoreError;
use crate::op_store::OpStoreResult;
use crate::op_store::OperationId;
use crate::operation::Operation;
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
    /// Failed to resolve the current operation heads.
    #[error(transparent)]
    OpHeadResolution(#[from] OpHeadResolutionError),
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
pub fn resolve_op_for_load(
    repo_loader: &RepoLoader,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let op_store = repo_loader.op_store();
    let op_heads_store = repo_loader.op_heads_store().as_ref();
    let get_current_op = || {
        op_heads_store::resolve_op_heads(op_heads_store, op_store, |op_heads| async move {
            Err(OpsetResolutionError::MultipleOperations {
                expr: "@".to_owned(),
                candidates: op_heads.iter().map(|op| op.id().clone()).collect(),
            }
            .into())
        })
        .block_on()
    };
    let get_head_ops = || get_current_head_ops(op_store, op_heads_store);
    resolve_single_op(op_store, get_current_op, get_head_ops, op_str)
}

/// Resolves operation set expression against the loaded repo.
///
/// The "@" symbol will be resolved to the operation the repo was loaded at.
pub fn resolve_op_with_repo(
    repo: &ReadonlyRepo,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    resolve_op_at(repo.op_store(), slice::from_ref(repo.operation()), op_str)
}

/// Resolves operation set expression at the given head operations.
pub fn resolve_op_at(
    op_store: &Arc<dyn OpStore>,
    head_ops: &[Operation],
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let get_current_op = || match head_ops {
        [head_op] => Ok(head_op.clone()),
        [] => Err(OpsetResolutionError::EmptyOperations("@".to_owned()).into()),
        _ => Err(OpsetResolutionError::MultipleOperations {
            expr: "@".to_owned(),
            candidates: head_ops.iter().map(|op| op.id().clone()).collect(),
        }
        .into()),
    };
    let get_head_ops = || Ok(head_ops.to_vec());
    resolve_single_op(op_store, get_current_op, get_head_ops, op_str)
}

/// Resolves operation set expression with the given "@" symbol resolution
/// callbacks.
fn resolve_single_op(
    op_store: &Arc<dyn OpStore>,
    get_current_op: impl FnOnce() -> Result<Operation, OpsetEvaluationError>,
    get_head_ops: impl FnOnce() -> Result<Vec<Operation>, OpsetEvaluationError>,
    op_str: &str,
) -> Result<Operation, OpsetEvaluationError> {
    let op_symbol = op_str.trim_end_matches(['-', '+']);
    let op_postfix = &op_str[op_symbol.len()..];
    let head_ops = op_postfix.contains('+').then(get_head_ops).transpose()?;
    let mut operation = match op_symbol {
        "@" => get_current_op(),
        s => resolve_single_op_from_store(op_store, s).block_on(),
    }?;
    for (i, c) in op_postfix.chars().enumerate() {
        let mut neighbor_ops = match c {
            '-' => operation.parents().try_collect()?,
            '+' => find_child_ops(head_ops.as_ref().unwrap(), operation.id())?,
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
pub fn get_current_head_ops(
    op_store: &Arc<dyn OpStore>,
    op_heads_store: &dyn OpHeadsStore,
) -> Result<Vec<Operation>, OpsetEvaluationError> {
    let mut head_ops: Vec<_> = op_heads_store
        .get_op_heads()
        .block_on()?
        .into_iter()
        .map(|id| -> OpStoreResult<Operation> {
            let data = op_store.read_operation(&id).block_on()?;
            Ok(Operation::new(op_store.clone(), id, data))
        })
        .try_collect()?;
    // To stabilize output, sort in the same order as resolve_op_heads()
    head_ops.sort_by_key(|op| op.metadata().time.end.timestamp);
    Ok(head_ops)
}

/// Looks up children of the `root_op_id` by traversing from the `head_ops`.
///
/// This will be slow if the `root_op_id` is far away (or unreachable) from the
/// `head_ops`.
fn find_child_ops(
    head_ops: &[Operation],
    root_op_id: &OperationId,
) -> OpStoreResult<Vec<Operation>> {
    walk_ancestors(head_ops)
        .take_while(|res| res.as_ref().map_or(true, |op| op.id() != root_op_id))
        .filter_ok(|op| op.parent_ids().iter().any(|id| id == root_op_id))
        .try_collect()
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct OperationByEndTime(Operation);

impl Ord for OperationByEndTime {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_end_time = &self.0.metadata().time.end;
        let other_end_time = &other.0.metadata().time.end;
        self_end_time
            .cmp(other_end_time)
            .then_with(|| self.0.cmp(&other.0)) // to comply with Eq
    }
}

impl PartialOrd for OperationByEndTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Walks `head_ops` and their ancestors in reverse topological order.
pub fn walk_ancestors(
    head_ops: &[Operation],
) -> impl Iterator<Item = OpStoreResult<Operation>> + use<> {
    let head_ops = head_ops
        .iter()
        .cloned()
        .map(OperationByEndTime)
        .collect_vec();
    // Lazily load operations based on timestamp-based heuristic. This works so long
    // as the operation history is mostly linear.
    dag_walk::topo_order_reverse_lazy_ok(
        head_ops.into_iter().map(Ok),
        |OperationByEndTime(op)| op.id().clone(),
        |OperationByEndTime(op)| op.parents().map_ok(OperationByEndTime).collect_vec(),
        |_| panic!("graph has cycle"),
    )
    .map_ok(|OperationByEndTime(op)| op)
}

/// Walks ancestors from `head_ops` in reverse topological order, excluding
/// ancestors of `root_ops`.
pub fn walk_ancestors_range(
    head_ops: &[Operation],
    root_ops: &[Operation],
) -> impl Iterator<Item = OpStoreResult<Operation>> + use<> {
    let mut start_ops = itertools::chain(head_ops, root_ops)
        .cloned()
        .map(OperationByEndTime)
        .collect_vec();

    // Consume items until root_ops to get rid of unwanted ops.
    let leading_items = if root_ops.is_empty() {
        vec![]
    } else {
        let unwanted_ids = root_ops.iter().map(|op| op.id().clone()).collect();
        collect_ancestors_until_roots(&mut start_ops, unwanted_ids)
    };

    // Lazily load operations based on timestamp-based heuristic. This works so long
    // as the operation history is mostly linear.
    let trailing_iter = dag_walk::topo_order_reverse_lazy_ok(
        start_ops.into_iter().map(Ok),
        |OperationByEndTime(op)| op.id().clone(),
        |OperationByEndTime(op)| op.parents().map_ok(OperationByEndTime).collect_vec(),
        |_| panic!("graph has cycle"),
    )
    .map_ok(|OperationByEndTime(op)| op);
    itertools::chain(leading_items, trailing_iter)
}

fn collect_ancestors_until_roots(
    start_ops: &mut Vec<OperationByEndTime>,
    mut unwanted_ids: HashSet<OperationId>,
) -> Vec<OpStoreResult<Operation>> {
    let sorted_ops = match dag_walk::topo_order_reverse_chunked(
        start_ops,
        |OperationByEndTime(op)| op.id().clone(),
        |OperationByEndTime(op)| op.parents().map_ok(OperationByEndTime).collect_vec(),
        |_| panic!("graph has cycle"),
    ) {
        Ok(sorted_ops) => sorted_ops,
        Err(err) => return vec![Err(err)],
    };
    let mut items = Vec::new();
    for OperationByEndTime(op) in sorted_ops {
        if unwanted_ids.contains(op.id()) {
            unwanted_ids.extend(op.parent_ids().iter().cloned());
        } else {
            items.push(Ok(op));
        }
    }
    // Don't visit ancestors of unwanted ops further.
    start_ops.retain(|OperationByEndTime(op)| !unwanted_ids.contains(op.id()));
    items
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
pub fn reparent_range(
    op_store: &dyn OpStore,
    root_ops: &[Operation],
    head_ops: &[Operation],
    dest_op: &Operation,
) -> OpStoreResult<ReparentStats> {
    let ops_to_reparent: Vec<_> = walk_ancestors_range(head_ops, root_ops).try_collect()?;
    let unreachable_count = walk_ancestors_range(root_ops, slice::from_ref(dest_op))
        .process_results(|iter| iter.count())?;

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
        let new_id = op_store.write_operation(&data).block_on()?;
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
