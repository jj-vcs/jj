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

#![expect(missing_docs)]

use std::fmt::Debug;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::file_util::persist_temp_file;
use jj_lib::lock::FileLock;
use jj_lib::lock::FileLockError;
use jj_lib::protos::workspace_store;
use jj_lib::ref_name::WorkspaceNameBuf;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceStoreError {
    #[error("No such workspace: {0}")]
    NoSuchWorkspace(String),
    #[error("Failed to lock workspace store")]
    Lock(#[source] FileLockError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    ProstDecode(#[from] prost::DecodeError),
}

pub trait WorkspaceStore: Sized + Send + Sync + Debug {
    fn name(&self) -> &str;

    fn load(repo_path: &Path) -> Result<Self, WorkspaceStoreError>;

    fn add(
        &self,
        workspace_name: &WorkspaceNameBuf,
        path: &Path,
    ) -> Result<(), WorkspaceStoreError>;

    fn forget(&self, workspace_names: &[WorkspaceNameBuf]) -> Result<(), WorkspaceStoreError>;

    fn rename(
        &self,
        old_name: &WorkspaceNameBuf,
        new_name: &WorkspaceNameBuf,
    ) -> Result<(), WorkspaceStoreError>;

    fn get_workspace(
        &self,
        workspace_name: &WorkspaceNameBuf,
    ) -> Result<Option<workspace_store::Workspace>, WorkspaceStoreError>;
}

#[derive(Debug)]
pub struct SimpleWorkspaceStore {
    repo_path: PathBuf,
    store_file: PathBuf,
    lock_file: PathBuf,
}

impl SimpleWorkspaceStore {
    fn lock(&self) -> Result<FileLock, WorkspaceStoreError> {
        FileLock::lock(self.lock_file.clone()).map_err(WorkspaceStoreError::Lock)
    }

    fn read_store(&self) -> Result<workspace_store::Workspaces, WorkspaceStoreError> {
        let workspace_data = fs::read(&self.store_file).context(&self.store_file)?;

        let workspaces_proto = workspace_store::Workspaces::decode(&*workspace_data)?;

        Ok(workspaces_proto)
    }

    fn write_store(
        &self,
        workspaces_proto: workspace_store::Workspaces,
    ) -> Result<(), WorkspaceStoreError> {
        let temp_file = NamedTempFile::new_in(&self.repo_path).context(&self.repo_path)?;

        temp_file
            .as_file()
            .write_all(&workspaces_proto.encode_to_vec())
            .context(temp_file.path())?;

        persist_temp_file(temp_file, &self.store_file).context(&self.store_file)?;

        Ok(())
    }
}

impl WorkspaceStore for SimpleWorkspaceStore {
    fn name(&self) -> &str {
        "simple"
    }

    fn load(repo_path: &Path) -> Result<Self, WorkspaceStoreError> {
        let file = repo_path.join("workspace_store");

        // Ensure the workspace_store directory exists. We need this
        // for repos that were created before workspace_store was added.
        if !file.exists() {
            let workspaces_proto = workspace_store::Workspaces::default();
            fs::write(&file, workspaces_proto.encode_to_vec()).context(&file)?;
        }

        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            store_file: file.clone(),
            lock_file: file.with_extension("lock"),
        })
    }

    fn add(
        &self,
        workspace_name: &WorkspaceNameBuf,
        path: &Path,
    ) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock()?;

        let mut workspaces_proto = self.read_store()?;

        workspaces_proto
            .workspaces
            .push(workspace_store::Workspace {
                name: workspace_name.as_symbol().to_string(),
                path: dunce::canonicalize(path)?.to_string_lossy().to_string(),
            });

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn forget(&self, workspace_names: &[WorkspaceNameBuf]) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock()?;

        let mut workspaces_proto = self.read_store()?;

        let workspace_names = workspace_names
            .iter()
            .map(|n| n.as_symbol().to_string())
            .collect::<Vec<_>>();

        workspaces_proto
            .workspaces
            .retain(|w| !workspace_names.contains(&w.name));

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn rename(
        &self,
        old_name: &WorkspaceNameBuf,
        new_name: &WorkspaceNameBuf,
    ) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock()?;

        let mut workspaces_proto = self.read_store()?;

        for workspace in &mut workspaces_proto.workspaces {
            if workspace.name == old_name.as_symbol().to_string() {
                workspace.name = new_name.as_symbol().to_string();
            }
        }

        self.write_store(workspaces_proto)?;

        Ok(())
    }

    fn get_workspace(
        &self,
        workspace_name: &WorkspaceNameBuf,
    ) -> Result<Option<workspace_store::Workspace>, WorkspaceStoreError> {
        let _lock = self.lock()?;

        let workspace = self
            .read_store()?
            .workspaces
            .iter()
            .find(|w| w.name == workspace_name.as_symbol().to_string())
            .cloned();

        Ok(workspace)
    }
}
