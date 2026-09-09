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

//! Contains the implementation of a simple file-based `WorkspaceStore`.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use jj_core::backend::BackendInitError;
use jj_core::backend::BackendLoadError;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::file_util::BadPathEncoding;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::path_from_bytes;
use crate::file_util::path_to_bytes;
use crate::file_util::persist_temp_file;
use crate::file_util::relative_path;
use crate::file_util::slash_path;
use crate::lock::FileLock;
use crate::lock::FileLockError;
use crate::protos::simple_workspace_store;
use crate::ref_name::WorkspaceName;
use crate::workspace_store::WorkspaceStore;
use crate::workspace_store::WorkspaceStoreError;

/// Errors specific to the `SimpleWorkspaceStore` implementation.
#[derive(Error, Debug)]
pub enum SimpleWorkspaceStoreError {
    /// An I/O error related to a file path.
    #[error(transparent)]
    Path(#[from] PathError),
    /// An error occurred while trying to lock the workspace store.
    #[error("Failed to lock workspace store")]
    Lock(#[from] FileLockError),
    /// An error occurred while decoding Protobuf data.
    #[error(transparent)]
    ProstDecode(#[from] prost::DecodeError),
    /// An error occurred due to bad path encoding.
    #[error(transparent)]
    BadPathEncoding(#[from] BadPathEncoding),
}

impl From<SimpleWorkspaceStoreError> for WorkspaceStoreError {
    fn from(err: SimpleWorkspaceStoreError) -> Self {
        Self::Other(Box::new(err))
    }
}

/// A simple file-based implementation of `WorkspaceStore`.
#[derive(Debug)]
pub struct SimpleWorkspaceStore {
    store_dir: PathBuf,
    store_file: PathBuf,
    lock_file: PathBuf,
}

impl SimpleWorkspaceStore {
    /// Returns the name of this WorkspaceStore implementation.
    pub fn name() -> &'static str {
        "simple_workspace_store"
    }

    fn new(store_dir: &Path) -> Self {
        let store_dir = store_dir.to_path_buf();
        let store_file = store_dir.join("index");
        let lock_file = store_file.with_extension("lock");
        Self {
            store_dir,
            store_file,
            lock_file,
        }
    }

    /// Loads the workspace store from the given store path and repo.
    pub fn load(store_dir: &Path) -> Result<Self, BackendLoadError> {
        let store = Self::new(store_dir);
        if !store.store_file.exists() {
            // TODO: Remove this in jj 0.57+: older repos do not have a workspace store index so we initialize it here.
            store
                .initialize()
                .map_err(|err| BackendLoadError(err.into()))?;
        }
        Ok(store)
    }

    /// Initializes this SimpleWorkspaceStore with an empty index.
    pub fn init(store_dir: &Path) -> Result<Self, BackendInitError> {
        let store = Self::new(store_dir);
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<(), BackendInitError> {
        let _lock = self.lock().map_err(|err| BackendInitError(err.into()))?;
        self.write_store(simple_workspace_store::Workspaces::default())
            .map_err(|err| BackendInitError(err.into()))?;
        Ok(())
    }

    fn lock(&self) -> Result<FileLock, FileLockError> {
        FileLock::lock(self.lock_file.clone())
    }

    fn read_store(&self) -> Result<simple_workspace_store::Workspaces, SimpleWorkspaceStoreError> {
        let workspace_data = fs::read(&self.store_file).context(&self.store_file)?;

        let workspaces_proto = simple_workspace_store::Workspaces::decode(&*workspace_data)?;

        Ok(workspaces_proto)
    }

    fn write_store(
        &self,
        workspaces_proto: simple_workspace_store::Workspaces,
    ) -> Result<(), SimpleWorkspaceStoreError> {
        // We had created the store dir in load(), so parent() must exist.
        let store_file_parent = self.store_file.parent().unwrap();
        let temp_file = NamedTempFile::new_in(store_file_parent).context(store_file_parent)?;

        temp_file
            .as_file()
            .write_all(&workspaces_proto.encode_to_vec())
            .context(temp_file.path())?;

        persist_temp_file(temp_file, &self.store_file).context(&self.store_file)?;

        Ok(())
    }
}

impl WorkspaceStore for SimpleWorkspaceStore {
    fn name(&self) -> &'static str {
        Self::name()
    }

    fn add(&self, workspace_name: &WorkspaceName, path: &Path) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;

        let mut workspaces_proto = self.read_store()?;

        // Delete any existing entry with the same name
        workspaces_proto
            .workspaces
            .retain(|w| w.name.as_str() != workspace_name.as_str());

        let repo_path = self
            .store_dir
            .parent()
            .expect("store_dir must be under the repo_path");
        let path_to_store = relative_path(repo_path, path);
        let path_to_store = if path_to_store.is_relative() {
            slash_path(&path_to_store).into_owned()
        } else {
            path_to_store
        };
        workspaces_proto
            .workspaces
            .push(simple_workspace_store::Workspace {
                name: workspace_name.as_str().to_string(),
                path: path_to_bytes(&path_to_store)
                    .map_err(SimpleWorkspaceStoreError::BadPathEncoding)?
                    .to_owned(),
            });

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn forget(&self, workspace_names: &[&WorkspaceName]) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;

        let mut workspaces_proto = self.read_store()?;

        workspaces_proto.workspaces.retain(|w| {
            !workspace_names
                .iter()
                .any(|name| w.name.as_str() == name.as_str())
        });

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn rename(
        &self,
        old_name: &WorkspaceName,
        new_name: &WorkspaceName,
    ) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;

        let mut workspaces_proto = self.read_store()?;

        for workspace in &mut workspaces_proto.workspaces {
            if workspace.name.as_str() == old_name.as_str() {
                workspace.name = new_name.as_str().to_string();
            }
        }

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError> {
        let workspace = self
            .read_store()?
            .workspaces
            .iter()
            .find(|w| w.name.as_str() == workspace_name.as_str())
            .cloned();

        Ok(workspace
            .map(|w| {
                path_from_bytes(&w.path)
                    .map(|p| p.to_path_buf())
                    .map_err(SimpleWorkspaceStoreError::BadPathEncoding)
            })
            .transpose()?)
    }
}
