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

//! Garbage collection related utilities for the CLI.

use std::slice;
use std::time::SystemTime;

use jj_lib::default_index::DefaultIndexStore;
use jj_lib::default_index::DefaultReadonlyIndex;
use jj_lib::operation::Operation;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo::RepoLoader;

use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;

/// Prunes unreachable operations and objects older than `keep_newer`.
pub async fn expire_unreachable(
    repo: &ReadonlyRepo,
    keep_newer: SystemTime,
) -> Result<(), CommandError> {
    repo.op_store()
        .gc(slice::from_ref(repo.op_id()), keep_newer)
        .await
        .map_err(|err| user_error(err.to_string()))?;

    repo.store()
        .gc(repo.index(), keep_newer)
        .map_err(|err| user_error(err.to_string()))?;

    Ok(())
}

pub async fn reindex_at_operation(
    repo_loader: &RepoLoader,
    op: &Operation,
) -> Result<DefaultReadonlyIndex, CommandError> {
    let index_store = repo_loader.index_store();

    if let Some(default_index_store) = index_store.downcast_ref::<DefaultIndexStore>() {
        default_index_store.reinit().map_err(internal_error)?;

        let default_index = default_index_store
            .build_index_at_operation(op, repo_loader.store())
            .await
            .map_err(internal_error)?;

        Ok(default_index)
    } else {
        Err(user_error(format!(
            "Cannot reindex indexes of type '{}'",
            index_store.name()
        )))
    }
}
