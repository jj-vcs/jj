// Copyright 2020 The Jujutsu Authors
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

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fs;
use std::path::Path;
use std::slice;
use std::sync::Arc;

use itertools::Itertools as _;
use once_cell::sync::OnceCell;
use pollster::FutureExt as _;
use thiserror::Error;
use tracing::instrument;

use self::dirty_cell::DirtyCell;
use crate::backend::Backend;
use crate::backend::BackendError;
use crate::backend::BackendInitError;
use crate::backend::BackendLoadError;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::MergedTreeId;
use crate::commit::Commit;
use crate::commit::CommitByCommitterTimestamp;
use crate::commit_builder::CommitBuilder;
use crate::commit_builder::DetachedCommitBuilder;
use crate::dag_walk;
use crate::default_index::DefaultIndexStore;
use crate::default_index::DefaultMutableIndex;
use crate::default_submodule_store::DefaultSubmoduleStore;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::index::ChangeIdIndex;
use crate::index::Index;
use crate::index::IndexError;
use crate::index::IndexResult;
use crate::index::IndexStore;
use crate::index::IndexStoreError;
use crate::index::MutableIndex;
use crate::index::ReadonlyIndex;
use crate::merge::MergeBuilder;
use crate::merge::SameChange;
use crate::merge::trivial_merge;
use crate::object_id::HexPrefix;
use crate::object_id::PrefixResolution;
use crate::op_heads_store;
use crate::op_heads_store::OpHeadResolutionError;
use crate::op_heads_store::OpHeadsStore;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_store;
use crate::op_store::OpStore;
use crate::op_store::OpStoreError;
use crate::op_store::OpStoreResult;
use crate::op_store::OperationId;
use crate::op_store::RefTarget;
use crate::op_store::RemoteRef;
use crate::op_store::RemoteRefState;
use crate::op_store::RootOperationData;
use crate::operation::Operation;
use crate::ref_name::GitRefName;
use crate::ref_name::RefName;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteRefSymbol;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::refs::diff_named_commit_ids;
use crate::refs::diff_named_ref_targets;
use crate::refs::diff_named_remote_refs;
use crate::refs::merge_ref_targets;
use crate::refs::merge_remote_refs;
use crate::revset;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetIteratorExt as _;
use crate::rewrite::CommitRewriter;
use crate::rewrite::RebaseOptions;
use crate::rewrite::RebasedCommit;
use crate::rewrite::RewriteRefsOptions;
use crate::rewrite::merge_commit_trees;
use crate::rewrite::rebase_commit_with_options;
use crate::settings::UserSettings;
use crate::signing::SignInitError;
use crate::signing::Signer;
use crate::simple_backend::SimpleBackend;
use crate::simple_op_heads_store::SimpleOpHeadsStore;
use crate::simple_op_store::SimpleOpStore;
use crate::store::Store;
use crate::submodule_store::SubmoduleStore;
use crate::transaction::Transaction;
use crate::transaction::TransactionCommitError;
use crate::tree_merge::MergeOptions;
use crate::view::RenameWorkspaceError;
use crate::view::View;

pub trait Repo {
    /// Base repository that contains all committed data. Returns `self` if this
    /// is a `ReadonlyRepo`,
    fn base_repo(&self) -> &ReadonlyRepo;

    fn store(&self) -> &Arc<Store>;

    fn op_store(&self) -> &Arc<dyn OpStore>;

    fn index(&self) -> &dyn Index;

    fn view(&self) -> &View;

    fn submodule_store(&self) -> &Arc<dyn SubmoduleStore>;

    fn resolve_change_id(&self, change_id: &ChangeId) -> IndexResult<Option<Vec<CommitId>>> {
        // Replace this if we added more efficient lookup method.
        let prefix = HexPrefix::from_id(change_id);
        match self.resolve_change_id_prefix(&prefix)? {
            PrefixResolution::NoMatch => Ok(None),
            PrefixResolution::SingleMatch(entries) => Ok(Some(entries)),
            PrefixResolution::AmbiguousMatch => panic!("complete change_id should be unambiguous"),
        }
    }

    fn resolve_change_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> IndexResult<PrefixResolution<Vec<CommitId>>>;

    fn shortest_unique_change_id_prefix_len(
        &self,
        target_id_bytes: &ChangeId,
    ) -> IndexResult<usize>;
}

pub struct ReadonlyRepo {
    loader: RepoLoader,
    operation: Operation,
    index: Box<dyn ReadonlyIndex>,
    change_id_index: OnceCell<Box<dyn ChangeIdIndex>>,
    // TODO: This should eventually become part of the index and not be stored fully in memory.
    view: View,
}

impl Debug for ReadonlyRepo {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("ReadonlyRepo")
            .field("store", &self.loader.store)
            .finish_non_exhaustive()
    }
}

#[derive(Error, Debug)]
pub enum RepoInitError {
    #[error(transparent)]
    Backend(#[from] BackendInitError),
    #[error(transparent)]
    OpHeadsStore(#[from] OpHeadsStoreError),
    #[error(transparent)]
    Path(#[from] PathError),
}

impl ReadonlyRepo {
    pub fn default_op_store_initializer() -> &'static OpStoreInitializer<'static> {
        &|_settings, store_path, root_data| {
            Ok(Box::new(SimpleOpStore::init(store_path, root_data)?))
        }
    }

    pub fn default_op_heads_store_initializer() -> &'static OpHeadsStoreInitializer<'static> {
        &|_settings, store_path| Ok(Box::new(SimpleOpHeadsStore::init(store_path)?))
    }

    pub fn default_index_store_initializer() -> &'static IndexStoreInitializer<'static> {
        &|_settings, store_path| Ok(Box::new(DefaultIndexStore::init(store_path)?))
    }

    pub fn default_submodule_store_initializer() -> &'static SubmoduleStoreInitializer<'static> {
        &|_settings, store_path| Ok(Box::new(DefaultSubmoduleStore::init(store_path)))
    }

    #[expect(clippy::too_many_arguments)]
    pub fn init(
        settings: &UserSettings,
        repo_path: &Path,
        backend_initializer: &BackendInitializer,
        signer: Signer,
        op_store_initializer: &OpStoreInitializer,
        op_heads_store_initializer: &OpHeadsStoreInitializer,
        index_store_initializer: &IndexStoreInitializer,
        submodule_store_initializer: &SubmoduleStoreInitializer,
    ) -> Result<Arc<Self>, RepoInitError> {
        let repo_path = dunce::canonicalize(repo_path).context(repo_path)?;

        let store_path = repo_path.join("store");
        fs::create_dir(&store_path).context(&store_path)?;
        let backend = backend_initializer(settings, &store_path)?;
        let backend_path = store_path.join("type");
        fs::write(&backend_path, backend.name()).context(&backend_path)?;
        let merge_options =
            MergeOptions::from_settings(settings).map_err(|err| BackendInitError(err.into()))?;
        let store = Store::new(backend, signer, merge_options);

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
        let op_heads_store = op_heads_store_initializer(settings, &op_heads_path)?;
        let op_heads_type_path = op_heads_path.join("type");
        fs::write(&op_heads_type_path, op_heads_store.name()).context(&op_heads_type_path)?;
        op_heads_store
            .update_op_heads(&[], op_store.root_operation_id())
            .block_on()?;
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

        let loader = RepoLoader {
            settings: settings.clone(),
            store,
            op_store,
            op_heads_store,
            index_store,
            submodule_store,
        };

        let root_operation = loader.root_operation();
        let root_view = root_operation.view().expect("failed to read root view");
        assert!(!root_view.heads().is_empty());
        let index = loader
            .index_store
            .get_index_at_op(&root_operation, &loader.store)
            // If the root op index couldn't be read, the index backend wouldn't
            // be initialized properly.
            .map_err(|err| BackendInitError(err.into()))?;
        Ok(Arc::new(Self {
            loader,
            operation: root_operation,
            index,
            change_id_index: OnceCell::new(),
            view: root_view,
        }))
    }

    pub fn loader(&self) -> &RepoLoader {
        &self.loader
    }

    pub fn op_id(&self) -> &OperationId {
        self.operation.id()
    }

    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn readonly_index(&self) -> &dyn ReadonlyIndex {
        self.index.as_ref()
    }

    fn change_id_index(&self) -> &dyn ChangeIdIndex {
        self.change_id_index
            .get_or_init(|| {
                self.readonly_index()
                    .change_id_index(&mut self.view().heads().iter())
            })
            .as_ref()
    }

    pub fn op_heads_store(&self) -> &Arc<dyn OpHeadsStore> {
        self.loader.op_heads_store()
    }

    pub fn index_store(&self) -> &Arc<dyn IndexStore> {
        self.loader.index_store()
    }

    pub fn settings(&self) -> &UserSettings {
        self.loader.settings()
    }

    pub fn start_transaction(self: &Arc<Self>) -> Transaction {
        let mut_repo = MutableRepo::new(self.clone(), self.readonly_index(), &self.view);
        Transaction::new(mut_repo, self.settings())
    }

    pub fn reload_at_head(&self) -> Result<Arc<Self>, RepoLoaderError> {
        self.loader().load_at_head()
    }

    #[instrument]
    pub fn reload_at(&self, operation: &Operation) -> Result<Arc<Self>, RepoLoaderError> {
        self.loader().load_at(operation)
    }
}

impl Repo for ReadonlyRepo {
    fn base_repo(&self) -> &ReadonlyRepo {
        self
    }

    fn store(&self) -> &Arc<Store> {
        self.loader.store()
    }

    fn op_store(&self) -> &Arc<dyn OpStore> {
        self.loader.op_store()
    }

    fn index(&self) -> &dyn Index {
        self.readonly_index().as_index()
    }

    fn view(&self) -> &View {
        &self.view
    }

    fn submodule_store(&self) -> &Arc<dyn SubmoduleStore> {
        self.loader.submodule_store()
    }

    fn resolve_change_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> IndexResult<PrefixResolution<Vec<CommitId>>> {
        self.change_id_index().resolve_prefix(prefix)
    }

    fn shortest_unique_change_id_prefix_len(&self, target_id: &ChangeId) -> IndexResult<usize> {
        self.change_id_index().shortest_unique_prefix_len(target_id)
    }
}

pub type BackendInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn Backend>, BackendInitError> + 'a;
#[rustfmt::skip] // auto-formatted line would exceed the maximum width
pub type OpStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path, RootOperationData) -> Result<Box<dyn OpStore>, BackendInitError>
    + 'a;
pub type OpHeadsStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn OpHeadsStore>, BackendInitError> + 'a;
pub type IndexStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn IndexStore>, BackendInitError> + 'a;
pub type SubmoduleStoreInitializer<'a> =
    dyn Fn(&UserSettings, &Path) -> Result<Box<dyn SubmoduleStore>, BackendInitError> + 'a;

type BackendFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn Backend>, BackendLoadError>>;
type OpStoreFactory = Box<
    dyn Fn(&UserSettings, &Path, RootOperationData) -> Result<Box<dyn OpStore>, BackendLoadError>,
>;
type OpHeadsStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn OpHeadsStore>, BackendLoadError>>;
type IndexStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn IndexStore>, BackendLoadError>>;
type SubmoduleStoreFactory =
    Box<dyn Fn(&UserSettings, &Path) -> Result<Box<dyn SubmoduleStore>, BackendLoadError>>;

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

pub struct StoreFactories {
    backend_factories: HashMap<String, BackendFactory>,
    op_store_factories: HashMap<String, OpStoreFactory>,
    op_heads_store_factories: HashMap<String, OpHeadsStoreFactory>,
    index_store_factories: HashMap<String, IndexStoreFactory>,
    submodule_store_factories: HashMap<String, SubmoduleStoreFactory>,
}

impl Default for StoreFactories {
    fn default() -> Self {
        let mut factories = Self::empty();

        // Backends
        factories.add_backend(
            SimpleBackend::name(),
            Box::new(|_settings, store_path| Ok(Box::new(SimpleBackend::load(store_path)))),
        );
        #[cfg(feature = "git")]
        factories.add_backend(
            crate::git_backend::GitBackend::name(),
            Box::new(|settings, store_path| {
                Ok(Box::new(crate::git_backend::GitBackend::load(
                    settings, store_path,
                )?))
            }),
        );
        #[cfg(feature = "testing")]
        factories.add_backend(
            crate::secret_backend::SecretBackend::name(),
            Box::new(|settings, store_path| {
                Ok(Box::new(crate::secret_backend::SecretBackend::load(
                    settings, store_path,
                )?))
            }),
        );

        // OpStores
        factories.add_op_store(
            SimpleOpStore::name(),
            Box::new(|_settings, store_path, root_data| {
                Ok(Box::new(SimpleOpStore::load(store_path, root_data)))
            }),
        );

        // OpHeadsStores
        factories.add_op_heads_store(
            SimpleOpHeadsStore::name(),
            Box::new(|_settings, store_path| Ok(Box::new(SimpleOpHeadsStore::load(store_path)))),
        );

        // Index
        factories.add_index_store(
            DefaultIndexStore::name(),
            Box::new(|_settings, store_path| Ok(Box::new(DefaultIndexStore::load(store_path)))),
        );

        // SubmoduleStores
        factories.add_submodule_store(
            DefaultSubmoduleStore::name(),
            Box::new(|_settings, store_path| Ok(Box::new(DefaultSubmoduleStore::load(store_path)))),
        );

        factories
    }
}

#[derive(Debug, Error)]
pub enum StoreLoadError {
    #[error("Unsupported {store} backend type '{store_type}'")]
    UnsupportedType {
        store: &'static str,
        store_type: String,
    },
    #[error("Failed to read {store} backend type")]
    ReadError {
        store: &'static str,
        source: PathError,
    },
    #[error(transparent)]
    Backend(#[from] BackendLoadError),
    #[error(transparent)]
    Signing(#[from] SignInitError),
}

impl StoreFactories {
    pub fn empty() -> Self {
        Self {
            backend_factories: HashMap::new(),
            op_store_factories: HashMap::new(),
            op_heads_store_factories: HashMap::new(),
            index_store_factories: HashMap::new(),
            submodule_store_factories: HashMap::new(),
        }
    }

    pub fn merge(&mut self, ext: Self) {
        let Self {
            backend_factories,
            op_store_factories,
            op_heads_store_factories,
            index_store_factories,
            submodule_store_factories,
        } = ext;

        merge_factories_map(&mut self.backend_factories, backend_factories);
        merge_factories_map(&mut self.op_store_factories, op_store_factories);
        merge_factories_map(&mut self.op_heads_store_factories, op_heads_store_factories);
        merge_factories_map(&mut self.index_store_factories, index_store_factories);
        merge_factories_map(
            &mut self.submodule_store_factories,
            submodule_store_factories,
        );
    }

    pub fn add_backend(&mut self, name: &str, factory: BackendFactory) {
        self.backend_factories.insert(name.to_string(), factory);
    }

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

    pub fn add_op_store(&mut self, name: &str, factory: OpStoreFactory) {
        self.op_store_factories.insert(name.to_string(), factory);
    }

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

    pub fn add_op_heads_store(&mut self, name: &str, factory: OpHeadsStoreFactory) {
        self.op_heads_store_factories
            .insert(name.to_string(), factory);
    }

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

    pub fn add_index_store(&mut self, name: &str, factory: IndexStoreFactory) {
        self.index_store_factories.insert(name.to_string(), factory);
    }

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

    pub fn add_submodule_store(&mut self, name: &str, factory: SubmoduleStoreFactory) {
        self.submodule_store_factories
            .insert(name.to_string(), factory);
    }

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

pub fn read_store_type(
    store: &'static str,
    path: impl AsRef<Path>,
) -> Result<String, StoreLoadError> {
    let path = path.as_ref();
    fs::read_to_string(path)
        .context(path)
        .map_err(|source| StoreLoadError::ReadError { store, source })
}

#[derive(Debug, Error)]
pub enum RepoLoaderError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    IndexStore(#[from] IndexStoreError),
    #[error(transparent)]
    OpHeadResolution(#[from] OpHeadResolutionError),
    #[error(transparent)]
    OpHeadsStoreError(#[from] OpHeadsStoreError),
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
    #[error(transparent)]
    TransactionCommit(#[from] TransactionCommitError),
}

/// Helps create `ReadonlyRepo` instances of a repo at the head operation or at
/// a given operation.
#[derive(Clone)]
pub struct RepoLoader {
    settings: UserSettings,
    store: Arc<Store>,
    op_store: Arc<dyn OpStore>,
    op_heads_store: Arc<dyn OpHeadsStore>,
    index_store: Arc<dyn IndexStore>,
    submodule_store: Arc<dyn SubmoduleStore>,
}

impl RepoLoader {
    pub fn new(
        settings: UserSettings,
        store: Arc<Store>,
        op_store: Arc<dyn OpStore>,
        op_heads_store: Arc<dyn OpHeadsStore>,
        index_store: Arc<dyn IndexStore>,
        submodule_store: Arc<dyn SubmoduleStore>,
    ) -> Self {
        Self {
            settings,
            store,
            op_store,
            op_heads_store,
            index_store,
            submodule_store,
        }
    }

    /// Creates a `RepoLoader` for the repo at `repo_path` by reading the
    /// various `.jj/repo/<backend>/type` files and loading the right
    /// backends from `store_factories`.
    pub fn init_from_file_system(
        settings: &UserSettings,
        repo_path: &Path,
        store_factories: &StoreFactories,
    ) -> Result<Self, StoreLoadError> {
        let merge_options =
            MergeOptions::from_settings(settings).map_err(|err| BackendLoadError(err.into()))?;
        let store = Store::new(
            store_factories.load_backend(settings, &repo_path.join("store"))?,
            Signer::from_settings(settings)?,
            merge_options,
        );
        let root_op_data = RootOperationData {
            root_commit_id: store.root_commit_id().clone(),
        };
        let op_store = Arc::from(store_factories.load_op_store(
            settings,
            &repo_path.join("op_store"),
            root_op_data,
        )?);
        let op_heads_store =
            Arc::from(store_factories.load_op_heads_store(settings, &repo_path.join("op_heads"))?);
        let index_store =
            Arc::from(store_factories.load_index_store(settings, &repo_path.join("index"))?);
        let submodule_store = Arc::from(
            store_factories.load_submodule_store(settings, &repo_path.join("submodule_store"))?,
        );
        Ok(Self {
            settings: settings.clone(),
            store,
            op_store,
            op_heads_store,
            index_store,
            submodule_store,
        })
    }

    pub fn settings(&self) -> &UserSettings {
        &self.settings
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn index_store(&self) -> &Arc<dyn IndexStore> {
        &self.index_store
    }

    pub fn op_store(&self) -> &Arc<dyn OpStore> {
        &self.op_store
    }

    pub fn op_heads_store(&self) -> &Arc<dyn OpHeadsStore> {
        &self.op_heads_store
    }

    pub fn submodule_store(&self) -> &Arc<dyn SubmoduleStore> {
        &self.submodule_store
    }

    pub fn load_at_head(&self) -> Result<Arc<ReadonlyRepo>, RepoLoaderError> {
        let op = op_heads_store::resolve_op_heads(
            self.op_heads_store.as_ref(),
            &self.op_store,
            |op_heads| self.resolve_op_heads(op_heads),
        )?;
        let view = op.view()?;
        self.finish_load(op, view)
    }

    #[instrument(skip(self))]
    pub fn load_at(&self, op: &Operation) -> Result<Arc<ReadonlyRepo>, RepoLoaderError> {
        let view = op.view()?;
        self.finish_load(op.clone(), view)
    }

    pub fn create_from(
        &self,
        operation: Operation,
        view: View,
        index: Box<dyn ReadonlyIndex>,
    ) -> Arc<ReadonlyRepo> {
        let repo = ReadonlyRepo {
            loader: self.clone(),
            operation,
            index,
            change_id_index: OnceCell::new(),
            view,
        };
        Arc::new(repo)
    }

    // If we add a higher-level abstraction of OpStore, root_operation() and
    // load_operation() will be moved there.

    /// Returns the root operation.
    pub fn root_operation(&self) -> Operation {
        self.load_operation(self.op_store.root_operation_id())
            .expect("failed to read root operation")
    }

    /// Loads the specified operation from the operation store.
    pub fn load_operation(&self, id: &OperationId) -> OpStoreResult<Operation> {
        let data = self.op_store.read_operation(id).block_on()?;
        Ok(Operation::new(self.op_store.clone(), id.clone(), data))
    }

    /// Merges the given `operations` into a single operation. Returns the root
    /// operation if the `operations` is empty.
    pub fn merge_operations(
        &self,
        operations: Vec<Operation>,
        tx_description: Option<&str>,
    ) -> Result<Operation, RepoLoaderError> {
        let num_operations = operations.len();
        let mut operations = operations.into_iter();
        let Some(base_op) = operations.next() else {
            return Ok(self.root_operation());
        };
        let final_op = if num_operations > 1 {
            let base_repo = self.load_at(&base_op)?;
            let mut tx = base_repo.start_transaction();
            for other_op in operations {
                tx.merge_operation(other_op)?;
                tx.repo_mut().rebase_descendants()?;
            }
            let tx_description = tx_description.map_or_else(
                || format!("merge {num_operations} operations"),
                |tx_description| tx_description.to_string(),
            );
            let merged_repo = tx.write(tx_description)?.leave_unpublished();
            merged_repo.operation().clone()
        } else {
            base_op
        };

        Ok(final_op)
    }

    fn resolve_op_heads(&self, op_heads: Vec<Operation>) -> Result<Operation, RepoLoaderError> {
        assert!(!op_heads.is_empty());
        self.merge_operations(op_heads, Some("reconcile divergent operations"))
    }

    fn finish_load(
        &self,
        operation: Operation,
        view: View,
    ) -> Result<Arc<ReadonlyRepo>, RepoLoaderError> {
        let index = self.index_store.get_index_at_op(&operation, &self.store)?;
        let repo = ReadonlyRepo {
            loader: self.clone(),
            operation,
            index,
            change_id_index: OnceCell::new(),
            view,
        };
        Ok(Arc::new(repo))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Rewrite {
    /// The old commit was rewritten as this new commit. Children should be
    /// rebased onto the new commit.
    Rewritten(CommitId),
    /// The old commit was rewritten as multiple other commits. Children should
    /// not be rebased.
    Divergent(Vec<CommitId>),
    /// The old commit was abandoned. Children should be rebased onto the given
    /// commits (typically the parents of the old commit).
    Abandoned(Vec<CommitId>),
}

impl Rewrite {
    fn new_parent_ids(&self) -> &[CommitId] {
        match self {
            Self::Rewritten(new_parent_id) => std::slice::from_ref(new_parent_id),
            Self::Divergent(new_parent_ids) => new_parent_ids.as_slice(),
            Self::Abandoned(new_parent_ids) => new_parent_ids.as_slice(),
        }
    }
}

pub struct MutableRepo {
    base_repo: Arc<ReadonlyRepo>,
    index: Box<dyn MutableIndex>,
    view: DirtyCell<View>,
    /// Mapping from new commit to its predecessors.
    ///
    /// This is similar to (the reverse of) `parent_mapping`, but
    /// `commit_predecessors` will never be cleared on `rebase_descendants()`.
    commit_predecessors: BTreeMap<CommitId, Vec<CommitId>>,
    // The commit identified by the key has been replaced by all the ones in the value.
    // * Bookmarks pointing to the old commit should be updated to the new commit, resulting in a
    //   conflict if there multiple new commits.
    // * Children of the old commit should be rebased onto the new commits. However, if the type is
    //   `Divergent`, they should be left in place.
    // * Working copies pointing to the old commit should be updated to the first of the new
    //   commits. However, if the type is `Abandoned`, a new working-copy commit should be created
    //   on top of all of the new commits instead.
    parent_mapping: HashMap<CommitId, Rewrite>,
}

impl MutableRepo {
    pub fn new(base_repo: Arc<ReadonlyRepo>, index: &dyn ReadonlyIndex, view: &View) -> Self {
        let mut_view = view.clone();
        let mut_index = index.start_modification();
        Self {
            base_repo,
            index: mut_index,
            view: DirtyCell::with_clean(mut_view),
            commit_predecessors: Default::default(),
            parent_mapping: Default::default(),
        }
    }

    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        &self.base_repo
    }

    fn view_mut(&mut self) -> &mut View {
        self.view.get_mut()
    }

    pub fn mutable_index(&self) -> &dyn MutableIndex {
        self.index.as_ref()
    }

    pub(crate) fn is_backed_by_default_index(&self) -> bool {
        self.index.downcast_ref::<DefaultMutableIndex>().is_some()
    }

    pub fn has_changes(&self) -> bool {
        self.view.ensure_clean(|v| self.enforce_view_invariants(v));
        !(self.commit_predecessors.is_empty()
            && self.parent_mapping.is_empty()
            && self.view() == &self.base_repo.view)
    }

    pub(crate) fn consume(
        self,
    ) -> (
        Box<dyn MutableIndex>,
        View,
        BTreeMap<CommitId, Vec<CommitId>>,
    ) {
        self.view.ensure_clean(|v| self.enforce_view_invariants(v));
        (self.index, self.view.into_inner(), self.commit_predecessors)
    }

    /// Returns a [`CommitBuilder`] to write new commit to the repo.
    pub fn new_commit(
        &mut self,
        parents: Vec<CommitId>,
        tree_id: MergedTreeId,
    ) -> CommitBuilder<'_> {
        let settings = self.base_repo.settings();
        DetachedCommitBuilder::for_new_commit(self, settings, parents, tree_id).attach(self)
    }

    /// Returns a [`CommitBuilder`] to rewrite an existing commit in the repo.
    pub fn rewrite_commit(&mut self, predecessor: &Commit) -> CommitBuilder<'_> {
        let settings = self.base_repo.settings();
        DetachedCommitBuilder::for_rewrite_from(self, settings, predecessor).attach(self)
        // CommitBuilder::write will record the rewrite in
        // `self.rewritten_commits`
    }

    pub(crate) fn set_predecessors(&mut self, id: CommitId, predecessors: Vec<CommitId>) {
        self.commit_predecessors.insert(id, predecessors);
    }

    /// Record a commit as having been rewritten to another commit in this
    /// transaction.
    ///
    /// This record is used by `rebase_descendants` to know which commits have
    /// children that need to be rebased, and where to rebase them to. See the
    /// docstring for `record_rewritten_commit` for details.
    pub fn set_rewritten_commit(&mut self, old_id: CommitId, new_id: CommitId) {
        assert_ne!(old_id, *self.store().root_commit_id());
        self.parent_mapping
            .insert(old_id, Rewrite::Rewritten(new_id));
    }

    /// Record a commit as being rewritten into multiple other commits in this
    /// transaction.
    ///
    /// A later call to `rebase_descendants()` will update bookmarks pointing to
    /// `old_id` be conflicted and pointing to all pf `new_ids`. Working copies
    /// pointing to `old_id` will be updated to point to the first commit in
    /// `new_ids``. Descendants of `old_id` will be left alone.
    pub fn set_divergent_rewrite(
        &mut self,
        old_id: CommitId,
        new_ids: impl IntoIterator<Item = CommitId>,
    ) {
        assert_ne!(old_id, *self.store().root_commit_id());
        self.parent_mapping.insert(
            old_id.clone(),
            Rewrite::Divergent(new_ids.into_iter().collect()),
        );
    }

    /// Record a commit as having been abandoned in this transaction.
    ///
    /// This record is used by `rebase_descendants` to know which commits have
    /// children that need to be rebased, and where to rebase the children to.
    ///
    /// The `rebase_descendants` logic will rebase the descendants of the old
    /// commit to become the descendants of parent(s) of the old commit. Any
    /// bookmarks at the old commit will be either moved to the parent(s) of the
    /// old commit or deleted depending on [`RewriteRefsOptions`].
    pub fn record_abandoned_commit(&mut self, old_commit: &Commit) {
        assert_ne!(old_commit.id(), self.store().root_commit_id());
        // Descendants should be rebased onto the commit's parents
        self.record_abandoned_commit_with_parents(
            old_commit.id().clone(),
            old_commit.parent_ids().iter().cloned(),
        );
    }

    /// Record a commit as having been abandoned in this transaction.
    ///
    /// A later `rebase_descendants()` will rebase children of `old_id` onto
    /// `new_parent_ids`. A working copy pointing to `old_id` will point to a
    /// new commit on top of `new_parent_ids`.
    pub fn record_abandoned_commit_with_parents(
        &mut self,
        old_id: CommitId,
        new_parent_ids: impl IntoIterator<Item = CommitId>,
    ) {
        assert_ne!(old_id, *self.store().root_commit_id());
        self.parent_mapping.insert(
            old_id,
            Rewrite::Abandoned(new_parent_ids.into_iter().collect()),
        );
    }

    pub fn has_rewrites(&self) -> bool {
        !self.parent_mapping.is_empty()
    }

    /// Calculates new parents for a commit that's currently based on the given
    /// parents. It does that by considering how previous commits have been
    /// rewritten and abandoned.
    ///
    /// If `parent_mapping` contains cycles, this function may either panic or
    /// drop parents that caused cycles.
    pub fn new_parents(&self, old_ids: &[CommitId]) -> Vec<CommitId> {
        self.rewritten_ids_with(old_ids, |rewrite| !matches!(rewrite, Rewrite::Divergent(_)))
    }

    fn rewritten_ids_with(
        &self,
        old_ids: &[CommitId],
        mut predicate: impl FnMut(&Rewrite) -> bool,
    ) -> Vec<CommitId> {
        assert!(!old_ids.is_empty());
        let mut new_ids = Vec::with_capacity(old_ids.len());
        let mut to_visit = old_ids.iter().rev().collect_vec();
        let mut visited = HashSet::new();
        while let Some(id) = to_visit.pop() {
            if !visited.insert(id) {
                continue;
            }
            match self.parent_mapping.get(id).filter(|&v| predicate(v)) {
                None => {
                    new_ids.push(id.clone());
                }
                Some(rewrite) => {
                    let replacements = rewrite.new_parent_ids();
                    assert!(
                        // Each commit must have a parent, so a parent can
                        // not just be mapped to nothing. This assertion
                        // could be removed if this function is used for
                        // mapping something other than a commit's parents.
                        !replacements.is_empty(),
                        "Found empty value for key {id:?} in the parent mapping",
                    );
                    to_visit.extend(replacements.iter().rev());
                }
            }
        }
        assert!(
            !new_ids.is_empty(),
            "new ids become empty because of cycle in the parent mapping"
        );
        debug_assert!(new_ids.iter().all_unique());
        new_ids
    }

    /// Fully resolves transitive replacements in `parent_mapping`.
    ///
    /// If `parent_mapping` contains cycles, this function will panic.
    fn resolve_rewrite_mapping_with(
        &self,
        mut predicate: impl FnMut(&Rewrite) -> bool,
    ) -> HashMap<CommitId, Vec<CommitId>> {
        let sorted_ids = dag_walk::topo_order_forward(
            self.parent_mapping.keys(),
            |&id| id,
            |&id| match self.parent_mapping.get(id).filter(|&v| predicate(v)) {
                None => &[],
                Some(rewrite) => rewrite.new_parent_ids(),
            },
        );
        let mut new_mapping: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        for old_id in sorted_ids {
            let Some(rewrite) = self.parent_mapping.get(old_id).filter(|&v| predicate(v)) else {
                continue;
            };
            let lookup = |id| new_mapping.get(id).map_or(slice::from_ref(id), |ids| ids);
            let new_ids = match rewrite.new_parent_ids() {
                [id] => lookup(id).to_vec(), // unique() not needed
                ids => ids.iter().flat_map(lookup).unique().cloned().collect(),
            };
            debug_assert_eq!(
                new_ids,
                self.rewritten_ids_with(slice::from_ref(old_id), &mut predicate)
            );
            new_mapping.insert(old_id.clone(), new_ids);
        }
        new_mapping
    }

    /// Updates bookmarks, working copies, and anonymous heads after rewriting
    /// and/or abandoning commits.
    pub fn update_rewritten_references(
        &mut self,
        options: &RewriteRefsOptions,
    ) -> BackendResult<()> {
        self.update_all_references(options)?;
        self.update_heads()
            .map_err(|err| err.into_backend_error())?;
        Ok(())
    }

    fn update_all_references(&mut self, options: &RewriteRefsOptions) -> BackendResult<()> {
        let rewrite_mapping = self.resolve_rewrite_mapping_with(|_| true);
        self.update_local_bookmarks(&rewrite_mapping, options);
        self.update_wc_commits(&rewrite_mapping)?;
        Ok(())
    }

    fn update_local_bookmarks(
        &mut self,
        rewrite_mapping: &HashMap<CommitId, Vec<CommitId>>,
        options: &RewriteRefsOptions,
    ) {
        let changed_branches = self
            .view()
            .local_bookmarks()
            .flat_map(|(name, target)| {
                target.added_ids().filter_map(|id| {
                    let change = rewrite_mapping.get_key_value(id)?;
                    Some((name.to_owned(), change))
                })
            })
            .collect_vec();
        for (bookmark_name, (old_commit_id, new_commit_ids)) in changed_branches {
            let should_delete = options.delete_abandoned_bookmarks
                && matches!(
                    self.parent_mapping.get(old_commit_id),
                    Some(Rewrite::Abandoned(_))
                );
            let old_target = RefTarget::normal(old_commit_id.clone());
            let new_target = if should_delete {
                RefTarget::absent()
            } else {
                let ids = itertools::intersperse(new_commit_ids, old_commit_id)
                    .map(|id| Some(id.clone()));
                RefTarget::from_merge(MergeBuilder::from_iter(ids).build())
            };

            self.merge_local_bookmark(&bookmark_name, &old_target, &new_target);
        }
    }

    fn update_wc_commits(
        &mut self,
        rewrite_mapping: &HashMap<CommitId, Vec<CommitId>>,
    ) -> BackendResult<()> {
        let changed_wc_commits = self
            .view()
            .wc_commit_ids()
            .iter()
            .filter_map(|(name, commit_id)| {
                let change = rewrite_mapping.get_key_value(commit_id)?;
                Some((name.to_owned(), change))
            })
            .collect_vec();
        let mut recreated_wc_commits: HashMap<&CommitId, Commit> = HashMap::new();
        for (name, (old_commit_id, new_commit_ids)) in changed_wc_commits {
            let abandoned_old_commit = matches!(
                self.parent_mapping.get(old_commit_id),
                Some(Rewrite::Abandoned(_))
            );
            let new_wc_commit = if !abandoned_old_commit {
                // We arbitrarily pick a new working-copy commit among the candidates.
                self.store().get_commit(&new_commit_ids[0])?
            } else if let Some(commit) = recreated_wc_commits.get(old_commit_id) {
                commit.clone()
            } else {
                let new_commits: Vec<_> = new_commit_ids
                    .iter()
                    .map(|id| self.store().get_commit(id))
                    .try_collect()?;
                let merged_parents_tree = merge_commit_trees(self, &new_commits).block_on()?;
                let commit = self
                    .new_commit(new_commit_ids.clone(), merged_parents_tree.id().clone())
                    .write()?;
                recreated_wc_commits.insert(old_commit_id, commit.clone());
                commit
            };
            self.edit(name, &new_wc_commit).map_err(|err| match err {
                EditCommitError::BackendError(backend_error) => backend_error,
                EditCommitError::WorkingCopyCommitNotFound(_)
                | EditCommitError::RewriteRootCommit(_) => panic!("unexpected error: {err:?}"),
            })?;
        }
        Ok(())
    }

    fn update_heads(&mut self) -> Result<(), RevsetEvaluationError> {
        let old_commits_expression =
            RevsetExpression::commits(self.parent_mapping.keys().cloned().collect());
        let heads_to_add_expression = old_commits_expression
            .parents()
            .minus(&old_commits_expression);
        let heads_to_add: Vec<_> = heads_to_add_expression
            .evaluate(self)?
            .iter()
            .try_collect()?;

        let mut view = self.view().store_view().clone();
        for commit_id in self.parent_mapping.keys() {
            view.head_ids.remove(commit_id);
        }
        view.head_ids.extend(heads_to_add);
        self.set_view(view);
        Ok(())
    }

    /// Find descendants of `root`, unless they've already been rewritten
    /// (according to `parent_mapping`).
    pub fn find_descendants_for_rebase(&self, roots: Vec<CommitId>) -> BackendResult<Vec<Commit>> {
        let to_visit_revset = RevsetExpression::commits(roots)
            .descendants()
            .minus(&RevsetExpression::commits(
                self.parent_mapping.keys().cloned().collect(),
            ))
            .evaluate(self)
            .map_err(|err| err.into_backend_error())?;
        let to_visit = to_visit_revset
            .iter()
            .commits(self.store())
            .try_collect()
            .map_err(|err| err.into_backend_error())?;
        Ok(to_visit)
    }

    /// Order a set of commits in an order they should be rebased in. The result
    /// is in reverse order so the next value can be removed from the end.
    fn order_commits_for_rebase(
        &self,
        to_visit: Vec<Commit>,
        new_parents_map: &HashMap<CommitId, Vec<CommitId>>,
    ) -> BackendResult<Vec<Commit>> {
        let to_visit_set: HashSet<CommitId> =
            to_visit.iter().map(|commit| commit.id().clone()).collect();
        let mut visited = HashSet::new();
        // Calculate an order where we rebase parents first, but if the parents were
        // rewritten, make sure we rebase the rewritten parent first.
        let store = self.store();
        dag_walk::topo_order_reverse_ok(
            to_visit.into_iter().map(Ok),
            |commit| commit.id().clone(),
            |commit| -> Vec<BackendResult<Commit>> {
                visited.insert(commit.id().clone());
                let mut dependents = vec![];
                let parent_ids = new_parents_map
                    .get(commit.id())
                    .map_or(commit.parent_ids(), |parent_ids| parent_ids);
                for parent_id in parent_ids {
                    let parent = store.get_commit(parent_id);
                    let Ok(parent) = parent else {
                        dependents.push(parent);
                        continue;
                    };
                    if let Some(rewrite) = self.parent_mapping.get(parent.id()) {
                        for target in rewrite.new_parent_ids() {
                            if to_visit_set.contains(target) && !visited.contains(target) {
                                dependents.push(store.get_commit(target));
                            }
                        }
                    }
                    if to_visit_set.contains(parent.id()) {
                        dependents.push(Ok(parent));
                    }
                }
                dependents
            },
            |_| panic!("graph has cycle"),
        )
    }

    /// Rewrite descendants of the given roots.
    ///
    /// The callback will be called for each commit with the new parents
    /// prepopulated. The callback may change the parents and write the new
    /// commit, or it may abandon the commit, or it may leave the old commit
    /// unchanged.
    ///
    /// The set of commits to visit is determined at the start. If the callback
    /// adds new descendants, then the callback will not be called for those.
    /// Similarly, if the callback rewrites unrelated commits, then the callback
    /// will not be called for descendants of those commits.
    pub fn transform_descendants(
        &mut self,
        roots: Vec<CommitId>,
        callback: impl AsyncFnMut(CommitRewriter) -> BackendResult<()>,
    ) -> BackendResult<()> {
        let options = RewriteRefsOptions::default();
        self.transform_descendants_with_options(roots, &HashMap::new(), &options, callback)
    }

    /// Rewrite descendants of the given roots with options.
    ///
    /// If a commit is in the `new_parents_map` is provided, it will be rebased
    /// onto the new parents provided in the map instead of its original
    /// parents.
    ///
    /// See [`Self::transform_descendants()`] for details.
    pub fn transform_descendants_with_options(
        &mut self,
        roots: Vec<CommitId>,
        new_parents_map: &HashMap<CommitId, Vec<CommitId>>,
        options: &RewriteRefsOptions,
        callback: impl AsyncFnMut(CommitRewriter) -> BackendResult<()>,
    ) -> BackendResult<()> {
        let descendants = self.find_descendants_for_rebase(roots)?;
        self.transform_commits(descendants, new_parents_map, options, callback)
    }

    /// Rewrite the given commits in reverse topological order.
    ///
    /// `commits` should be a connected range.
    ///
    /// This function is similar to
    /// [`Self::transform_descendants_with_options()`], but only rewrites the
    /// `commits` provided, and does not rewrite their descendants.
    pub fn transform_commits(
        &mut self,
        commits: Vec<Commit>,
        new_parents_map: &HashMap<CommitId, Vec<CommitId>>,
        options: &RewriteRefsOptions,
        mut callback: impl AsyncFnMut(CommitRewriter) -> BackendResult<()>,
    ) -> BackendResult<()> {
        let mut to_visit = self.order_commits_for_rebase(commits, new_parents_map)?;
        while let Some(old_commit) = to_visit.pop() {
            let parent_ids = new_parents_map
                .get(old_commit.id())
                .map_or(old_commit.parent_ids(), |parent_ids| parent_ids);
            let new_parent_ids = self.new_parents(parent_ids);
            let rewriter = CommitRewriter::new(self, old_commit, new_parent_ids);
            callback(rewriter).block_on()?;
        }
        self.update_rewritten_references(options)?;
        // Since we didn't necessarily visit all descendants of rewritten commits (e.g.
        // if they were rewritten in the callback), there can still be commits left to
        // rebase, so we don't clear `parent_mapping` here.
        // TODO: Should we make this stricter? We could check that there were no
        // rewrites before this function was called, and we can check that only
        // commits in the `to_visit` set were added by the callback. Then we
        // could clear `parent_mapping` here and not have to scan it again at
        // the end of the transaction when we call `rebase_descendants()`.

        Ok(())
    }

    /// Rebase descendants of the rewritten commits with options and callback.
    ///
    /// The descendants of the commits registered in `self.parent_mappings` will
    /// be recursively rebased onto the new version of their parents.
    ///
    /// If `options.empty` is the default (`EmptyBehavior::Keep`), all rebased
    /// descendant commits will be preserved even if they were emptied following
    /// the rebase operation. Otherwise, this function may rebase some commits
    /// and abandon others, based on the given `EmptyBehavior`. The behavior is
    /// such that only commits with a single parent will ever be abandoned. The
    /// parent will inherit the descendants and the bookmarks of the abandoned
    /// commit.
    ///
    /// The `progress` callback will be invoked for each rebase operation with
    /// `(old_commit, rebased_commit)` as arguments.
    pub fn rebase_descendants_with_options(
        &mut self,
        options: &RebaseOptions,
        mut progress: impl FnMut(Commit, RebasedCommit),
    ) -> BackendResult<()> {
        let roots = self.parent_mapping.keys().cloned().collect();
        self.transform_descendants_with_options(
            roots,
            &HashMap::new(),
            &options.rewrite_refs,
            async |rewriter| {
                if rewriter.parents_changed() {
                    let old_commit = rewriter.old_commit().clone();
                    let rebased_commit = rebase_commit_with_options(rewriter, options)?;
                    progress(old_commit, rebased_commit);
                }
                Ok(())
            },
        )?;
        self.parent_mapping.clear();
        Ok(())
    }

    /// Rebase descendants of the rewritten commits.
    ///
    /// The descendants of the commits registered in `self.parent_mappings` will
    /// be recursively rebased onto the new version of their parents.
    /// Returns the number of rebased descendants.
    ///
    /// All rebased descendant commits will be preserved even if they were
    /// emptied following the rebase operation. To customize the rebase
    /// behavior, use [`MutableRepo::rebase_descendants_with_options`].
    pub fn rebase_descendants(&mut self) -> BackendResult<usize> {
        let options = RebaseOptions::default();
        let mut num_rebased = 0;
        self.rebase_descendants_with_options(&options, |_old_commit, _rebased_commit| {
            num_rebased += 1;
        })?;
        Ok(num_rebased)
    }

    /// Reparent descendants of the rewritten commits.
    ///
    /// The descendants of the commits registered in `self.parent_mappings` will
    /// be recursively reparented onto the new version of their parents.
    /// The content of those descendants will remain untouched.
    /// Returns the number of reparented descendants.
    pub fn reparent_descendants(&mut self) -> BackendResult<usize> {
        let roots = self.parent_mapping.keys().cloned().collect_vec();
        let mut num_reparented = 0;
        self.transform_descendants(roots, async |rewriter| {
            if rewriter.parents_changed() {
                let builder = rewriter.reparent();
                builder.write()?;
                num_reparented += 1;
            }
            Ok(())
        })?;
        self.parent_mapping.clear();
        Ok(num_reparented)
    }

    pub fn set_wc_commit(
        &mut self,
        name: WorkspaceNameBuf,
        commit_id: CommitId,
    ) -> Result<(), RewriteRootCommit> {
        if &commit_id == self.store().root_commit_id() {
            return Err(RewriteRootCommit);
        }
        self.view_mut().set_wc_commit(name, commit_id);
        Ok(())
    }

    pub fn remove_wc_commit(&mut self, name: &WorkspaceName) -> Result<(), EditCommitError> {
        self.maybe_abandon_wc_commit(name)?;
        self.view_mut().remove_wc_commit(name);
        Ok(())
    }

    /// Merges working-copy commit. If there's a conflict, and if the workspace
    /// isn't removed at either side, we keep the self side.
    fn merge_wc_commit(
        &mut self,
        name: &WorkspaceName,
        base_id: Option<&CommitId>,
        other_id: Option<&CommitId>,
    ) {
        let view = self.view.get_mut();
        let self_id = view.get_wc_commit_id(name);
        // Not using merge_ref_targets(). Since the working-copy pointer moves
        // towards random direction, it doesn't make sense to resolve conflict
        // based on ancestry.
        let new_id = if let Some(resolved) =
            trivial_merge(&[self_id, base_id, other_id], SameChange::Accept)
        {
            resolved.cloned()
        } else if self_id.is_none() || other_id.is_none() {
            // We want to remove the workspace even if the self side changed the
            // working-copy commit.
            None
        } else {
            self_id.cloned()
        };
        match new_id {
            Some(id) => view.set_wc_commit(name.to_owned(), id),
            None => view.remove_wc_commit(name),
        }
    }

    pub fn rename_workspace(
        &mut self,
        old_name: &WorkspaceName,
        new_name: WorkspaceNameBuf,
    ) -> Result<(), RenameWorkspaceError> {
        self.view_mut().rename_workspace(old_name, new_name)
    }

    pub fn check_out(
        &mut self,
        name: WorkspaceNameBuf,
        commit: &Commit,
    ) -> Result<Commit, CheckOutCommitError> {
        let wc_commit = self
            .new_commit(vec![commit.id().clone()], commit.tree_id().clone())
            .write()?;
        self.edit(name, &wc_commit)?;
        Ok(wc_commit)
    }

    pub fn edit(&mut self, name: WorkspaceNameBuf, commit: &Commit) -> Result<(), EditCommitError> {
        self.maybe_abandon_wc_commit(&name)?;
        self.add_head(commit)?;
        Ok(self.set_wc_commit(name, commit.id().clone())?)
    }

    fn maybe_abandon_wc_commit(
        &mut self,
        workspace_name: &WorkspaceName,
    ) -> Result<(), EditCommitError> {
        let is_commit_referenced = |view: &View, commit_id: &CommitId| -> bool {
            view.wc_commit_ids()
                .iter()
                .filter(|&(name, _)| name != workspace_name)
                .map(|(_, wc_id)| wc_id)
                .chain(
                    view.local_bookmarks()
                        .flat_map(|(_, target)| target.added_ids()),
                )
                .any(|id| id == commit_id)
        };

        let maybe_wc_commit_id = self
            .view
            .with_ref(|v| v.get_wc_commit_id(workspace_name).cloned());
        if let Some(wc_commit_id) = maybe_wc_commit_id {
            let wc_commit = self
                .store()
                .get_commit(&wc_commit_id)
                .map_err(EditCommitError::WorkingCopyCommitNotFound)?;
            if wc_commit.is_discardable(self)?
                && self
                    .view
                    .with_ref(|v| !is_commit_referenced(v, wc_commit.id()))
                && self.view().heads().contains(wc_commit.id())
            {
                // Abandon the working-copy commit we're leaving if it's
                // discardable, not pointed by local bookmark or other working
                // copies, and a head commit.
                self.record_abandoned_commit(&wc_commit);
            }
        }

        Ok(())
    }

    fn enforce_view_invariants(&self, view: &mut View) {
        let view = view.store_view_mut();
        let root_commit_id = self.store().root_commit_id();
        if view.head_ids.is_empty() {
            view.head_ids.insert(root_commit_id.clone());
        } else if view.head_ids.len() > 1 {
            // An empty head_ids set is padded with the root_commit_id, but the
            // root id is unwanted during the heads resolution.
            view.head_ids.remove(root_commit_id);
            // It is unclear if `heads` can never fail for default implementation,
            // but it can definitely fail for non-default implementations.
            // TODO: propagate errors.
            view.head_ids = self
                .index()
                .heads(&mut view.head_ids.iter())
                .unwrap()
                .into_iter()
                .collect();
        }
        assert!(!view.head_ids.is_empty());
    }

    /// Ensures that the given `head` and ancestor commits are reachable from
    /// the visible heads.
    pub fn add_head(&mut self, head: &Commit) -> BackendResult<()> {
        self.add_heads(slice::from_ref(head))
    }

    /// Ensures that the given `heads` and ancestor commits are reachable from
    /// the visible heads.
    ///
    /// The `heads` may contain redundant commits such as already visible ones
    /// and ancestors of the other heads. The `heads` and ancestor commits
    /// should exist in the store.
    pub fn add_heads(&mut self, heads: &[Commit]) -> BackendResult<()> {
        let current_heads = self.view.get_mut().heads();
        // Use incremental update for common case of adding a single commit on top a
        // current head. TODO: Also use incremental update when adding a single
        // commit on top a non-head.
        match heads {
            [] => {}
            [head]
                if head
                    .parent_ids()
                    .iter()
                    .all(|parent_id| current_heads.contains(parent_id)) =>
            {
                self.index
                    .add_commit(head)
                    // TODO: indexing error shouldn't be a "BackendError"
                    .map_err(|err| BackendError::Other(err.into()))?;
                self.view.get_mut().add_head(head.id());
                for parent_id in head.parent_ids() {
                    self.view.get_mut().remove_head(parent_id);
                }
            }
            _ => {
                let missing_commits = dag_walk::topo_order_reverse_ord_ok(
                    heads
                        .iter()
                        .cloned()
                        .map(CommitByCommitterTimestamp)
                        .map(Ok),
                    |CommitByCommitterTimestamp(commit)| commit.id().clone(),
                    |CommitByCommitterTimestamp(commit)| {
                        commit
                            .parent_ids()
                            .iter()
                            .filter_map(|id| match self.index().has_id(id) {
                                Ok(false) => Some(
                                    self.store().get_commit(id).map(CommitByCommitterTimestamp),
                                ),
                                Ok(true) => None,
                                // TODO: indexing error shouldn't be a "BackendError"
                                Err(err) => Some(Err(BackendError::Other(err.into()))),
                            })
                            .collect_vec()
                    },
                    |_| panic!("graph has cycle"),
                )?;
                for CommitByCommitterTimestamp(missing_commit) in missing_commits.iter().rev() {
                    self.index
                        .add_commit(missing_commit)
                        // TODO: indexing error shouldn't be a "BackendError"
                        .map_err(|err| BackendError::Other(err.into()))?;
                }
                for head in heads {
                    self.view.get_mut().add_head(head.id());
                }
                self.view.mark_dirty();
            }
        }
        Ok(())
    }

    pub fn remove_head(&mut self, head: &CommitId) {
        self.view_mut().remove_head(head);
        self.view.mark_dirty();
    }

    pub fn get_local_bookmark(&self, name: &RefName) -> RefTarget {
        self.view.with_ref(|v| v.get_local_bookmark(name).clone())
    }

    pub fn set_local_bookmark_target(&mut self, name: &RefName, target: RefTarget) {
        let view = self.view_mut();
        for id in target.added_ids() {
            view.add_head(id);
        }
        view.set_local_bookmark_target(name, target);
        self.view.mark_dirty();
    }

    pub fn merge_local_bookmark(
        &mut self,
        name: &RefName,
        base_target: &RefTarget,
        other_target: &RefTarget,
    ) {
        let view = self.view.get_mut();
        let index = self.index.as_index();
        let self_target = view.get_local_bookmark(name);
        let new_target = merge_ref_targets(index, self_target, base_target, other_target);
        self.set_local_bookmark_target(name, new_target);
    }

    pub fn get_remote_bookmark(&self, symbol: RemoteRefSymbol<'_>) -> RemoteRef {
        self.view
            .with_ref(|v| v.get_remote_bookmark(symbol).clone())
    }

    pub fn set_remote_bookmark(&mut self, symbol: RemoteRefSymbol<'_>, remote_ref: RemoteRef) {
        self.view_mut().set_remote_bookmark(symbol, remote_ref);
    }

    fn merge_remote_bookmark(
        &mut self,
        symbol: RemoteRefSymbol<'_>,
        base_ref: &RemoteRef,
        other_ref: &RemoteRef,
    ) {
        let view = self.view.get_mut();
        let index = self.index.as_index();
        let self_ref = view.get_remote_bookmark(symbol);
        let new_ref = merge_remote_refs(index, self_ref, base_ref, other_ref);
        view.set_remote_bookmark(symbol, new_ref);
    }

    /// Merges the specified remote bookmark in to local bookmark, and starts
    /// tracking it.
    pub fn track_remote_bookmark(&mut self, symbol: RemoteRefSymbol<'_>) {
        let mut remote_ref = self.get_remote_bookmark(symbol);
        let base_target = remote_ref.tracked_target();
        self.merge_local_bookmark(symbol.name, base_target, &remote_ref.target);
        remote_ref.state = RemoteRefState::Tracked;
        self.set_remote_bookmark(symbol, remote_ref);
    }

    /// Stops tracking the specified remote bookmark.
    pub fn untrack_remote_bookmark(&mut self, symbol: RemoteRefSymbol<'_>) {
        let mut remote_ref = self.get_remote_bookmark(symbol);
        remote_ref.state = RemoteRefState::New;
        self.set_remote_bookmark(symbol, remote_ref);
    }

    pub fn ensure_remote(&mut self, remote_name: &RemoteName) {
        self.view_mut().ensure_remote(remote_name);
    }

    pub fn remove_remote(&mut self, remote_name: &RemoteName) {
        self.view_mut().remove_remote(remote_name);
    }

    pub fn rename_remote(&mut self, old: &RemoteName, new: &RemoteName) {
        self.view_mut().rename_remote(old, new);
    }

    pub fn get_local_tag(&self, name: &RefName) -> RefTarget {
        self.view.with_ref(|v| v.get_local_tag(name).clone())
    }

    pub fn set_local_tag_target(&mut self, name: &RefName, target: RefTarget) {
        self.view_mut().set_local_tag_target(name, target);
    }

    pub fn merge_local_tag(
        &mut self,
        name: &RefName,
        base_target: &RefTarget,
        other_target: &RefTarget,
    ) {
        let view = self.view.get_mut();
        let index = self.index.as_index();
        let self_target = view.get_local_tag(name);
        let new_target = merge_ref_targets(index, self_target, base_target, other_target);
        view.set_local_tag_target(name, new_target);
    }

    pub fn get_remote_tag(&self, symbol: RemoteRefSymbol<'_>) -> RemoteRef {
        self.view.with_ref(|v| v.get_remote_tag(symbol).clone())
    }

    pub fn set_remote_tag(&mut self, symbol: RemoteRefSymbol<'_>, remote_ref: RemoteRef) {
        self.view_mut().set_remote_tag(symbol, remote_ref);
    }

    fn merge_remote_tag(
        &mut self,
        symbol: RemoteRefSymbol<'_>,
        base_ref: &RemoteRef,
        other_ref: &RemoteRef,
    ) {
        let view = self.view.get_mut();
        let index = self.index.as_index();
        let self_ref = view.get_remote_tag(symbol);
        let new_ref = merge_remote_refs(index, self_ref, base_ref, other_ref);
        view.set_remote_tag(symbol, new_ref);
    }

    pub fn get_git_ref(&self, name: &GitRefName) -> RefTarget {
        self.view.with_ref(|v| v.get_git_ref(name).clone())
    }

    pub fn set_git_ref_target(&mut self, name: &GitRefName, target: RefTarget) {
        self.view_mut().set_git_ref_target(name, target);
    }

    fn merge_git_ref(
        &mut self,
        name: &GitRefName,
        base_target: &RefTarget,
        other_target: &RefTarget,
    ) {
        let view = self.view.get_mut();
        let index = self.index.as_index();
        let self_target = view.get_git_ref(name);
        let new_target = merge_ref_targets(index, self_target, base_target, other_target);
        view.set_git_ref_target(name, new_target);
    }

    pub fn git_head(&self) -> RefTarget {
        self.view.with_ref(|v| v.git_head().clone())
    }

    pub fn set_git_head_target(&mut self, target: RefTarget) {
        self.view_mut().set_git_head_target(target);
    }

    pub fn set_view(&mut self, data: op_store::View) {
        self.view_mut().set_view(data);
        self.view.mark_dirty();
    }

    pub fn merge(
        &mut self,
        base_repo: &ReadonlyRepo,
        other_repo: &ReadonlyRepo,
    ) -> Result<(), RepoLoaderError> {
        // First, merge the index, so we can take advantage of a valid index when
        // merging the view. Merging in base_repo's index isn't typically
        // necessary, but it can be if base_repo is ahead of either self or other_repo
        // (e.g. because we're undoing an operation that hasn't been published).
        self.index.merge_in(base_repo.readonly_index())?;
        self.index.merge_in(other_repo.readonly_index())?;

        self.view.ensure_clean(|v| self.enforce_view_invariants(v));
        self.merge_view(&base_repo.view, &other_repo.view)?;
        self.view.mark_dirty();
        Ok(())
    }

    pub fn merge_index(&mut self, other_repo: &ReadonlyRepo) -> IndexResult<()> {
        self.index.merge_in(other_repo.readonly_index())
    }

    fn merge_view(&mut self, base: &View, other: &View) -> BackendResult<()> {
        let changed_wc_commits = diff_named_commit_ids(base.wc_commit_ids(), other.wc_commit_ids());
        for (name, (base_id, other_id)) in changed_wc_commits {
            self.merge_wc_commit(name, base_id, other_id);
        }

        let base_heads = base.heads().iter().cloned().collect_vec();
        let own_heads = self.view().heads().iter().cloned().collect_vec();
        let other_heads = other.heads().iter().cloned().collect_vec();

        // HACK: Don't walk long ranges of commits to find rewrites when using other
        // custom implementations. The only custom index implementation we're currently
        // aware of is Google's. That repo has too high commit rate for it to be
        // feasible to walk all added and removed commits.
        // TODO: Fix this somehow. Maybe a method on `Index` to find rewritten commits
        // given `base_heads`, `own_heads` and `other_heads`?
        if self.is_backed_by_default_index() {
            self.record_rewrites(&base_heads, &own_heads)?;
            self.record_rewrites(&base_heads, &other_heads)?;
            // No need to remove heads removed by `other` because we already
            // marked them abandoned or rewritten.
        } else {
            for removed_head in base.heads().difference(other.heads()) {
                self.view_mut().remove_head(removed_head);
            }
        }
        for added_head in other.heads().difference(base.heads()) {
            self.view_mut().add_head(added_head);
        }

        let changed_local_bookmarks =
            diff_named_ref_targets(base.local_bookmarks(), other.local_bookmarks());
        for (name, (base_target, other_target)) in changed_local_bookmarks {
            self.merge_local_bookmark(name, base_target, other_target);
        }

        let changed_local_tags = diff_named_ref_targets(base.local_tags(), other.local_tags());
        for (name, (base_target, other_target)) in changed_local_tags {
            self.merge_local_tag(name, base_target, other_target);
        }

        let changed_git_refs = diff_named_ref_targets(base.git_refs(), other.git_refs());
        for (name, (base_target, other_target)) in changed_git_refs {
            self.merge_git_ref(name, base_target, other_target);
        }

        let changed_remote_bookmarks =
            diff_named_remote_refs(base.all_remote_bookmarks(), other.all_remote_bookmarks());
        for (symbol, (base_ref, other_ref)) in changed_remote_bookmarks {
            self.merge_remote_bookmark(symbol, base_ref, other_ref);
        }

        let changed_remote_tags =
            diff_named_remote_refs(base.all_remote_tags(), other.all_remote_tags());
        for (symbol, (base_ref, other_ref)) in changed_remote_tags {
            self.merge_remote_tag(symbol, base_ref, other_ref);
        }

        let new_git_head_target = merge_ref_targets(
            self.index(),
            self.view().git_head(),
            base.git_head(),
            other.git_head(),
        );
        self.set_git_head_target(new_git_head_target);

        Ok(())
    }

    /// Finds and records commits that were rewritten or abandoned between
    /// `old_heads` and `new_heads`.
    fn record_rewrites(
        &mut self,
        old_heads: &[CommitId],
        new_heads: &[CommitId],
    ) -> BackendResult<()> {
        let mut removed_changes: HashMap<ChangeId, Vec<CommitId>> = HashMap::new();
        for item in revset::walk_revs(self, old_heads, new_heads)
            .map_err(|err| err.into_backend_error())?
            .commit_change_ids()
        {
            let (commit_id, change_id) = item.map_err(|err| err.into_backend_error())?;
            removed_changes
                .entry(change_id)
                .or_default()
                .push(commit_id);
        }
        if removed_changes.is_empty() {
            return Ok(());
        }

        let mut rewritten_changes = HashSet::new();
        let mut rewritten_commits: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        for item in revset::walk_revs(self, new_heads, old_heads)
            .map_err(|err| err.into_backend_error())?
            .commit_change_ids()
        {
            let (commit_id, change_id) = item.map_err(|err| err.into_backend_error())?;
            if let Some(old_commits) = removed_changes.get(&change_id) {
                for old_commit in old_commits {
                    rewritten_commits
                        .entry(old_commit.clone())
                        .or_default()
                        .push(commit_id.clone());
                }
            }
            rewritten_changes.insert(change_id);
        }
        for (old_commit, new_commits) in rewritten_commits {
            if new_commits.len() == 1 {
                self.set_rewritten_commit(
                    old_commit.clone(),
                    new_commits.into_iter().next().unwrap(),
                );
            } else {
                self.set_divergent_rewrite(old_commit.clone(), new_commits);
            }
        }

        for (change_id, removed_commit_ids) in &removed_changes {
            if !rewritten_changes.contains(change_id) {
                for id in removed_commit_ids {
                    let commit = self.store().get_commit(id)?;
                    self.record_abandoned_commit(&commit);
                }
            }
        }

        Ok(())
    }
}

impl Repo for MutableRepo {
    fn base_repo(&self) -> &ReadonlyRepo {
        &self.base_repo
    }

    fn store(&self) -> &Arc<Store> {
        self.base_repo.store()
    }

    fn op_store(&self) -> &Arc<dyn OpStore> {
        self.base_repo.op_store()
    }

    fn index(&self) -> &dyn Index {
        self.index.as_index()
    }

    fn view(&self) -> &View {
        self.view
            .get_or_ensure_clean(|v| self.enforce_view_invariants(v))
    }

    fn submodule_store(&self) -> &Arc<dyn SubmoduleStore> {
        self.base_repo.submodule_store()
    }

    fn resolve_change_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> IndexResult<PrefixResolution<Vec<CommitId>>> {
        let change_id_index = self.index.change_id_index(&mut self.view().heads().iter());
        change_id_index.resolve_prefix(prefix)
    }

    fn shortest_unique_change_id_prefix_len(&self, target_id: &ChangeId) -> IndexResult<usize> {
        let change_id_index = self.index.change_id_index(&mut self.view().heads().iter());
        change_id_index.shortest_unique_prefix_len(target_id)
    }
}

/// Error from attempts to check out the root commit for editing
#[derive(Debug, Error)]
#[error("Cannot rewrite the root commit")]
pub struct RewriteRootCommit;

/// Error from attempts to edit a commit
#[derive(Debug, Error)]
pub enum EditCommitError {
    #[error("Current working-copy commit not found")]
    WorkingCopyCommitNotFound(#[source] BackendError),
    #[error(transparent)]
    RewriteRootCommit(#[from] RewriteRootCommit),
    #[error(transparent)]
    BackendError(#[from] BackendError),
}

/// Error from attempts to check out a commit
#[derive(Debug, Error)]
pub enum CheckOutCommitError {
    #[error("Failed to create new working-copy commit")]
    CreateCommit(#[from] BackendError),
    #[error("Failed to edit commit")]
    EditCommit(#[from] EditCommitError),
}

mod dirty_cell {
    use std::cell::OnceCell;
    use std::cell::RefCell;

    /// Cell that lazily updates the value after `mark_dirty()`.
    ///
    /// A clean value can be immutably borrowed within the `self` lifetime.
    #[derive(Clone, Debug)]
    pub struct DirtyCell<T> {
        // Either clean or dirty value is set. The value is boxed to reduce stack space
        // and memcopy overhead.
        clean: OnceCell<Box<T>>,
        dirty: RefCell<Option<Box<T>>>,
    }

    impl<T> DirtyCell<T> {
        pub fn with_clean(value: T) -> Self {
            Self {
                clean: OnceCell::from(Box::new(value)),
                dirty: RefCell::new(None),
            }
        }

        pub fn get_or_ensure_clean(&self, f: impl FnOnce(&mut T)) -> &T {
            self.clean.get_or_init(|| {
                // Panics if ensure_clean() is invoked from with_ref() callback for example.
                let mut value = self.dirty.borrow_mut().take().unwrap();
                f(&mut value);
                value
            })
        }

        pub fn ensure_clean(&self, f: impl FnOnce(&mut T)) {
            self.get_or_ensure_clean(f);
        }

        pub fn into_inner(self) -> T {
            *self
                .clean
                .into_inner()
                .or_else(|| self.dirty.into_inner())
                .unwrap()
        }

        pub fn with_ref<R>(&self, f: impl FnOnce(&T) -> R) -> R {
            if let Some(value) = self.clean.get() {
                f(value)
            } else {
                f(self.dirty.borrow().as_ref().unwrap())
            }
        }

        pub fn get_mut(&mut self) -> &mut T {
            self.clean
                .get_mut()
                .or_else(|| self.dirty.get_mut().as_mut())
                .unwrap()
        }

        pub fn mark_dirty(&mut self) {
            if let Some(value) = self.clean.take() {
                *self.dirty.get_mut() = Some(value);
            }
        }
    }
}
