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
    dir: PathBuf,
}

impl Debug for SimpleOpHeadsStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleOpHeadsStore")
            .field("dir", &self.dir)
            .finish()
    }
}

impl SimpleOpHeadsStore {
    pub fn name() -> &'static str {
        "simple_op_heads_store"
    }

    pub fn init(dir: &Path, root_op_id: &OperationId) -> Result<Self, SimpleOpHeadsStoreInitError> {
        let op_heads_dir = dir.join("heads");
        fs::create_dir(&op_heads_dir).context(&op_heads_dir)?;
        let store = Self { dir: op_heads_dir };
        store.add_op_head(root_op_id)?;
        Ok(store)
    }

    pub fn load(dir: &Path) -> Self {
        let op_heads_dir = dir.join("heads");
        Self { dir: op_heads_dir }
    }

    fn add_op_head(&self, id: &OperationId) -> Result<(), PathError> {
        let path = self.dir.join(id.hex());
        std::fs::write(&path, "").context(path)
    }

    fn remove_op_head(&self, id: &OperationId) -> Result<(), PathError> {
        let path = self.dir.join(id.hex());
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
        old_ids: &[OperationId],
        new_id: &OperationId,
    ) -> Result<(), OpHeadsStoreError> {
        assert!(!old_ids.contains(new_id));
        self.add_op_head(new_id)
            .map_err(|err| OpHeadsStoreError::Write {
                new_op_id: new_id.clone(),
                source: err.into(),
            })?;
        for old_id in old_ids {
            self.remove_op_head(old_id)
                .map_err(|err| OpHeadsStoreError::Write {
                    new_op_id: new_id.clone(),
                    source: err.into(),
                })?;
        }
        Ok(())
    }

    async fn get_op_heads(&self) -> Result<Vec<OperationId>, OpHeadsStoreError> {
        let mut op_heads = vec![];
        for op_head_entry in
            std::fs::read_dir(&self.dir).map_err(|err| OpHeadsStoreError::Read(err.into()))?
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

    async fn lock(&self) -> Result<Box<dyn OpHeadsStoreLock + '_>, OpHeadsStoreError> {
        let lock = FileLock::lock(self.dir.join("lock"))
            .map_err(|err| OpHeadsStoreError::Lock(err.into()))?;
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

        let op1 = OperationId::from_hex("1111");
        let op2 = OperationId::from_hex("2222");
        let op3 = OperationId::from_hex("3333");
        let op4 = OperationId::from_hex("4444");

        // Initial op head is respected
        let op_heads_store = SimpleOpHeadsStore::init(dir.path(), &op1)?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op1.clone()]);

        // Simple replacement
        op_heads_store
            .update_op_heads(slice::from_ref(&op1), &op2)
            .block_on()?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Duplicating is a no-op
        op_heads_store.update_op_heads(&[], &op2).block_on()?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Deleting non-head is a no-op
        op_heads_store
            .update_op_heads(slice::from_ref(&op1), &op2)
            .block_on()?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op2.clone()]);

        // Can create multiple heads
        op_heads_store.update_op_heads(&[], &op3).block_on()?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op2.clone(), op3.clone()]);

        // Can replace multiple heads
        op_heads_store
            .update_op_heads(&[op2.clone(), op3.clone()], &op4)
            .block_on()?;
        let op_heads = op_heads_store.get_op_heads().block_on()?;
        assert_eq!(op_heads, vec![op4.clone()]);

        Ok(())
    }
}
