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
use jj_lib::file_util::create_or_reuse_dir;
use jj_lib::file_util::persist_temp_file;
use jj_lib::protos::workspace_store;
use jj_lib::ref_name::WorkspaceNameBuf;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceStoreError {
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

    fn exists(&self, workspace_name: &WorkspaceNameBuf) -> bool;

    fn get_path(
        &self,
        workspace_name: &WorkspaceNameBuf,
    ) -> Result<workspace_store::Workspace, WorkspaceStoreError>;

    fn set_path(
        &self,
        workspace_name: &WorkspaceNameBuf,
        path: &Path,
    ) -> Result<(), WorkspaceStoreError>;

    fn remove_path(&self, workspace_name: &WorkspaceNameBuf) -> Result<(), WorkspaceStoreError>;
}

#[derive(Debug)]
pub struct SimpleWorkspaceStore {
    workspace_store_dir: PathBuf,
}

impl SimpleWorkspaceStore {
    fn get_file(&self, workspace_name: &WorkspaceNameBuf) -> PathBuf {
        self.workspace_store_dir
            .join(workspace_name.as_symbol().to_string())
    }
}

impl WorkspaceStore for SimpleWorkspaceStore {
    fn name(&self) -> &str {
        "simple"
    }

    fn load(repo_path: &Path) -> Result<Self, WorkspaceStoreError> {
        let dir = repo_path.join("workspace_store");

        // Ensure the workspace_store directory exists. We need this
        // for repos that were created before workspace_store was added.
        create_or_reuse_dir(&dir).context(&dir)?;

        Ok(Self {
            workspace_store_dir: dir,
        })
    }

    fn exists(&self, workspace_name: &WorkspaceNameBuf) -> bool {
        self.get_file(workspace_name).exists()
    }

    fn get_path(
        &self,
        workspace_name: &WorkspaceNameBuf,
    ) -> Result<workspace_store::Workspace, WorkspaceStoreError> {
        let workspace_file = self.get_file(workspace_name);

        let workspace_data = fs::read(&workspace_file).context(&workspace_file)?;

        let workspace_proto = workspace_store::Workspace::decode(&*workspace_data)?;

        Ok(workspace_proto)
    }

    fn set_path(
        &self,
        workspace_name: &WorkspaceNameBuf,
        path: &Path,
    ) -> Result<(), WorkspaceStoreError> {
        let workspace_file = self.get_file(workspace_name);
        let workspace_name_string = workspace_name.as_symbol().to_string();

        let workspace_proto = workspace_store::Workspace {
            name: workspace_name_string.clone(),
            path: dunce::canonicalize(path)?.to_string_lossy().to_string(),
        };

        let temp_file =
            NamedTempFile::new_in(&self.workspace_store_dir).context(&self.workspace_store_dir)?;

        temp_file
            .as_file()
            .write_all(&workspace_proto.encode_to_vec())
            .context(temp_file.path())?;

        persist_temp_file(temp_file, &workspace_file).context(&workspace_file)?;

        Ok(())
    }

    fn remove_path(&self, workspace_name: &WorkspaceNameBuf) -> Result<(), WorkspaceStoreError> {
        let workspace_file = self.get_file(workspace_name);

        fs::remove_file(&workspace_file).context(&workspace_file)?;

        Ok(())
    }
}
