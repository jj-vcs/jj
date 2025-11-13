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

pub fn get_workspace_store_dir(repo_path: &Path) -> Result<PathBuf, WorkspaceStoreError> {
    let dir = repo_path.join("workspace_store");

    // Ensure the workspace_store directory exists. We need this
    // for repos that were created before workspace_store was added.
    create_or_reuse_dir(&dir).context(&dir)?;

    Ok(dir)
}

pub fn workspace_store_add(
    workspace_store_dir: &PathBuf,
    workspace_name: &WorkspaceNameBuf,
    destination_path: &PathBuf,
) -> Result<(), WorkspaceStoreError> {
    let workspace_name_string = workspace_name.as_symbol().to_string();

    let workspace_proto = workspace_store::Workspace {
        name: workspace_name_string.clone(),
        path: dunce::canonicalize(destination_path)?
            .to_string_lossy()
            .to_string(),
    };

    let temp_file = NamedTempFile::new_in(workspace_store_dir).context(workspace_store_dir)?;

    temp_file
        .as_file()
        .write_all(&workspace_proto.encode_to_vec())
        .context(temp_file.path())?;

    let workspace_file = workspace_store_dir.join(workspace_name_string);
    persist_temp_file(temp_file, &workspace_file).context(&workspace_file)?;

    Ok(())
}

pub fn workspace_store_read(
    workspace_store_dir: &Path,
    workspace_name: &WorkspaceNameBuf,
) -> Result<workspace_store::Workspace, WorkspaceStoreError> {
    let workspace_file = workspace_store_dir.join(workspace_name.as_symbol().to_string());
    let workspace_data = fs::read(&workspace_file).context(&workspace_file)?;

    let workspace_proto = workspace_store::Workspace::decode(&*workspace_data)?;

    Ok(workspace_proto)
}
