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

//! A model of the contents of a working copy for property-based tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use hegel::TestCase;
use hegel::generators;
use hegel::generators::Generator as _;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathComponent;

use crate::create_tree_with;

fn draw_file_contents(tc: &TestCase) -> Vec<u8> {
    tc.draw(hegel::one_of!(
        // Empty files represent a significant edge case, so we want to increase the likelihood of
        // empty file contents in subsequent transitions.
        generators::just(Vec::new()),
        // [0] is the simplest "binary" file and it's included here to increase the likelihood of
        // identical binary file contents in subsequent transition.
        generators::just(vec![0_u8]),
        // Diffing is line-oriented, so try to generate files with relatively
        // many newlines.
        generators::vecs(hegel::one_of!(
            generators::just('\n'),
            generators::characters()
                .min_codepoint('a' as u32)
                .max_codepoint('z' as u32),
            generators::characters().exclude_categories(&["Cc", "Cf", "Cs", "Co", "Cn"]),
        ))
        .map(|chars| chars.into_iter().collect::<String>().into_bytes()),
        // Arbitrary binary contents, not limited to valid UTF-8.
        generators::binary().max_size(31),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirEntry {
    File { contents: Vec<u8>, executable: bool },

    // TODO: Only files are created for now; extend test to include symlinks.
    Symlink { target: String },

    // TODO: Only files are created for now; extend test to include submodules.
    GitSubmodule { commit: CommitId },
}

fn draw_dir_entry(tc: &TestCase) -> DirEntry {
    DirEntry::File {
        contents: draw_file_contents(tc),
        executable: tc.draw(generators::booleans()),
    }
}

fn draw_path_component(tc: &TestCase) -> String {
    // HACK: Forbidding `.` here to avoid `.`/`..` in the path components, which
    // causes downstream errors.
    tc.draw(hegel::one_of!(
        generators::sampled_from(vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
        ]),
        generators::text()
            .min_size(1)
            .exclude_categories(&["Cc", "Cf", "Cs", "Co", "Cn"])
            .exclude_characters("/."),
    ))
}

/// Create a new [`DirEntry`] at [`path`](Self::path).
/// - If there is already a file or directory at `path`, it is first deleted.
///   (Directories will be recursively deleted.)
/// - If [`dir_entry`](Self::dir_entry) is [`None`], the entry at `path` is
///   deleted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SetDirEntry {
    pub path: RepoPathBuf,
    pub dir_entry: Option<DirEntry>,
}

/// Model of the contents of a working copy: a map from file paths to the
/// [`DirEntry`] at that path. Directories exist only implicitly as ancestors
/// of the entries.
#[derive(Clone, Debug, Default)]
pub struct WorkingCopyModel {
    entries: BTreeMap<RepoPathBuf, DirEntry>,
}

impl WorkingCopyModel {
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

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

    fn extant_directories(&self) -> Vec<RepoPathBuf> {
        if self.entries.is_empty() {
            vec![RepoPathBuf::root()]
        } else {
            self.entries
                .keys()
                .flat_map(|file_path| file_path.ancestors().skip(1))
                .map(|path| path.to_owned())
                .unique()
                .collect_vec()
        }
    }

    fn extant_paths(&self) -> Vec<RepoPathBuf> {
        self.entries
            .keys()
            .flat_map(|file_path| file_path.ancestors())
            .filter(|path| !path.is_root())
            .map(|path| path.to_owned())
            .unique()
            .collect_vec()
    }

    /// Draw a transition creating a new [`DirEntry`] at a path nested under
    /// some extant directory.
    pub fn draw_create_transition(&self, tc: &TestCase) -> SetDirEntry {
        let mut path =
            tc.draw(generators::sampled_from(self.extant_directories()).print_as_debug());
        let num_components = tc.draw(generators::integers::<usize>().min_value(1).max_value(2));
        for _ in 0..num_components {
            let component = draw_path_component(tc);
            path.extend([RepoPathComponent::new(&component).unwrap()]);
        }
        SetDirEntry {
            path,
            dir_entry: Some(draw_dir_entry(tc)),
        }
    }

    /// Draw a transition modifying or deleting an extant path (either a file
    /// or a directory). Callers must ensure the model is not empty.
    pub fn draw_modify_transition(&self, tc: &TestCase) -> SetDirEntry {
        let path = tc.draw(generators::sampled_from(self.extant_paths()).print_as_debug());
        let dir_entry = if tc.draw(generators::booleans()) {
            Some(draw_dir_entry(tc))
        } else {
            None
        };
        SetDirEntry { path, dir_entry }
    }

    pub fn apply(&mut self, transition: &SetDirEntry) {
        let SetDirEntry { path, dir_entry } = transition;
        assert_ne!(path.as_ref(), RepoPath::root());
        let entries = &mut self.entries;
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
        self.check_invariants();
    }
}
