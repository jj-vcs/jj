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

#![allow(missing_docs)]

use std::cmp::Ordering;
use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

use crate::backend::CommitId;
use crate::op_store;
use crate::op_store::OpStore;
use crate::op_store::OpStoreResult;
use crate::op_store::OperationId;
use crate::op_store::OperationMetadata;
use crate::op_store::ViewId;
use crate::view::View;

/// A wrapper around [`op_store::Operation`] that defines additional methods and
/// stores a pointer to the `OpStore` the operation belongs to.
#[derive(Clone)]
pub struct Operation {
    op_store: Arc<dyn OpStore>,
    id: OperationId,
    data: Arc<op_store::Operation>, // allow cheap clone
}

impl Debug for Operation {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("Operation").field("id", &self.id).finish()
    }
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Operation {}

impl Ord for Operation {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl PartialOrd for Operation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Operation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Operation {
    pub fn new(
        op_store: Arc<dyn OpStore>,
        id: OperationId,
        data: impl Into<Arc<op_store::Operation>>,
    ) -> Self {
        Operation {
            op_store,
            id,
            data: data.into(),
        }
    }

    pub fn op_store(&self) -> Arc<dyn OpStore> {
        self.op_store.clone()
    }

    pub fn id(&self) -> &OperationId {
        &self.id
    }

    pub fn view_id(&self) -> &ViewId {
        &self.data.view_id
    }

    pub fn parent_ids(&self) -> &[OperationId] {
        &self.data.parents
    }

    pub fn parents(&self) -> impl ExactSizeIterator<Item = OpStoreResult<Operation>> + use<'_> {
        let op_store = &self.op_store;
        self.data.parents.iter().map(|parent_id| {
            let data = op_store.read_operation(parent_id)?;
            Ok(Operation::new(op_store.clone(), parent_id.clone(), data))
        })
    }

    pub fn view(&self) -> OpStoreResult<View> {
        let data = self.op_store.read_view(&self.data.view_id)?;
        Ok(View::new(data))
    }

    pub fn metadata(&self) -> &OperationMetadata {
        &self.data.metadata
    }

    /// Returns true if predecessors are recorded in this operation.
    ///
    /// This returns false only if the operation was written by jj < 0.30.
    pub fn stores_commit_predecessors(&self) -> bool {
        self.data.commit_predecessors.is_some()
    }

    /// Returns predecessors of the specified commit if recorded.
    pub fn predecessors_for_commit(&self, commit_id: &CommitId) -> Option<&[CommitId]> {
        let map = self.data.commit_predecessors.as_ref()?;
        Some(map.get(commit_id)?)
    }

    pub fn store_operation(&self) -> &op_store::Operation {
        &self.data
    }
}
