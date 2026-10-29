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

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use jj_core::backend::BackendInitError;
use jj_core::backend::BackendLoadError;
use jj_core::backend::WorkspaceId;
use jj_core::ref_name::WorkspaceNameBuf;
use jj_core::workspace_store::WorkspaceMetadata;
use jj_core::workspace_store::WorkspaceType;
use prost::Message as _;
use thiserror::Error;

use crate::file_util::BadPathEncoding;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::new_temp_file_with_default_permissions;
use crate::file_util::normalize_path;
use crate::file_util::path_from_bytes;
use crate::file_util::path_to_bytes;
use crate::file_util::persist_temp_file;
use crate::file_util::relative_path;
use crate::file_util::slash_path;
use crate::lock::FileLock;
use crate::lock::FileLockError;
use crate::object_id::ObjectId as _;
use crate::protos::simple_workspace_store;
use crate::ref_name::WorkspaceName;
use crate::settings::JJRng;
use crate::settings::UserSettings;
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
    rng: Arc<JJRng>,
}

struct WorkspaceStoreState {
    repo_path: PathBuf,
    workspaces: BTreeMap<WorkspaceNameBuf, WorkspaceMetadata>,
    path_to_name: HashMap<PathBuf, WorkspaceNameBuf>,
    rng: Arc<JJRng>,
}

impl WorkspaceStoreState {
    fn empty(repo_path: &Path, rng: Arc<JJRng>) -> Self {
        Self {
            repo_path: repo_path.to_path_buf(),
            workspaces: BTreeMap::new(),
            path_to_name: HashMap::new(),
            rng,
        }
    }

    fn new(
        repo_path: &Path,
        workspaces_proto: simple_workspace_store::Workspaces,
        rng: Arc<JJRng>,
    ) -> Result<Self, SimpleWorkspaceStoreError> {
        let mut workspaces = BTreeMap::new();
        let mut id_to_name = HashMap::with_capacity(workspaces_proto.workspaces.len());
        let mut path_to_name = HashMap::with_capacity(workspaces_proto.workspaces.len());
        for w in &workspaces_proto.workspaces {
            let workspace_name: WorkspaceNameBuf = w.name.clone().into();
            let workspace_type = match w.workspace_type() {
                simple_workspace_store::WorkspaceType::Regular => WorkspaceType::Regular,
                simple_workspace_store::WorkspaceType::Independent => WorkspaceType::Independent,
            };
            let workspace_id =
                Self::proto_to_workspace_id(&rng, workspace_type, w.workspace_id.as_ref());
            let workspace_path = path_from_bytes(&w.path)
                .map_err(SimpleWorkspaceStoreError::BadPathEncoding)?
                .to_owned();
            // There should not be two workspaces with the same name, but if there are any, the last one wins.
            workspaces.insert(
                workspace_name.clone(),
                WorkspaceMetadata::new(
                    workspace_name,
                    workspace_type,
                    workspace_id,
                    workspace_path,
                ),
            );
        }

        let mut corrupt_entries = HashSet::new();
        for (name, metadata) in &workspaces {
            if let Some(prev_name) =
                path_to_name.insert(metadata.workspace_path().to_path_buf(), name.clone())
            {
                corrupt_entries.insert(prev_name);
            }
            if let Some(id) = metadata.workspace_id()
                && let Some(prev_name) = id_to_name.insert(id, name.clone())
            {
                corrupt_entries.insert(prev_name);
            }
        }
        workspaces.retain(|name, _metadata| !corrupt_entries.contains(name));

        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            workspaces,
            path_to_name,
            rng,
        })
    }

    fn into_proto(self) -> Result<simple_workspace_store::Workspaces, SimpleWorkspaceStoreError> {
        let mut workspaces_proto = simple_workspace_store::Workspaces::default();
        for (workspace_name, metadata) in self.workspaces {
            let workspace_type = match metadata.workspace_type() {
                WorkspaceType::Regular => {
                    crate::protos::simple_workspace_store::WorkspaceType::Regular
                }
                WorkspaceType::Independent => {
                    crate::protos::simple_workspace_store::WorkspaceType::Independent
                }
            };
            let workspace_id = metadata
                .workspace_id()
                .map(|id| simple_workspace_store::WorkspaceId { id: id.hex() });
            workspaces_proto
                .workspaces
                .push(simple_workspace_store::Workspace {
                    workspace_id,
                    name: workspace_name.as_str().to_string(),
                    path: path_to_bytes(metadata.workspace_path())
                        .map_err(SimpleWorkspaceStoreError::BadPathEncoding)?
                        .to_owned(),
                    workspace_type: workspace_type.into(),
                });
        }
        Ok(workspaces_proto)
    }

    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError> {
        if let Some(metadata) = self.workspaces.get(workspace_name) {
            // Resolve ".." components literally and convert to native separators
            Ok(Some(normalize_path(
                &self.repo_path.join(metadata.workspace_path()),
            )))
        } else {
            Ok(None)
        }
    }

    fn get_workspace_metadata_by_workspace_name(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Option<WorkspaceMetadata> {
        self.workspaces.get(workspace_name).cloned()
    }

    fn get_workspace_metadata_by_workspace_path(&self, path: &Path) -> Option<WorkspaceMetadata> {
        let workspace_path = self.to_workspace_path(path);
        if let Some(workspace_name) = self.path_to_name.get(&workspace_path) {
            self.get_workspace_metadata_by_workspace_name(workspace_name)
        } else {
            None
        }
    }

    fn add_workspace(
        &mut self,
        workspace_name: &WorkspaceName,
        path: &Path,
        workspace_type: WorkspaceType,
    ) -> Result<Option<WorkspaceId>, WorkspaceStoreError> {
        let workspace_path = self.to_workspace_path(path);
        if let Some(current_workspace_name) = self.path_to_name.get(&workspace_path)
            && workspace_name != current_workspace_name
        {
            return Err(WorkspaceStoreError::Other(
                "There is already a workspace with the same path".into(),
            ));
        }
        let workspace_id = Self::generate_workspace_id(&self.rng, workspace_type);
        self.workspaces.insert(
            workspace_name.to_owned(),
            WorkspaceMetadata::new(
                workspace_name.to_owned(),
                workspace_type,
                workspace_id.clone(),
                workspace_path.clone(),
            ),
        );
        self.path_to_name
            .insert(workspace_path, workspace_name.into());
        Ok(workspace_id)
    }

    fn rename_workspace(
        &mut self,
        old_name: &WorkspaceName,
        new_name: &WorkspaceName,
    ) -> Result<(), WorkspaceStoreError> {
        if self.workspaces.contains_key(new_name) {
            // There is already a workspace registered under the new name.
            // TODO: This should fail with WorkspaceStoreError::WorkspaceAlreadyExists(new_name)
            return Ok(());
        }
        let Some(old_metadata) = self.workspaces.remove(old_name) else {
            // There is no workspace currently registered under the old name.
            // TODO: This should fail with WorkspaceStoreError::NoWorkspaceWithGivenName(old_name)
            return Ok(());
        };
        let new_metadata = WorkspaceMetadata::new(
            new_name.to_owned(),
            old_metadata.workspace_type(),
            old_metadata.workspace_id(),
            old_metadata.workspace_path().to_owned(),
        );
        let workspace_path = new_metadata.workspace_path().to_path_buf();
        self.workspaces.insert(new_name.to_owned(), new_metadata);
        self.path_to_name
            .insert(workspace_path, new_name.to_owned());
        Ok(())
    }

    fn forget(&mut self, workspace_names: &[&WorkspaceName]) {
        for to_forget in workspace_names {
            if let Some(metadata) = self.workspaces.remove(*to_forget) {
                self.path_to_name.remove(metadata.workspace_path());
            }
        }
    }

    fn to_workspace_path(&self, path: &Path) -> PathBuf {
        let path_to_store = relative_path(&self.repo_path, path);
        if path_to_store.is_relative() {
            slash_path(&path_to_store).into_owned()
        } else {
            path_to_store
        }
    }

    fn proto_to_workspace_id(
        rng: &Arc<JJRng>,
        workspace_type: WorkspaceType,
        proto: Option<&simple_workspace_store::WorkspaceId>,
    ) -> Option<WorkspaceId> {
        match proto {
            Some(workspace_id) => {
                Some(WorkspaceId::try_from_hex(workspace_id.id.as_str()).unwrap())
            }
            None => {
                // Compatibility logic: earlier SimpleWorkspaceStore on-disk data may not
                // have workspace_id set.
                Self::generate_workspace_id(rng, workspace_type)
            }
        }
    }

    // TODO: generate WorkspaceId for all workspaces, not just independent ones. We
    // don't do this yet because it causes changes in many tests due to the rng state
    // advancing when tests add a workspace, and then ChangeIds get different random
    // values.
    fn generate_workspace_id(
        rng: &Arc<JJRng>,
        workspace_type: WorkspaceType,
    ) -> Option<WorkspaceId> {
        match workspace_type {
            WorkspaceType::Regular => None,
            WorkspaceType::Independent => Some(rng.new_workspace_id(8)),
        }
    }
}

impl SimpleWorkspaceStore {
    /// The name of this WorkspaceStore implementation.
    pub const NAME: &str = "simple_workspace_store";

    fn new(settings: &UserSettings, store_dir: &Path) -> Self {
        let store_dir = store_dir.to_path_buf();
        let store_file = store_dir.join("index");
        let lock_file = store_file.with_extension("lock");
        let rng = settings.get_rng();
        Self {
            store_dir,
            store_file,
            lock_file,
            rng,
        }
    }

    /// Loads the workspace store from the given store path and repo.
    pub fn load(settings: &UserSettings, store_dir: &Path) -> Result<Self, BackendLoadError> {
        let store = Self::new(settings, store_dir);
        if !store.store_file.exists() {
            // TODO: Remove this in jj 0.57+: older repos do not have a workspace store index so we initialize it here.
            store
                .initialize()
                .map_err(|err| BackendLoadError(err.into()))?;
        }
        Ok(store)
    }

    /// Initializes this SimpleWorkspaceStore with an empty index.
    pub fn init(settings: &UserSettings, store_dir: &Path) -> Result<Self, BackendInitError> {
        let store = Self::new(settings, store_dir);
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<(), BackendInitError> {
        let _lock = self.lock().map_err(|err| BackendInitError(err.into()))?;
        let state = WorkspaceStoreState::empty(self.repo_path(), self.rng.clone());
        self.write_store(state)
            .map_err(|err| BackendInitError(err.into()))?;
        Ok(())
    }

    fn repo_path(&self) -> &Path {
        self.store_dir
            .parent()
            .expect("store_dir must be under the repo_path")
    }

    fn lock(&self) -> Result<FileLock, FileLockError> {
        FileLock::lock(self.lock_file.clone())
    }

    fn read_store(&self) -> Result<WorkspaceStoreState, SimpleWorkspaceStoreError> {
        let workspace_data = fs::read(&self.store_file).context(&self.store_file)?;
        let workspaces_proto = simple_workspace_store::Workspaces::decode(&*workspace_data)?;
        let state = WorkspaceStoreState::new(self.repo_path(), workspaces_proto, self.rng.clone())?;
        Ok(state)
    }

    fn write_store(&self, state: WorkspaceStoreState) -> Result<(), SimpleWorkspaceStoreError> {
        let workspaces_proto = state.into_proto()?;
        let temp_file =
            new_temp_file_with_default_permissions(self.repo_path()).context(self.repo_path())?;
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
        Self::NAME
    }

    fn add(
        &self,
        workspace_name: &WorkspaceName,
        path: &Path,
        workspace_type: WorkspaceType,
    ) -> Result<Option<WorkspaceId>, WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;
        let mut state = self.read_store()?;
        let workspace_id = state.add_workspace(workspace_name, path, workspace_type)?;
        self.write_store(state)?;
        Ok(workspace_id)
    }

    fn forget(&self, workspace_names: &[&WorkspaceName]) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;
        let mut state = self.read_store()?;
        state.forget(workspace_names);
        self.write_store(state)?;
        Ok(())
    }

    fn rename(
        &self,
        old_name: &WorkspaceName,
        new_name: &WorkspaceName,
    ) -> Result<(), WorkspaceStoreError> {
        let _lock = self.lock().map_err(SimpleWorkspaceStoreError::Lock)?;
        let mut state = self.read_store()?;
        state.rename_workspace(old_name, new_name)?;
        self.write_store(state)?;
        Ok(())
    }

    fn get_workspace_path(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<PathBuf>, WorkspaceStoreError> {
        let state = self.read_store()?;
        state.get_workspace_path(workspace_name)
    }

    fn get_workspace_metadata_by_workspace_name(
        &self,
        workspace_name: &WorkspaceName,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceStoreError> {
        let state = self.read_store()?;
        Ok(state.get_workspace_metadata_by_workspace_name(workspace_name))
    }

    fn get_workspace_metadata_by_workspace_path(
        &self,
        path: &Path,
    ) -> Result<Option<WorkspaceMetadata>, WorkspaceStoreError> {
        let state = self.read_store()?;
        Ok(state.get_workspace_metadata_by_workspace_path(path))
    }
}
