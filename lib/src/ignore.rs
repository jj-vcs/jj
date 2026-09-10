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

//! Contains the [`Ignore`] trait which determines which files the [`WorkingCopy`] ignores.

use std::any::Any;
use std::fmt::Debug;

use crate::repo_path::RepoPath;

/// The [`Ignore`] trait is used by the [`WorkingCopy`] trait to determine which paths to ignore
/// when interacting with snapshots.
// TODO: support nesting because Git is able to do that.
pub trait Ignore: Send + Sync + Debug {
    /// Returns the name of this implementation.
    fn name(&self) -> &str;

    /// Returns whether the specified file path should be ignored.
    ///
    /// This method does not directly define which files should not be tracked
    /// in the repository. Instead, it performs a simple matching against the
    /// last applicable .gitignore line.
    ///
    /// This only performs exact matching; callers handle recursion of parent
    /// directories. Callers shouldn't recursively match inside ignored
    /// directories, because all (untracked) child files should also be ignored;
    /// the exact matching logic won't give correct results in that case.
    fn matches_file(&self, path: &RepoPath) -> bool;

    /// Returns whether the specified directory path should be ignored.
    fn matches_dir(&self, path: &RepoPath) -> bool;
}

impl dyn Ignore {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: Ignore>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}
