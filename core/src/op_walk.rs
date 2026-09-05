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

//! Walking the operation graph.

use std::cmp::Ordering;
use std::collections::HashSet;

use futures::Stream;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use itertools::Itertools as _;
use pollster::FutureExt as _;

use crate::dag_walk_async;
use crate::op_store::OpStoreResult;
use crate::op_store::OperationId;
use crate::operation::Operation;

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
) -> impl Stream<Item = OpStoreResult<Operation>> + use<> {
    let head_ops = head_ops
        .iter()
        .cloned()
        .map(OperationByEndTime)
        .collect_vec();
    // Lazily load operations based on timestamp-based heuristic. This works so long
    // as the operation history is mostly linear.
    dag_walk_async::topo_order_reverse_lazy(
        head_ops.into_iter().map(Ok),
        |OperationByEndTime(op)| op.id().clone(),
        async |OperationByEndTime(op)| match op.parents().await {
            Ok(parents) => parents
                .into_iter()
                .map(|parent| Ok(OperationByEndTime(parent)))
                .collect_vec(),
            Err(err) => vec![Err(err)],
        },
        |_| panic!("graph has cycle"),
    )
    .map_ok(|OperationByEndTime(op)| op)
}

/// Walks ancestors from `head_ops` in reverse topological order, excluding
/// ancestors of `root_ops`.
pub fn walk_ancestors_range(
    head_ops: &[Operation],
    root_ops: &[Operation],
) -> impl Stream<Item = OpStoreResult<Operation>> + use<> {
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
    let trailing_stream = dag_walk_async::topo_order_reverse_lazy(
        start_ops.into_iter().map(Ok),
        |OperationByEndTime(op)| op.id().clone(),
        async |OperationByEndTime(op)| match op.parents().await {
            Ok(parents) => parents
                .into_iter()
                .map(|op| Ok(OperationByEndTime(op)))
                .collect_vec(),
            Err(err) => vec![Err(err)],
        },
        |_| panic!("graph has cycle"),
    )
    .map_ok(|OperationByEndTime(op)| op);
    stream::iter(leading_items).chain(trailing_stream)
}

fn collect_ancestors_until_roots(
    start_ops: &mut Vec<OperationByEndTime>,
    mut unwanted_ids: HashSet<OperationId>,
) -> Vec<OpStoreResult<Operation>> {
    let sorted_ops = match dag_walk_async::topo_order_reverse_chunked(
        start_ops,
        |OperationByEndTime(op)| op.id().clone(),
        async |OperationByEndTime(op)| match op.parents().await {
            Ok(parents) => parents
                .into_iter()
                .map(|op| Ok(OperationByEndTime(op)))
                .collect_vec(),
            Err(err) => vec![Err(err)],
        },
        |_| panic!("graph has cycle"),
    )
    .block_on()
    {
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

/// Finds the closest common ancestor of `set1` and `set2`. Uses the end
/// timestamp as a heuristic.
// TODO: We should probably make this a function on `OpStore` instead of
// relying on heuristics (even if the implementation of the trait might rely on
// the same heuristic initially).
pub async fn closest_common_ancestors(
    set1: impl IntoIterator<Item = Operation>,
    set2: impl IntoIterator<Item = Operation>,
) -> OpStoreResult<Vec<Operation>> {
    let ancestor_ops = dag_walk_async::closest_common_nodes(
        set1.into_iter().map(OperationByEndTime),
        set2.into_iter().map(OperationByEndTime),
        |op: &OperationByEndTime| op.0.id().clone(),
        async |op: &OperationByEndTime| {
            op.0.parents()
                .await
                .map(|parents| parents.into_iter().map(OperationByEndTime))
        },
    )
    .await?;
    Ok(ancestor_ops.into_iter().map(|op| op.0).collect())
}
