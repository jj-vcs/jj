// Copyright 2021 The Jujutsu Authors
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

#![allow(missing_docs)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::sync::Arc;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use thiserror::Error;

use crate::backend::BackendInitError;
use crate::backend::MergedTreeId;
use crate::commit::Commit;
use crate::file_util::canonicalize_path;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::local_working_copy::LocalWorkingCopy;
use crate::local_working_copy::LocalWorkingCopyFactory;
use crate::op_heads_store::OpHeadsStoreError;
use crate::op_store::OperationId;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::repo::read_store_type;
use crate::repo::BackendInitializer;
use crate::repo::CheckOutCommitError;
use crate::repo::IndexStoreInitializer;
use crate::repo::OpHeadsStoreInitializer;
use crate::repo::OpStoreInitializer;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RepoInitError;
use crate::repo::RepoLoader;
use crate::repo::StoreFactories;
use crate::repo::StoreLoadError;
use crate::repo::SubmoduleStoreInitializer;
use crate::settings::UserSettings;
use crate::signing::SignInitError;
use crate::signing::Signer;
use crate::simple_backend::SimpleBackend;
use crate::transaction::TransactionCommitError;
use crate::working_copy::CheckoutError;
use crate::working_copy::CheckoutOptions;
use crate::working_copy::CheckoutStats;
use crate::working_copy::LockedWorkingCopy;
use crate::working_copy::WorkingCopy;
use crate::working_copy::WorkingCopyFactory;
use crate::working_copy::WorkingCopyStateError;

#[derive(Error, Debug)]
pub enum WorkspaceInitError {
    #[error("The destination repo ({0}) already exists")]
    DestinationExists(Utf8PathBuf),
    #[error("Repo path could not be interpreted as Unicode text")]
    NonUnicodePath,
    #[error(transparent)]
    CheckOutCommit(#[from] CheckOutCommitError),
    #[error(transparent)]
    WorkingCopyState(#[from] WorkingCopyStateError),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    OpHeadsStore(OpHeadsStoreError),
    #[error(transparent)]
    Backend(#[from] BackendInitError),
    #[error(transparent)]
    SignInit(#[from] SignInitError),
    #[error(transparent)]
    TransactionCommit(#[from] TransactionCommitError),
}

#[derive(Error, Debug)]
pub enum WorkspaceLoadError {
    #[error("The repo appears to no longer be at {0}")]
    RepoDoesNotExist(Utf8PathBuf),
    #[error("There is no Jujutsu repo in {0}")]
    NoWorkspaceHere(Utf8PathBuf),
    #[error("Cannot read the repo")]
    StoreLoadError(#[from] StoreLoadError),
    #[error("Repo path could not be interpreted as Unicode text")]
    NonUnicodePath,
    #[error(transparent)]
    WorkingCopyState(#[from] WorkingCopyStateError),
    #[error(transparent)]
    Path(#[from] PathError),
}

/// The combination of a repo and a working copy.
///
/// Represents the combination of a repo and working copy, i.e. what's typically
/// the .jj/ directory and its parent. See
/// <https://github.com/jj-vcs/jj/blob/main/docs/working-copy.md#workspaces>
/// for more information.
pub struct Workspace {
    // Path to the workspace root (typically the parent of a .jj/ directory), which is where
    // working copy files live.
    workspace_root: Utf8PathBuf,
    repo_path: Utf8PathBuf,
    repo_loader: RepoLoader,
    working_copy: Box<dyn WorkingCopy>,
}

fn create_jj_dir(workspace_root: &Utf8Path) -> Result<Utf8PathBuf, WorkspaceInitError> {
    let jj_dir = workspace_root.join(".jj");
    match std::fs::create_dir(&jj_dir).context(&jj_dir) {
        Ok(()) => Ok(jj_dir),
        Err(ref e) if e.error.kind() == io::ErrorKind::AlreadyExists => {
            Err(WorkspaceInitError::DestinationExists(jj_dir))
        }
        Err(e) => Err(e.into()),
    }
}

fn init_working_copy(
    repo: &Arc<ReadonlyRepo>,
    workspace_root: &Utf8Path,
    jj_dir: &Utf8Path,
    working_copy_factory: &dyn WorkingCopyFactory,
    workspace_name: WorkspaceNameBuf,
) -> Result<(Box<dyn WorkingCopy>, Arc<ReadonlyRepo>), WorkspaceInitError> {
    let working_copy_state_path = jj_dir.join("working_copy");
    std::fs::create_dir(&working_copy_state_path).context(&working_copy_state_path)?;

    let mut tx = repo.start_transaction();
    tx.repo_mut()
        .check_out(workspace_name.clone(), &repo.store().root_commit())?;
    let repo = tx.commit(format!("add workspace '{}'", workspace_name.as_symbol()))?;

    let working_copy = working_copy_factory.init_working_copy(
        repo.store().clone(),
        workspace_root.to_path_buf(),
        working_copy_state_path.clone(),
        repo.op_id().clone(),
        workspace_name,
    )?;
    let working_copy_type_path = working_copy_state_path.join("type");
    fs::write(&working_copy_type_path, working_copy.name()).context(&working_copy_type_path)?;
    Ok((working_copy, repo))
}

impl Workspace {
    pub fn new(
        workspace_root: &Utf8Path,
        repo_path: Utf8PathBuf,
        working_copy: Box<dyn WorkingCopy>,
        repo_loader: RepoLoader,
    ) -> Result<Workspace, PathError> {
        let workspace_root = canonicalize_path(workspace_root).context(workspace_root)?;
        Ok(Self::new_no_canonicalize(
            workspace_root,
            repo_path,
            working_copy,
            repo_loader,
        ))
    }

    pub fn new_no_canonicalize(
        workspace_root: Utf8PathBuf,
        repo_path: Utf8PathBuf,
        working_copy: Box<dyn WorkingCopy>,
        repo_loader: RepoLoader,
    ) -> Self {
        Self {
            workspace_root,
            repo_path,
            repo_loader,
            working_copy,
        }
    }

    pub fn init_simple(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let backend_initializer: &BackendInitializer =
            &|_settings, store_path| Ok(Box::new(SimpleBackend::init(store_path)));
        let signer = Signer::from_settings(user_settings)?;
        Self::init_with_backend(user_settings, workspace_root, backend_initializer, signer)
    }

    /// Initializes a workspace with a new Git backend and bare Git repo in
    /// `.jj/repo/store/git`.
    #[cfg(feature = "git")]
    pub fn init_internal_git(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let backend_initializer: &BackendInitializer = &|settings, store_path| {
            Ok(Box::new(crate::git_backend::GitBackend::init_internal(
                settings, store_path,
            )?))
        };
        let signer = Signer::from_settings(user_settings)?;
        Self::init_with_backend(user_settings, workspace_root, backend_initializer, signer)
    }

    /// Initializes a workspace with a new Git backend and Git repo that shares
    /// the same working copy.
    #[cfg(feature = "git")]
    pub fn init_colocated_git(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let backend_initializer = |settings: &UserSettings,
                                   store_path: &Utf8Path|
         -> Result<Box<dyn crate::backend::Backend>, _> {
            // TODO: Clean up path normalization. store_path is canonicalized by
            // ReadonlyRepo::init(). workspace_root will be canonicalized by
            // Workspace::new(), but it's not yet here.
            let store_relative_workspace_root =
                if let Ok(workspace_root) = canonicalize_path(workspace_root) {
                    crate::file_util::relative_path(store_path, &workspace_root)
                } else {
                    workspace_root.to_owned()
                };
            let backend = crate::git_backend::GitBackend::init_colocated(
                settings,
                store_path,
                &store_relative_workspace_root,
            )?;
            Ok(Box::new(backend))
        };
        let signer = Signer::from_settings(user_settings)?;
        Self::init_with_backend(user_settings, workspace_root, &backend_initializer, signer)
    }

    /// Initializes a workspace with an existing Git repo at the specified path.
    ///
    /// The `git_repo_path` usually ends with `.git`. It's the path to the Git
    /// repo directory, not the working directory.
    #[cfg(feature = "git")]
    pub fn init_external_git(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
        git_repo_path: &Utf8Path,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let backend_initializer = |settings: &UserSettings,
                                   store_path: &Utf8Path|
         -> Result<Box<dyn crate::backend::Backend>, _> {
            // If the git repo is inside the workspace, use a relative path to it so the
            // whole workspace can be moved without breaking.
            // TODO: Clean up path normalization. store_path is canonicalized by
            // ReadonlyRepo::init(). workspace_root will be canonicalized by
            // Workspace::new(), but it's not yet here.
            let store_relative_git_repo_path = match (
                canonicalize_path(workspace_root),
                crate::git_backend::canonicalize_git_repo_path(git_repo_path),
            ) {
                (Ok(workspace_root), Ok(git_repo_path))
                    if git_repo_path.starts_with(&workspace_root) =>
                {
                    crate::file_util::relative_path(store_path, &git_repo_path)
                }
                _ => git_repo_path.to_owned(),
            };
            let backend = crate::git_backend::GitBackend::init_external(
                settings,
                store_path,
                &store_relative_git_repo_path,
            )?;
            Ok(Box::new(backend))
        };
        let signer = Signer::from_settings(user_settings)?;
        Self::init_with_backend(user_settings, workspace_root, &backend_initializer, signer)
    }

    #[expect(clippy::too_many_arguments)]
    pub fn init_with_factories(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
        backend_initializer: &BackendInitializer,
        signer: Signer,
        op_store_initializer: &OpStoreInitializer,
        op_heads_store_initializer: &OpHeadsStoreInitializer,
        index_store_initializer: &IndexStoreInitializer,
        submodule_store_initializer: &SubmoduleStoreInitializer,
        working_copy_factory: &dyn WorkingCopyFactory,
        workspace_name: WorkspaceNameBuf,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let jj_dir = create_jj_dir(workspace_root)?;
        (|| {
            let repo_dir = jj_dir.join("repo");
            std::fs::create_dir(&repo_dir).context(&repo_dir)?;
            let repo = ReadonlyRepo::init(
                user_settings,
                &repo_dir,
                backend_initializer,
                signer,
                op_store_initializer,
                op_heads_store_initializer,
                index_store_initializer,
                submodule_store_initializer,
            )
            .map_err(|repo_init_err| match repo_init_err {
                RepoInitError::Backend(err) => WorkspaceInitError::Backend(err),
                RepoInitError::OpHeadsStore(err) => WorkspaceInitError::OpHeadsStore(err),
                RepoInitError::Path(err) => WorkspaceInitError::Path(err),
            })?;
            let (working_copy, repo) = init_working_copy(
                &repo,
                workspace_root,
                &jj_dir,
                working_copy_factory,
                workspace_name,
            )?;
            let repo_loader = repo.loader().clone();
            let workspace = Workspace::new(workspace_root, repo_dir, working_copy, repo_loader)?;
            Ok((workspace, repo))
        })()
        .inspect_err(|_err| {
            let _ = std::fs::remove_dir_all(jj_dir);
        })
    }

    pub fn init_with_backend(
        user_settings: &UserSettings,
        workspace_root: &Utf8Path,
        backend_initializer: &BackendInitializer,
        signer: Signer,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        Self::init_with_factories(
            user_settings,
            workspace_root,
            backend_initializer,
            signer,
            ReadonlyRepo::default_op_store_initializer(),
            ReadonlyRepo::default_op_heads_store_initializer(),
            ReadonlyRepo::default_index_store_initializer(),
            ReadonlyRepo::default_submodule_store_initializer(),
            &*default_working_copy_factory(),
            WorkspaceName::DEFAULT.to_owned(),
        )
    }

    pub fn init_workspace_with_existing_repo(
        workspace_root: &Utf8Path,
        repo_path: &Utf8Path,
        repo: &Arc<ReadonlyRepo>,
        working_copy_factory: &dyn WorkingCopyFactory,
        workspace_name: WorkspaceNameBuf,
    ) -> Result<(Self, Arc<ReadonlyRepo>), WorkspaceInitError> {
        let jj_dir = create_jj_dir(workspace_root)?;

        let repo_dir = canonicalize_path(repo_path).context(repo_path)?;
        let repo_file_path = jj_dir.join("repo");
        fs::write(&repo_file_path, repo_dir.as_str()).context(&repo_file_path)?;

        let (working_copy, repo) = init_working_copy(
            repo,
            workspace_root,
            &jj_dir,
            working_copy_factory,
            workspace_name,
        )?;
        let workspace = Workspace::new(
            workspace_root,
            repo_dir,
            working_copy,
            repo.loader().clone(),
        )?;
        Ok((workspace, repo))
    }

    pub fn load(
        user_settings: &UserSettings,
        workspace_path: &Utf8Path,
        store_factories: &StoreFactories,
        working_copy_factories: &WorkingCopyFactories,
    ) -> Result<Self, WorkspaceLoadError> {
        let loader = DefaultWorkspaceLoader::new(workspace_path)?;
        let workspace = loader.load(user_settings, store_factories, working_copy_factories)?;
        Ok(workspace)
    }

    pub fn workspace_root(&self) -> &Utf8Path {
        &self.workspace_root
    }

    pub fn workspace_name(&self) -> &WorkspaceName {
        self.working_copy.workspace_name()
    }

    pub fn repo_path(&self) -> &Utf8Path {
        &self.repo_path
    }

    pub fn repo_loader(&self) -> &RepoLoader {
        &self.repo_loader
    }

    /// Settings for this workspace.
    pub fn settings(&self) -> &UserSettings {
        self.repo_loader.settings()
    }

    pub fn working_copy(&self) -> &dyn WorkingCopy {
        self.working_copy.as_ref()
    }

    pub fn start_working_copy_mutation(
        &mut self,
    ) -> Result<LockedWorkspace<'_>, WorkingCopyStateError> {
        let locked_wc = self.working_copy.start_mutation()?;
        Ok(LockedWorkspace {
            base: self,
            locked_wc,
        })
    }

    pub fn check_out(
        &mut self,
        operation_id: OperationId,
        old_tree_id: Option<&MergedTreeId>,
        commit: &Commit,
        options: &CheckoutOptions,
    ) -> Result<CheckoutStats, CheckoutError> {
        let mut locked_ws =
            self.start_working_copy_mutation()
                .map_err(|err| CheckoutError::Other {
                    message: "Failed to start editing the working copy state".to_string(),
                    err: err.into(),
                })?;
        // Check if the current working-copy commit has changed on disk compared to what
        // the caller expected. It's safe to check out another commit
        // regardless, but it's probably not what  the caller wanted, so we let
        // them know.
        if let Some(old_tree_id) = old_tree_id {
            if old_tree_id != locked_ws.locked_wc().old_tree_id() {
                return Err(CheckoutError::ConcurrentCheckout);
            }
        }
        let stats = locked_ws.locked_wc().check_out(commit, options)?;
        locked_ws
            .finish(operation_id)
            .map_err(|err| CheckoutError::Other {
                message: "Failed to save the working copy state".to_string(),
                err: err.into(),
            })?;
        Ok(stats)
    }
}

pub struct LockedWorkspace<'a> {
    base: &'a mut Workspace,
    locked_wc: Box<dyn LockedWorkingCopy>,
}

impl LockedWorkspace<'_> {
    pub fn locked_wc(&mut self) -> &mut dyn LockedWorkingCopy {
        self.locked_wc.as_mut()
    }

    pub fn finish(self, operation_id: OperationId) -> Result<(), WorkingCopyStateError> {
        let new_wc = self.locked_wc.finish(operation_id)?;
        self.base.working_copy = new_wc;
        Ok(())
    }
}

// Factory trait to build WorkspaceLoaders given the workspace root.
pub trait WorkspaceLoaderFactory {
    fn create(
        &self,
        workspace_root: &Utf8Path,
    ) -> Result<Box<dyn WorkspaceLoader>, WorkspaceLoadError>;
}

pub fn get_working_copy_factory<'a>(
    workspace_loader: &dyn WorkspaceLoader,
    working_copy_factories: &'a WorkingCopyFactories,
) -> Result<&'a dyn WorkingCopyFactory, StoreLoadError> {
    let working_copy_type = workspace_loader.get_working_copy_type()?;

    if let Some(factory) = working_copy_factories.get(&working_copy_type) {
        Ok(factory.as_ref())
    } else {
        Err(StoreLoadError::UnsupportedType {
            store: "working copy",
            store_type: working_copy_type.to_string(),
        })
    }
}

// Loader assigned to a specific workspace root that knows how to load a
// Workspace object for that path.
pub trait WorkspaceLoader {
    // The root of the Workspace to be loaded.
    fn workspace_root(&self) -> &Utf8Path;

    // The path to the repo/ dir for this Workspace.
    fn repo_path(&self) -> &Utf8Path;

    // Loads the specified Workspace with the provided factories.
    fn load(
        &self,
        user_settings: &UserSettings,
        store_factories: &StoreFactories,
        working_copy_factories: &WorkingCopyFactories,
    ) -> Result<Workspace, WorkspaceLoadError>;

    // Returns the type identifier for the WorkingCopy trait in this Workspace.
    fn get_working_copy_type(&self) -> Result<String, StoreLoadError>;
}

pub struct DefaultWorkspaceLoaderFactory;

impl WorkspaceLoaderFactory for DefaultWorkspaceLoaderFactory {
    fn create(
        &self,
        workspace_root: &Utf8Path,
    ) -> Result<Box<dyn WorkspaceLoader>, WorkspaceLoadError> {
        Ok(Box::new(DefaultWorkspaceLoader::new(workspace_root)?))
    }
}

/// Helps create a `Workspace` instance by reading `.jj/repo/` and
/// `.jj/working_copy/` from the file system.
#[derive(Clone, Debug)]
struct DefaultWorkspaceLoader {
    workspace_root: Utf8PathBuf,
    repo_path: Utf8PathBuf,
    working_copy_state_path: Utf8PathBuf,
}

pub type WorkingCopyFactories = HashMap<String, Box<dyn WorkingCopyFactory>>;

impl DefaultWorkspaceLoader {
    pub fn new(workspace_root: &Utf8Path) -> Result<Self, WorkspaceLoadError> {
        let jj_dir = workspace_root.join(".jj");
        if !jj_dir.is_dir() {
            return Err(WorkspaceLoadError::NoWorkspaceHere(
                workspace_root.to_owned(),
            ));
        }
        let mut repo_dir = jj_dir.join("repo");
        // If .jj/repo is a file, then we interpret its contents as a relative path to
        // the actual repo directory (typically in another workspace).
        if repo_dir.is_file() {
            let buf = fs::read(&repo_dir).context(&repo_dir)?;
            let repo_path = String::from_utf8(buf)
                .map(Utf8PathBuf::from)
                .map_err(|_| WorkspaceLoadError::NonUnicodePath)?;
            repo_dir = canonicalize_path(jj_dir.join(&repo_path)).context(&repo_path)?;
            if !repo_dir.is_dir() {
                return Err(WorkspaceLoadError::RepoDoesNotExist(repo_dir));
            }
        }
        let working_copy_state_path = jj_dir.join("working_copy");
        Ok(Self {
            workspace_root: workspace_root.to_owned(),
            repo_path: repo_dir,
            working_copy_state_path,
        })
    }
}

impl WorkspaceLoader for DefaultWorkspaceLoader {
    fn workspace_root(&self) -> &Utf8Path {
        &self.workspace_root
    }

    fn repo_path(&self) -> &Utf8Path {
        &self.repo_path
    }

    fn load(
        &self,
        user_settings: &UserSettings,
        store_factories: &StoreFactories,
        working_copy_factories: &WorkingCopyFactories,
    ) -> Result<Workspace, WorkspaceLoadError> {
        let repo_loader =
            RepoLoader::init_from_file_system(user_settings, &self.repo_path, store_factories)?;
        let working_copy_factory = get_working_copy_factory(self, working_copy_factories)?;
        let working_copy = working_copy_factory.load_working_copy(
            repo_loader.store().clone(),
            self.workspace_root.clone(),
            self.working_copy_state_path.clone(),
        )?;
        let workspace = Workspace::new(
            &self.workspace_root,
            self.repo_path.clone(),
            working_copy,
            repo_loader,
        )?;
        Ok(workspace)
    }

    fn get_working_copy_type(&self) -> Result<String, StoreLoadError> {
        read_store_type("working copy", self.working_copy_state_path.join("type"))
    }
}

pub fn default_working_copy_factories() -> WorkingCopyFactories {
    let mut factories = WorkingCopyFactories::new();
    factories.insert(
        LocalWorkingCopy::name().to_owned(),
        Box::new(LocalWorkingCopyFactory {}),
    );
    factories
}

pub fn default_working_copy_factory() -> Box<dyn WorkingCopyFactory> {
    Box::new(LocalWorkingCopyFactory {})
}
