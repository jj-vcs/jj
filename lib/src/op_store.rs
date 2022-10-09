// Copyright 2020 Google LLC
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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{Debug, Error, Formatter};

use thiserror::Error;

use crate::backend::{CommitId, Timestamp};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct WorkspaceId(String);

impl Debug for WorkspaceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_tuple("WorkspaceId").field(&self.0).finish()
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self("default".to_string())
    }
}

impl WorkspaceId {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct ViewId(Vec<u8>);

impl Debug for ViewId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_tuple("ViewId").field(&self.hex()).finish()
    }
}

impl ViewId {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    pub fn from_hex(hex: &str) -> Self {
        Self(hex::decode(hex).unwrap())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.clone()
    }

    pub fn hex(&self) -> String {
        hex::encode(&self.0)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct OperationId(Vec<u8>);

impl Debug for OperationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_tuple("OperationId").field(&self.hex()).finish()
    }
}

impl OperationId {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    pub fn from_hex(hex: &str) -> Self {
        Self(hex::decode(hex).unwrap())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.clone()
    }

    pub fn hex(&self) -> String {
        hex::encode(&self.0)
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum RefTarget {
    Normal(CommitId),
    Conflict {
        removes: Vec<CommitId>,
        adds: Vec<CommitId>,
    },
}

impl RefTarget {
    pub fn is_conflict(&self) -> bool {
        matches!(self, RefTarget::Conflict { .. })
    }

    pub fn adds(&self) -> Vec<CommitId> {
        match self {
            RefTarget::Normal(id) => {
                vec![id.clone()]
            }
            RefTarget::Conflict { removes: _, adds } => adds.clone(),
        }
    }

    pub fn has_add(&self, needle: &CommitId) -> bool {
        match self {
            RefTarget::Normal(id) => id == needle,
            RefTarget::Conflict { removes: _, adds } => adds.contains(needle),
        }
    }

    pub fn removes(&self) -> Vec<CommitId> {
        match self {
            RefTarget::Normal(_) => {
                vec![]
            }
            RefTarget::Conflict { removes, adds: _ } => removes.clone(),
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Debug)]
pub struct BranchTarget {
    /// The commit the branch points to locally. `None` if the branch has been
    /// deleted locally.
    pub local_target: Option<RefTarget>,
    // TODO: Do we need to support tombstones for remote branches? For example, if the branch
    // has been deleted locally and you pull from a remote, maybe it should make a difference
    // whether the branch is known to have existed on the remote. We may not want to resurrect
    // the branch if the branch's state on the remote was just not known.
    pub remote_targets: BTreeMap<String, RefTarget>,
}

/// Represents the way the repo looks at a given time, just like how a Tree
/// object represents how the file system looks at a given time.
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct View {
    /// All head commits
    pub head_ids: HashSet<CommitId>,
    /// Heads of the set of public commits.
    pub public_head_ids: HashSet<CommitId>,
    pub branches: BTreeMap<String, BranchTarget>,
    pub tags: BTreeMap<String, RefTarget>,
    pub git_refs: BTreeMap<String, RefTarget>,
    /// The commit the Git HEAD points to.
    // TODO: Support multiple Git worktrees?
    // TODO: Do we want to store the current branch name too?
    pub git_head: Option<CommitId>,
    // The commit that *should be* checked out in the workspace. Note that the working copy
    // (.jj/working_copy/) has the source of truth about which commit *is* checked out (to be
    // precise: the commit to which we most recently completed an update to).
    pub wc_commit_ids: HashMap<WorkspaceId, CommitId>,
}

/// Represents an operation (transaction) on the repo view, just like how a
/// Commit object represents an operation on the tree.
///
/// Operations and views are not meant to be exchanged between repos or users;
/// they represent local state and history.
///
/// The operation history will almost always be linear. It will only have
/// forks when parallel operations occurred. The parent is determined when
/// the transaction starts. When the transaction commits, a lock will be
/// taken and it will be checked that the current head of the operation
/// graph is unchanged. If the current head has changed, there has been
/// concurrent operation.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Operation {
    pub view_id: ViewId,
    pub parents: Vec<OperationId>,
    pub metadata: OperationMetadata,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct OperationMetadata {
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    // Whatever is useful to the user, such as exact command line call
    pub description: String,
    pub hostname: String,
    pub username: String,
    pub tags: HashMap<String, String>,
}

impl OperationMetadata {
    pub fn new(description: String, start_time: Timestamp) -> Self {
        let end_time = Timestamp::now();
        let hostname = whoami::hostname();
        let username = whoami::username();
        OperationMetadata {
            start_time,
            end_time,
            description,
            hostname,
            username,
            tags: Default::default(),
        }
    }
}

#[derive(Debug, Error)]
pub enum OpStoreError {
    #[error("Operation not found")]
    NotFound,
    #[error("{0}")]
    Other(String),
}

pub type OpStoreResult<T> = Result<T, OpStoreError>;

pub trait OpStore: Send + Sync + Debug {
    fn read_view(&self, id: &ViewId) -> OpStoreResult<View>;

    fn write_view(&self, contents: &View) -> OpStoreResult<ViewId>;

    fn read_operation(&self, id: &OperationId) -> OpStoreResult<Operation>;

    fn write_operation(&self, contents: &Operation) -> OpStoreResult<OperationId>;
}
