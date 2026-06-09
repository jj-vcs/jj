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

//! Contains the [`Matcher`] trait which is used for Filesystem traversal.

use std::collections::HashSet;
use std::fmt::Debug;

use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathComponentBuf;

/// Describes how to traverse a Filesystem or Tree.
#[derive(PartialEq, Eq, Debug)]
pub enum Visit {
    /// Everything in the directory is *guaranteed* to match, no need to check
    /// descendants
    AllRecursively,
    /// Visit only the specified directories or files.
    Specific {
        /// Visit these specific directories.
        dirs: VisitDirs,
        /// Visit these specific files.
        files: VisitFiles,
    },
    /// Nothing in the directory or its subdirectories will match.
    ///
    /// This is the same as `Specific` with no directories or files. Use
    /// `Visit::set()` to get create an instance that's `Specific` or
    /// `Nothing` depending on the values at runtime.
    Nothing,
}

impl Visit {
    /// All entries in the directory need to be visited, but they are not
    /// guaranteed to match.
    pub const SOME: Self = Self::Specific {
        dirs: VisitDirs::All,
        files: VisitFiles::All,
    };

    /// Visit these sets of `dirs` and `files`.
    pub fn sets(dirs: HashSet<RepoPathComponentBuf>, files: HashSet<RepoPathComponentBuf>) -> Self {
        if dirs.is_empty() && files.is_empty() {
            Self::Nothing
        } else {
            Self::Specific {
                dirs: VisitDirs::Set(dirs),
                files: VisitFiles::Set(files),
            }
        }
    }

    /// Returns true if nothing is matched.
    pub fn is_nothing(&self) -> bool {
        *self == Self::Nothing
    }
}

/// Visit all or some specific directories.
#[derive(PartialEq, Eq, Debug)]
pub enum VisitDirs {
    /// Visit all possible directories.
    All,
    /// Visit the specified set of directories.
    Set(HashSet<RepoPathComponentBuf>),
}

/// Visit all or some specific files.
#[derive(PartialEq, Eq, Debug)]
pub enum VisitFiles {
    /// Visit all possible files.
    All,
    /// Visit the specified set of files.
    Set(HashSet<RepoPathComponentBuf>),
}

/// `Matcher`'s are used to specify how the snapshotting path traverses
/// directories and files.
pub trait Matcher: Debug + Send + Sync {
    /// Returns true if the `file` matches the traversal.
    fn matches(&self, file: &RepoPath) -> bool;
    /// Returns a `Visit` which specifies how further traversal should commence.
    fn visit(&self, dir: &RepoPath) -> Visit;
}

impl<T: Matcher + ?Sized> Matcher for &T {
    fn matches(&self, file: &RepoPath) -> bool {
        <T as Matcher>::matches(self, file)
    }

    fn visit(&self, dir: &RepoPath) -> Visit {
        <T as Matcher>::visit(self, dir)
    }
}

impl<T: Matcher + ?Sized> Matcher for Box<T> {
    fn matches(&self, file: &RepoPath) -> bool {
        <T as Matcher>::matches(self, file)
    }

    fn visit(&self, dir: &RepoPath) -> Visit {
        <T as Matcher>::visit(self, dir)
    }
}

/// Match no Path and don't recursively visit any subtree.
// This is a layering violation, since jj-core should just contain traits.
#[derive(PartialEq, Eq, Debug)]
pub struct NothingMatcher;

impl Matcher for NothingMatcher {
    fn matches(&self, _file: &RepoPath) -> bool {
        false
    }

    fn visit(&self, _dir: &RepoPath) -> Visit {
        Visit::Nothing
    }
}

/// Match every Path and recursively visit any subtree.
// This is a layering violation, since jj-core should just contain traits.
#[derive(PartialEq, Eq, Debug)]
pub struct EverythingMatcher;

impl Matcher for EverythingMatcher {
    fn matches(&self, _file: &RepoPath) -> bool {
        true
    }

    fn visit(&self, _dir: &RepoPath) -> Visit {
        Visit::AllRecursively
    }
}
