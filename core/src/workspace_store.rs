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
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;

use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;

/// Errors that can occur when interacting with a workspace store.
#[derive(Error, Debug)]
pub enum WorkspaceStoreError {
    /// An unspecified error occurred.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// The type of workspace.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum WorkspaceType {
    /// A regular workspace does not have its own OpHeads, it uses the OpHeads
    /// of the repo.
    Regular,
    /// An independent workspace has its own OpHeads.
    Independent,
}

/// Metadata about a workspace.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct WorkspaceMetadata {
    name: WorkspaceNameBuf,
    workspace_type: WorkspaceType,
}

impl WorkspaceMetadata {
    /// Creates a new `WorkspaceMetadata` instance.
    pub fn new(name: WorkspaceNameBuf, workspace_type: WorkspaceType) -> Self {
        Self {
            name,
            workspace_type,
        }
    }

    /// Returns the name of the workspace.
    pub fn name(&self) -> &WorkspaceName {
        &self.name
    }

    /// Returns the type of the workspace.
    pub fn workspace_type(&self) -> WorkspaceType {
        self.workspace_type
    }
}

/// A storage backend for workspace metadata.
pub trait WorkspaceStore: Send + Sync + Debug {
    /// Returns the name of this workspace store implementation.
    fn name(&self) -> &str;

    /// Adds a workspace with the given name and path to the store.
    fn add(
        &self,
        workspace_name: &WorkspaceName,
        path: &Path,
        workspace_type: WorkspaceType,
    ) -> Result<(), WorkspaceStoreError>;

    /// Forgets the workspaces with the given names.
    fn forget(&self, workspace_names: &[&WorkspaceName]) -> Result<(), WorkspaceStoreError>;

    /// Renames a workspace from `old_name` to `new_name`.
    fn rename(
        &self,
        old_name: &WorkspaceName,
        new_name: &WorkspaceName,
    ) -> Result<(), WorkspaceStoreError>;

    /// Returns the metadata for the workspace matching the given path in this store, if it exists.
    fn get_workspace_metadata_by_workspace_path(
        &self,
        workspace_path: &Path,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceStoreError>;

    /// Gets the path of the workspace with the given name, if it exists.
    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError>;

    /// Gets the type of the workspace with the given name, if it exists.
    fn get_workspace_type(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<WorkspaceType>, WorkspaceStoreError>;
}
