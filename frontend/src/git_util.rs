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

//! Git-related helpers.

use jj_lib::git;
use jj_lib::git_backend::GitRepoAtWorkdirError;
use jj_lib::workspace::Workspace;

/// Returns whether the working copy of `workspace` is shared with a Git
/// repository.
pub fn is_colocated_git_workspace(workspace: &Workspace) -> Result<bool, GitRepoAtWorkdirError> {
    let Ok(git_backend) = git::get_git_backend(workspace.repo_loader().store()) else {
        return Ok(false);
    };
    match git_backend.open_git_repo_at_workdir(workspace.workspace_root()) {
        Ok(_) => Ok(true),
        Err(GitRepoAtWorkdirError::NotFound { .. }) => Ok(false),
        Err(GitRepoAtWorkdirError::Unrelated { .. }) => Ok(false),
        Err(err @ GitRepoAtWorkdirError::Other(_)) => Err(err),
    }
}
