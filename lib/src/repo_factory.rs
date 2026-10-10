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

//! Contains a factory function to create a initialized [`ReadonlyRepo`] from the
//! given `UserSettings` and `path`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use thiserror::Error;

use crate::backend::BackendInitError;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::index::ChangeIdIndex;
use crate::index_store::IndexStore;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_store::OpStore;
use crate::op_store::RootOperationData;
use crate::repo::ReadonlyRepo;
use crate::repo::RepoLoader;
use crate::settings::UserSettings;
use crate::signing::Signer;
use crate::store::Store;
use crate::store_factories::BackendInitializer;
use crate::store_factories::IndexStoreInitializer;
use crate::store_factories::OpHeadsStoreInitializer;
use crate::store_factories::OpStoreInitializer;
use crate::store_factories::SubmoduleStoreInitializer;
use crate::store_factories::WorkspaceStoreInitializer;
use crate::workspace_store::WorkspaceStore;

/// Error which occurs when initializing a `Repo` from the file system.
#[derive(Error, Debug)]
pub enum RepoInitError {
    /// Unable to initialize the backend.
    #[error(transparent)]
    Backend(#[from] BackendInitError),
    /// Unable to initialiize the OpHeadsStore.
    #[error(transparent)]
    OpHeadsStore(#[from] OpHeadsStoreError),
    /// Unable to read the path.
    #[error(transparent)]
    Path(#[from] PathError),
}

/// Create a new `ReadonlyRepo` from the given `settings` and while initializing the initializers
/// on `repo_path`.
#[expect(clippy::too_many_arguments)]
pub async fn repo_from_path_and_settings(
    settings: &UserSettings,
    repo_path: &Path,
    backend_initializer: &BackendInitializer<'_>,
    signer: Signer,
    workspace_store_initializer: &WorkspaceStoreInitializer<'_>,
    op_store_initializer: &OpStoreInitializer<'_>,
    op_heads_store_initializer: &OpHeadsStoreInitializer<'_>,
    index_store_initializer: &IndexStoreInitializer<'_>,
    submodule_store_initializer: &SubmoduleStoreInitializer<'_>,
) -> Result<Arc<ReadonlyRepo>, RepoInitError> {
    let repo_path = dunce::canonicalize(repo_path).context(repo_path)?;

    let store_path = repo_path.join("store");
    fs::create_dir(&store_path).context(&store_path)?;
    let backend = backend_initializer(settings, &store_path)?;
    let backend_path = store_path.join("type");
    fs::write(&backend_path, backend.name()).context(&backend_path)?;
    let merge_options = settings.merge_options().map_err(BackendInitError::new)?;
    let store = Store::new(backend, signer, merge_options);

    let workspace_store_path = repo_path.join("workspace_store");
    fs::create_dir(&workspace_store_path).context(&workspace_store_path)?;
    let workspace_store = workspace_store_initializer(settings, &workspace_store_path)?;
    let workspace_store_type_path = workspace_store_path.join("type");
    fs::write(&workspace_store_type_path, workspace_store.name())
        .context(&workspace_store_type_path)?;
    let workspace_store: Arc<dyn WorkspaceStore> = Arc::from(workspace_store);

    let op_store_path = repo_path.join("op_store");
    fs::create_dir(&op_store_path).context(&op_store_path)?;
    let root_op_data = RootOperationData {
        root_commit_id: store.root_commit_id().clone(),
    };
    let op_store = op_store_initializer(settings, &op_store_path, root_op_data)?;
    let op_store_type_path = op_store_path.join("type");
    fs::write(&op_store_type_path, op_store.name()).context(&op_store_type_path)?;
    let op_store: Arc<dyn OpStore> = Arc::from(op_store);

    let op_heads_path = repo_path.join("op_heads");
    fs::create_dir(&op_heads_path).context(&op_heads_path)?;
    let op_heads_store =
        op_heads_store_initializer(settings, &op_heads_path, op_store.root_operation_id())?;
    let op_heads_type_path = op_heads_path.join("type");
    fs::write(&op_heads_type_path, op_heads_store.name()).context(&op_heads_type_path)?;
    let op_heads_store: Arc<dyn OpHeadsStore> = Arc::from(op_heads_store);

    let index_path = repo_path.join("index");
    fs::create_dir(&index_path).context(&index_path)?;
    let index_store = index_store_initializer(settings, &index_path)?;
    let index_type_path = index_path.join("type");
    fs::write(&index_type_path, index_store.name()).context(&index_type_path)?;
    let index_store: Arc<dyn IndexStore> = Arc::from(index_store);

    let submodule_store_path = repo_path.join("submodule_store");
    fs::create_dir(&submodule_store_path).context(&submodule_store_path)?;
    let submodule_store = submodule_store_initializer(settings, &submodule_store_path)?;
    let submodule_store_type_path = submodule_store_path.join("type");
    fs::write(&submodule_store_type_path, submodule_store.name())
        .context(&submodule_store_type_path)?;
    let submodule_store = Arc::from(submodule_store);

    let loader = RepoLoader::new(
        settings.clone(),
        store,
        workspace_store,
        op_store,
        op_heads_store,
        index_store,
        submodule_store,
    );

    let root_operation = loader.root_operation().await;
    let root_view = root_operation
        .view()
        .await
        .expect("failed to read root view");
    assert!(!root_view.heads().is_empty());
    let index = loader
        .index_store()
        .get_index_at_op(&root_operation, loader.store())
        .await
        // If the root op index couldn't be read, the index backend wouldn't
        // be initialized properly.
        .map_err(BackendInitError::new)?;
    Ok(Arc::new(ReadonlyRepo::new(
        loader,
        root_operation,
        index,
        OnceCell::<Box<dyn ChangeIdIndex + 'static>>::new(),
        root_view,
    )))
}
