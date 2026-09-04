// Copyright 2021-2022 The Jujutsu Authors
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

#![expect(missing_docs)]

use std::fmt::Debug;
use std::fmt::Formatter;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;
use jj_core::ref_name::WorkspaceName;
use jj_core::ref_name::WorkspaceNameBuf;
use thiserror::Error;

use crate::backend::BackendInitError;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::hex_util;
use crate::lock::FileLock;
use crate::object_id::ObjectId as _;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_heads_store::OpHeadsStoreLock;
use crate::op_store::OperationId;
use crate::workspace::WorkspaceType;

/// Error that may occur during [`SimpleOpHeadsStore`] initialization.
#[derive(Debug, Error)]
#[error("Failed to initialize simple operation heads store")]
pub struct SimpleOpHeadsStoreInitError(#[from] pub PathError);

impl From<SimpleOpHeadsStoreInitError> for BackendInitError {
    fn from(err: SimpleOpHeadsStoreInitError) -> Self {
        Self(err.into())
    }
}

pub struct SimpleOpHeadsStore {
    root_dir: PathBuf,
    _workspace_name: WorkspaceNameBuf,
    _workspace_type: WorkspaceType,
}

impl Debug for SimpleOpHeadsStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleOpHeadsStore")
            .field("root_dir", &self.root_dir)
            .finish()
    }
}

impl SimpleOpHeadsStore {
    pub fn name() -> &'static str {
        "simple_op_heads_store"
    }

    pub fn init(
        root_dir: &Path,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
        root_op_id: &OperationId,
    ) -> Result<Self, SimpleOpHeadsStoreInitError> {
        let store = Self {
            root_dir: root_dir.to_path_buf(),
            _workspace_name: workspace_name.to_owned(),
            _workspace_type: workspace_type,
        };
        store.initialize(workspace_name, workspace_type, root_op_id)?;
        Ok(store)
    }

    fn initialize(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
        root_op_id: &OperationId,
    ) -> Result<(), SimpleOpHeadsStoreInitError> {
        let repo_heads_dir = self.repo_heads_dir();
        fs::create_dir(&repo_heads_dir).context(&repo_heads_dir)?;
        let workspace_heads_root = self.root_dir.join("workspace_heads");
        fs::create_dir(&workspace_heads_root).context(&workspace_heads_root)?;
        match workspace_type {
            WorkspaceType::Regular => {
                // Nothing to do here
            }
            WorkspaceType::Independent => {
                // TODO: Reuse existing logic:
                // self.init_per_workspace_op_heads(workspace_name, root_op_id)?;
                let workspace_opheads_dir = self.workspace_heads_dir(workspace_name);
                fs::create_dir(&workspace_opheads_dir).context(&workspace_opheads_dir)?;
            }
        }
        self.add_op_head(workspace_name, workspace_type, root_op_id)?;
        Ok(())
    }

    pub fn load(
        root_dir: &Path,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
    ) -> Self {
        Self {
            root_dir: root_dir.to_path_buf(),
            _workspace_name: workspace_name.to_owned(),
            _workspace_type: workspace_type,
        }
    }

    fn add_op_head(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
        id: &OperationId,
    ) -> Result<(), PathError> {
        let dir = self.operations_dir(workspace_name, workspace_type);
        let path = dir.join(id.hex());
        std::fs::write(&path, "").context(path)
    }

    fn remove_op_head(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
        id: &OperationId,
    ) -> Result<(), PathError> {
        let dir = self.operations_dir(workspace_name, workspace_type);
        let path = dir.join(id.hex());
        std::fs::remove_file(&path)
            .or_else(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    // It's fine if the old head was not found. It probably means
                    // that we're on a distributed file system where the locking
                    // doesn't work. We'll probably end up with two current
                    // heads. We'll detect that next time we load the view.
                    Ok(())
                } else {
                    Err(err)
                }
            })
            .context(path)
    }

    fn operations_dir(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
    ) -> PathBuf {
        match workspace_type {
            WorkspaceType::Regular => self.repo_heads_dir(),
            WorkspaceType::Independent => self.workspace_heads_dir(workspace_name),
        }
    }

    fn repo_heads_dir(&self) -> PathBuf {
        self.root_dir.join("heads")
    }

    fn workspace_heads_dir(&self, workspace_name: &WorkspaceName) -> PathBuf {
        // TODO: XXX:W use a different identifier for the directory containing the workspaces op heads.
        self.root_dir
            .join("workspace_heads")
            .join(workspace_name.as_str())
    }
}

struct SimpleOpHeadsStoreLock {
    _lock: FileLock,
}

impl OpHeadsStoreLock for SimpleOpHeadsStoreLock {}

#[async_trait]
impl OpHeadsStore for SimpleOpHeadsStore {
    fn name(&self) -> &str {
        Self::name()
    }

    async fn update_op_heads(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
        old_ids: &[OperationId],
        new_id: &OperationId,
    ) -> Result<(), OpHeadsStoreError> {
        self.add_op_head(workspace_name, workspace_type, new_id)
            .map_err(|err| OpHeadsStoreError::Write {
                new_op_id: new_id.clone(),
                source: err.into(),
            })?;
        for old_id in old_ids {
            if old_id == new_id {
                continue;
            }
            self.remove_op_head(workspace_name, workspace_type, old_id)
                .map_err(|err| OpHeadsStoreError::Write {
                    new_op_id: new_id.clone(),
                    source: err.into(),
                })?;
        }
        Ok(())
    }

    async fn get_op_heads(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
    ) -> Result<Vec<OperationId>, OpHeadsStoreError> {
        let mut op_heads = vec![];
        let dir = self.operations_dir(workspace_name, workspace_type);
        for op_head_entry in
            std::fs::read_dir(&dir).map_err(|err| OpHeadsStoreError::Read(err.into()))?
        {
            let op_head_file_name = op_head_entry
                .map_err(|err| OpHeadsStoreError::Read(err.into()))?
                .file_name();
            let op_head_file_name = op_head_file_name.to_str().ok_or_else(|| {
                OpHeadsStoreError::Read(
                    format!("Non-utf8 in op head file name: {op_head_file_name:?}").into(),
                )
            })?;
            if let Some(op_head) = hex_util::decode_hex(op_head_file_name) {
                op_heads.push(OperationId::new(op_head));
            }
        }
        op_heads.sort();
        if op_heads.is_empty() {
            Err(OpHeadsStoreError::Read(
                "Corrupt repository: no head operation".into(),
            ))
        } else {
            Ok(op_heads)
        }
    }

    async fn init_per_workspace_op_heads(
        &self,
        workspace_name: &WorkspaceName,
        root_op_id: &OperationId,
    ) -> Result<(), OpHeadsStoreError> {
        let workspace_op_heads_dir =
            self.operations_dir(workspace_name, WorkspaceType::Independent);
        fs::create_dir(&workspace_op_heads_dir)
            .context(&workspace_op_heads_dir)
            .map_err(|err| OpHeadsStoreError::Init(Box::new(err)))?;
        self.add_op_head(workspace_name, WorkspaceType::Independent, root_op_id)
            .map_err(|err| OpHeadsStoreError::Init(Box::new(err)))?;
        Ok(())
    }

    async fn lock(
        &self,
        workspace_name: &WorkspaceName,
        workspace_type: WorkspaceType,
    ) -> Result<Box<dyn OpHeadsStoreLock + '_>, OpHeadsStoreError> {
        let dir = self.operations_dir(workspace_name, workspace_type);
        let lock =
            FileLock::lock(dir.join("lock")).map_err(|err| OpHeadsStoreError::Lock(err.into()))?;
        Ok(Box::new(SimpleOpHeadsStoreLock { _lock: lock }))
    }
}

#[cfg(test)]
mod tests {

    use std::slice;

    use pollster::FutureExt as _;

    use super::*;
    use crate::tests::TestResult;

    #[test]
    fn test_op_heads() -> TestResult {
        let dir = tempfile::tempdir()?;
        let workspace_name = WorkspaceName::DEFAULT.to_owned();
        let workspace_type = WorkspaceType::Regular;

        let op1 = OperationId::from_hex("1111");
        let op2 = OperationId::from_hex("2222");
        let op3 = OperationId::from_hex("3333");
        let op4 = OperationId::from_hex("4444");

        // Initial op head is respected
        let op_heads_store =
            SimpleOpHeadsStore::init(dir.path(), &workspace_name, workspace_type, &op1)?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op1.clone()]);

        // Simple replacement
        op_heads_store
            .update_op_heads(&workspace_name, workspace_type, slice::from_ref(&op1), &op2)
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Duplicating is a no-op
        op_heads_store
            .update_op_heads(&workspace_name, workspace_type, &[], &op2)
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Deleting non-head is a no-op
        op_heads_store
            .update_op_heads(&workspace_name, workspace_type, slice::from_ref(&op1), &op2)
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Can create multiple heads
        op_heads_store
            .update_op_heads(&workspace_name, workspace_type, &[], &op3)
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op2.clone(), op3.clone()]);

        // Can replace multiple heads
        op_heads_store
            .update_op_heads(
                &workspace_name,
                workspace_type,
                &[op2.clone(), op3.clone()],
                &op4,
            )
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op4.clone()]);

        // Can replace multiple heads by one of the old heads
        op_heads_store
            .update_op_heads(&workspace_name, workspace_type, &[], &op3)
            .block_on()?;
        op_heads_store
            .update_op_heads(
                &workspace_name,
                workspace_type,
                &[op3.clone(), op4.clone()],
                &op4,
            )
            .block_on()?;
        let op_heads = op_heads_store
            .get_op_heads(&workspace_name, workspace_type)
            .block_on()?;
        assert_eq!(op_heads, vec![op4.clone()]);

        Ok(())
    }
}
