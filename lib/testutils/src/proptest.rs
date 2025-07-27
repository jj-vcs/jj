// Copyright 2025 The Jujutsu Authors
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

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathComponent;
use proptest::prelude::*;
use proptest_derive::Arbitrary;
use proptest_state_machine::ReferenceStateMachine;

use crate::create_tree_with;

fn arb_file_contents() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Empty files represent a significant edge case, so we want to increase the likelihood of
        // empty file contents in subsequent transitions.
        Just(vec![]),
        // [0] is the simplest "binary" file and it's included here to increase the likelihood of
        // identical binary file contents in subsequent transition.
        Just(vec![0]),
        // Diffing is line-oriented, so try to generate files with relatively
        // many newlines.
        "(\n|[a-z]|\\PC)*".prop_map(|s| s.into_bytes()),
        // Arbitrary binary contents, not limited to valid UTF-8.
        proptest::collection::vec(any::<u8>(), 0..32),
    ]
}

#[derive(Arbitrary, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirEntry {
    File {
        #[proptest(strategy = "arb_file_contents()")]
        contents: Vec<u8>,
        executable: bool,
    },

    // TODO: Only files are created for now; extend test to include symlinks.
    #[proptest(skip)]
    Symlink { target: String },

    // TODO: Only files are created for now; extend test to include submodules.
    #[proptest(skip)]
    GitSubmodule { commit: CommitId },
}

fn arb_path_component() -> impl Strategy<Value = String> {
    // HACK: Forbidding `.` here to avoid `.`/`..` in the path components, which
    // causes downstream errors.
    "(a|b|c|d|[\\PC&&[^/.]]+)"
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transition {
    /// Create a new [`DirEntry`] at [`path`](Self::SetDirEntry::path).
    /// - If there is already a file or directory at `path`, it is first
    ///   deleted. (Directories will be recursively deleted.)
    /// - If [`dir_entry`](Self::SetDirEntry::path) is [`None`], the entry at
    ///   `path` is deleted.
    SetDirEntry {
        path: RepoPathBuf,
        dir_entry: Option<DirEntry>,
    },

    /// Commit the current working copy. Used by the system under test.
    Commit,
}

#[derive(Clone, Debug, Default)]
pub struct WorkingCopyReferenceStateMachine {
    entries: BTreeMap<RepoPathBuf, DirEntry>,
}

impl WorkingCopyReferenceStateMachine {
    /// Check invariants that should be maintained by the test code itself
    /// (rather than the library code). If these fail, then the test harness is
    /// buggy.
    fn check_invariants(&self) {
        for file_path in self.entries.keys() {
            for ancestor in file_path.ancestors().skip(1) {
                assert!(
                    !self.entries.contains_key(ancestor),
                    "file {file_path:?} exists, but {ancestor:?} is not a directory"
                );
            }
        }
    }

    pub fn paths(&self) -> impl Iterator<Item = &RepoPath> {
        self.entries.keys().map(AsRef::as_ref)
    }

    pub fn create_tree(&self, repo: &Arc<ReadonlyRepo>) -> MergedTree {
        create_tree_with(repo, |builder| {
            for (path, dir_entry) in &self.entries {
                match dir_entry.clone() {
                    DirEntry::File {
                        contents,
                        executable,
                    } => {
                        builder.file(path, contents).executable(executable);
                    }
                    DirEntry::Symlink { target } => builder.symlink(path, &target),
                    DirEntry::GitSubmodule { commit } => builder.submodule(path, commit),
                }
            }
        })
    }
}

impl WorkingCopyReferenceStateMachine {
    fn arb_extant_directory(&self) -> impl Strategy<Value = RepoPathBuf> + use<> {
        let extant_directories = if self.entries.is_empty() {
            vec![RepoPathBuf::root()]
        } else {
            self.entries
                .keys()
                .flat_map(|file_path| file_path.ancestors().skip(1))
                .map(|path| path.to_owned())
                .unique()
                .collect_vec()
        };

        proptest::sample::select(extant_directories)
    }

    fn arb_extant_path(&self) -> impl Strategy<Value = RepoPathBuf> + use<> {
        proptest::sample::select(
            self.entries
                .keys()
                .flat_map(|file_path| file_path.ancestors())
                .filter(|path| !path.is_root())
                .map(|path| path.to_owned())
                .unique()
                .collect_vec(),
        )
    }

    fn arb_transition_create(&self) -> impl Strategy<Value = Transition> + use<> {
        (
            self.arb_extant_directory(),
            proptest::collection::vec(arb_path_component(), 1..3),
            any::<DirEntry>(),
        )
            .prop_map(|(extant_dir_path, new_path_components, dir_entry)| {
                let mut path = extant_dir_path;
                path.extend(
                    new_path_components
                        .iter()
                        .map(|c| RepoPathComponent::new(c).unwrap()),
                );
                Transition::SetDirEntry {
                    path,
                    dir_entry: Some(dir_entry),
                }
            })
    }

    fn arb_transition_modify(&self) -> impl Strategy<Value = Transition> + use<> {
        (self.arb_extant_path(), any::<Option<DirEntry>>()).prop_map(|(path, new_dir_entry)| {
            Transition::SetDirEntry {
                path,
                dir_entry: new_dir_entry,
            }
        })
    }
}

impl ReferenceStateMachine for WorkingCopyReferenceStateMachine {
    type State = Self;

    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self::State> {
        Just(Self::State::default()).boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        // NOTE: Using `prop_oneof` here instead of `proptest::sample::select`
        // since it seems to minimize better?
        if !state.entries.is_empty() {
            prop_oneof![
                Just(Transition::Commit),
                state.arb_transition_create(),
                state.arb_transition_modify(),
            ]
            .boxed()
        } else {
            prop_oneof![Just(Transition::Commit), state.arb_transition_create()].boxed()
        }
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Commit => {
                // Do nothing; this is handled by the system under test.
            }

            Transition::SetDirEntry { path, dir_entry } => {
                assert_ne!(path.as_ref(), RepoPath::root());
                let entries = &mut state.entries;
                // Remove all entries which are contained within `path` (in case it is a
                // pre-existing directory).
                entries.retain(|extant_path, _| !extant_path.starts_with(path));
                for new_dir in path.ancestors().skip(1) {
                    entries.remove(new_dir);
                }
                match dir_entry {
                    Some(dir_entry) => {
                        entries.insert(path.to_owned(), dir_entry.to_owned());
                    }
                    None => {
                        assert!(!entries.contains_key(path));
                    }
                }
            }
        }
        state.check_invariants();
        state
    }
}
