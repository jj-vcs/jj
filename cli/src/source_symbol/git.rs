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

use std::io;

use jj_lib::git_backend::GitBackend;
use jj_lib::git_backend::GitRepoAtWorkdirError;
use jj_lib::store::Store;
use jj_lib::ui_path::RepoPathUiConverter;

pub(crate) fn repository(
    store: &Store,
    path_converter: &RepoPathUiConverter,
) -> io::Result<Option<gix::Repository>> {
    let Some(backend) = store.backend_impl::<GitBackend>() else {
        return Ok(None);
    };
    let RepoPathUiConverter::Fs { base, .. } = path_converter;
    let repo = match backend.open_git_repo_at_workdir(base) {
        Ok(repo) => repo,
        Err(GitRepoAtWorkdirError::NotFound { .. } | GitRepoAtWorkdirError::Unrelated { .. }) => {
            backend.git_repo()
        }
        Err(err) => return Err(io::Error::other(err)),
    };
    Ok(Some(repo))
}

pub(crate) fn attributes<'repo>(
    repo: &'repo gix::Repository,
    path_converter: &RepoPathUiConverter,
) -> io::Result<gix::AttributeStack<'repo>> {
    let RepoPathUiConverter::Fs { base, .. } = path_converter;
    let colocated = repo.workdir().is_some_and(|workdir| {
        workdir == base || dunce::canonicalize(base).is_ok_and(|base| workdir == base)
    });
    // Only a colocated workspace shares the Git index. Other JJ workspaces read
    // their own attribute files, independently of the backing Git worktree.
    let index = colocated
        .then(|| repo.index_or_load_from_head_or_empty())
        .transpose()
        .map_err(io::Error::other)?;
    let empty_index = gix::index::State::new(repo.object_hash());
    repo.attributes_only_with_root(
        index.as_ref().map_or(&empty_index, |index| index),
        gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
        base,
    )
    .map_err(io::Error::other)
}
