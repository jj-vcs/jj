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

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;

use blake2::{Blake2b512, Digest};
use itertools::Itertools;
use protobuf::{Message, MessageField};
use tempfile::{NamedTempFile, PersistError};

use crate::backend::{CommitId, MillisSinceEpoch, Timestamp};
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

impl From<protobuf::Error> for OpStoreError {
    fn from(err: protobuf::Error) -> Self {
        OpStoreError::Other(err.to_string())
    }
}

#[derive(Debug)]
pub struct SimpleOpStore {
    path: PathBuf,
}

impl SimpleOpStore {
    pub fn init(store_path: PathBuf) -> Self {
        fs::create_dir(store_path.join("views")).unwrap();
        fs::create_dir(store_path.join("operations")).unwrap();
        Self::load(store_path)
    }

    pub fn load(store_path: PathBuf) -> Self {
        SimpleOpStore { path: store_path }
    }

    fn view_path(&self, id: &ViewId) -> PathBuf {
        self.path.join("views").join(id.hex())
    }

    fn operation_path(&self, id: &OperationId) -> PathBuf {
        self.path.join("operations").join(id.hex())
    }
}

fn not_found_to_store_error(err: std::io::Error) -> OpStoreError {
    if err.kind() == ErrorKind::NotFound {
        OpStoreError::NotFound
    } else {
        OpStoreError::from(err)
    }
}

impl OpStore for SimpleOpStore {
    fn read_view(&self, id: &ViewId) -> OpStoreResult<View> {
        let path = self.view_path(id);
        let mut file = File::open(path).map_err(not_found_to_store_error)?;

        let proto: crate::protos::op_store::View = Message::parse_from_reader(&mut file)?;
        Ok(view_from_proto(&proto))
    }

    fn write_view(&self, view: &View) -> OpStoreResult<ViewId> {
        let temp_file = NamedTempFile::new_in(&self.path)?;

        let proto = view_to_proto(view);
        let mut proto_bytes: Vec<u8> = Vec::new();
        proto.write_to_writer(&mut proto_bytes)?;

        temp_file.as_file().write_all(&proto_bytes)?;

        let id = ViewId::new(Blake2b512::digest(&proto_bytes).to_vec());

        persist_content_addressed_temp_file(temp_file, self.view_path(&id))?;
        Ok(id)
    }

    fn read_operation(&self, id: &OperationId) -> OpStoreResult<Operation> {
        let path = self.operation_path(id);
        let mut file = File::open(path).map_err(not_found_to_store_error)?;

        let proto: crate::protos::op_store::Operation = Message::parse_from_reader(&mut file)?;
        Ok(operation_from_proto(&proto))
    }

    fn write_operation(&self, operation: &Operation) -> OpStoreResult<OperationId> {
        let temp_file = NamedTempFile::new_in(&self.path)?;

        let proto = operation_to_proto(operation);
        let mut proto_bytes: Vec<u8> = Vec::new();
        proto.write_to_writer(&mut proto_bytes)?;

        temp_file.as_file().write_all(&proto_bytes)?;

        let id = OperationId::new(Blake2b512::digest(&proto_bytes).to_vec());

        persist_content_addressed_temp_file(temp_file, self.operation_path(&id))?;
        Ok(id)
    }
}

fn timestamp_to_proto(timestamp: &Timestamp) -> crate::protos::op_store::Timestamp {
    let mut proto = crate::protos::op_store::Timestamp::new();
    proto.millis_since_epoch = timestamp.timestamp.0;
    proto.tz_offset = timestamp.tz_offset;
    proto
}

fn timestamp_from_proto(proto: &crate::protos::op_store::Timestamp) -> Timestamp {
    Timestamp {
        timestamp: MillisSinceEpoch(proto.millis_since_epoch),
        tz_offset: proto.tz_offset,
    }
}

fn operation_metadata_to_proto(
    metadata: &OperationMetadata,
) -> crate::protos::op_store::OperationMetadata {
    let mut proto = crate::protos::op_store::OperationMetadata::new();
    proto.start_time = MessageField::some(timestamp_to_proto(&metadata.start_time));
    proto.end_time = MessageField::some(timestamp_to_proto(&metadata.end_time));
    proto.description = metadata.description.clone();
    proto.hostname = metadata.hostname.clone();
    proto.username = metadata.username.clone();
    proto.tags = metadata.tags.clone();
    proto
}

fn operation_metadata_from_proto(
    proto: &crate::protos::op_store::OperationMetadata,
) -> OperationMetadata {
    let start_time = timestamp_from_proto(&proto.start_time);
    let end_time = timestamp_from_proto(&proto.end_time);
    let description = proto.description.to_owned();
    let hostname = proto.hostname.to_owned();
    let username = proto.username.to_owned();
    let tags = proto.tags.clone();
    OperationMetadata {
        start_time,
        end_time,
        description,
        hostname,
        username,
        tags,
    }
}

fn operation_to_proto(operation: &Operation) -> crate::protos::op_store::Operation {
    let mut proto = crate::protos::op_store::Operation::new();
    proto.view_id = operation.view_id.as_bytes().to_vec();
    for parent in &operation.parents {
        proto.parents.push(parent.to_bytes());
    }
    proto.metadata = MessageField::some(operation_metadata_to_proto(&operation.metadata));
    proto
}

fn operation_from_proto(proto: &crate::protos::op_store::Operation) -> Operation {
    let operation_id_from_proto = |parent: &Vec<u8>| OperationId::new(parent.clone());
    let parents = proto.parents.iter().map(operation_id_from_proto).collect();
    let view_id = ViewId::new(proto.view_id.clone());
    let metadata = operation_metadata_from_proto(&proto.metadata);
    Operation {
        view_id,
        parents,
        metadata,
    }
}

fn view_to_proto(view: &View) -> crate::protos::op_store::View {
    let mut proto = crate::protos::op_store::View::new();
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
        let mut branch_proto = crate::protos::op_store::Branch::new();
        branch_proto.name = name.clone();
        if let Some(local_target) = &target.local_target {
            branch_proto.local_target = MessageField::some(ref_target_to_proto(local_target));
        }
        for (remote_name, target) in &target.remote_targets {
            let mut remote_branch_proto = crate::protos::op_store::RemoteBranch::new();
            remote_branch_proto.remote_name = remote_name.clone();
            remote_branch_proto.target = MessageField::some(ref_target_to_proto(target));
            branch_proto.remote_branches.push(remote_branch_proto);
        }
        proto.branches.push(branch_proto);
    }

    for (name, target) in &view.tags {
        let mut tag_proto = crate::protos::op_store::Tag::new();
        tag_proto.name = name.clone();
        tag_proto.target = MessageField::some(ref_target_to_proto(target));
        proto.tags.push(tag_proto);
    }

    for (git_ref_name, target) in &view.git_refs {
        let mut git_ref_proto = crate::protos::op_store::GitRef::new();
        git_ref_proto.name = git_ref_name.clone();
        git_ref_proto.target = MessageField::some(ref_target_to_proto(target));
        proto.git_refs.push(git_ref_proto);
    }

    if let Some(git_head) = &view.git_head {
        proto.git_head = git_head.to_bytes();
    }

    proto
}

fn view_from_proto(proto: &crate::protos::op_store::View) -> View {
    let mut view = View::default();
    // For compatibility with old repos before we had support for multiple working
    // copies
    if !proto.wc_commit_id.is_empty() {
        view.wc_commit_ids.insert(
            WorkspaceId::default(),
            CommitId::new(proto.wc_commit_id.clone()),
        );
    }
    for (workspace_id, commit_id) in &proto.wc_commit_ids {
        view.wc_commit_ids.insert(
            WorkspaceId::new(workspace_id.clone()),
            CommitId::new(commit_id.clone()),
        );
    }
    for head_id_bytes in &proto.head_ids {
        view.head_ids.insert(CommitId::from_bytes(head_id_bytes));
    }
    for head_id_bytes in &proto.public_head_ids {
        view.public_head_ids
            .insert(CommitId::from_bytes(head_id_bytes));
    }

    for branch_proto in &proto.branches {
        let local_target = branch_proto
            .local_target
            .as_ref()
            .map(ref_target_from_proto);

        let mut remote_targets = BTreeMap::new();
        for remote_branch in &branch_proto.remote_branches {
            remote_targets.insert(
                remote_branch.remote_name.clone(),
                ref_target_from_proto(&remote_branch.target),
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

    for tag_proto in &proto.tags {
        view.tags.insert(
            tag_proto.name.clone(),
            ref_target_from_proto(&tag_proto.target),
        );
    }

    for git_ref in &proto.git_refs {
        if let Some(target) = git_ref.target.as_ref() {
            view.git_refs
                .insert(git_ref.name.clone(), ref_target_from_proto(target));
        } else {
            // Legacy format
            view.git_refs.insert(
                git_ref.name.clone(),
                RefTarget::Normal(CommitId::new(git_ref.commit_id.clone())),
            );
        }
    }

    if !proto.git_head.is_empty() {
        view.git_head = Some(CommitId::new(proto.git_head.clone()));
    }

    view
}

fn ref_target_to_proto(value: &RefTarget) -> crate::protos::op_store::RefTarget {
    let mut proto = crate::protos::op_store::RefTarget::new();
    match value {
        RefTarget::Normal(id) => {
            proto.set_commit_id(id.to_bytes());
        }
        RefTarget::Conflict { removes, adds } => {
            let mut ref_conflict_proto = crate::protos::op_store::RefConflict::new();
            for id in removes {
                ref_conflict_proto.removes.push(id.to_bytes());
            }
            for id in adds {
                ref_conflict_proto.adds.push(id.to_bytes());
            }
            proto.set_conflict(ref_conflict_proto);
        }
    }
    proto
}

fn ref_target_from_proto(proto: &crate::protos::op_store::RefTarget) -> RefTarget {
    match proto.value.as_ref().unwrap() {
        crate::protos::op_store::ref_target::Value::CommitId(id) => {
            RefTarget::Normal(CommitId::from_bytes(id))
        }
        crate::protos::op_store::ref_target::Value::Conflict(conflict) => {
            let removes = conflict
                .removes
                .iter()
                .map(|id_bytes| CommitId::from_bytes(id_bytes))
                .collect_vec();
            let adds = conflict
                .adds
                .iter()
                .map(|id_bytes| CommitId::from_bytes(id_bytes))
                .collect_vec();
            RefTarget::Conflict { removes, adds }
        }
    }
}

#[cfg(test)]
mod tests {
    use maplit::{btreemap, hashmap, hashset};

    use super::*;
    use crate::testutils;

    #[test]
    fn test_read_write_view() {
        let temp_dir = testutils::new_temp_dir();
        let store = SimpleOpStore::init(temp_dir.path().to_owned());
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
        let view = View {
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
            git_head: Some(CommitId::from_hex("fff111")),
            wc_commit_ids: hashmap! {
                WorkspaceId::default() => default_wc_commit_id,
                WorkspaceId::new("test".to_string()) => test_wc_commit_id,
            },
        };
        let view_id = store.write_view(&view).unwrap();
        let read_view = store.read_view(&view_id).unwrap();
        assert_eq!(read_view, view);
    }

    #[test]
    fn test_read_write_operation() {
        let temp_dir = testutils::new_temp_dir();
        let store = SimpleOpStore::init(temp_dir.path().to_owned());
        let operation = Operation {
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
        };
        let op_id = store.write_operation(&operation).unwrap();
        let read_operation = store.read_operation(&op_id).unwrap();
        assert_eq!(read_operation, operation);
    }
}
