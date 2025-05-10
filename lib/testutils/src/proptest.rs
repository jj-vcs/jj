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
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::MergedTreeId;
use jj_lib::backend::TreeValue;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::store::Store;
use proptest::prelude::*;
use proptest_derive::Arbitrary;
use proptest_state_machine::ReferenceStateMachine;

use crate::write_file;

fn arb_file_contents() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("".to_string()),
        // Diffing is line-oriented, so try to generate files with relatively
        // many newlines.
        "(\n|[a-z]|.)*".prop_map(|s| s.to_string()),
    ]
}

#[derive(Arbitrary, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirEntry {
    File {
        #[proptest(strategy = "arb_file_contents()")]
        contents: String,
        executable: bool,
    },
    Directory,
}

fn arb_path_component() -> impl Strategy<Value = PathBuf> {
    // HACK: Forbidding `.` here to avoid `.`/`..` in the path components, which
    // causes downstream errors.
    "(a|b|c|d|[\\PC&&[^/.]]+)".prop_map(PathBuf::from)
}

#[derive(Arbitrary, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Transition {
    /// Create a new [`DirEntry`] at [`path`].
    /// - If there is already a file or directory at [`path`], it is first
    ///   deleted. (Directories will be recursively deleted.)
    /// - If [`dir_entry`] is [`None`], the entry at [`path`] is deleted.
    SetDirEntry {
        #[proptest(strategy = "arb_path_component()")]
        path: PathBuf,
        dir_entry: Option<DirEntry>,
    },

    /// Commit the current working copy. Used by the system under test.
    Commit,
}

#[derive(Clone, Debug)]
pub struct WorkingCopyReferenceStateMachine {
    entries: BTreeMap<PathBuf, DirEntry>,
}

impl WorkingCopyReferenceStateMachine {
    fn root_dir() -> &'static Path {
        Path::new("")
    }

    /// Check invariants that should be maintained by the test code itself
    /// (rather than the library code). If these fail, then the test harness is
    /// buggy.
    fn check_invariants(&self) {
        let root_dir_entry = self
            .entries
            .get(Self::root_dir())
            .expect("working copy should always contain root dir");
        assert_eq!(root_dir_entry, &DirEntry::Directory);

        for (path, dir_entry) in &self.entries {
            match dir_entry {
                DirEntry::File { .. } => {
                    let parent_path = path.parent().unwrap();
                    let parent_dir_entry = self
                        .entries
                        .get(parent_path)
                        .expect("file should have a parent directory");
                    assert!(
                        matches!(parent_dir_entry, DirEntry::Directory),
                        "parent of {path:?} is not a directory: {self:?}"
                    );
                }
                DirEntry::Directory => {}
            }
        }
    }

    pub fn write_tree(&self, store: &Arc<Store>) -> BackendResult<MergedTreeId> {
        let mut tree_builder = MergedTreeBuilder::new(store.empty_merged_tree_id());
        for (path, dir_entry) in self.entries.iter() {
            match dir_entry {
                DirEntry::Directory => {
                    // Do nothing, as we currently don't represent empty directories?
                    // TODO: Or write directly to `test_repo`?
                }

                DirEntry::File {
                    contents,
                    executable,
                } => {
                    let path = RepoPathBuf::from_relative_path(path).unwrap();
                    let id = write_file(store, &path, contents);
                    tree_builder.set_or_remove(
                        path,
                        Merge::resolved(Some(TreeValue::File {
                            id: id.clone(),
                            executable: *executable,
                        })),
                    );
                }
            }
        }
        tree_builder.write_tree(store)
    }
}

impl Default for WorkingCopyReferenceStateMachine {
    fn default() -> Self {
        let mut entries = BTreeMap::new();
        entries.insert(Self::root_dir().to_owned(), DirEntry::Directory);
        Self { entries }
    }
}

impl WorkingCopyReferenceStateMachine {
    fn arb_extant_dir_entry(
        &self,
        include_root_dir: bool,
    ) -> impl Strategy<Value = (PathBuf, DirEntry)> {
        proptest::sample::select(
            self.entries
                .iter()
                .filter(|(path, _)| {
                    // NOTE: Avoid using `prop_filter` because it will reject
                    // often, which will cause the entire test to fail.
                    if include_root_dir {
                        true
                    } else {
                        *path != Self::root_dir()
                    }
                })
                .map(|(path, file)| (path.clone(), file.clone()))
                .collect_vec(),
        )
    }

    fn arb_transition_create(&self) -> impl Strategy<Value = Transition> {
        (
            self.arb_extant_dir_entry(true),
            arb_path_component(),
            any::<DirEntry>(),
        )
            .prop_map(|(extant_dir_entry, basename, dir_entry)| {
                let (extant_dir_path, _extant_dir_entry) = extant_dir_entry;
                let path = extant_dir_path.join(basename);
                Transition::SetDirEntry {
                    path,
                    dir_entry: Some(dir_entry),
                }
            })
    }

    fn arb_transition_modify(&self) -> impl Strategy<Value = Transition> {
        (self.arb_extant_dir_entry(false), any::<Option<DirEntry>>()).prop_map(
            |(extant_dir_entry, new_dir_entry)| {
                let (path, _extant_dir_entry) = extant_dir_entry;
                Transition::SetDirEntry {
                    path,
                    dir_entry: new_dir_entry,
                }
            },
        )
    }

    fn arb_transition(&self) -> impl Strategy<Value = Transition> {
        // NOTE: Using `prop_oneof` here instead of `proptest::sample::select`
        // since it seems to minimize better?
        if self.entries.len() > [Self::root_dir()].len() {
            prop_oneof![
                Just(Transition::Commit),
                self.arb_transition_create(),
                self.arb_transition_modify()
            ]
            .boxed()
        } else {
            prop_oneof![Just(Transition::Commit), self.arb_transition_create()].boxed()
        }
    }
}

impl ReferenceStateMachine for WorkingCopyReferenceStateMachine {
    type State = Self;

    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self::State> {
        Just(Self::State::default()).boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        state.check_invariants();
        state.arb_transition().boxed()
    }

    fn apply(state: Self::State, transition: &Self::Transition) -> Self::State {
        let state = match transition {
            Transition::Commit => {
                // Do nothing; this is handled by the system under test.
                state
            }

            Transition::SetDirEntry { path, dir_entry } => {
                assert_ne!(path, Self::root_dir());
                let Self { entries } = state;
                let mut entries: BTreeMap<_, _> = entries
                    .into_iter()
                    .filter(|(extant_path, _)| !extant_path.starts_with(path))
                    .collect();
                match dir_entry {
                    Some(dir_entry) => {
                        for parent_path in path.ancestors() {
                            entries.insert(parent_path.to_owned(), DirEntry::Directory);
                        }
                        entries.insert(path.to_owned(), dir_entry.to_owned());
                    }
                    None => {
                        assert!(!entries.contains_key(path));
                    }
                }
                Self { entries }
            }
        };
        state.check_invariants();
        state
    }
}
