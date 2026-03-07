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

//! Contains the `WorkspaceOperationRunner` which is a simple wrapper around a
//! `Workspace`, `WorkspaceEnvironment` and a `ReadonlyUserRepo`.

use crate::readonly_user_repo::ReadonlyUserRepo;
use crate::workspace::Workspace;
use crate::workspace_util::WorkspaceEnvironment;

/// TODO: A `WorkspaceOperationRunner is ...?
pub struct WorkspaceOperationRunner {
    /// The `WorkspaceEnvironment` associated with this runner.
    // TODO: add and make private
    pub env: WorkspaceEnvironment,
    /// The `Workspace` we're currently operating on.
    // TODO: make private
    pub workspace: Workspace,
    /// The `ReadonlyUserRepo` which we're currently operating on.
    // TODO: make private
    pub user_repo: ReadonlyUserRepo,
}
