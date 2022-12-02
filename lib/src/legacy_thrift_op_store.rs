// Copyright 2022 The Jujutsu Authors
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
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;

use itertools::Itertools;
use tempfile::NamedTempFile;
use thrift::protocol::{TCompactInputProtocol, TCompactOutputProtocol, TSerializable};

use crate::backend::{CommitId, MillisSinceEpoch, Timestamp};
use crate::content_hash::blake2b_hash;
use crate::file_util::persist_content_addressed_temp_file;
use crate::op_store::{
    BranchTarget, OpStore, OpStoreError, OpStoreResult, Operation, OperationId, OperationMetadata,
    RefTarget, View, ViewId, WorkspaceId,
};
use crate::simple_op_store_model;

impl From<thrift::Error> for OpStoreError {
    fn from(err: thrift::Error) -> Self {
        OpStoreError::Other(err.to_string())
    }
}

fn not_found_to_store_error(err: std::io::Error) -> OpStoreError {
    if err.kind() == ErrorKind::NotFound {
        OpStoreError::NotFound
    } else {
        OpStoreError::from(err)
    }
}

#[derive(Debug)]
pub struct ThriftOpStore {
    path: PathBuf,
}

impl ThriftOpStore {
    pub fn load(store_path: PathBuf) -> Self {
        ThriftOpStore { path: store_path }
    }

    fn view_path(&self, id: &ViewId) -> PathBuf {
        self.path.join("views").join(id.hex())
    }

    fn operation_path(&self, id: &OperationId) -> PathBuf {
        self.path.join("operations").join(id.hex())
    }
}

impl OpStore for ThriftOpStore {
    fn read_view(&self, id: &ViewId) -> OpStoreResult<View> {
        let path = self.view_path(id);
        let mut file = File::open(path).map_err(not_found_to_store_error)?;
        let thrift_view = read_thrift(&mut file)?;
        Ok(View::from(&thrift_view))
    }

    fn write_view(&self, view: &View) -> OpStoreResult<ViewId> {
        let id = ViewId::new(blake2b_hash(view).to_vec());
        let temp_file = NamedTempFile::new_in(&self.path)?;
        let thrift_view = simple_op_store_model::View::from(view);
        write_thrift(&thrift_view, &mut temp_file.as_file())?;
        persist_content_addressed_temp_file(temp_file, self.view_path(&id))?;
        Ok(id)
    }

    fn read_operation(&self, id: &OperationId) -> OpStoreResult<Operation> {
        let path = self.operation_path(id);
        let mut file = File::open(path).map_err(not_found_to_store_error)?;
        let thrift_operation = read_thrift(&mut file)?;
        Ok(Operation::from(&thrift_operation))
    }

    fn write_operation(&self, operation: &Operation) -> OpStoreResult<OperationId> {
        let id = OperationId::new(blake2b_hash(operation).to_vec());
        let temp_file = NamedTempFile::new_in(&self.path)?;
        let thrift_operation = simple_op_store_model::Operation::from(operation);
        write_thrift(&thrift_operation, &mut temp_file.as_file())?;
        persist_content_addressed_temp_file(temp_file, self.operation_path(&id))?;
        Ok(id)
    }
}

pub fn read_thrift<T: TSerializable>(input: &mut impl Read) -> OpStoreResult<T> {
    let mut protocol = TCompactInputProtocol::new(input);
    Ok(TSerializable::read_from_in_protocol(&mut protocol).unwrap())
}

pub fn write_thrift<T: TSerializable>(
    thrift_object: &T,
    output: &mut impl Write,
) -> OpStoreResult<()> {
    let mut protocol = TCompactOutputProtocol::new(output);
    thrift_object.write_to_out_protocol(&mut protocol)?;
    Ok(())
}

impl From<&Timestamp> for simple_op_store_model::Timestamp {
    fn from(timestamp: &Timestamp) -> Self {
        simple_op_store_model::Timestamp::new(timestamp.timestamp.0, timestamp.tz_offset)
    }
}

impl From<&simple_op_store_model::Timestamp> for Timestamp {
    fn from(timestamp: &simple_op_store_model::Timestamp) -> Self {
        Timestamp {
            timestamp: MillisSinceEpoch(timestamp.millis_since_epoch),
            tz_offset: timestamp.tz_offset,
        }
    }
}

impl From<&OperationMetadata> for simple_op_store_model::OperationMetadata {
    fn from(metadata: &OperationMetadata) -> Self {
        let start_time = simple_op_store_model::Timestamp::from(&metadata.start_time);
        let end_time = simple_op_store_model::Timestamp::from(&metadata.end_time);
        let description = metadata.description.clone();
        let hostname = metadata.hostname.clone();
        let username = metadata.username.clone();
        let tags: BTreeMap<String, String> = metadata
            .tags
            .iter()
            .map(|(x, y)| (x.clone(), y.clone()))
            .collect();
        simple_op_store_model::OperationMetadata::new(
            start_time,
            end_time,
            description,
            hostname,
            username,
            tags,
        )
    }
}

impl From<&simple_op_store_model::OperationMetadata> for OperationMetadata {
    fn from(metadata: &simple_op_store_model::OperationMetadata) -> Self {
        let start_time = Timestamp::from(&metadata.start_time);
        let end_time = Timestamp::from(&metadata.end_time);
        let description = metadata.description.to_owned();
        let hostname = metadata.hostname.to_owned();
        let username = metadata.username.to_owned();
        let tags = metadata
            .tags
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        OperationMetadata {
            start_time,
            end_time,
            description,
            hostname,
            username,
            tags,
        }
    }
}

impl From<&Operation> for simple_op_store_model::Operation {
    fn from(operation: &Operation) -> Self {
        let view_id = operation.view_id.as_bytes().to_vec();
        let mut parents = vec![];
        for parent in &operation.parents {
            parents.push(parent.to_bytes());
        }
        let metadata = Box::new(simple_op_store_model::OperationMetadata::from(
            &operation.metadata,
        ));
        simple_op_store_model::Operation::new(view_id, parents, metadata)
    }
}

impl From<&View> for simple_op_store_model::View {
    fn from(view: &View) -> Self {
        let mut wc_commit_ids = BTreeMap::new();
        for (workspace_id, commit_id) in &view.wc_commit_ids {
            wc_commit_ids.insert(workspace_id.as_str().to_string(), commit_id.to_bytes());
        }

        let mut head_ids = vec![];
        for head_id in &view.head_ids {
            head_ids.push(head_id.to_bytes());
        }

        let mut public_head_ids = vec![];
        for head_id in &view.public_head_ids {
            public_head_ids.push(head_id.to_bytes());
        }

        let mut branches = vec![];
        for (name, target) in &view.branches {
            let local_target = target
                .local_target
                .as_ref()
                .map(simple_op_store_model::RefTarget::from);
            let mut remote_branches = vec![];
            for (remote_name, target) in &target.remote_targets {
                remote_branches.push(simple_op_store_model::RemoteBranch::new(
                    remote_name.clone(),
                    simple_op_store_model::RefTarget::from(target),
                ));
            }
            branches.push(simple_op_store_model::Branch::new(
                name.clone(),
                local_target,
                remote_branches,
            ));
        }

        let mut tags = vec![];
        for (name, target) in &view.tags {
            tags.push(simple_op_store_model::Tag::new(
                name.clone(),
                simple_op_store_model::RefTarget::from(target),
            ));
        }

        let mut git_refs = vec![];
        for (git_ref_name, target) in &view.git_refs {
            git_refs.push(simple_op_store_model::GitRef::new(
                git_ref_name.clone(),
                simple_op_store_model::RefTarget::from(target),
            ));
        }

        let git_head = view.git_head.as_ref().map(|git_head| git_head.to_bytes());

        simple_op_store_model::View::new(
            head_ids,
            public_head_ids,
            wc_commit_ids,
            branches,
            tags,
            git_refs,
            git_head,
        )
    }
}

impl From<&simple_op_store_model::Operation> for Operation {
    fn from(operation: &simple_op_store_model::Operation) -> Self {
        let operation_id_from_thrift = |parent: &Vec<u8>| OperationId::new(parent.clone());
        let parents = operation
            .parents
            .iter()
            .map(operation_id_from_thrift)
            .collect();
        let view_id = ViewId::new(operation.view_id.clone());
        let metadata = OperationMetadata::from(operation.metadata.as_ref());
        Operation {
            view_id,
            parents,
            metadata,
        }
    }
}

impl From<&simple_op_store_model::View> for View {
    fn from(thrift_view: &simple_op_store_model::View) -> Self {
        let mut view = View::default();
        for (workspace_id, commit_id) in &thrift_view.wc_commit_ids {
            view.wc_commit_ids.insert(
                WorkspaceId::new(workspace_id.clone()),
                CommitId::new(commit_id.clone()),
            );
        }
        for head_id_bytes in &thrift_view.head_ids {
            view.head_ids.insert(CommitId::from_bytes(head_id_bytes));
        }
        for head_id_bytes in &thrift_view.public_head_ids {
            view.public_head_ids
                .insert(CommitId::from_bytes(head_id_bytes));
        }

        for thrift_branch in &thrift_view.branches {
            let local_target = thrift_branch.local_target.as_ref().map(RefTarget::from);

            let mut remote_targets = BTreeMap::new();
            for remote_branch in &thrift_branch.remote_branches {
                remote_targets.insert(
                    remote_branch.remote_name.clone(),
                    RefTarget::from(&remote_branch.target),
                );
            }

            view.branches.insert(
                thrift_branch.name.clone(),
                BranchTarget {
                    local_target,
                    remote_targets,
                },
            );
        }

        for thrift_tag in &thrift_view.tags {
            view.tags
                .insert(thrift_tag.name.clone(), RefTarget::from(&thrift_tag.target));
        }

        for git_ref in &thrift_view.git_refs {
            view.git_refs
                .insert(git_ref.name.clone(), RefTarget::from(&git_ref.target));
        }

        view.git_head = thrift_view
            .git_head
            .as_ref()
            .map(|head| CommitId::new(head.clone()));

        view
    }
}

impl From<&RefTarget> for simple_op_store_model::RefTarget {
    fn from(ref_target: &RefTarget) -> Self {
        match ref_target {
            RefTarget::Normal(id) => simple_op_store_model::RefTarget::CommitId(id.to_bytes()),
            RefTarget::Conflict { removes, adds } => {
                let adds = adds.iter().map(|id| id.to_bytes()).collect_vec();
                let removes = removes.iter().map(|id| id.to_bytes()).collect_vec();
                let ref_conflict_thrift = simple_op_store_model::RefConflict::new(removes, adds);
                simple_op_store_model::RefTarget::Conflict(ref_conflict_thrift)
            }
        }
    }
}

impl From<&simple_op_store_model::RefTarget> for RefTarget {
    fn from(thrift_ref_target: &simple_op_store_model::RefTarget) -> Self {
        match thrift_ref_target {
            simple_op_store_model::RefTarget::CommitId(commit_id) => {
                RefTarget::Normal(CommitId::from_bytes(commit_id))
            }
            simple_op_store_model::RefTarget::Conflict(conflict) => {
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
}
