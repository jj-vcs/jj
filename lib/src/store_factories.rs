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

//! Contains factory and initializer types and the [`StoreFactories`] registry.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use thiserror::Error;

use crate::backend::Backend;
use crate::backend::BackendInitError;
use crate::backend::BackendLoadError;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::index_store::IndexStore;
use crate::op_heads_store::OpHeadsStore;
use crate::op_store::OpStore;
use crate::op_store::OperationId;
use crate::op_store::RootOperationData;
use crate::settings::UserSettings;
use crate::signing_factory::SignInitError;
use crate::simple_workspace_store::SimpleWorkspaceStore;
use crate::submodule_store::SubmoduleStore;
use crate::workspace_store::WorkspaceStore;

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
