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

//! Provides the default backend factories and initializers, i.e. `GitCommitStore`
//! (if the `git` feature is enabled), `SimpleOpStore`, `LocalWorkingCopy`, etc.

use jj_default_index::DefaultIndexStore;
#[cfg(feature = "testing")]
use jj_git_backend::secret_backend::SecretCommitStore;
use jj_simple_backend::simple_backend::SimpleCommitStore;
use jj_simple_backend::simple_op_heads_store::SimpleOpHeadsStore;
use jj_simple_backend::simple_op_store::SimpleOpStore;

use crate::default_submodule_store::DefaultSubmoduleStore;
use crate::local_working_copy::LocalWorkingCopy;
use crate::local_working_copy::LocalWorkingCopyFactory;
use crate::simple_workspace_store::SimpleWorkspaceStore;
use crate::store_factories::CommitStoreInitializer;
use crate::store_factories::IndexStoreInitializer;
use crate::store_factories::OpHeadsStoreInitializer;
use crate::store_factories::OpStoreInitializer;
use crate::store_factories::StoreFactories;
use crate::store_factories::StoreInitializers;
use crate::store_factories::SubmoduleStoreInitializer;
use crate::store_factories::WorkspaceStoreInitializer;
use crate::working_copy::WorkingCopyFactory;
use crate::workspace::DefaultWorkspaceLoaderFactory;
use crate::workspace::WorkingCopyFactories;
use crate::workspace::WorkspaceLoaderFactory;

/// Returns default store factories.
pub fn default_commit_store_factories() -> StoreFactories {
    let mut factories = StoreFactories::empty();

    // Backends
    factories.add_commit_store(
        SimpleCommitStore::NAME,
        Box::new(|_settings, store_path| Ok(Box::new(SimpleCommitStore::load(store_path)))),
    );
    #[cfg(feature = "git")]
    factories.add_commit_store(
        jj_git_backend::git_backend::GitCommitStore::NAME,
        Box::new(|settings, store_path| {
            let git_settings = settings
                .git_backend_settings()
                .map_err(jj_core::commit_store::CommitStoreLoadError::new)?;
            Ok(Box::new(jj_git_backend::git_backend::GitCommitStore::load(
                &git_settings,
                store_path,
            )?))
        }),
    );
    #[cfg(feature = "testing")]
    factories.add_commit_store(
        SecretCommitStore::NAME,
        Box::new(|settings, store_path| {
            let git_settings = settings
                .git_backend_settings()
                .map_err(jj_core::commit_store::CommitStoreLoadError::new)?;
            Ok(Box::new(SecretCommitStore::load(
                &git_settings,
                store_path,
            )?))
        }),
    );

    // WorkspaceStores
    factories.add_workspace_store(
        SimpleWorkspaceStore::NAME,
        Box::new(|_settings, store_path| Ok(Box::new(SimpleWorkspaceStore::load(store_path)?))),
    );

    // OpStores
    factories.add_op_store(
        SimpleOpStore::NAME,
        Box::new(|_settings, store_path, root_data| {
            Ok(Box::new(SimpleOpStore::load(store_path, root_data)))
        }),
    );

    // OpHeadsStores
    factories.add_op_heads_store(
        SimpleOpHeadsStore::NAME,
        Box::new(|_settings, store_path| Ok(Box::new(SimpleOpHeadsStore::load(store_path)))),
    );

    // Index
    factories.add_index_store(
        DefaultIndexStore::NAME,
        Box::new(|_settings, store_path| Ok(Box::new(DefaultIndexStore::load(store_path)))),
    );

    // SubmoduleStores
    factories.add_submodule_store(
        DefaultSubmoduleStore::NAME,
        Box::new(|_settings, store_path| Ok(Box::new(DefaultSubmoduleStore::load(store_path)))),
    );

    factories
}

/// Returns the default store initializers.
pub fn default_store_initializers() -> StoreInitializers<'static> {
    StoreInitializers {
        commit_store: default_commit_store_initializer(),
        workspace_store: default_workspace_store_initializer(),
        op_store: default_op_store_initializer(),
        op_heads_store: default_op_heads_store_initializer(),
        index_store: default_index_store_initializer(),
        submodule_store: default_submodule_store_initializer(),
    }
}

/// Returns the initializer for the default backend.
pub fn default_commit_store_initializer() -> &'static CommitStoreInitializer<'static> {
    &|_settings, store_path| Ok(Box::new(SimpleCommitStore::init(store_path)))
}

#[cfg(feature = "git")]
pub use git::git_commit_store_initializer;

/// Returns the initializer for the default workspace store.
pub fn default_workspace_store_initializer() -> &'static WorkspaceStoreInitializer<'static> {
    &|_settings, store_path| Ok(Box::new(SimpleWorkspaceStore::init(store_path)?))
}

/// Returns the initializer for the default operation store.
pub fn default_op_store_initializer() -> &'static OpStoreInitializer<'static> {
    &|_settings, store_path, root_data| Ok(Box::new(SimpleOpStore::init(store_path, root_data)?))
}

/// Returns the initializer for the default operation heads store.
pub fn default_op_heads_store_initializer() -> &'static OpHeadsStoreInitializer<'static> {
    &|_settings, store_path, root_op_id| {
        Ok(Box::new(SimpleOpHeadsStore::init(store_path, root_op_id)?))
    }
}

/// Returns the initializer for the default index store.
pub fn default_index_store_initializer() -> &'static IndexStoreInitializer<'static> {
    &|_settings, store_path| Ok(Box::new(DefaultIndexStore::init(store_path)?))
}

/// Returns the initializer for the default submodule store.
pub fn default_submodule_store_initializer() -> &'static SubmoduleStoreInitializer<'static> {
    &|_settings, store_path| Ok(Box::new(DefaultSubmoduleStore::init(store_path)))
}

/// Returns default working copy factories.
pub fn default_working_copy_factories() -> WorkingCopyFactories {
    let mut factories = WorkingCopyFactories::new();
    factories.insert(
        LocalWorkingCopy::NAME.to_owned(),
        Box::new(LocalWorkingCopyFactory {}),
    );
    factories
}

/// Returns the default (local-disk) working copy factory.
pub fn default_working_copy_factory() -> Box<dyn WorkingCopyFactory> {
    Box::new(LocalWorkingCopyFactory {})
}

/// Returns the default workspace loader factory.
pub fn default_workspace_loader_factory() -> Box<dyn WorkspaceLoaderFactory> {
    Box::new(DefaultWorkspaceLoaderFactory)
}

#[cfg(feature = "git")]
mod git {
    use std::path::Path;

    use jj_core::commit_store::CommitStore;
    use jj_core::commit_store::CommitStoreInitError;
    use jj_git_backend::git_backend::GitBackendInitError;
    use jj_git_backend::git_backend::GitBackendSettings;
    use jj_git_backend::git_backend::GitCommitStore;

    use crate::settings::UserSettings;

    /// Returns a backend initializer that creates a `GitCommitStore` by calling `init`
    /// with the Git backend settings from the user settings.
    pub fn git_commit_store_initializer<'a, F>(
        init: F,
    ) -> impl Fn(&UserSettings, &Path) -> Result<Box<dyn CommitStore>, CommitStoreInitError> + 'a
    where
        F: Fn(&GitBackendSettings, &Path) -> Result<GitCommitStore, Box<GitBackendInitError>> + 'a,
    {
        move |settings, store_path| {
            let git_settings = settings
                .git_backend_settings()
                .map_err(CommitStoreInitError::new)?;
            Ok(Box::new(init(&git_settings, store_path)?))
        }
    }
}
