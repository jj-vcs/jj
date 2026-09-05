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

//! Contains factory and initializer types and the [`StoreFactories`] and
//! [`StoreInitializers`] registries.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use jj_config::settings::UserSettings;
use jj_core::backend::Backend;
use jj_core::backend::BackendInitError;
use jj_core::backend::BackendLoadError;
use jj_core::index_store::IndexStore;
use jj_core::op_heads_store::OpHeadsStore;
use jj_core::op_heads_store::OpHeadsStoreError;
use jj_core::op_store::OpStore;
use jj_core::op_store::OperationId;
use jj_core::op_store::RootOperationData;
use jj_core::signing::Signer;
use jj_core::store::Store;
use jj_core::workspace_store::WorkspaceStore;
use jj_local_util::file_util;
use jj_local_util::file_util::IoResultExt as _;
use jj_local_util::file_util::PathError;
use thiserror::Error;

use crate::repo::ReadonlyRepo;
use crate::repo::RepoLoader;
use crate::signing_factory::SignInitError;
use crate::signing_factory::signer_from_settings;
use crate::simple_workspace_store::SimpleWorkspaceStore;
use crate::submodule_store::SubmoduleStore;

/// Sets up a new `Backend` store for the given `UserSettings` and `path`.
pub type BackendInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn Backend>, BackendInitError> + 'a;

/// Sets up a new `WorkspaceStore` for the given `UserSettings` and `path`.
pub type WorkspaceStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn WorkspaceStore>, BackendInitError> + 'a;

/// Sets up a new `OpStore` for the given `UserSettings`, `path` and
/// `RootOperationData`.
#[rustfmt::skip] // auto-formatted line would exceed the maximum width
pub type OpStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path, RootOperationData) -> Result<Box<dyn OpStore>, BackendInitError>
    + 'a;

/// Sets up a new `OpHeadsStore` for the given `UserSettings`, `path` and
/// `OperationId`.
#[rustfmt::skip] // auto-formatted line would exceed the maximum width
pub type OpHeadsStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path, &OperationId)
    -> Result<Box<dyn OpHeadsStore>, BackendInitError>
    + 'a;

/// Sets up a new `IndexStore` for the given `UserSettings` and `path`.
pub type IndexStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn IndexStore>, BackendInitError> + 'a;

/// Sets up a new `SubmoduleStore` for the given `UserSettings` and `path`.
pub type SubmoduleStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn SubmoduleStore>, BackendInitError> + 'a;

/// Loads an existing `Backend` for the given `UserSettings` and `path`.
type BackendFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn Backend>, BackendLoadError>>;

/// Loads an existing `OpStore` for the given `UserSettings`, `path` and
/// `RootOperationData`.
type OpStoreFactory = Box<
    dyn Fn(&UserSettings, &Path, RootOperationData) -> Result<Box<dyn OpStore>, BackendLoadError>,
>;

/// Loads an existing `WorkspaceStore` for the given `UserSettings` and `path`.
type WorkspaceStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn WorkspaceStore>, BackendLoadError>>;

/// Loads an existing `OpHeadsStore` for the given `UserSettings` and `path`.
type OpHeadsStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn OpHeadsStore>, BackendLoadError>>;

/// Loads an existing `IndexStore` for the given `UserSettings` and `path`.
type IndexStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn IndexStore>, BackendLoadError>>;

/// Loads an existing `SubmoduleStore` for the given `UserSettings` and `path`.
type SubmoduleStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn SubmoduleStore>, BackendLoadError>>;

/// Merges `ext` into `base`. Panics if a name already is registered in `base`.
pub fn merge_factories_map<F>(base: &mut HashMap<String, F>, ext: HashMap<String, F>) {
    for (name, factory) in ext {
        match base.entry(name) {
            Entry::Vacant(v) => {
                v.insert(factory);
            }
            Entry::Occupied(o) => {
                panic!("Conflicting factory definitions for '{}' factory", o.key())
            }
        }
    }
}

/// `StoreFactories` holds different factories and uses them to load stores with.
pub struct StoreFactories {
    backend_factories: HashMap<String, BackendFactory>,
    workspace_store_factories: HashMap<String, WorkspaceStoreFactory>,
    op_store_factories: HashMap<String, OpStoreFactory>,
    op_heads_store_factories: HashMap<String, OpHeadsStoreFactory>,
    index_store_factories: HashMap<String, IndexStoreFactory>,
    submodule_store_factories: HashMap<String, SubmoduleStoreFactory>,
}

/// An error which occurs when trying to load a specific store from `StoreFactories`.
#[derive(Debug, Error)]
pub enum StoreLoadError {
    /// No factory registered under for the store type name.
    #[error("Unsupported {store} backend type '{store_type}'")]
    UnsupportedType {
        /// The store which we tried to load, e.g. `commit` or `operation heads`.
        store: &'static str,
        /// The store type, e.g. `git`.
        store_type: String,
    },
    /// Failed to read the store backend type from disk.
    #[error("Failed to read {store} backend type")]
    ReadError {
        /// The store which we tried to read, e.g. `commit` or `operation heads`.
        store: &'static str,
        /// The error source.
        source: PathError,
    },
    /// The backend failed to load.
    #[error(transparent)]
    Backend(#[from] BackendLoadError),
    /// The signer failed to initialize.
    #[error(transparent)]
    Signing(#[from] SignInitError),
}

impl StoreFactories {
    /// Creates a `StoreFactories` without any values.
    pub fn empty() -> Self {
        Self {
            backend_factories: HashMap::new(),
            workspace_store_factories: HashMap::new(),
            op_store_factories: HashMap::new(),
            op_heads_store_factories: HashMap::new(),
            index_store_factories: HashMap::new(),
            submodule_store_factories: HashMap::new(),
        }
    }

    /// Merges `ext` into `self`, panics if a store kind is registered under the same name on both
    /// sides.
    pub fn merge(&mut self, ext: Self) {
        let Self {
            backend_factories,
            workspace_store_factories,
            op_store_factories,
            op_heads_store_factories,
            index_store_factories,
            submodule_store_factories,
        } = ext;

        merge_factories_map(&mut self.backend_factories, backend_factories);
        merge_factories_map(
            &mut self.workspace_store_factories,
            workspace_store_factories,
        );
        merge_factories_map(&mut self.op_store_factories, op_store_factories);
        merge_factories_map(&mut self.op_heads_store_factories, op_heads_store_factories);
        merge_factories_map(&mut self.index_store_factories, index_store_factories);
        merge_factories_map(
            &mut self.submodule_store_factories,
            submodule_store_factories,
        );
    }

    /// Registers a new `Backend` for the given `name` and `factory`.
    pub fn add_backend(&mut self, name: &str, factory: BackendFactory) {
        self.backend_factories.insert(name.to_string(), factory);
    }

    /// Loads a registered `Backend` for the given `settings` and `store_path`.
    pub fn load_backend(
        &self,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn Backend>, StoreLoadError> {
        let backend_type = read_store_type("commit", store_path.join("type"))?;
        let backend_factory = self.backend_factories.get(&backend_type).ok_or_else(|| {
            StoreLoadError::UnsupportedType {
                store: "commit",
                store_type: backend_type.clone(),
            }
        })?;
        Ok(backend_factory(settings, store_path)?)
    }

    /// Registers a `WorkspaceStore` for the given `name` and `factory`.
    pub fn add_workspace_store(&mut self, name: &str, factory: WorkspaceStoreFactory) {
        self.workspace_store_factories
            .insert(name.to_string(), factory);
    }

    /// Loads a registered `WorkspaceStore` for the given `settings` and `store_path`.
    pub fn load_workspace_store(
        &self,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn WorkspaceStore>, StoreLoadError> {
        // TODO: The .jj/repo/workspace_store/type file is a recent addition, older
        // repos do not have this file. In jj 0.51+ switch from read_store_type_compat
        // to read_store_type.
        let workspace_store_type = read_store_type_compat(
            "workspace",
            store_path.join("type"),
            SimpleWorkspaceStore::NAME,
        )?;
        let workspace_store_factory = self
            .workspace_store_factories
            .get(&workspace_store_type)
            .ok_or_else(|| StoreLoadError::UnsupportedType {
                store: "workspace",
                store_type: workspace_store_type.clone(),
            })?;
        Ok(workspace_store_factory(settings, store_path)?)
    }

    /// Registers an `OpStore` for the given `name` and `factory`.
    pub fn add_op_store(&mut self, name: &str, factory: OpStoreFactory) {
        self.op_store_factories.insert(name.to_string(), factory);
    }

    /// Loads a registered `OpStore` for the given `settings`, `store_path` and `root_data`.
    pub fn load_op_store(
        &self,
        settings: &UserSettings,
        store_path: &Path,
        root_data: RootOperationData,
    ) -> Result<Box<dyn OpStore>, StoreLoadError> {
        let op_store_type = read_store_type("operation", store_path.join("type"))?;
        let op_store_factory = self.op_store_factories.get(&op_store_type).ok_or_else(|| {
            StoreLoadError::UnsupportedType {
                store: "operation",
                store_type: op_store_type.clone(),
            }
        })?;
        Ok(op_store_factory(settings, store_path, root_data)?)
    }

    /// Registers an `OpHeadsStore` for the given `name` and `factory`.
    pub fn add_op_heads_store(&mut self, name: &str, factory: OpHeadsStoreFactory) {
        self.op_heads_store_factories
            .insert(name.to_string(), factory);
    }

    /// Loads a registered `OpHeadsStore` for the given `settings` and `store_path`.
    pub fn load_op_heads_store(
        &self,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn OpHeadsStore>, StoreLoadError> {
        let op_heads_store_type = read_store_type("operation heads", store_path.join("type"))?;
        let op_heads_store_factory = self
            .op_heads_store_factories
            .get(&op_heads_store_type)
            .ok_or_else(|| StoreLoadError::UnsupportedType {
                store: "operation heads",
                store_type: op_heads_store_type.clone(),
            })?;
        Ok(op_heads_store_factory(settings, store_path)?)
    }

    /// Registers an `IndexStore` for the given `name` and `factory`.
    pub fn add_index_store(&mut self, name: &str, factory: IndexStoreFactory) {
        self.index_store_factories.insert(name.to_string(), factory);
    }

    /// Loads a registered `IndexStore` for the given `settings` and `store_path`.
    pub fn load_index_store(
        &self,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn IndexStore>, StoreLoadError> {
        let index_store_type = read_store_type("index", store_path.join("type"))?;
        let index_store_factory = self
            .index_store_factories
            .get(&index_store_type)
            .ok_or_else(|| StoreLoadError::UnsupportedType {
                store: "index",
                store_type: index_store_type.clone(),
            })?;
        Ok(index_store_factory(settings, store_path)?)
    }

    /// Registers a `SubmoduleStore` for the given `name` and `factory`.
    pub fn add_submodule_store(&mut self, name: &str, factory: SubmoduleStoreFactory) {
        self.submodule_store_factories
            .insert(name.to_string(), factory);
    }

    /// Loads a registered `SubmoduleStore` for the given `settings` and `store_path`.
    pub fn load_submodule_store(
        &self,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn SubmoduleStore>, StoreLoadError> {
        let submodule_store_type = read_store_type("submodule_store", store_path.join("type"))?;
        let submodule_store_factory = self
            .submodule_store_factories
            .get(&submodule_store_type)
            .ok_or_else(|| StoreLoadError::UnsupportedType {
                store: "submodule_store",
                store_type: submodule_store_type.clone(),
            })?;

        Ok(submodule_store_factory(settings, store_path)?)
    }

    /// Creates a `RepoLoader` for the repo at `repo_path` by reading the
    /// various `.jj/repo/<backend>/type` files and loading the right
    /// backends from the registered factories.
    pub fn load_repo(
        &self,
        settings: &UserSettings,
        repo_path: &Path,
    ) -> Result<RepoLoader, StoreLoadError> {
        let merge_options = settings.merge_options().map_err(BackendLoadError::new)?;
        let store = Store::new(
            self.load_backend(settings, &repo_path.join("store"))?,
            signer_from_settings(settings)?,
            merge_options,
        );
        let workspace_store =
            Arc::from(self.load_workspace_store(settings, &repo_path.join("workspace_store"))?);
        let root_op_data = RootOperationData {
            root_commit_id: store.root_commit_id().clone(),
        };
        let op_store =
            Arc::from(self.load_op_store(settings, &repo_path.join("op_store"), root_op_data)?);
        let op_heads_store =
            Arc::from(self.load_op_heads_store(settings, &repo_path.join("op_heads"))?);
        let index_store = Arc::from(self.load_index_store(settings, &repo_path.join("index"))?);
        let submodule_store =
            Arc::from(self.load_submodule_store(settings, &repo_path.join("submodule_store"))?);
        Ok(RepoLoader::new(
            settings.clone(),
            store,
            workspace_store,
            op_store,
            op_heads_store,
            index_store,
            submodule_store,
        ))
    }
}

/// Error from initializing a repo.
#[derive(Error, Debug)]
pub enum RepoInitError {
    /// Failed to initialize the commit backend.
    #[error(transparent)]
    Backend(#[from] BackendInitError),
    /// Failed to initialize the operation heads store.
    #[error(transparent)]
    OpHeadsStore(#[from] OpHeadsStoreError),
    /// Failed to create or write a file in the repo directory.
    #[error(transparent)]
    Path(#[from] PathError),
}

/// `StoreInitializers` holds the initializer for each store and uses them to
/// create a new repo with.
pub struct StoreInitializers<'a> {
    /// Initializes the commit backend.
    pub backend: &'a BackendInitializer<'a>,
    /// Initializes the workspace store.
    pub workspace_store: &'a WorkspaceStoreInitializer<'a>,
    /// Initializes the operation store.
    pub op_store: &'a OpStoreInitializer<'a>,
    /// Initializes the operation heads store.
    pub op_heads_store: &'a OpHeadsStoreInitializer<'a>,
    /// Initializes the index store.
    pub index_store: &'a IndexStoreInitializer<'a>,
    /// Initializes the submodule store.
    pub submodule_store: &'a SubmoduleStoreInitializer<'a>,
}

impl StoreInitializers<'_> {
    /// Creates a new repo at `repo_path` (which must exist), initializing each
    /// store with the initializers, and returns the repo at its root
    /// operation.
    pub async fn init_repo(
        &self,
        settings: &UserSettings,
        repo_path: &Path,
        signer: Signer,
    ) -> Result<Arc<ReadonlyRepo>, RepoInitError> {
        let repo_path = file_util::canonicalize(repo_path).context(repo_path)?;

        let store_path = repo_path.join("store");
        fs::create_dir(&store_path).context(&store_path)?;
        let backend = (self.backend)(settings, &store_path)?;
        let backend_path = store_path.join("type");
        fs::write(&backend_path, backend.name()).context(&backend_path)?;
        let merge_options = settings.merge_options().map_err(BackendInitError::new)?;
        let store = Store::new(backend, signer, merge_options);

        let workspace_store_path = repo_path.join("workspace_store");
        fs::create_dir(&workspace_store_path).context(&workspace_store_path)?;
        let workspace_store = (self.workspace_store)(settings, &workspace_store_path)?;
        let workspace_store_type_path = workspace_store_path.join("type");
        fs::write(&workspace_store_type_path, workspace_store.name())
            .context(&workspace_store_type_path)?;
        let workspace_store: Arc<dyn WorkspaceStore> = Arc::from(workspace_store);

        let op_store_path = repo_path.join("op_store");
        fs::create_dir(&op_store_path).context(&op_store_path)?;
        let root_op_data = RootOperationData {
            root_commit_id: store.root_commit_id().clone(),
        };
        let op_store = (self.op_store)(settings, &op_store_path, root_op_data)?;
        let op_store_type_path = op_store_path.join("type");
        fs::write(&op_store_type_path, op_store.name()).context(&op_store_type_path)?;
        let op_store: Arc<dyn OpStore> = Arc::from(op_store);

        let op_heads_path = repo_path.join("op_heads");
        fs::create_dir(&op_heads_path).context(&op_heads_path)?;
        let op_heads_store =
            (self.op_heads_store)(settings, &op_heads_path, op_store.root_operation_id())?;
        let op_heads_type_path = op_heads_path.join("type");
        fs::write(&op_heads_type_path, op_heads_store.name()).context(&op_heads_type_path)?;
        let op_heads_store: Arc<dyn OpHeadsStore> = Arc::from(op_heads_store);

        let index_path = repo_path.join("index");
        fs::create_dir(&index_path).context(&index_path)?;
        let index_store = (self.index_store)(settings, &index_path)?;
        let index_type_path = index_path.join("type");
        fs::write(&index_type_path, index_store.name()).context(&index_type_path)?;
        let index_store: Arc<dyn IndexStore> = Arc::from(index_store);

        let submodule_store_path = repo_path.join("submodule_store");
        fs::create_dir(&submodule_store_path).context(&submodule_store_path)?;
        let submodule_store = (self.submodule_store)(settings, &submodule_store_path)?;
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
        Ok(loader.create_from(root_operation, root_view, index))
    }
}

/// Reads the store's type name from `path`.
pub fn read_store_type(
    store: &'static str,
    path: impl AsRef<Path>,
) -> Result<String, StoreLoadError> {
    let path = path.as_ref();
    fs::read_to_string(path)
        .context(path)
        .map_err(|source| StoreLoadError::ReadError { store, source })
}

fn read_store_type_compat(
    store: &'static str,
    path: impl AsRef<Path>,
    default_type: &str,
) -> Result<String, StoreLoadError> {
    let path = path.as_ref();
    let read_or_write_default = || match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            fs::create_dir(path.parent().unwrap()).ok();
            fs::write(path, default_type)?;
            Ok(default_type.to_owned())
        }
        Err(err) => Err(err),
    };
    read_or_write_default()
        .context(path)
        .map_err(|source| StoreLoadError::ReadError { store, source })
}
