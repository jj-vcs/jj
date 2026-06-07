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

//! Workspace store for managing workspace metadata.

use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;

use crate::ref_name::WorkspaceName;

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
