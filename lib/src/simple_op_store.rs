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

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use itertools::Itertools;
use prost::Message;
use tempfile::{NamedTempFile, PersistError};

use crate::backend::{CommitId, MillisSinceEpoch, ObjectId, Timestamp};
use crate::content_hash::blake2b_hash;
use crate::file_util::persist_content_addressed_temp_file;
use crate::op_store::{
    BranchTarget, OpStore, OpStoreError, OpStoreResult, Operation, OperationId, OperationMetadata,
    RefTarget, View, ViewId, WorkspaceId,
};

impl From<std::io::Error> for OpStoreError {
    fn from(err: std::io::Error) -> Self {
        OpStoreError::Other(err.to_string())
    }
}

impl From<PersistError> for OpStoreError {
    fn from(err: PersistError) -> Self {
        OpStoreError::Other(err.to_string())
    }
}

impl From<prost::DecodeError> for OpStoreError {
    fn from(err: prost::DecodeError) -> Self {
        OpStoreError::Other(err.to_string())
    }
}

#[derive(Debug)]
pub struct SimpleOpStore {
    path: PathBuf,
}

impl SimpleOpStore {
    /// Creates an empty OpStore, panics if it already exists
    pub fn init(store_path: &Path) -> Self {
        fs::create_dir(store_path.join("views")).unwrap();
        fs::create_dir(store_path.join("operations")).unwrap();
        SimpleOpStore {
            path: store_path.to_owned(),
        }
    }

    /// Load an existing OpStore
    pub fn load(store_path: &Path) -> Self {
        SimpleOpStore {
            path: store_path.to_path_buf(),
        }
    }

    fn view_path(&self, id: &ViewId) -> PathBuf {
        self.path.join("views").join(id.hex())
    }

    fn operation_path(&self, id: &OperationId) -> PathBuf {
        self.path.join("operations").join(id.hex())
    }
}

impl OpStore for SimpleOpStore {
    fn name(&self) -> &str {
        "simple_op_store"
    }

    fn read_view(&self, id: &ViewId) -> OpStoreResult<View> {
        let path = self.view_path(id);
        let buf = fs::read(path)?;

        let proto = crate::protos::op_store::View::decode(&*buf)?;
        Ok(view_from_proto(proto))
    }

    fn write_view(&self, view: &View) -> OpStoreResult<ViewId> {
        let temp_file = NamedTempFile::new_in(&self.path)?;

        let proto = view_to_proto(view);
        temp_file.as_file().write_all(&proto.encode_to_vec())?;

        let id = ViewId::new(blake2b_hash(view).to_vec());

        persist_content_addressed_temp_file(temp_file, self.view_path(&id))?;
        Ok(id)
    }

    fn read_operation(&self, id: &OperationId) -> OpStoreResult<Operation> {
        let path = self.operation_path(id);
        let buf = fs::read(path).map_err(not_found_to_store_error)?;

        let proto = crate::protos::op_store::Operation::decode(&*buf)?;
        Ok(operation_from_proto(proto))
    }

    fn write_operation(&self, operation: &Operation) -> OpStoreResult<OperationId> {
        let temp_file = NamedTempFile::new_in(&self.path)?;

        let proto = operation_to_proto(operation);
        temp_file.as_file().write_all(&proto.encode_to_vec())?;

        let id = OperationId::new(blake2b_hash(operation).to_vec());

        persist_content_addressed_temp_file(temp_file, self.operation_path(&id))?;
        Ok(id)
    }
}

fn not_found_to_store_error(err: std::io::Error) -> OpStoreError {
    if err.kind() == ErrorKind::NotFound {
        OpStoreError::NotFound
    } else {
        OpStoreError::from(err)
    }
}

fn timestamp_to_proto(timestamp: &Timestamp) -> crate::protos::op_store::Timestamp {
    crate::protos::op_store::Timestamp {
        millis_since_epoch: timestamp.timestamp.0,
        tz_offset: timestamp.tz_offset,
    }
}

fn timestamp_from_proto(proto: crate::protos::op_store::Timestamp) -> Timestamp {
    Timestamp {
        timestamp: MillisSinceEpoch(proto.millis_since_epoch),
        tz_offset: proto.tz_offset,
    }
}

fn operation_metadata_to_proto(
    metadata: &OperationMetadata,
) -> crate::protos::op_store::OperationMetadata {
    crate::protos::op_store::OperationMetadata {
        start_time: Some(timestamp_to_proto(&metadata.start_time)),
        end_time: Some(timestamp_to_proto(&metadata.end_time)),
        description: metadata.description.clone(),
        hostname: metadata.hostname.clone(),
        username: metadata.username.clone(),
        tags: metadata.tags.clone(),
    }
}

fn operation_metadata_from_proto(
    proto: crate::protos::op_store::OperationMetadata,
) -> OperationMetadata {
    let start_time = timestamp_from_proto(proto.start_time.unwrap_or_default());
    let end_time = timestamp_from_proto(proto.end_time.unwrap_or_default());
    OperationMetadata {
        start_time,
        end_time,
        description: proto.description,
        hostname: proto.hostname,
        username: proto.username,
        tags: proto.tags,
    }
}

fn operation_to_proto(operation: &Operation) -> crate::protos::op_store::Operation {
    let mut proto = crate::protos::op_store::Operation {
        view_id: operation.view_id.as_bytes().to_vec(),
        metadata: Some(operation_metadata_to_proto(&operation.metadata)),
        ..Default::default()
    };
    for parent in &operation.parents {
        proto.parents.push(parent.to_bytes());
    }
    proto
}

fn operation_from_proto(proto: crate::protos::op_store::Operation) -> Operation {
    let parents = proto.parents.into_iter().map(OperationId::new).collect();
    let view_id = ViewId::new(proto.view_id);
    let metadata = operation_metadata_from_proto(proto.metadata.unwrap_or_default());
    Operation {
        view_id,
        parents,
        metadata,
    }
}

fn view_to_proto(view: &View) -> crate::protos::op_store::View {
    let mut proto = crate::protos::op_store::View::default();
    for (workspace_id, commit_id) in &view.wc_commit_ids {
        proto
            .wc_commit_ids
            .insert(workspace_id.as_str().to_string(), commit_id.to_bytes());
    }
    for head_id in &view.head_ids {
        proto.head_ids.push(head_id.to_bytes());
    }
    for head_id in &view.public_head_ids {
        proto.public_head_ids.push(head_id.to_bytes());
    }

    for (name, target) in &view.branches {
        let mut branch_proto = crate::protos::op_store::Branch {
            name: name.clone(),
            ..Default::default()
        };
        branch_proto.name = name.clone();
        if let Some(local_target) = &target.local_target {
            branch_proto.local_target = Some(ref_target_to_proto(local_target));
        }
        for (remote_name, target) in &target.remote_targets {
            branch_proto
                .remote_branches
                .push(crate::protos::op_store::RemoteBranch {
                    remote_name: remote_name.clone(),
                    target: Some(ref_target_to_proto(target)),
                });
        }
        proto.branches.push(branch_proto);
    }

    for (name, target) in &view.tags {
        proto.tags.push(crate::protos::op_store::Tag {
            name: name.clone(),
            target: Some(ref_target_to_proto(target)),
        });
    }

    for (git_ref_name, target) in &view.git_refs {
        proto.git_refs.push(crate::protos::op_store::GitRef {
            name: git_ref_name.clone(),
            target: Some(ref_target_to_proto(target)),
            ..Default::default()
        });
    }

    if let Some(git_head) = &view.git_head {
        proto.git_head = Some(ref_target_to_proto(git_head));
    }

    proto
}

fn view_from_proto(proto: crate::protos::op_store::View) -> View {
    let mut view = View::default();
    // For compatibility with old repos before we had support for multiple working
    // copies
    #[allow(deprecated)]
    if !proto.wc_commit_id.is_empty() {
        view.wc_commit_ids
            .insert(WorkspaceId::default(), CommitId::new(proto.wc_commit_id));
    }
    for (workspace_id, commit_id) in proto.wc_commit_ids {
        view.wc_commit_ids
            .insert(WorkspaceId::new(workspace_id), CommitId::new(commit_id));
    }
    for head_id_bytes in proto.head_ids {
        view.head_ids.insert(CommitId::new(head_id_bytes));
    }
    for head_id_bytes in proto.public_head_ids {
        view.public_head_ids.insert(CommitId::new(head_id_bytes));
    }

    for branch_proto in proto.branches {
        let local_target = branch_proto.local_target.map(ref_target_from_proto);

        let mut remote_targets = BTreeMap::new();
        for remote_branch in branch_proto.remote_branches {
            remote_targets.insert(
                remote_branch.remote_name,
                ref_target_from_proto(remote_branch.target.unwrap_or_default()),
            );
        }

        view.branches.insert(
            branch_proto.name.clone(),
            BranchTarget {
                local_target,
                remote_targets,
            },
        );
    }

    for tag_proto in proto.tags {
        view.tags.insert(
            tag_proto.name,
            ref_target_from_proto(tag_proto.target.unwrap_or_default()),
        );
    }

    for git_ref in proto.git_refs {
        if let Some(target) = git_ref.target {
            view.git_refs
                .insert(git_ref.name, ref_target_from_proto(target));
        } else {
            // Legacy format
            view.git_refs.insert(
                git_ref.name,
                RefTarget::Normal(CommitId::new(git_ref.commit_id)),
            );
        }
    }

    #[allow(deprecated)]
    if let Some(git_head) = proto.git_head.as_ref() {
        view.git_head = Some(ref_target_from_proto(git_head.clone()));
    } else if !proto.git_head_legacy.is_empty() {
        view.git_head = Some(RefTarget::Normal(CommitId::new(proto.git_head_legacy)));
    }

    view
}

fn ref_target_to_proto(value: &RefTarget) -> crate::protos::op_store::RefTarget {
    let mut proto = crate::protos::op_store::RefTarget::default();
    match value {
        RefTarget::Normal(id) => {
            proto.value = Some(crate::protos::op_store::ref_target::Value::CommitId(
                id.to_bytes(),
            ));
        }
        RefTarget::Conflict { removes, adds } => {
            let mut ref_conflict_proto = crate::protos::op_store::RefConflict::default();
            for id in removes {
                ref_conflict_proto.removes.push(id.to_bytes());
            }
            for id in adds {
                ref_conflict_proto.adds.push(id.to_bytes());
            }
            proto.value = Some(crate::protos::op_store::ref_target::Value::Conflict(
                ref_conflict_proto,
            ));
        }
    }
    proto
}

fn ref_target_from_proto(proto: crate::protos::op_store::RefTarget) -> RefTarget {
    match proto.value.unwrap() {
        crate::protos::op_store::ref_target::Value::CommitId(id) => {
            RefTarget::Normal(CommitId::new(id))
        }
        crate::protos::op_store::ref_target::Value::Conflict(conflict) => {
            let removes = conflict
                .removes
                .into_iter()
                .map(CommitId::new)
                .collect_vec();
            let adds = conflict.adds.into_iter().map(CommitId::new).collect_vec();
            RefTarget::Conflict { removes, adds }
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use maplit::{btreemap, hashmap, hashset};

    use super::*;
    use crate::backend::{CommitId, MillisSinceEpoch, ObjectId, Timestamp};
    use crate::content_hash::blake2b_hash;
    use crate::op_store::{BranchTarget, OperationMetadata, RefTarget, WorkspaceId};

    fn create_view() -> View {
        let head_id1 = CommitId::from_hex("aaa111");
        let head_id2 = CommitId::from_hex("aaa222");
        let public_head_id1 = CommitId::from_hex("bbb444");
        let public_head_id2 = CommitId::from_hex("bbb555");
        let branch_main_local_target = RefTarget::Normal(CommitId::from_hex("ccc111"));
        let branch_main_origin_target = RefTarget::Normal(CommitId::from_hex("ccc222"));
        let branch_deleted_origin_target = RefTarget::Normal(CommitId::from_hex("ccc333"));
        let tag_v1_target = RefTarget::Normal(CommitId::from_hex("ddd111"));
        let git_refs_main_target = RefTarget::Normal(CommitId::from_hex("fff111"));
        let git_refs_feature_target = RefTarget::Conflict {
            removes: vec![CommitId::from_hex("fff111")],
            adds: vec![CommitId::from_hex("fff222"), CommitId::from_hex("fff333")],
        };
        let default_wc_commit_id = CommitId::from_hex("abc111");
        let test_wc_commit_id = CommitId::from_hex("abc222");
        View {
            head_ids: hashset! {head_id1, head_id2},
            public_head_ids: hashset! {public_head_id1, public_head_id2},
            branches: btreemap! {
                "main".to_string() => BranchTarget {
                    local_target: Some(branch_main_local_target),
                    remote_targets: btreemap! {
                        "origin".to_string() => branch_main_origin_target,
                    }
                },
                "deleted".to_string() => BranchTarget {
                    local_target: None,
                    remote_targets: btreemap! {
                        "origin".to_string() => branch_deleted_origin_target,
                    }
                },
            },
            tags: btreemap! {
                "v1.0".to_string() => tag_v1_target,
            },
            git_refs: btreemap! {
                "refs/heads/main".to_string() => git_refs_main_target,
                "refs/heads/feature".to_string() => git_refs_feature_target
            },
            git_head: Some(RefTarget::Normal(CommitId::from_hex("fff111"))),
            wc_commit_ids: hashmap! {
                WorkspaceId::default() => default_wc_commit_id,
                WorkspaceId::new("test".to_string()) => test_wc_commit_id,
            },
        }
    }

    fn create_operation() -> Operation {
        Operation {
            view_id: ViewId::from_hex("aaa111"),
            parents: vec![
                OperationId::from_hex("bbb111"),
                OperationId::from_hex("bbb222"),
            ],
            metadata: OperationMetadata {
                start_time: Timestamp {
                    timestamp: MillisSinceEpoch(123456789),
                    tz_offset: 3600,
                },
                end_time: Timestamp {
                    timestamp: MillisSinceEpoch(123456800),
                    tz_offset: 3600,
                },
                description: "check out foo".to_string(),
                hostname: "some.host.example.com".to_string(),
                username: "someone".to_string(),
                tags: hashmap! {
                    "key1".to_string() => "value1".to_string(),
                    "key2".to_string() => "value2".to_string(),
                },
            },
        }
    }

    #[test]
    fn test_hash_view() {
        // Test exact output so we detect regressions in compatibility
        assert_snapshot!(
            ViewId::new(blake2b_hash(&create_view()).to_vec()).hex(),
            @"7f47fa81494d7189cb1827b83b3f834662f0f61b4c4090298067e85cdc60f773bf639c4e6a3554a4e401650218ca240291ce591f45a1c501ade1d2b9f97e1a37"
        );
    }

    #[test]
    fn test_hash_operation() {
        // Test exact output so we detect regressions in compatibility
        assert_snapshot!(
            OperationId::new(blake2b_hash(&create_operation()).to_vec()).hex(),
            @"3ec986c29ff8eb808ea8f6325d6307cea75ef02987536c8e4645406aba51afc8e229957a6e855170d77a66098c58912309323f5e0b32760caa2b59dc84d45fcf"
        );
    }

    #[test]
    fn test_read_write_view() {
        let temp_dir = testutils::new_temp_dir();
        let store = SimpleOpStore::init(temp_dir.path());
        let view = create_view();
        let view_id = store.write_view(&view).unwrap();
        let read_view = store.read_view(&view_id).unwrap();
        assert_eq!(read_view, view);
    }

    #[test]
    fn test_read_write_operation() {
        let temp_dir = testutils::new_temp_dir();
        let store = SimpleOpStore::init(temp_dir.path());
        let operation = create_operation();
        let op_id = store.write_operation(&operation).unwrap();
        let read_operation = store.read_operation(&op_id).unwrap();
        assert_eq!(read_operation, operation);
    }
}
