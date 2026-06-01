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

//! Provides the default backend factories, i.e. `GitBackend` (if `git` feature
//! enabled), `SimpleOpStore`, `LocalWorkingCopy`, etc.

use crate::default_index::DefaultIndexStore;
use crate::default_submodule_store::DefaultSubmoduleStore;
use crate::local_working_copy::LocalWorkingCopy;
use crate::local_working_copy::LocalWorkingCopyFactory;
use crate::repo::StoreFactories;
use crate::simple_backend::SimpleBackend;
use crate::simple_op_heads_store::SimpleOpHeadsStore;
use crate::simple_op_store::SimpleOpStore;
use crate::working_copy::WorkingCopyFactory;
use crate::workspace::WorkingCopyFactories;

/// Returns default store factories.
pub fn default_backend_factories() -> StoreFactories {
    let mut factories = StoreFactories::empty();

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

/// Returns default working copy factories.
pub fn default_working_copy_factories() -> WorkingCopyFactories {
    let mut factories = WorkingCopyFactories::new();
    factories.insert(
        LocalWorkingCopy::name().to_owned(),
        Box::new(LocalWorkingCopyFactory {}),
    );
    factories
}

/// Returns the default (local-disk) working copy factory.
pub fn default_working_copy_factory() -> Box<dyn WorkingCopyFactory> {
    Box::new(LocalWorkingCopyFactory {})
}
