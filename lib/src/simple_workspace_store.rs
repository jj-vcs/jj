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

use std::fmt::Debug;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use jj_core::workspace_store::WorkspaceMetadata;
use jj_core::workspace_store::WorkspaceStoreFactory;
use jj_core::workspace_store::WorkspaceType;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::file_util::BadPathEncoding;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
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

/// Builds SimpleWorkspaceStore instances.
pub struct SimpleWorkspaceStoreFactory;

impl WorkspaceStoreFactory for SimpleWorkspaceStoreFactory {
    fn load(&self, repo_path: &Path) -> Result<Box<dyn WorkspaceStore>, WorkspaceStoreError> {
        Ok(Box::new(SimpleWorkspaceStore::load(repo_path)?))
    }
}

/// A simple file-based implementation of `WorkspaceStore`.
#[derive(Clone, Debug)]
pub struct SimpleWorkspaceStore {
    repo_path: PathBuf,
    store_file: PathBuf,
    lock_file: PathBuf,
}

impl SimpleWorkspaceStore {
    fn new(repo_path: &Path, create_if_missing: bool) -> Result<Self, WorkspaceStoreError> {
        let repo_path = dunce::canonicalize(repo_path)
            .context(repo_path)
            .map_err(SimpleWorkspaceStoreError::Path)?;
        let store_dir = repo_path.join("workspace_store");
        let file = store_dir.join("index");
        let store = Self {
            repo_path,
            store_file: file.clone(),
            lock_file: file.with_extension("lock"),
        };
        if create_if_missing {
            // Ensure the workspace_store directory exists. We need this
            // for repos that were created before workspace_store was added.
            if !store_dir.exists() {
                fs::create_dir(&store_dir)
                    .context(store_dir)
                    .map_err(SimpleWorkspaceStoreError::Path)?;

                let _lock = store.lock()?;
                store.write_store(simple_workspace_store::Workspaces::default())?;
            } else {
                // Make sure we can read the state.
                let _lock = store.lock()?;
                let _unused = store.read_store()?;
            }
        } else {
            // Make sure we can read the state.
            // TODO: XXX:W The error handling here is a bit weird.
            if !store_dir
                .metadata()
                .context(&store_dir)
                .map_err(SimpleWorkspaceStoreError::Path)?
                .is_dir()
            {
                return Err(WorkspaceStoreError::StoreNotFound(store_dir));
            }
            let _lock = store.lock()?;
            let _unused = store.read_store()?;
        }
        Ok(store)
    }

    /// Initializes a workspace store at the given path, creating it if necessary.
    pub fn init(repo_path: &Path) -> Result<Self, WorkspaceStoreError> {
        let create_if_missing = true;
        Self::new(repo_path, create_if_missing)
    }

    /// Loads the workspace store from the given repository path.
    fn load(repo_path: &Path) -> Result<Self, WorkspaceStoreError> {
        let create_if_missing = false;
        Self::new(repo_path, create_if_missing)
    }

    fn get_workspace_metadata_by_workspace_name(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceStoreError> {
        let store_proto = self.read_store()?;
        let Some(workspace_proto) = store_proto
            .workspaces
            .iter()
            .find(|w| w.name.as_str() == workspace_name.as_str())
        else {
            return Ok(None);
        };
        let workspace_metadata = Self::proto_to_workspace_metadata(workspace_proto)?;
        Ok(Some(workspace_metadata))
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

    fn to_canonical_workspace_path(
        &self,
        workspace_path: &Path,
    ) -> Result<PathBuf, SimpleWorkspaceStoreError> {
        let workspace_path = dunce::canonicalize(workspace_path)
            .context(workspace_path)
            .map_err(SimpleWorkspaceStoreError::Path)?;
        let workspace_path = relative_path(&self.repo_path, &workspace_path);
        if workspace_path.is_relative() {
            Ok(slash_path(&workspace_path).into_owned())
        } else {
            Ok(workspace_path)
        }
    }

    fn proto_to_workspace_metadata(
        workspace_proto: &simple_workspace_store::Workspace,
    ) -> Result<WorkspaceMetadata, SimpleWorkspaceStoreError> {
        let name = workspace_proto.name.clone().into();
        let path = Self::path_from_bytes(&workspace_proto.path)?;
        let workspace_type = match workspace_proto.workspace_type() {
            simple_workspace_store::WorkspaceType::Regular => WorkspaceType::Regular,
            simple_workspace_store::WorkspaceType::Independent => WorkspaceType::Independent,
        };
        Ok(WorkspaceMetadata::new(name, &path, workspace_type))
    }

    fn workspace_metadata_to_proto(
        workspace_metadata: &WorkspaceMetadata,
    ) -> Result<simple_workspace_store::Workspace, SimpleWorkspaceStoreError> {
        let workspace_type = match workspace_metadata.workspace_type() {
            WorkspaceType::Regular => crate::protos::simple_workspace_store::WorkspaceType::Regular,
            WorkspaceType::Independent => {
                crate::protos::simple_workspace_store::WorkspaceType::Independent
            }
        };
        Ok(simple_workspace_store::Workspace {
            name: workspace_metadata.name().as_str().to_string(),
            path: Self::path_to_bytes(workspace_metadata.workspace_path())?,
            workspace_type: workspace_type.into(),
        })
    }

    fn path_to_bytes(path: &Path) -> Result<Vec<u8>, SimpleWorkspaceStoreError> {
        Ok(crate::file_util::path_to_bytes(path)
            .map_err(SimpleWorkspaceStoreError::BadPathEncoding)?
            .to_owned())
    }

    fn path_from_bytes(path: &[u8]) -> Result<PathBuf, SimpleWorkspaceStoreError> {
        crate::file_util::path_from_bytes(path)
            .map(|p| p.to_path_buf())
            .map_err(SimpleWorkspaceStoreError::BadPathEncoding)
    }
}

impl WorkspaceStore for SimpleWorkspaceStore {
    fn name(&self) -> &'static str {
        "simple"
    }

    // TODO: XXX:W If the add call is adding a workspace that already exists,
    // make sure the workspace type is the same as the existing record.
    fn add(
        &self,
        workspace_name: &WorkspaceName,
        path: &Path,
        workspace_type: WorkspaceType,
    ) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock()?;

        let workspace_path = self.to_canonical_workspace_path(path)?;
        let workspace_metadata =
            WorkspaceMetadata::new(workspace_name.to_owned(), &workspace_path, workspace_type);

        let mut workspaces_proto = self.read_store()?;
        // Delete any existing entry with the same name
        workspaces_proto
            .workspaces
            .retain(|w| w.name.as_str() != workspace_name.as_str());
        workspaces_proto
            .workspaces
            .push(Self::workspace_metadata_to_proto(&workspace_metadata)?);
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

    fn get_workspace_metadata_by_workspace_path(
        &self,
        workspace_path: &Path,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceStoreError> {
        let canonical_workspace_path = self.to_canonical_workspace_path(workspace_path)?;
        let store_proto = self.read_store()?;
        let metadata = store_proto.workspaces.iter().find_map(|workspace_proto| {
            if let Ok(m) = Self::proto_to_workspace_metadata(workspace_proto)
                && m.workspace_path() == canonical_workspace_path
            {
                Some(m)
            } else {
                None
            }
        });
        Ok(metadata)
    }

    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError> {
        let Some(workspace_metadata) =
            self.get_workspace_metadata_by_workspace_name(workspace_name)?
        else {
            return Ok(None);
        };
        Ok(Some(workspace_metadata.workspace_path().to_path_buf()))
    }

    fn get_workspace_type(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<WorkspaceType>, WorkspaceStoreError> {
        let Some(workspace_metadata) =
            self.get_workspace_metadata_by_workspace_name(workspace_name)?
        else {
            return Ok(None);
        };
        Ok(Some(workspace_metadata.workspace_type()))
    }

    fn get_all_workspaces(&self) -> Result<Vec<WorkspaceMetadata>, WorkspaceStoreError> {
        let mut workspaces = self.read_store()?;
        workspaces.workspaces.sort_by(|a, b| a.name.cmp(&b.name));
        let workspaces = workspaces
            .workspaces
            .into_iter()
            .map(|w| Self::proto_to_workspace_metadata(&w))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(workspaces)
    }
}
