// Copyright 2025 The Jujutsu Authors
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

//! Workspace store for managing workspace metadata.

use std::fmt::Debug;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use jj_lib::file_util::BadPathEncoding;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::file_util::path_from_bytes;
use jj_lib::file_util::path_to_bytes;
use jj_lib::file_util::persist_temp_file;
use jj_lib::file_util::relative_path;
use jj_lib::file_util::slash_path;
use jj_lib::lock::FileLock;
use jj_lib::lock::FileLockError;
use jj_lib::protos::simple_workspace_store;
use jj_lib::ref_name::WorkspaceName;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

/// Errors that can occur when interacting with a workspace store.
#[derive(Error, Debug)]
pub enum WorkspaceStoreError {
    /// An unspecified error occurred.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// A storage backend for workspace metadata.
pub trait WorkspaceStore: Send + Sync + Debug {
    /// Returns the name of this workspace store implementation.
    fn name(&self) -> &str;

    /// Adds a workspace with the given name and path to the store.
    fn add(&self, workspace_name: &WorkspaceName, path: &Path) -> Result<(), WorkspaceStoreError>;

    /// Forgets the workspaces with the given names.
    fn forget(&self, workspace_names: &[&WorkspaceName]) -> Result<(), WorkspaceStoreError>;

    /// Renames a workspace from `old_name` to `new_name`.
    fn rename(
        &self,
        old_name: &WorkspaceName,
        new_name: &WorkspaceName,
    ) -> Result<(), WorkspaceStoreError>;

    /// Gets the path of the workspace with the given name, if it exists.
    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError>;
}

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
    repo_path: PathBuf,
    store_file: PathBuf,
    lock_file: PathBuf,
}

impl SimpleWorkspaceStore {
    /// Loads the workspace store from the given repository path.
    pub fn load(repo_path: &Path) -> Result<Self, WorkspaceStoreError> {
        let store_dir = repo_path.join("workspace_store");
        let file = store_dir.join("index");

        let store = Self {
            repo_path: repo_path.to_path_buf(),
            store_file: file.clone(),
            lock_file: file.with_extension("lock"),
        };

        // Ensure the workspace_store directory exists. We need this
        // for repos that were created before workspace_store was added.
        if !store_dir.exists() {
            fs::create_dir(&store_dir)
                .context(store_dir)
                .map_err(SimpleWorkspaceStoreError::Path)?;

            let _lock = store.lock()?;

            store.write_store(simple_workspace_store::Workspaces::default())?;
        }

        Ok(store)
    }

    fn lock(&self) -> Result<FileLock, SimpleWorkspaceStoreError> {
        Ok(FileLock::lock(self.lock_file.clone())?)
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
        "simple"
    }

    fn add(&self, workspace_name: &WorkspaceName, path: &Path) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock()?;

        let mut workspaces_proto = self.read_store()?;

        // Delete any existing entry with the same name
        workspaces_proto
            .workspaces
            .retain(|w| w.name.as_str() != workspace_name.as_str());

        let path_to_store = relative_path(&self.repo_path, path);
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
        let _lock = self.lock()?;

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
        let _lock = self.lock()?;

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
