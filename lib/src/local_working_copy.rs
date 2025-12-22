// Copyright 2020 The Jujutsu Authors
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

#![expect(missing_docs)]

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::fs::DirEntry;
use std::fs::File;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::io;
use std::io::Read as _;
use std::io::Write as _;
use std::iter;
use std::mem;
use std::ops::Range;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::slice;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use either::Either;
use futures::StreamExt as _;
use itertools::EitherOrBoth;
use itertools::Itertools as _;
use once_cell::unsync::OnceCell;
use pollster::FutureExt as _;
use prost::Message as _;
use rayon::iter::IntoParallelIterator as _;
use rayon::prelude::IndexedParallelIterator as _;
use rayon::prelude::ParallelIterator as _;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;
use tracing::instrument;
use tracing::trace_span;

use crate::backend::BackendError;
use crate::backend::CopyId;
use crate::backend::FileId;
use crate::backend::MillisSinceEpoch;
use crate::backend::SymlinkId;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::commit::Commit;
use crate::config::ConfigGetError;
use crate::conflict_labels::ConflictLabels;
use crate::conflicts;
use crate::conflicts::ConflictMarkerStyle;
use crate::conflicts::ConflictMaterializeOptions;
use crate::conflicts::MIN_CONFLICT_MARKER_LEN;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::choose_materialized_conflict_marker_len;
use crate::conflicts::materialize_merge_result_to_bytes;
use crate::conflicts::materialize_tree_value;
pub use crate::eol::EolConversionMode;
use crate::eol::TargetEolStrategy;
use crate::file_util::BlockingAsyncReader;
use crate::file_util::check_symlink_support;
use crate::file_util::copy_async_to_sync;
use crate::file_util::persist_temp_file;
use crate::file_util::try_symlink;
use crate::fsmonitor::FsmonitorSettings;
#[cfg(feature = "watchman")]
use crate::fsmonitor::WatchmanConfig;
#[cfg(feature = "watchman")]
use crate::fsmonitor::watchman;
use crate::gitignore::GitIgnoreFile;
use crate::lock::FileLock;
use crate::matchers::DifferenceMatcher;
use crate::matchers::EverythingMatcher;
use crate::matchers::FilesMatcher;
use crate::matchers::IntersectionMatcher;
use crate::matchers::Matcher;
use crate::matchers::PrefixMatcher;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::merge::MergedTreeValue;
use crate::merge::SameChange;
use crate::merged_tree::MergedTree;
use crate::merged_tree::MergedTreeBuilder;
use crate::merged_tree::TreeDiffEntry;
use crate::object_id::ObjectId as _;
use crate::op_store::OperationId;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::settings::UserSettings;
use crate::store::Store;
use crate::working_copy::CheckoutError;
use crate::working_copy::CheckoutStats;
use crate::working_copy::LockedWorkingCopy;
use crate::working_copy::ResetError;
use crate::working_copy::SnapshotError;
use crate::working_copy::SnapshotOptions;
use crate::working_copy::SnapshotProgress;
use crate::working_copy::SnapshotStats;
use crate::working_copy::UntrackedReason;
use crate::working_copy::WorkingCopy;
use crate::working_copy::WorkingCopyFactory;
use crate::working_copy::WorkingCopyStateError;

fn symlink_target_convert_to_store(path: &Path) -> Option<Cow<'_, str>> {
    let path = path.to_str()?;
    if std::path::MAIN_SEPARATOR == '/' {
        Some(Cow::Borrowed(path))
    } else {
        // When storing the symlink target on Windows, convert "\" to "/", so that the
        // symlink remains valid on Unix.
        //
        // Note that we don't use std::path to handle the conversion, because it
        // performs poorly with Windows verbatim paths like \\?\Global\C:\file.txt.
        Some(Cow::Owned(path.replace(std::path::MAIN_SEPARATOR_STR, "/")))
    }
}

fn symlink_target_convert_to_disk(path: &str) -> PathBuf {
    let path = if std::path::MAIN_SEPARATOR == '/' {
        Cow::Borrowed(path)
    } else {
        // Use the main separator to reformat the input path to avoid creating a broken
        // symlink with the incorrect separator "/".
        //
        // See https://github.com/jj-vcs/jj/issues/6934 for the relevant bug.
        Cow::Owned(path.replace("/", std::path::MAIN_SEPARATOR_STR))
    };
    PathBuf::from(path.as_ref())
}

/// How to propagate executable bit changes in file metadata to/from the repo.
///
/// On Windows, executable bits are always ignored, but on Unix they are
/// respected by default, but may be ignored by user settings or if we find
/// that the filesystem of the working copy doesn't support executable bits.
#[derive(Clone, Copy, Debug)]
enum ExecChangePolicy {
    Ignore,
    #[cfg_attr(windows, expect(dead_code))]
    Respect,
}

/// The executable bit change setting as exposed to the user.
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecChangeSetting {
    Ignore,
    Respect,
    #[default]
    Auto,
}

impl ExecChangePolicy {
    /// Get the executable bit policy based on user settings and executable bit
    /// support in the working copy's state path.
    ///
    /// On Unix we check whether executable bits are supported in the working
    /// copy to determine respect/ignorance, but we default to respect.
    #[cfg_attr(windows, expect(unused_variables))]
    fn new(exec_change_setting: ExecChangeSetting, state_path: &Path) -> Self {
        #[cfg(windows)]
        return Self::Ignore;
        #[cfg(unix)]
        return match exec_change_setting {
            ExecChangeSetting::Ignore => Self::Ignore,
            ExecChangeSetting::Respect => Self::Respect,
            ExecChangeSetting::Auto => {
                match crate::file_util::check_executable_bit_support(state_path) {
                    Ok(false) => Self::Ignore,
                    Ok(true) => Self::Respect,
                    Err(err) => {
                        tracing::warn!(?err, "Error when checking for executable bit support");
                        Self::Respect
                    }
                }
            }
        };
    }
}

/// On-disk state of file executable as cached in the file states. This does
/// *not* necessarily equal the `executable` field of [`TreeValue::File`]: the
/// two are allowed to diverge if and only if we're ignoring executable bit
/// changes.
///
/// This will only ever be true on Windows if the repo is also being accessed
/// from a Unix version of jj, such as when accessed from WSL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecBit(bool);

impl ExecBit {
    /// Get the executable bit for a tree value to write to the repo store.
    ///
    /// If we're ignoring the executable bit, then we fallback to the previous
    /// in-repo executable bit if present.
    fn for_tree_value(
        self,
        exec_policy: ExecChangePolicy,
        prev_in_repo: impl FnOnce() -> Option<bool>,
    ) -> bool {
        match exec_policy {
            ExecChangePolicy::Ignore => prev_in_repo().unwrap_or(false),
            ExecChangePolicy::Respect => self.0,
        }
    }

    /// Set the on-disk executable bit to be written based on the in-repo bit or
    /// the previous on-disk executable bit.
    ///
    /// On Windows, we return `false` because when we later write files, we
    /// always create them anew, and the executable bit will be `false` even if
    /// shared with a Unix machine.
    ///
    /// `prev_on_disk` is a closure because it is somewhat expensive and is only
    /// used if ignoring the executable bit on Unix.
    fn new_from_repo(
        in_repo: bool,
        exec_policy: ExecChangePolicy,
        prev_on_disk: impl FnOnce() -> Option<Self>,
    ) -> Self {
        match exec_policy {
            _ if cfg!(windows) => Self(false),
            ExecChangePolicy::Ignore => prev_on_disk().unwrap_or(Self(false)),
            ExecChangePolicy::Respect => Self(in_repo),
        }
    }

    /// Load the on-disk executable bit from file metadata.
    #[cfg_attr(windows, expect(unused_variables))]
    fn new_from_disk(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        return Self(metadata.permissions().mode() & 0o111 != 0);
        #[cfg(windows)]
        return Self(false);
    }
}

/// Set the executable bit of a file on-disk. This is a no-op on Windows.
///
/// On Unix, we manually set the executable bit to the previous value on-disk.
/// This is necessary because we write all files by creating them new, so files
/// won't preserve their permissions naturally.
#[cfg_attr(windows, expect(unused_variables))]
fn set_executable(exec_bit: ExecBit, disk_path: &Path) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        let mode = if exec_bit.0 { 0o755 } else { 0o644 };
        fs::set_permissions(disk_path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FileType {
    Normal { exec_bit: ExecBit },
    Symlink,
    GitSubmodule,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MaterializedConflictData {
    pub conflict_marker_len: u32,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FileState {
    pub file_type: FileType,
    pub mtime: MillisSinceEpoch,
    pub size: u64,
    pub materialized_conflict_data: Option<MaterializedConflictData>,
    /* TODO: What else do we need here? Git stores a lot of fields.
     * TODO: Could possibly handle case-insensitive file systems keeping an
     *       Option<PathBuf> with the actual path here. */
}

impl FileState {
    /// Check whether a file state appears clean compared to a previous file
    /// state, ignoring materialized conflict data.
    pub fn is_clean(&self, old_file_state: &Self) -> bool {
        self.file_type == old_file_state.file_type
            && self.mtime == old_file_state.mtime
            && self.size == old_file_state.size
    }

    /// Indicates that a file exists in the tree but that it needs to be
    /// re-stat'ed on the next snapshot.
    fn placeholder() -> Self {
        Self {
            file_type: FileType::Normal {
                exec_bit: ExecBit(false),
            },
            mtime: MillisSinceEpoch(0),
            size: 0,
            materialized_conflict_data: None,
        }
    }

    fn for_file(exec_bit: ExecBit, size: u64, metadata: &Metadata) -> Self {
        Self {
            file_type: FileType::Normal { exec_bit },
            mtime: mtime_from_metadata(metadata),
            size,
            materialized_conflict_data: None,
        }
    }

    fn for_symlink(metadata: &Metadata) -> Self {
        // When using fscrypt, the reported size is not the content size. So if
        // we were to record the content size here (like we do for regular files), we
        // would end up thinking the file has changed every time we snapshot.
        Self {
            file_type: FileType::Symlink,
            mtime: mtime_from_metadata(metadata),
            size: metadata.len(),
            materialized_conflict_data: None,
        }
    }

    fn for_gitsubmodule() -> Self {
        Self {
            file_type: FileType::GitSubmodule,
            mtime: MillisSinceEpoch(0),
            size: 0,
            materialized_conflict_data: None,
        }
    }
}

/// Owned map of path to file states, backed by proto data.
#[derive(Clone, Debug)]
struct FileStatesMap {
    data: Vec<crate::protos::local_working_copy::FileStateEntry>,
}

impl FileStatesMap {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn from_proto(
        mut data: Vec<crate::protos::local_working_copy::FileStateEntry>,
        is_sorted: bool,
    ) -> Self {
        if !is_sorted {
            data.sort_unstable_by(|entry1, entry2| {
                let path1 = RepoPath::from_internal_string(&entry1.path).unwrap();
                let path2 = RepoPath::from_internal_string(&entry2.path).unwrap();
                path1.cmp(path2)
            });
        }
        debug_assert!(is_file_state_entries_proto_unique_and_sorted(&data));
        Self { data }
    }

    /// Merges changed and deleted entries into this map. The changed entries
    /// must be sorted by path.
    fn merge_in(
        &mut self,
        changed_file_states: Vec<(RepoPathBuf, FileState)>,
        deleted_files: &HashSet<RepoPathBuf>,
    ) {
        if changed_file_states.is_empty() && deleted_files.is_empty() {
            return;
        }
        debug_assert!(
            changed_file_states.is_sorted_by(|(path1, _), (path2, _)| path1 < path2),
            "changed_file_states must be sorted and have no duplicates"
        );
        self.data = itertools::merge_join_by(
            mem::take(&mut self.data),
            changed_file_states,
            |old_entry, (changed_path, _)| {
                RepoPath::from_internal_string(&old_entry.path)
                    .unwrap()
                    .cmp(changed_path)
            },
        )
        .filter_map(|diff| match diff {
            EitherOrBoth::Both(_, (path, state)) | EitherOrBoth::Right((path, state)) => {
                debug_assert!(!deleted_files.contains(&path));
                Some(file_state_entry_to_proto(path, &state))
            }
            EitherOrBoth::Left(entry) => {
                let present =
                    !deleted_files.contains(RepoPath::from_internal_string(&entry.path).unwrap());
                present.then_some(entry)
            }
        })
        .collect();
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    /// Returns read-only map containing all file states.
    fn all(&self) -> FileStates<'_> {
        FileStates::from_sorted(&self.data)
    }
}

/// Read-only map of path to file states, possibly filtered by path prefix.
#[derive(Clone, Copy, Debug)]
pub struct FileStates<'a> {
    data: &'a [crate::protos::local_working_copy::FileStateEntry],
}

impl<'a> FileStates<'a> {
    fn from_sorted(data: &'a [crate::protos::local_working_copy::FileStateEntry]) -> Self {
        debug_assert!(is_file_state_entries_proto_unique_and_sorted(data));
        Self { data }
    }

    /// Returns file states under the given directory path.
    pub fn prefixed(&self, base: &RepoPath) -> Self {
        let range = self.prefixed_range(base);
        Self::from_sorted(&self.data[range])
    }

    /// Faster version of `prefixed("<dir>/<base>")`. Requires that all entries
    /// share the same prefix `dir`.
    fn prefixed_at(&self, dir: &RepoPath, base: &RepoPathComponent) -> Self {
        let range = self.prefixed_range_at(dir, base);
        Self::from_sorted(&self.data[range])
    }

    /// Returns true if this contains no entries.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns true if the given `path` exists.
    pub fn contains_path(&self, path: &RepoPath) -> bool {
        self.exact_position(path).is_some()
    }

    /// Returns file state for the given `path`.
    pub fn get(&self, path: &RepoPath) -> Option<FileState> {
        let pos = self.exact_position(path)?;
        let (_, state) = file_state_entry_from_proto(&self.data[pos]);
        Some(state)
    }

    /// Returns the executable bit state if `path` is a normal file.
    pub fn get_exec_bit(&self, path: &RepoPath) -> Option<ExecBit> {
        match self.get(path)?.file_type {
            FileType::Normal { exec_bit } => Some(exec_bit),
            FileType::Symlink | FileType::GitSubmodule => None,
        }
    }

    /// Faster version of `get("<dir>/<name>")`. Requires that all entries share
    /// the same prefix `dir`.
    fn get_at(&self, dir: &RepoPath, name: &RepoPathComponent) -> Option<FileState> {
        let pos = self.exact_position_at(dir, name)?;
        let (_, state) = file_state_entry_from_proto(&self.data[pos]);
        Some(state)
    }

    fn exact_position(&self, path: &RepoPath) -> Option<usize> {
        self.data
            .binary_search_by(|entry| {
                RepoPath::from_internal_string(&entry.path)
                    .unwrap()
                    .cmp(path)
            })
            .ok()
    }

    fn exact_position_at(&self, dir: &RepoPath, name: &RepoPathComponent) -> Option<usize> {
        debug_assert!(self.paths().all(|path| path.starts_with(dir)));
        let slash_len = usize::from(!dir.is_root());
        let prefix_len = dir.as_internal_file_string().len() + slash_len;
        self.data
            .binary_search_by(|entry| {
                let tail = entry.path.get(prefix_len..).unwrap_or("");
                match tail.split_once('/') {
                    // "<name>/*" > "<name>"
                    Some((pre, _)) => pre.cmp(name.as_internal_str()).then(Ordering::Greater),
                    None => tail.cmp(name.as_internal_str()),
                }
            })
            .ok()
    }

    fn prefixed_range(&self, base: &RepoPath) -> Range<usize> {
        let start = self
            .data
            .partition_point(|entry| RepoPath::from_internal_string(&entry.path).unwrap() < base);
        let len = self.data[start..].partition_point(|entry| {
            RepoPath::from_internal_string(&entry.path)
                .unwrap()
                .starts_with(base)
        });
        start..(start + len)
    }

    fn prefixed_range_at(&self, dir: &RepoPath, base: &RepoPathComponent) -> Range<usize> {
        debug_assert!(self.paths().all(|path| path.starts_with(dir)));
        let slash_len = usize::from(!dir.is_root());
        let prefix_len = dir.as_internal_file_string().len() + slash_len;
        let start = self.data.partition_point(|entry| {
            let tail = entry.path.get(prefix_len..).unwrap_or("");
            let entry_name = tail.split_once('/').map_or(tail, |(name, _)| name);
            entry_name < base.as_internal_str()
        });
        let len = self.data[start..].partition_point(|entry| {
            let tail = entry.path.get(prefix_len..).unwrap_or("");
            let entry_name = tail.split_once('/').map_or(tail, |(name, _)| name);
            entry_name == base.as_internal_str()
        });
        start..(start + len)
    }

    /// Iterates file state entries sorted by path.
    pub fn iter(&self) -> FileStatesIter<'a> {
        self.data.iter().map(file_state_entry_from_proto)
    }

    /// Iterates sorted file paths.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = &'a RepoPath> + use<'a> {
        self.data
            .iter()
            .map(|entry| RepoPath::from_internal_string(&entry.path).unwrap())
    }
}

type FileStatesIter<'a> = iter::Map<
    slice::Iter<'a, crate::protos::local_working_copy::FileStateEntry>,
    fn(&crate::protos::local_working_copy::FileStateEntry) -> (&RepoPath, FileState),
>;

impl<'a> IntoIterator for FileStates<'a> {
    type Item = (&'a RepoPath, FileState);
    type IntoIter = FileStatesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

fn file_state_from_proto(proto: &crate::protos::local_working_copy::FileState) -> FileState {
    let file_type = match proto.file_type() {
        crate::protos::local_working_copy::FileType::Normal => FileType::Normal {
            exec_bit: ExecBit(false),
        },
        // On Windows, `FileType::Executable` can exist if the repo is being
        // shared with a Unix version of jj, such as when accessed from WSL.
        crate::protos::local_working_copy::FileType::Executable => FileType::Normal {
            exec_bit: ExecBit(true),
        },
        crate::protos::local_working_copy::FileType::Symlink => FileType::Symlink,
        crate::protos::local_working_copy::FileType::Conflict => FileType::Normal {
            exec_bit: ExecBit(false),
        },
        crate::protos::local_working_copy::FileType::GitSubmodule => FileType::GitSubmodule,
    };
    FileState {
        file_type,
        mtime: MillisSinceEpoch(proto.mtime_millis_since_epoch),
        size: proto.size,
        materialized_conflict_data: proto.materialized_conflict_data.as_ref().map(|data| {
            MaterializedConflictData {
                conflict_marker_len: data.conflict_marker_len,
            }
        }),
    }
}

fn file_state_to_proto(file_state: &FileState) -> crate::protos::local_working_copy::FileState {
    let mut proto = crate::protos::local_working_copy::FileState::default();
    let file_type = match &file_state.file_type {
        FileType::Normal { exec_bit } => {
            if exec_bit.0 {
                crate::protos::local_working_copy::FileType::Executable
            } else {
                crate::protos::local_working_copy::FileType::Normal
            }
        }
        FileType::Symlink => crate::protos::local_working_copy::FileType::Symlink,
        FileType::GitSubmodule => crate::protos::local_working_copy::FileType::GitSubmodule,
    };
    proto.file_type = file_type as i32;
    proto.mtime_millis_since_epoch = file_state.mtime.0;
    proto.size = file_state.size;
    proto.materialized_conflict_data = file_state.materialized_conflict_data.map(|data| {
        crate::protos::local_working_copy::MaterializedConflictData {
            conflict_marker_len: data.conflict_marker_len,
        }
    });
    proto
}

fn file_state_entry_from_proto(
    proto: &crate::protos::local_working_copy::FileStateEntry,
) -> (&RepoPath, FileState) {
    let path = RepoPath::from_internal_string(&proto.path).unwrap();
    (path, file_state_from_proto(proto.state.as_ref().unwrap()))
}

fn file_state_entry_to_proto(
    path: RepoPathBuf,
    state: &FileState,
) -> crate::protos::local_working_copy::FileStateEntry {
    crate::protos::local_working_copy::FileStateEntry {
        path: path.into_internal_string(),
        state: Some(file_state_to_proto(state)),
    }
}

fn is_file_state_entries_proto_unique_and_sorted(
    data: &[crate::protos::local_working_copy::FileStateEntry],
) -> bool {
    data.iter()
        .map(|entry| RepoPath::from_internal_string(&entry.path).unwrap())
        .is_sorted_by(|path1, path2| path1 < path2)
}

fn sparse_patterns_from_proto(
    proto: Option<&crate::protos::local_working_copy::SparsePatterns>,
) -> Vec<RepoPathBuf> {
    let mut sparse_patterns = vec![];
    if let Some(proto_sparse_patterns) = proto {
        for prefix in &proto_sparse_patterns.prefixes {
            sparse_patterns.push(RepoPathBuf::from_internal_string(prefix).unwrap());
        }
    } else {
        // For compatibility with old working copies.
        // TODO: Delete this is late 2022 or so.
        sparse_patterns.push(RepoPathBuf::root());
    }
    sparse_patterns
}

/// Creates intermediate directories from the `working_copy_path` to the
/// `repo_path` parent. Returns disk path for the `repo_path` file.
///
/// If an intermediate directory exists and if it is a file or symlink, this
/// function returns `Ok(None)` to signal that the path should be skipped.
/// The `working_copy_path` directory may be a symlink.
///
/// If an existing or newly-created sub directory points to ".git" or ".jj",
/// this function returns an error.
///
/// Note that this does not prevent TOCTOU bugs caused by concurrent checkouts.
/// Another process may remove the directory created by this function and put a
/// symlink there.
fn create_parent_dirs(
    working_copy_path: &Path,
    repo_path: &RepoPath,
) -> Result<Option<PathBuf>, CheckoutError> {
    let (parent_path, basename) = repo_path.split().expect("repo path shouldn't be root");
    let mut dir_path = working_copy_path.to_owned();
    for c in parent_path.components() {
        // Ensure that the name is a normal entry of the current dir_path.
        dir_path.push(c.to_fs_name().map_err(|err| err.with_path(repo_path))?);
        // A directory named ".git" or ".jj" can be temporarily created. It
        // might trick workspace path discovery, but is harmless so long as the
        // directory is empty.
        let new_dir_created = match fs::create_dir(&dir_path) {
            Ok(()) => true, // New directory
            Err(err) => match dir_path.symlink_metadata() {
                Ok(m) if m.is_dir() => false, // Existing directory
                Ok(_) => {
                    return Ok(None); // Skip existing file or symlink
                }
                Err(_) => {
                    return Err(CheckoutError::Other {
                        message: format!(
                            "Failed to create parent directories for {}",
                            repo_path.to_fs_path_unchecked(working_copy_path).display(),
                        ),
                        err: err.into(),
                    });
                }
            },
        };
        // Invalid component (e.g. "..") should have been rejected.
        // The current dir_path should be an entry of dir_path.parent().
        reject_reserved_existing_path(&dir_path).inspect_err(|_| {
            if new_dir_created {
                fs::remove_dir(&dir_path).ok();
            }
        })?;
    }

    let mut file_path = dir_path;
    file_path.push(
        basename
            .to_fs_name()
            .map_err(|err| err.with_path(repo_path))?,
    );
    Ok(Some(file_path))
}

/// Removes existing file named `disk_path` if any. Returns `Ok(true)` if the
/// file was there and got removed, meaning that new file can be safely created.
///
/// If the existing file points to ".git" or ".jj", this function returns an
/// error.
fn remove_old_file(disk_path: &Path) -> Result<bool, CheckoutError> {
    reject_reserved_existing_path(disk_path)?;
    match fs::remove_file(disk_path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        // TODO: Use io::ErrorKind::IsADirectory if it gets stabilized
        Err(_) if disk_path.symlink_metadata().is_ok_and(|m| m.is_dir()) => Ok(false),
        Err(err) => Err(CheckoutError::Other {
            message: format!("Failed to remove file {}", disk_path.display()),
            err: err.into(),
        }),
    }
}

/// Checks if new file or symlink named `disk_path` can be created.
///
/// If the file already exists, this function return `Ok(false)` to signal
/// that the path should be skipped.
///
/// If the path may point to ".git" or ".jj" entry, this function returns an
/// error.
///
/// This function can fail if `disk_path.parent()` isn't a directory.
fn can_create_new_file(disk_path: &Path) -> Result<bool, CheckoutError> {
    // New file or symlink will be created by caller. If it were pointed to by
    // name ".git" or ".jj", git/jj CLI could be tricked to load configuration
    // from an attacker-controlled location. So we first test the path by
    // creating an empty file.
    let new_file = match OpenOptions::new()
        .write(true)
        .create_new(true) // Don't overwrite, don't follow symlink
        .open(disk_path)
    {
        Ok(file) => Some(file),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => None,
        // Workaround for "Access is denied. (os error 5)" error on Windows.
        Err(_) => match disk_path.symlink_metadata() {
            Ok(_) => None,
            Err(err) => {
                return Err(CheckoutError::Other {
                    message: format!("Failed to stat {}", disk_path.display()),
                    err: err.into(),
                });
            }
        },
    };

    let new_file_created = new_file.is_some();

    if let Some(new_file) = new_file {
        reject_reserved_existing_file(new_file, disk_path).inspect_err(|_| {
            // We keep the error from `reject_reserved_existing_file`
            fs::remove_file(disk_path).ok();
        })?;

        fs::remove_file(disk_path).map_err(|err| CheckoutError::Other {
            message: format!("Failed to remove temporary file {}", disk_path.display()),
            err: err.into(),
        })?;
    } else {
        reject_reserved_existing_path(disk_path)?;
    }
    Ok(new_file_created)
}

const RESERVED_DIR_NAMES: &[&str] = &[".git", ".jj"];

fn same_file_handle_from_path(disk_path: &Path) -> io::Result<Option<same_file::Handle>> {
    match same_file::Handle::from_path(disk_path) {
        Ok(handle) => Ok(Some(handle)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Wrapper for [`reject_reserved_existing_handle`] which avoids a syscall
/// by converting the provided `file` to a `same_file::Handle` via its
/// file descriptor.
///
/// See [`reject_reserved_existing_handle`] for more info.
fn reject_reserved_existing_file(file: File, disk_path: &Path) -> Result<(), CheckoutError> {
    // Note: since the file is open, we don't expect that it's possible for
    // `io::ErrorKind::NotFound` to be a possible error returned here.
    let file_handle = same_file::Handle::from_file(file).map_err(|err| CheckoutError::Other {
        message: format!("Failed to validate path {}", disk_path.display()),
        err: err.into(),
    })?;

    reject_reserved_existing_handle(file_handle, disk_path)
}

/// Wrapper for [`reject_reserved_existing_handle`] which converts
/// the provided `disk_path` to a `same_file::Handle`.
///
/// See [`reject_reserved_existing_handle`] for more info.
///
/// # Remarks
///
/// Incurs an additional syscall cost to open and close the file
/// descriptor/`HANDLE` for `disk_path`.
fn reject_reserved_existing_path(disk_path: &Path) -> Result<(), CheckoutError> {
    let Some(disk_handle) =
        same_file_handle_from_path(disk_path).map_err(|err| CheckoutError::Other {
            message: format!("Failed to validate path {}", disk_path.display()),
            err: err.into(),
        })?
    else {
        // If the existing disk_path pointed to the reserved path, we would have
        // gotten a handle back. Since we got nothing, the file does not exist
        // and cannot be a reserved path name.
        return Ok(());
    };

    reject_reserved_existing_handle(disk_handle, disk_path)
}

/// Suppose the `disk_path` exists, checks if the last component points to
/// ".git" or ".jj" in the same parent directory.
///
/// `disk_handle` is expected to be a handle to the file described by
/// `disk_path`.
///
/// # Remarks
///
/// Incurs a syscall cost to open and close a file descriptor/`HANDLE` for
/// each filename in `RESERVED_DIR_NAMES`.
fn reject_reserved_existing_handle(
    disk_handle: same_file::Handle,
    disk_path: &Path,
) -> Result<(), CheckoutError> {
    let parent_dir_path = disk_path.parent().expect("content path shouldn't be root");
    for name in RESERVED_DIR_NAMES {
        let reserved_path = parent_dir_path.join(name);

        let Some(reserved_handle) =
            same_file_handle_from_path(&reserved_path).map_err(|err| CheckoutError::Other {
                message: format!("Failed to validate path {}", disk_path.display()),
                err: err.into(),
            })?
        else {
            // If the existing disk_path pointed to the reserved path, we would have
            // gotten a handle back. Since we got nothing, the file does not exist
            // and cannot be a reserved path name.
            continue;
        };

        if disk_handle == reserved_handle {
            return Err(CheckoutError::ReservedPathComponent {
                path: disk_path.to_owned(),
                name,
            });
        }
    }

    Ok(())
}

fn mtime_from_metadata(metadata: &Metadata) -> MillisSinceEpoch {
    let time = metadata
        .modified()
        .expect("File mtime not supported on this platform?");
    let since_epoch = time
        .duration_since(UNIX_EPOCH)
        .expect("mtime before unix epoch");

    MillisSinceEpoch(
        i64::try_from(since_epoch.as_millis())
            .expect("mtime billions of years into the future or past"),
    )
}

/// Create a new [`FileState`] from metadata.
fn file_state(metadata: &Metadata) -> Option<FileState> {
    let metadata_file_type = metadata.file_type();
    let file_type = if metadata_file_type.is_dir() {
        None
    } else if metadata_file_type.is_symlink() {
        Some(FileType::Symlink)
    } else if metadata_file_type.is_file() {
        let exec_bit = ExecBit::new_from_disk(metadata);
        Some(FileType::Normal { exec_bit })
    } else {
        None
    };
    file_type.map(|file_type| {
        let mtime = mtime_from_metadata(metadata);
        let size = metadata.len();
        FileState {
            file_type,
            mtime,
            size,
            materialized_conflict_data: None,
        }
    })
}

struct FsmonitorMatcher {
    matcher: Option<Box<dyn Matcher>>,
    watchman_clock: Option<crate::protos::local_working_copy::WatchmanClock>,
}

/// Settings specific to the tree state of the [`LocalWorkingCopy`] backend.
#[derive(Clone, Debug)]
pub struct TreeStateSettings {
    /// Conflict marker style to use when materializing files or when checking
    /// changed files.
    pub conflict_marker_style: ConflictMarkerStyle,
    /// Configuring auto-converting CRLF line endings into LF when you add a
    /// file to the backend, and vice versa when it checks out code onto your
    /// filesystem.
    pub eol_conversion_mode: EolConversionMode,
    /// Whether to ignore changes to the executable bit for files on Unix.
    pub exec_change_setting: ExecChangeSetting,
    /// The fsmonitor (e.g. Watchman) to use, if any.
    pub fsmonitor_settings: FsmonitorSettings,
}

impl TreeStateSettings {
    /// Create [`TreeStateSettings`] from [`UserSettings`].
    pub fn try_from_user_settings(user_settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            conflict_marker_style: user_settings.get("ui.conflict-marker-style")?,
            eol_conversion_mode: EolConversionMode::try_from_settings(user_settings)?,
            exec_change_setting: user_settings.get("working-copy.exec-bit-change")?,
            fsmonitor_settings: FsmonitorSettings::from_settings(user_settings)?,
        })
    }
}

pub struct TreeState {
    store: Arc<Store>,
    working_copy_path: PathBuf,
    state_path: PathBuf,
    tree: MergedTree,
    file_states: FileStatesMap,
    // Currently only path prefixes
    sparse_patterns: Vec<RepoPathBuf>,
    own_mtime: MillisSinceEpoch,
    symlink_support: bool,

    /// The most recent clock value returned by Watchman. Will only be set if
    /// the repo is configured to use the Watchman filesystem monitor and
    /// Watchman has been queried at least once.
    watchman_clock: Option<crate::protos::local_working_copy::WatchmanClock>,

    conflict_marker_style: ConflictMarkerStyle,
    exec_policy: ExecChangePolicy,
    fsmonitor_settings: FsmonitorSettings,
    target_eol_strategy: TargetEolStrategy,
}

#[derive(Debug, Error)]
pub enum TreeStateError {
    #[error("Reading tree state from {path}")]
    ReadTreeState { path: PathBuf, source: io::Error },
    #[error("Decoding tree state from {path}")]
    DecodeTreeState {
        path: PathBuf,
        source: prost::DecodeError,
    },
    #[error("Writing tree state to temporary file {path}")]
    WriteTreeState { path: PathBuf, source: io::Error },
    #[error("Persisting tree state to file {path}")]
    PersistTreeState { path: PathBuf, source: io::Error },
    #[error("Filesystem monitor error")]
    Fsmonitor(#[source] Box<dyn Error + Send + Sync>),
}

impl TreeState {
    pub fn working_copy_path(&self) -> &Path {
        &self.working_copy_path
    }

    pub fn current_tree(&self) -> &MergedTree {
        &self.tree
    }

    pub fn file_states(&self) -> FileStates<'_> {
        self.file_states.all()
    }

    pub fn sparse_patterns(&self) -> &Vec<RepoPathBuf> {
        &self.sparse_patterns
    }

    fn sparse_matcher(&self) -> Box<dyn Matcher> {
        Box::new(PrefixMatcher::new(&self.sparse_patterns))
    }

    pub fn init(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        tree_state_settings: &TreeStateSettings,
    ) -> Result<Self, TreeStateError> {
        let mut wc = Self::empty(store, working_copy_path, state_path, tree_state_settings);
        wc.save()?;
        Ok(wc)
    }

    fn empty(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        &TreeStateSettings {
            conflict_marker_style,
            eol_conversion_mode,
            exec_change_setting,
            ref fsmonitor_settings,
        }: &TreeStateSettings,
    ) -> Self {
        let exec_policy = ExecChangePolicy::new(exec_change_setting, &state_path);
        Self {
            store: store.clone(),
            working_copy_path,
            state_path,
            tree: store.empty_merged_tree(),
            file_states: FileStatesMap::new(),
            sparse_patterns: vec![RepoPathBuf::root()],
            own_mtime: MillisSinceEpoch(0),
            symlink_support: check_symlink_support().unwrap_or(false),
            watchman_clock: None,
            conflict_marker_style,
            exec_policy,
            fsmonitor_settings: fsmonitor_settings.clone(),
            target_eol_strategy: TargetEolStrategy::new(eol_conversion_mode),
        }
    }

    pub fn load(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        tree_state_settings: &TreeStateSettings,
    ) -> Result<Self, TreeStateError> {
        let tree_state_path = state_path.join("tree_state");
        let file = match File::open(&tree_state_path) {
            Err(ref err) if err.kind() == io::ErrorKind::NotFound => {
                return Self::init(store, working_copy_path, state_path, tree_state_settings);
            }
            Err(err) => {
                return Err(TreeStateError::ReadTreeState {
                    path: tree_state_path,
                    source: err,
                });
            }
            Ok(file) => file,
        };

        let mut wc = Self::empty(store, working_copy_path, state_path, tree_state_settings);
        wc.read(&tree_state_path, file)?;
        Ok(wc)
    }

    fn update_own_mtime(&mut self) {
        if let Ok(metadata) = self.state_path.join("tree_state").symlink_metadata() {
            self.own_mtime = mtime_from_metadata(&metadata);
        } else {
            self.own_mtime = MillisSinceEpoch(0);
        }
    }

    fn read(&mut self, tree_state_path: &Path, mut file: File) -> Result<(), TreeStateError> {
        self.update_own_mtime();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|err| TreeStateError::ReadTreeState {
                path: tree_state_path.to_owned(),
                source: err,
            })?;
        let proto = crate::protos::local_working_copy::TreeState::decode(&*buf).map_err(|err| {
            TreeStateError::DecodeTreeState {
                path: tree_state_path.to_owned(),
                source: err,
            }
        })?;
        #[expect(deprecated)]
        if proto.tree_ids.is_empty() {
            self.tree = MergedTree::resolved(
                self.store.clone(),
                TreeId::new(proto.legacy_tree_id.clone()),
            );
        } else {
            let tree_ids_builder: MergeBuilder<TreeId> = proto
                .tree_ids
                .iter()
                .map(|id| TreeId::new(id.clone()))
                .collect();
            self.tree = MergedTree::new(
                self.store.clone(),
                tree_ids_builder.build(),
                ConflictLabels::from_vec(proto.conflict_labels),
            );
        }
        self.file_states =
            FileStatesMap::from_proto(proto.file_states, proto.is_file_states_sorted);
        self.sparse_patterns = sparse_patterns_from_proto(proto.sparse_patterns.as_ref());
        self.watchman_clock = proto.watchman_clock;
        Ok(())
    }

    #[expect(clippy::assigning_clones, clippy::field_reassign_with_default)]
    pub fn save(&mut self) -> Result<(), TreeStateError> {
        let mut proto: crate::protos::local_working_copy::TreeState = Default::default();
        proto.tree_ids = self
            .tree
            .tree_ids()
            .iter()
            .map(|id| id.to_bytes())
            .collect();
        proto.conflict_labels = self.tree.labels().as_slice().to_owned();
        proto.file_states = self.file_states.data.clone();
        // `FileStatesMap` is guaranteed to be sorted.
        proto.is_file_states_sorted = true;
        let mut sparse_patterns = crate::protos::local_working_copy::SparsePatterns::default();
        for path in &self.sparse_patterns {
            sparse_patterns
                .prefixes
                .push(path.as_internal_file_string().to_owned());
        }
        proto.sparse_patterns = Some(sparse_patterns);
        proto.watchman_clock = self.watchman_clock.clone();

        let wrap_write_err = |source| TreeStateError::WriteTreeState {
            path: self.state_path.clone(),
            source,
        };
        let mut temp_file = NamedTempFile::new_in(&self.state_path).map_err(wrap_write_err)?;
        temp_file
            .as_file_mut()
            .write_all(&proto.encode_to_vec())
            .map_err(wrap_write_err)?;
        // update own write time while we before we rename it, so we know
        // there is no unknown data in it
        self.update_own_mtime();
        // TODO: Retry if persisting fails (it will on Windows if the file happened to
        // be open for read).
        let target_path = self.state_path.join("tree_state");
        persist_temp_file(temp_file, &target_path).map_err(|source| {
            TreeStateError::PersistTreeState {
                path: target_path.clone(),
                source,
            }
        })?;
        Ok(())
    }

    fn reset_watchman(&mut self) {
        self.watchman_clock.take();
    }

    #[cfg(feature = "watchman")]
    #[instrument(skip(self))]
    pub async fn query_watchman(
        &self,
        config: &WatchmanConfig,
    ) -> Result<(watchman::Clock, Option<Vec<PathBuf>>), TreeStateError> {
        let previous_clock = self.watchman_clock.clone().map(watchman::Clock::from);

        let tokio_fn = async || {
            let fsmonitor = watchman::Fsmonitor::init(&self.working_copy_path, config)
                .await
                .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))?;
            fsmonitor
                .query_changed_files(previous_clock)
                .await
                .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))
        };

        match tokio::runtime::Handle::try_current() {
            Ok(_handle) => tokio_fn().await,
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))?;
                runtime.block_on(tokio_fn())
            }
        }
    }

    #[cfg(feature = "watchman")]
    #[instrument(skip(self))]
    pub async fn is_watchman_trigger_registered(
        &self,
        config: &WatchmanConfig,
    ) -> Result<bool, TreeStateError> {
        let tokio_fn = async || {
            let fsmonitor = watchman::Fsmonitor::init(&self.working_copy_path, config)
                .await
                .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))?;
            fsmonitor
                .is_trigger_registered()
                .await
                .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))
        };

        match tokio::runtime::Handle::try_current() {
            Ok(_handle) => tokio_fn().await,
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|err| TreeStateError::Fsmonitor(Box::new(err)))?;
                runtime.block_on(tokio_fn())
            }
        }
    }
}

/// Functions to snapshot local-disk files to the store.
impl TreeState {
    /// Look for changes to the working copy. If there are any changes, create
    /// a new tree from it.
    #[instrument(skip_all)]
    pub async fn snapshot<'a>(
        &mut self,
        options: &SnapshotOptions<'a>,
    ) -> Result<(bool, SnapshotStats), SnapshotError> {
        let &SnapshotOptions {
            ref base_ignores,
            progress,
            start_tracking_matcher,
            force_tracking_matcher,
            max_new_file_size,
        } = options;

        let sparse_matcher = self.sparse_matcher();

        let fsmonitor_clock_needs_save = self.fsmonitor_settings != FsmonitorSettings::None;
        let mut is_dirty = fsmonitor_clock_needs_save;
        let FsmonitorMatcher {
            matcher: fsmonitor_matcher,
            watchman_clock,
        } = self
            .make_fsmonitor_matcher(&self.fsmonitor_settings)
            .await?;
        let fsmonitor_matcher = match fsmonitor_matcher.as_ref() {
            None => &EverythingMatcher,
            Some(fsmonitor_matcher) => fsmonitor_matcher.as_ref(),
        };

        let matcher = IntersectionMatcher::new(sparse_matcher.as_ref(), fsmonitor_matcher);
        if matcher.visit(RepoPath::root()).is_nothing() {
            // No need to load the current tree, set up channels, etc.
            self.watchman_clock = watchman_clock;
            return Ok((is_dirty, SnapshotStats::default()));
        }

        let (tree_entries_tx, tree_entries_rx) = channel();
        let (file_states_tx, file_states_rx) = channel();
        let (untracked_paths_tx, untracked_paths_rx) = channel();
        let (deleted_files_tx, deleted_files_rx) = channel();

        trace_span!("traverse filesystem").in_scope(|| -> Result<(), SnapshotError> {
            let snapshotter = FileSnapshotter {
                tree_state: self,
                current_tree: &self.tree,
                matcher: &matcher,
                start_tracking_matcher,
                force_tracking_matcher,
                // Move tx sides so they'll be dropped at the end of the scope.
                tree_entries_tx,
                file_states_tx,
                untracked_paths_tx,
                deleted_files_tx,
                error: OnceLock::new(),
                progress,
                max_new_file_size,
            };
            let directory_to_visit = DirectoryToVisit {
                dir: RepoPathBuf::root(),
                disk_dir: self.working_copy_path.clone(),
                git_ignore: base_ignores.clone(),
                file_states: self.file_states.all(),
            };
            // Here we use scope as a queue of per-directory jobs.
            rayon::scope(|scope| {
                snapshotter.spawn_ok(scope, |scope| {
                    snapshotter.visit_directory(directory_to_visit, scope)
                });
            });
            snapshotter.into_result()
        })?;

        let stats = SnapshotStats {
            untracked_paths: untracked_paths_rx.into_iter().collect(),
        };
        let mut tree_builder = MergedTreeBuilder::new(self.tree.clone());
        trace_span!("process tree entries").in_scope(|| {
            for (path, tree_values) in &tree_entries_rx {
                tree_builder.set_or_remove(path, tree_values);
            }
        });
        let deleted_files = trace_span!("process deleted tree entries").in_scope(|| {
            let deleted_files = HashSet::from_iter(deleted_files_rx);
            is_dirty |= !deleted_files.is_empty();
            for file in &deleted_files {
                tree_builder.set_or_remove(file.clone(), Merge::absent());
            }
            deleted_files
        });
        trace_span!("process file states").in_scope(|| {
            let changed_file_states = file_states_rx
                .iter()
                .sorted_unstable_by(|(path1, _), (path2, _)| path1.cmp(path2))
                .collect_vec();
            is_dirty |= !changed_file_states.is_empty();
            self.file_states
                .merge_in(changed_file_states, &deleted_files);
        });
        trace_span!("write tree").in_scope(|| -> Result<(), BackendError> {
            let new_tree = tree_builder.write_tree()?;
            is_dirty |= new_tree.tree_ids_and_labels() != self.tree.tree_ids_and_labels();
            self.tree = new_tree.clone();
            Ok(())
        })?;
        if cfg!(debug_assertions) {
            let tree_paths: HashSet<_> = self
                .tree
                .entries_matching(sparse_matcher.as_ref())
                .filter_map(|(path, result)| result.is_ok().then_some(path))
                .collect();
            let file_states = self.file_states.all();
            let state_paths: HashSet<_> = file_states.paths().map(|path| path.to_owned()).collect();
            assert_eq!(state_paths, tree_paths);
        }
        // Since untracked paths aren't cached in the tree state, we'll need to
        // rescan the working directory changes to report or track them later.
        // TODO: store untracked paths and update watchman_clock?
        if stats.untracked_paths.is_empty() || watchman_clock.is_none() {
            self.watchman_clock = watchman_clock;
        } else {
            tracing::info!("not updating watchman clock because there are untracked files");
        }
        Ok((is_dirty, stats))
    }

    #[instrument(skip_all)]
    async fn make_fsmonitor_matcher(
        &self,
        fsmonitor_settings: &FsmonitorSettings,
    ) -> Result<FsmonitorMatcher, SnapshotError> {
        let (watchman_clock, changed_files) = match fsmonitor_settings {
            FsmonitorSettings::None => (None, None),
            FsmonitorSettings::Test { changed_files } => (None, Some(changed_files.clone())),
            #[cfg(feature = "watchman")]
            FsmonitorSettings::Watchman(config) => match self.query_watchman(config).await {
                Ok((watchman_clock, changed_files)) => (Some(watchman_clock.into()), changed_files),
                Err(err) => {
                    tracing::warn!(?err, "Failed to query filesystem monitor");
                    (None, None)
                }
            },
            #[cfg(not(feature = "watchman"))]
            FsmonitorSettings::Watchman(_) => {
                return Err(SnapshotError::Other {
                    message: "Failed to query the filesystem monitor".to_string(),
                    err: "Cannot query Watchman because jj was not compiled with the `watchman` \
                          feature (consider disabling `fsmonitor.backend`)"
                        .into(),
                });
            }
        };
        let matcher: Option<Box<dyn Matcher>> = match changed_files {
            None => None,
            Some(changed_files) => {
                let repo_paths = trace_span!("processing fsmonitor paths").in_scope(|| {
                    changed_files
                        .into_iter()
                        .filter_map(|path| RepoPathBuf::from_relative_path(path).ok())
                        .collect_vec()
                });

                Some(Box::new(FilesMatcher::new(repo_paths)))
            }
        };
        Ok(FsmonitorMatcher {
            matcher,
            watchman_clock,
        })
    }
}

struct DirectoryToVisit<'a> {
    dir: RepoPathBuf,
    disk_dir: PathBuf,
    git_ignore: Arc<GitIgnoreFile>,
    file_states: FileStates<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PresentDirEntryKind {
    Dir,
    File,
}

#[derive(Clone, Debug)]
struct PresentDirEntries {
    dirs: HashSet<String>,
    files: HashSet<String>,
}

/// Helper to scan local-disk directories and files in parallel.
struct FileSnapshotter<'a> {
    tree_state: &'a TreeState,
    current_tree: &'a MergedTree,
    matcher: &'a dyn Matcher,
    start_tracking_matcher: &'a dyn Matcher,
    force_tracking_matcher: &'a dyn Matcher,
    tree_entries_tx: Sender<(RepoPathBuf, MergedTreeValue)>,
    file_states_tx: Sender<(RepoPathBuf, FileState)>,
    untracked_paths_tx: Sender<(RepoPathBuf, UntrackedReason)>,
    deleted_files_tx: Sender<RepoPathBuf>,
    error: OnceLock<SnapshotError>,
    progress: Option<&'a SnapshotProgress<'a>>,
    max_new_file_size: u64,
}

impl FileSnapshotter<'_> {
    fn spawn_ok<'scope, F>(&'scope self, scope: &rayon::Scope<'scope>, body: F)
    where
        F: FnOnce(&rayon::Scope<'scope>) -> Result<(), SnapshotError> + Send + 'scope,
    {
        scope.spawn(|scope| {
            if self.error.get().is_some() {
                return;
            }
            match body(scope) {
                Ok(()) => {}
                Err(err) => self.error.set(err).unwrap_or(()),
            };
        });
    }

    /// Extracts the result of the snapshot.
    fn into_result(self) -> Result<(), SnapshotError> {
        match self.error.into_inner() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Visits the directory entries, spawns jobs to recurse into sub
    /// directories.
    fn visit_directory<'scope>(
        &'scope self,
        directory_to_visit: DirectoryToVisit<'scope>,
        scope: &rayon::Scope<'scope>,
    ) -> Result<(), SnapshotError> {
        let DirectoryToVisit {
            dir,
            disk_dir,
            git_ignore,
            file_states,
        } = directory_to_visit;

        let git_ignore = git_ignore
            .chain_with_file(&dir.to_internal_dir_string(), disk_dir.join(".gitignore"))?;
        let dir_entries: Vec<_> = disk_dir
            .read_dir()
            .and_then(|entries| entries.try_collect())
            .map_err(|err| SnapshotError::Other {
                message: format!("Failed to read directory {}", disk_dir.display()),
                err: err.into(),
            })?;
        let (dirs, files) = dir_entries
            .into_par_iter()
            // Don't split into too many small jobs. For a small directory,
            // sequential scan should be fast enough.
            .with_min_len(100)
            .filter_map(|entry| {
                self.process_dir_entry(&dir, &git_ignore, file_states, &entry, scope)
                    .transpose()
            })
            .map(|item| match item {
                Ok((PresentDirEntryKind::Dir, name)) => Ok(Either::Left(name)),
                Ok((PresentDirEntryKind::File, name)) => Ok(Either::Right(name)),
                Err(err) => Err(err),
            })
            .collect::<Result<_, _>>()?;
        let present_entries = PresentDirEntries { dirs, files };
        self.emit_deleted_files(&dir, file_states, &present_entries);
        Ok(())
    }

    fn process_dir_entry<'scope>(
        &'scope self,
        dir: &RepoPath,
        git_ignore: &Arc<GitIgnoreFile>,
        file_states: FileStates<'scope>,
        entry: &DirEntry,
        scope: &rayon::Scope<'scope>,
    ) -> Result<Option<(PresentDirEntryKind, String)>, SnapshotError> {
        let file_type = entry.file_type().unwrap();
        let file_name = entry.file_name();
        let name_string = file_name
            .into_string()
            .map_err(|path| SnapshotError::InvalidUtf8Path { path })?;

        if RESERVED_DIR_NAMES.contains(&name_string.as_str()) {
            return Ok(None);
        }
        let name = RepoPathComponent::new(&name_string).unwrap();
        let path = dir.join(name);
        let maybe_current_file_state = file_states.get_at(dir, name);
        if let Some(file_state) = &maybe_current_file_state
            && file_state.file_type == FileType::GitSubmodule
        {
            return Ok(None);
        }

        if file_type.is_dir() {
            let file_states = file_states.prefixed_at(dir, name);
            // If a submodule was added in commit C, and a user decides to run
            // `jj new <something before C>` from after C, then the submodule
            // files stick around but it is no longer seen as a submodule.
            // We need to ensure that it is not tracked as if it was added to
            // the main repo.
            // See https://github.com/jj-vcs/jj/issues/4349.
            // To solve this, we ignore all nested repos entirely.
            let disk_dir = entry.path();
            for &name in RESERVED_DIR_NAMES {
                if disk_dir.join(name).symlink_metadata().is_ok() {
                    return Ok(None);
                }
            }

            if git_ignore.matches(&path.to_internal_dir_string())
                && self.force_tracking_matcher.visit(&path).is_nothing()
            {
                // If the whole directory is ignored by .gitignore, visit only
                // paths we're already tracking. This is because .gitignore in
                // ignored directory must be ignored. It's also more efficient.
                // start_tracking_matcher is NOT tested here because we need to
                // scan directory entries to report untracked paths.
                self.spawn_ok(scope, move |_| self.visit_tracked_files(file_states));
            } else if !self.matcher.visit(&path).is_nothing() {
                let directory_to_visit = DirectoryToVisit {
                    dir: path,
                    disk_dir,
                    git_ignore: git_ignore.clone(),
                    file_states,
                };
                self.spawn_ok(scope, |scope| {
                    self.visit_directory(directory_to_visit, scope)
                });
            }
            // Whether or not the directory path matches, any child file entries
            // shouldn't be touched within the current recursion step.
            Ok(Some((PresentDirEntryKind::Dir, name_string)))
        } else if self.matcher.matches(&path) {
            if let Some(progress) = self.progress {
                progress(&path);
            }
            if maybe_current_file_state.is_none()
                && (git_ignore.matches(path.as_internal_file_string())
                    && !self.force_tracking_matcher.matches(&path))
            {
                // If it wasn't already tracked and it matches
                // the ignored paths, then ignore it.
                Ok(None)
            } else if maybe_current_file_state.is_none()
                && !self.start_tracking_matcher.matches(&path)
            {
                // Leave the file untracked
                self.untracked_paths_tx
                    .send((path, UntrackedReason::FileNotAutoTracked))
                    .ok();
                Ok(None)
            } else {
                let metadata = entry.metadata().map_err(|err| SnapshotError::Other {
                    message: format!("Failed to stat file {}", entry.path().display()),
                    err: err.into(),
                })?;
                if maybe_current_file_state.is_none()
                    && (metadata.len() > self.max_new_file_size
                        && !self.force_tracking_matcher.matches(&path))
                {
                    // Leave the large file untracked
                    let reason = UntrackedReason::FileTooLarge {
                        size: metadata.len(),
                        max_size: self.max_new_file_size,
                    };
                    self.untracked_paths_tx.send((path, reason)).ok();
                    Ok(None)
                } else if let Some(new_file_state) = file_state(&metadata) {
                    self.process_present_file(
                        path,
                        &entry.path(),
                        maybe_current_file_state.as_ref(),
                        new_file_state,
                    )?;
                    Ok(Some((PresentDirEntryKind::File, name_string)))
                } else {
                    // Special file is not considered present
                    Ok(None)
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Visits only paths we're already tracking.
    fn visit_tracked_files(&self, file_states: FileStates<'_>) -> Result<(), SnapshotError> {
        for (tracked_path, current_file_state) in file_states {
            if current_file_state.file_type == FileType::GitSubmodule {
                continue;
            }
            if !self.matcher.matches(tracked_path) {
                continue;
            }
            let disk_path = tracked_path.to_fs_path(&self.tree_state.working_copy_path)?;
            let metadata = match disk_path.symlink_metadata() {
                Ok(metadata) => Some(metadata),
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => {
                    return Err(SnapshotError::Other {
                        message: format!("Failed to stat file {}", disk_path.display()),
                        err: err.into(),
                    });
                }
            };
            if let Some(new_file_state) = metadata.as_ref().and_then(file_state) {
                self.process_present_file(
                    tracked_path.to_owned(),
                    &disk_path,
                    Some(&current_file_state),
                    new_file_state,
                )?;
            } else {
                self.deleted_files_tx.send(tracked_path.to_owned()).ok();
            }
        }
        Ok(())
    }

    fn process_present_file(
        &self,
        path: RepoPathBuf,
        disk_path: &Path,
        maybe_current_file_state: Option<&FileState>,
        mut new_file_state: FileState,
    ) -> Result<(), SnapshotError> {
        let update = self.get_updated_tree_value(
            &path,
            disk_path,
            maybe_current_file_state,
            &new_file_state,
        )?;
        // Preserve materialized conflict data for normal, non-resolved files
        if matches!(new_file_state.file_type, FileType::Normal { .. })
            && !update.as_ref().is_some_and(|update| update.is_resolved())
        {
            new_file_state.materialized_conflict_data =
                maybe_current_file_state.and_then(|state| state.materialized_conflict_data);
        }
        if let Some(tree_value) = update {
            self.tree_entries_tx.send((path.clone(), tree_value)).ok();
        }
        if Some(&new_file_state) != maybe_current_file_state {
            self.file_states_tx.send((path, new_file_state)).ok();
        }
        Ok(())
    }

    /// Emits file paths that don't exist in the `present_entries`.
    fn emit_deleted_files(
        &self,
        dir: &RepoPath,
        file_states: FileStates<'_>,
        present_entries: &PresentDirEntries,
    ) {
        let file_state_chunks = file_states.iter().chunk_by(|(path, _state)| {
            // Extract <name> from <dir>, <dir>/<name>, or <dir>/<name>/**.
            // (file_states may contain <dir> file on file->dir transition.)
            debug_assert!(path.starts_with(dir));
            let slash = usize::from(!dir.is_root());
            let len = dir.as_internal_file_string().len() + slash;
            let tail = path.as_internal_file_string().get(len..).unwrap_or("");
            match tail.split_once('/') {
                Some((name, _)) => (PresentDirEntryKind::Dir, name),
                None => (PresentDirEntryKind::File, tail),
            }
        });
        file_state_chunks
            .into_iter()
            .filter(|&((kind, name), _)| match kind {
                PresentDirEntryKind::Dir => !present_entries.dirs.contains(name),
                PresentDirEntryKind::File => !present_entries.files.contains(name),
            })
            .flat_map(|(_, chunk)| chunk)
            // Whether or not the entry exists, submodule should be ignored
            .filter(|(_, state)| state.file_type != FileType::GitSubmodule)
            .filter(|(path, _)| self.matcher.matches(path))
            .try_for_each(|(path, _)| self.deleted_files_tx.send(path.to_owned()))
            .ok();
    }

    fn get_updated_tree_value(
        &self,
        repo_path: &RepoPath,
        disk_path: &Path,
        maybe_current_file_state: Option<&FileState>,
        new_file_state: &FileState,
    ) -> Result<Option<MergedTreeValue>, SnapshotError> {
        let clean = match maybe_current_file_state {
            None => {
                // untracked
                false
            }
            Some(current_file_state) => {
                // If the file's mtime was set at the same time as this state file's own mtime,
                // then we don't know if the file was modified before or after this state file.
                new_file_state.is_clean(current_file_state)
                    && current_file_state.mtime < self.tree_state.own_mtime
            }
        };
        if clean {
            Ok(None)
        } else {
            let current_tree_values = self.current_tree.path_value(repo_path)?;
            let new_file_type = if !self.tree_state.symlink_support {
                let mut new_file_type = new_file_state.file_type.clone();
                if matches!(new_file_type, FileType::Normal { .. })
                    && matches!(current_tree_values.as_normal(), Some(TreeValue::Symlink(_)))
                {
                    new_file_type = FileType::Symlink;
                }
                new_file_type
            } else {
                new_file_state.file_type.clone()
            };
            let new_tree_values = match new_file_type {
                FileType::Normal { exec_bit } => self
                    .write_path_to_store(
                        repo_path,
                        disk_path,
                        &current_tree_values,
                        exec_bit,
                        maybe_current_file_state.and_then(|state| state.materialized_conflict_data),
                    )
                    .block_on()?,
                FileType::Symlink => {
                    let id = self
                        .write_symlink_to_store(repo_path, disk_path)
                        .block_on()?;
                    Merge::normal(TreeValue::Symlink(id))
                }
                FileType::GitSubmodule => panic!("git submodule cannot be written to store"),
            };
            if new_tree_values != current_tree_values {
                Ok(Some(new_tree_values))
            } else {
                Ok(None)
            }
        }
    }

    fn store(&self) -> &Store {
        &self.tree_state.store
    }

    async fn write_path_to_store(
        &self,
        repo_path: &RepoPath,
        disk_path: &Path,
        current_tree_values: &MergedTreeValue,
        exec_bit: ExecBit,
        materialized_conflict_data: Option<MaterializedConflictData>,
    ) -> Result<MergedTreeValue, SnapshotError> {
        if let Some(current_tree_value) = current_tree_values.as_resolved() {
            let id = self.write_file_to_store(repo_path, disk_path).await?;
            // On Windows, we preserve the executable bit from the current tree.
            let executable = exec_bit.for_tree_value(self.tree_state.exec_policy, || {
                if let Some(TreeValue::File {
                    id: _,
                    executable,
                    copy_id: _,
                }) = current_tree_value
                {
                    Some(*executable)
                } else {
                    None
                }
            });
            // Preserve the copy id from the current tree
            let copy_id = {
                if let Some(TreeValue::File {
                    id: _,
                    executable: _,
                    copy_id,
                }) = current_tree_value
                {
                    copy_id.clone()
                } else {
                    CopyId::placeholder()
                }
            };
            Ok(Merge::normal(TreeValue::File {
                id,
                executable,
                copy_id,
            }))
        } else if let Some(old_file_ids) = current_tree_values.to_file_merge() {
            // Safe to unwrap because the copy id exists exactly on the file variant
            let copy_id_merge = current_tree_values.to_copy_id_merge().unwrap();
            let copy_id = copy_id_merge
                .resolve_trivial(SameChange::Accept)
                .cloned()
                .flatten()
                .unwrap_or_else(CopyId::placeholder);
            let mut contents = vec![];
            let file = File::open(disk_path).map_err(|err| SnapshotError::Other {
                message: format!("Failed to open file {}", disk_path.display()),
                err: err.into(),
            })?;
            self.tree_state
                .target_eol_strategy
                .convert_eol_for_snapshot(BlockingAsyncReader::new(file))
                .await
                .map_err(|err| SnapshotError::Other {
                    message: "Failed to convert the EOL".to_string(),
                    err: err.into(),
                })?
                .read_to_end(&mut contents)
                .await
                .map_err(|err| SnapshotError::Other {
                    message: "Failed to read the EOL converted contents".to_string(),
                    err: err.into(),
                })?;
            // If the file contained a conflict before and is a normal file on
            // disk, we try to parse any conflict markers in the file into a
            // conflict.
            let new_file_ids = conflicts::update_from_content(
                &old_file_ids,
                self.store(),
                repo_path,
                &contents,
                materialized_conflict_data.map_or(MIN_CONFLICT_MARKER_LEN, |data| {
                    data.conflict_marker_len as usize
                }),
            )
            .await?;
            match new_file_ids.into_resolved() {
                Ok(file_id) => {
                    // On Windows, we preserve the executable bit from the merged trees.
                    let executable = exec_bit.for_tree_value(self.tree_state.exec_policy, || {
                        current_tree_values
                            .to_executable_merge()
                            .as_ref()
                            .and_then(conflicts::resolve_file_executable)
                    });
                    Ok(Merge::normal(TreeValue::File {
                        id: file_id.unwrap(),
                        executable,
                        copy_id,
                    }))
                }
                Err(new_file_ids) => {
                    if new_file_ids != old_file_ids {
                        Ok(current_tree_values.with_new_file_ids(&new_file_ids))
                    } else {
                        Ok(current_tree_values.clone())
                    }
                }
            }
        } else {
            Ok(current_tree_values.clone())
        }
    }

    async fn write_file_to_store(
        &self,
        path: &RepoPath,
        disk_path: &Path,
    ) -> Result<FileId, SnapshotError> {
        let file = File::open(disk_path).map_err(|err| SnapshotError::Other {
            message: format!("Failed to open file {}", disk_path.display()),
            err: err.into(),
        })?;
        let mut contents = self
            .tree_state
            .target_eol_strategy
            .convert_eol_for_snapshot(BlockingAsyncReader::new(file))
            .await
            .map_err(|err| SnapshotError::Other {
                message: "Failed to convert the EOL".to_string(),
                err: err.into(),
            })?;
        Ok(self.store().write_file(path, &mut contents).await?)
    }

    async fn write_symlink_to_store(
        &self,
        path: &RepoPath,
        disk_path: &Path,
    ) -> Result<SymlinkId, SnapshotError> {
        if self.tree_state.symlink_support {
            let target = disk_path.read_link().map_err(|err| SnapshotError::Other {
                message: format!("Failed to read symlink {}", disk_path.display()),
                err: err.into(),
            })?;
            let str_target = symlink_target_convert_to_store(&target).ok_or_else(|| {
                SnapshotError::InvalidUtf8SymlinkTarget {
                    path: disk_path.to_path_buf(),
                }
            })?;
            Ok(self.store().write_symlink(path, &str_target).await?)
        } else {
            let target = fs::read(disk_path).map_err(|err| SnapshotError::Other {
                message: format!("Failed to read file {}", disk_path.display()),
                err: err.into(),
            })?;
            let string_target =
                String::from_utf8(target).map_err(|_| SnapshotError::InvalidUtf8SymlinkTarget {
                    path: disk_path.to_path_buf(),
                })?;
            Ok(self.store().write_symlink(path, &string_target).await?)
        }
    }
}

/// Functions to update local-disk files from the store.
impl TreeState {
    async fn write_file(
        &self,
        disk_path: &Path,
        contents: impl AsyncRead + Send + Unpin,
        exec_bit: ExecBit,
        apply_eol_conversion: bool,
    ) -> Result<FileState, CheckoutError> {
        let mut file = File::options()
            .write(true)
            .create_new(true) // Don't overwrite un-ignored file. Don't follow symlink.
            .open(disk_path)
            .map_err(|err| CheckoutError::Other {
                message: format!("Failed to open file {} for writing", disk_path.display()),
                err: err.into(),
            })?;
        let contents = if apply_eol_conversion {
            self.target_eol_strategy
                .convert_eol_for_update(contents)
                .await
                .map_err(|err| CheckoutError::Other {
                    message: "Failed to convert the EOL for the content".to_string(),
                    err: err.into(),
                })?
        } else {
            Box::new(contents)
        };
        let size = copy_async_to_sync(contents, &mut file)
            .await
            .map_err(|err| CheckoutError::Other {
                message: format!(
                    "Failed to write the content to the file {}",
                    disk_path.display()
                ),
                err: err.into(),
            })?;
        set_executable(exec_bit, disk_path)
            .map_err(|err| checkout_error_for_stat_error(err, disk_path))?;
        // Read the file state from the file descriptor. That way, know that the file
        // exists and is of the expected type, and the stat information is most likely
        // accurate, except for other processes modifying the file concurrently (The
        // mtime is set at write time and won't change when we close the file.)
        let metadata = file
            .metadata()
            .map_err(|err| checkout_error_for_stat_error(err, disk_path))?;
        Ok(FileState::for_file(exec_bit, size as u64, &metadata))
    }

    fn write_symlink(&self, disk_path: &Path, target: String) -> Result<FileState, CheckoutError> {
        let target = symlink_target_convert_to_disk(&target);

        if cfg!(windows) {
            // On Windows, "/" can't be part of valid file name, and "/" is also not a valid
            // separator for the symlink target. See an example of this issue in
            // https://github.com/jj-vcs/jj/issues/6934.
            //
            // We use debug_assert_* instead of assert_* because we want to avoid panic in
            // release build, and we are sure that we shouldn't create invalid symlinks in
            // tests.
            debug_assert_ne!(
                target.as_os_str().to_str().map(|path| path.contains("/")),
                Some(true),
                "Expect the symlink target doesn't contain \"/\", but got invalid symlink target: \
                 {}.",
                target.display()
            );
        }

        try_symlink(&target, disk_path).map_err(|err| CheckoutError::Other {
            message: format!(
                "Failed to create symlink from {} to {}",
                disk_path.display(),
                target.display()
            ),
            err: err.into(),
        })?;
        let metadata = disk_path
            .symlink_metadata()
            .map_err(|err| checkout_error_for_stat_error(err, disk_path))?;
        Ok(FileState::for_symlink(&metadata))
    }

    async fn write_conflict(
        &self,
        disk_path: &Path,
        contents: &[u8],
        exec_bit: ExecBit,
    ) -> Result<FileState, CheckoutError> {
        let contents = self
            .target_eol_strategy
            .convert_eol_for_update(contents)
            .await
            .map_err(|err| CheckoutError::Other {
                message: "Failed to convert the EOL when writing a merge conflict".to_string(),
                err: err.into(),
            })?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true) // Don't overwrite un-ignored file. Don't follow symlink.
            .open(disk_path)
            .map_err(|err| CheckoutError::Other {
                message: format!("Failed to open file {} for writing", disk_path.display()),
                err: err.into(),
            })?;
        let size = copy_async_to_sync(contents, &mut file)
            .await
            .map_err(|err| CheckoutError::Other {
                message: format!("Failed to write conflict to file {}", disk_path.display()),
                err: err.into(),
            })? as u64;
        set_executable(exec_bit, disk_path)
            .map_err(|err| checkout_error_for_stat_error(err, disk_path))?;
        let metadata = file
            .metadata()
            .map_err(|err| checkout_error_for_stat_error(err, disk_path))?;
        Ok(FileState::for_file(exec_bit, size, &metadata))
    }

    pub fn check_out(&mut self, new_tree: &MergedTree) -> Result<CheckoutStats, CheckoutError> {
        let old_tree = self.tree.clone();
        let stats = self
            .update(&old_tree, new_tree, self.sparse_matcher().as_ref())
            .block_on()?;
        self.tree = new_tree.clone();
        Ok(stats)
    }

    pub fn set_sparse_patterns(
        &mut self,
        sparse_patterns: Vec<RepoPathBuf>,
    ) -> Result<CheckoutStats, CheckoutError> {
        let tree = self.tree.clone();
        let old_matcher = PrefixMatcher::new(&self.sparse_patterns);
        let new_matcher = PrefixMatcher::new(&sparse_patterns);
        let added_matcher = DifferenceMatcher::new(&new_matcher, &old_matcher);
        let removed_matcher = DifferenceMatcher::new(&old_matcher, &new_matcher);
        let empty_tree = self.store.empty_merged_tree();
        let added_stats = self.update(&empty_tree, &tree, &added_matcher).block_on()?;
        let removed_stats = self
            .update(&tree, &empty_tree, &removed_matcher)
            .block_on()?;
        self.sparse_patterns = sparse_patterns;
        assert_eq!(added_stats.updated_files, 0);
        assert_eq!(added_stats.removed_files, 0);
        assert_eq!(removed_stats.updated_files, 0);
        assert_eq!(removed_stats.added_files, 0);
        assert_eq!(removed_stats.skipped_files, 0);
        Ok(CheckoutStats {
            updated_files: 0,
            added_files: added_stats.added_files,
            removed_files: removed_stats.removed_files,
            skipped_files: added_stats.skipped_files,
        })
    }

    async fn update(
        &mut self,
        old_tree: &MergedTree,
        new_tree: &MergedTree,
        matcher: &dyn Matcher,
    ) -> Result<CheckoutStats, CheckoutError> {
        // TODO: maybe it's better not include the skipped counts in the "intended"
        // counts
        let mut stats = CheckoutStats {
            updated_files: 0,
            added_files: 0,
            removed_files: 0,
            skipped_files: 0,
        };
        let mut changed_file_states = Vec::new();
        let mut deleted_files = HashSet::new();
        let mut prev_created_path: RepoPathBuf = RepoPathBuf::root();

        let mut process_diff_entry = async |path: RepoPathBuf,
                                            before: MergedTreeValue,
                                            after: MaterializedTreeValue|
               -> Result<(), CheckoutError> {
            if after.is_absent() {
                stats.removed_files += 1;
            } else if before.is_absent() {
                stats.added_files += 1;
            } else {
                stats.updated_files += 1;
            }

            // Existing Git submodule can be a non-empty directory on disk. We
            // shouldn't attempt to manage it as a tracked path.
            //
            // TODO: It might be better to add general support for paths not
            // tracked by jj than processing submodules specially. For example,
            // paths excluded by .gitignore can be marked as such so that
            // newly-"unignored" paths won't be snapshotted automatically.
            if matches!(before.as_normal(), Some(TreeValue::GitSubmodule(_)))
                && matches!(after, MaterializedTreeValue::GitSubmodule(_))
            {
                eprintln!("ignoring git submodule at {path:?}");
                // Not updating the file state as if there were no diffs. Leave
                // the state type as FileType::GitSubmodule if it was before.
                return Ok(());
            }

            // This path and the previous one we did work for may have a common prefix. We
            // can adjust the "working copy" path to the parent directory which we know
            // is already created. If there is no common prefix, this will by default use
            // RepoPath::root() as the common prefix.
            let (common_prefix, adjusted_diff_file_path) =
                path.split_common_prefix(&prev_created_path);

            let disk_path = if adjusted_diff_file_path.is_root() {
                // The path being "root" here implies that the entire path has already been
                // created.
                //
                // e.g we may have have already processed a path like: "foo/bar/baz" and this is
                // our `prev_created_path`.
                //
                // and the current path is:
                // "foo/bar"
                //
                // This results in a common prefix of "foo/bar" with empty string for the
                // remainder since its entire prefix has already been created.
                // This means that we _dont_ need to create its parent dirs
                // either.

                path.to_fs_path(self.working_copy_path())?
            } else {
                let adjusted_working_copy_path =
                    common_prefix.to_fs_path(self.working_copy_path())?;

                // Create parent directories no matter if after.is_present(). This
                // ensures that the path never traverses symlinks.
                let Some(disk_path) =
                    create_parent_dirs(&adjusted_working_copy_path, adjusted_diff_file_path)?
                else {
                    changed_file_states.push((path, FileState::placeholder()));
                    stats.skipped_files += 1;
                    return Ok(());
                };

                // Cache this path for the next iteration. This must occur after
                // `create_parent_dirs` to ensure that the path is only set when
                // no symlinks are encountered. Otherwise there could be
                // opportunity for a filesystem write-what-where attack.
                prev_created_path = path
                    .parent()
                    .map(RepoPath::to_owned)
                    .expect("diff path has no parent");

                disk_path
            };

            // If the path was present, check reserved path first and delete it.
            let present_file_deleted = before.is_present() && remove_old_file(&disk_path)?;
            // If not, create temporary file to test the path validity.
            if !present_file_deleted && !can_create_new_file(&disk_path)? {
                changed_file_states.push((path, FileState::placeholder()));
                stats.skipped_files += 1;
                return Ok(());
            }

            // We get the previous executable bit from the file states and not
            // the tree value because only the file states store the on-disk
            // executable bit.
            let get_prev_exec = || self.file_states().get_exec_bit(&path);

            // TODO: Check that the file has not changed before overwriting/removing it.
            let file_state = match after {
                MaterializedTreeValue::Absent | MaterializedTreeValue::AccessDenied(_) => {
                    // Reset the previous path to avoid scenarios where this path is deleted,
                    // then on the next iteration recreation is skipped because of this
                    // optimization.
                    prev_created_path = RepoPathBuf::root();

                    let mut parent_dir = disk_path.parent().unwrap();
                    loop {
                        if fs::remove_dir(parent_dir).is_err() {
                            break;
                        }

                        parent_dir = parent_dir.parent().unwrap();
                    }
                    deleted_files.insert(path);
                    return Ok(());
                }
                MaterializedTreeValue::File(file) => {
                    let exec_bit =
                        ExecBit::new_from_repo(file.executable, self.exec_policy, get_prev_exec);
                    self.write_file(&disk_path, file.reader, exec_bit, true)
                        .await?
                }
                MaterializedTreeValue::Symlink { id: _, target } => {
                    if self.symlink_support {
                        self.write_symlink(&disk_path, target)?
                    } else {
                        // The fake symlink file shouldn't be executable.
                        self.write_file(&disk_path, target.as_bytes(), ExecBit(false), false)
                            .await?
                    }
                }
                MaterializedTreeValue::GitSubmodule(_) => {
                    eprintln!("ignoring git submodule at {path:?}");
                    FileState::for_gitsubmodule()
                }
                MaterializedTreeValue::Tree(_) => {
                    panic!("unexpected tree entry in diff at {path:?}");
                }
                MaterializedTreeValue::FileConflict(file) => {
                    let conflict_marker_len =
                        choose_materialized_conflict_marker_len(&file.contents);
                    let options = ConflictMaterializeOptions {
                        marker_style: self.conflict_marker_style,
                        marker_len: Some(conflict_marker_len),
                        merge: self.store.merge_options().clone(),
                    };
                    let exec_bit = ExecBit::new_from_repo(
                        file.executable.unwrap_or(false),
                        self.exec_policy,
                        get_prev_exec,
                    );
                    let contents =
                        materialize_merge_result_to_bytes(&file.contents, &file.labels, &options);
                    let mut file_state =
                        self.write_conflict(&disk_path, &contents, exec_bit).await?;
                    file_state.materialized_conflict_data = Some(MaterializedConflictData {
                        conflict_marker_len: conflict_marker_len.try_into().unwrap_or(u32::MAX),
                    });
                    file_state
                }
                MaterializedTreeValue::OtherConflict { id, labels } => {
                    // Unless all terms are regular files, we can't do much
                    // better than trying to describe the merge.
                    let contents = id.describe(&labels);
                    // Since this is a dummy file, it shouldn't be executable.
                    self.write_conflict(&disk_path, contents.as_bytes(), ExecBit(false))
                        .await?
                }
            };
            changed_file_states.push((path, file_state));
            Ok(())
        };

        let mut diff_stream = old_tree
            .diff_stream_for_file_system(new_tree, matcher)
            .map(async |TreeDiffEntry { path, values }| match values {
                Ok(diff) => {
                    let result =
                        materialize_tree_value(&self.store, &path, diff.after, new_tree.labels())
                            .await;
                    (path, result.map(|value| (diff.before, value)))
                }
                Err(err) => (path, Err(err)),
            })
            .buffered(self.store.concurrency().max(1));

        // If a conflicted file didn't change between the two trees, but the conflict
        // labels did, we still need to re-materialize it in the working copy. We don't
        // need to do this if the conflicts have different numbers of sides though since
        // these conflicts are considered different, so they will be materialized by
        // `MergedTree::diff_stream_for_file_system` already.
        let mut conflicts_to_rematerialize: HashMap<RepoPathBuf, MergedTreeValue> =
            if old_tree.tree_ids().num_sides() == new_tree.tree_ids().num_sides()
                && old_tree.labels() != new_tree.labels()
            {
                // TODO: it might be better to use an async stream here and merge it with the
                // other diff stream, but it could be difficult since the diff stream is not
                // sorted in the same order as the conflicts iterator.
                new_tree
                    .conflicts_matching(matcher)
                    .map(|(path, value)| value.map(|value| (path, value)))
                    .try_collect()?
            } else {
                HashMap::new()
            };

        while let Some((path, data)) = diff_stream.next().await {
            let (before, after) = data?;
            conflicts_to_rematerialize.remove(&path);
            process_diff_entry(path, before, after).await?;
        }

        if !conflicts_to_rematerialize.is_empty() {
            for (path, conflict) in conflicts_to_rematerialize {
                let materialized =
                    materialize_tree_value(&self.store, &path, conflict.clone(), new_tree.labels())
                        .await?;
                process_diff_entry(path, conflict, materialized).await?;
            }

            // We need to re-sort the changed file states since we may have inserted a
            // conflicted file out of order.
            changed_file_states.sort_unstable_by(|(path1, _), (path2, _)| path1.cmp(path2));
        }

        self.file_states
            .merge_in(changed_file_states, &deleted_files);
        Ok(stats)
    }

    pub async fn reset(&mut self, new_tree: &MergedTree) -> Result<(), ResetError> {
        let matcher = self.sparse_matcher();
        let mut changed_file_states = Vec::new();
        let mut deleted_files = HashSet::new();
        let mut diff_stream = self
            .tree
            .diff_stream_for_file_system(new_tree, matcher.as_ref());
        while let Some(TreeDiffEntry { path, values }) = diff_stream.next().await {
            let after = values?.after;
            if after.is_absent() {
                deleted_files.insert(path);
            } else {
                let file_type = match after.into_resolved() {
                    Ok(value) => match value.unwrap() {
                        TreeValue::File {
                            id: _,
                            executable,
                            copy_id: _,
                        } => {
                            let get_prev_exec = || self.file_states().get_exec_bit(&path);
                            let exec_bit =
                                ExecBit::new_from_repo(executable, self.exec_policy, get_prev_exec);
                            FileType::Normal { exec_bit }
                        }
                        TreeValue::Symlink(_id) => FileType::Symlink,
                        TreeValue::GitSubmodule(_id) => {
                            eprintln!("ignoring git submodule at {path:?}");
                            FileType::GitSubmodule
                        }
                        TreeValue::Tree(_id) => {
                            panic!("unexpected tree entry in diff at {path:?}");
                        }
                    },
                    Err(_values) => {
                        // TODO: Try to set the executable bit based on the conflict
                        FileType::Normal {
                            exec_bit: ExecBit(false),
                        }
                    }
                };
                let file_state = FileState {
                    file_type,
                    mtime: MillisSinceEpoch(0),
                    size: 0,
                    materialized_conflict_data: None,
                };
                changed_file_states.push((path, file_state));
            }
        }
        self.file_states
            .merge_in(changed_file_states, &deleted_files);
        self.tree = new_tree.clone();
        Ok(())
    }

    pub async fn recover(&mut self, new_tree: &MergedTree) -> Result<(), ResetError> {
        self.file_states.clear();
        self.tree = self.store.empty_merged_tree();
        self.reset(new_tree).await
    }
}

fn checkout_error_for_stat_error(err: io::Error, path: &Path) -> CheckoutError {
    CheckoutError::Other {
        message: format!("Failed to stat file {}", path.display()),
        err: err.into(),
    }
}

/// Working copy state stored in "checkout" file.
#[derive(Clone, Debug)]
struct CheckoutState {
    operation_id: OperationId,
    workspace_name: WorkspaceNameBuf,
}

impl CheckoutState {
    fn load(state_path: &Path) -> Result<Self, WorkingCopyStateError> {
        let wrap_err = |err| WorkingCopyStateError {
            message: "Failed to read checkout state".to_owned(),
            err,
        };
        let buf = fs::read(state_path.join("checkout")).map_err(|err| wrap_err(err.into()))?;
        let proto = crate::protos::local_working_copy::Checkout::decode(&*buf)
            .map_err(|err| wrap_err(err.into()))?;
        Ok(Self {
            operation_id: OperationId::new(proto.operation_id),
            workspace_name: if proto.workspace_name.is_empty() {
                // For compatibility with old working copies.
                // TODO: Delete in mid 2022 or so
                WorkspaceName::DEFAULT.to_owned()
            } else {
                proto.workspace_name.into()
            },
        })
    }

    #[instrument(skip_all)]
    fn save(&self, state_path: &Path) -> Result<(), WorkingCopyStateError> {
        let wrap_err = |err| WorkingCopyStateError {
            message: "Failed to write checkout state".to_owned(),
            err,
        };
        let proto = crate::protos::local_working_copy::Checkout {
            operation_id: self.operation_id.to_bytes(),
            workspace_name: (*self.workspace_name).into(),
        };
        let mut temp_file =
            NamedTempFile::new_in(state_path).map_err(|err| wrap_err(err.into()))?;
        temp_file
            .as_file_mut()
            .write_all(&proto.encode_to_vec())
            .map_err(|err| wrap_err(err.into()))?;
        // TODO: Retry if persisting fails (it will on Windows if the file happened to
        // be open for read).
        persist_temp_file(temp_file, state_path.join("checkout"))
            .map_err(|err| wrap_err(err.into()))?;
        Ok(())
    }
}

pub struct LocalWorkingCopy {
    store: Arc<Store>,
    working_copy_path: PathBuf,
    state_path: PathBuf,
    checkout_state: CheckoutState,
    tree_state: OnceCell<TreeState>,
    tree_state_settings: TreeStateSettings,
}

impl WorkingCopy for LocalWorkingCopy {
    fn name(&self) -> &str {
        Self::name()
    }

    fn workspace_name(&self) -> &WorkspaceName {
        &self.checkout_state.workspace_name
    }

    fn operation_id(&self) -> &OperationId {
        &self.checkout_state.operation_id
    }

    fn tree(&self) -> Result<&MergedTree, WorkingCopyStateError> {
        Ok(self.tree_state()?.current_tree())
    }

    fn sparse_patterns(&self) -> Result<&[RepoPathBuf], WorkingCopyStateError> {
        Ok(self.tree_state()?.sparse_patterns())
    }

    fn start_mutation(&self) -> Result<Box<dyn LockedWorkingCopy>, WorkingCopyStateError> {
        let lock_path = self.state_path.join("working_copy.lock");
        let lock = FileLock::lock(lock_path).map_err(|err| WorkingCopyStateError {
            message: "Failed to lock working copy".to_owned(),
            err: err.into(),
        })?;

        let wc = Self {
            store: self.store.clone(),
            working_copy_path: self.working_copy_path.clone(),
            state_path: self.state_path.clone(),
            // Re-read the state after taking the lock
            checkout_state: CheckoutState::load(&self.state_path)?,
            // Empty so we re-read the state after taking the lock
            // TODO: It's expensive to reload the whole tree. We should copy it from `self` if it
            // hasn't changed.
            tree_state: OnceCell::new(),
            tree_state_settings: self.tree_state_settings.clone(),
        };
        let old_operation_id = wc.operation_id().clone();
        let old_tree = wc.tree()?.clone();
        Ok(Box::new(LockedLocalWorkingCopy {
            wc,
            old_operation_id,
            old_tree,
            tree_state_dirty: false,
            new_workspace_name: None,
            _lock: lock,
        }))
    }
}

impl LocalWorkingCopy {
    pub fn name() -> &'static str {
        "local"
    }

    /// Initializes a new working copy at `working_copy_path`. The working
    /// copy's state will be stored in the `state_path` directory. The working
    /// copy will have the empty tree checked out.
    pub fn init(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        operation_id: OperationId,
        workspace_name: WorkspaceNameBuf,
        user_settings: &UserSettings,
    ) -> Result<Self, WorkingCopyStateError> {
        let checkout_state = CheckoutState {
            operation_id,
            workspace_name,
        };
        checkout_state.save(&state_path)?;
        let tree_state_settings = TreeStateSettings::try_from_user_settings(user_settings)
            .map_err(|err| WorkingCopyStateError {
                message: "Failed to read the tree state settings".to_string(),
                err: err.into(),
            })?;
        let tree_state = TreeState::init(
            store.clone(),
            working_copy_path.clone(),
            state_path.clone(),
            &tree_state_settings,
        )
        .map_err(|err| WorkingCopyStateError {
            message: "Failed to initialize working copy state".to_string(),
            err: err.into(),
        })?;
        Ok(Self {
            store,
            working_copy_path,
            state_path,
            checkout_state,
            tree_state: OnceCell::with_value(tree_state),
            tree_state_settings,
        })
    }

    pub fn load(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        user_settings: &UserSettings,
    ) -> Result<Self, WorkingCopyStateError> {
        let checkout_state = CheckoutState::load(&state_path)?;
        let tree_state_settings = TreeStateSettings::try_from_user_settings(user_settings)
            .map_err(|err| WorkingCopyStateError {
                message: "Failed to read the tree state settings".to_string(),
                err: err.into(),
            })?;
        Ok(Self {
            store,
            working_copy_path,
            state_path,
            checkout_state,
            tree_state: OnceCell::new(),
            tree_state_settings,
        })
    }

    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    #[instrument(skip_all)]
    fn tree_state(&self) -> Result<&TreeState, WorkingCopyStateError> {
        self.tree_state.get_or_try_init(|| {
            TreeState::load(
                self.store.clone(),
                self.working_copy_path.clone(),
                self.state_path.clone(),
                &self.tree_state_settings,
            )
            .map_err(|err| WorkingCopyStateError {
                message: "Failed to read working copy state".to_string(),
                err: err.into(),
            })
        })
    }

    fn tree_state_mut(&mut self) -> Result<&mut TreeState, WorkingCopyStateError> {
        self.tree_state()?; // ensure loaded
        Ok(self.tree_state.get_mut().unwrap())
    }

    pub fn file_states(&self) -> Result<FileStates<'_>, WorkingCopyStateError> {
        Ok(self.tree_state()?.file_states())
    }

    #[cfg(feature = "watchman")]
    pub async fn query_watchman(
        &self,
        config: &WatchmanConfig,
    ) -> Result<(watchman::Clock, Option<Vec<PathBuf>>), WorkingCopyStateError> {
        self.tree_state()?
            .query_watchman(config)
            .await
            .map_err(|err| WorkingCopyStateError {
                message: "Failed to query watchman".to_string(),
                err: err.into(),
            })
    }

    #[cfg(feature = "watchman")]
    pub async fn is_watchman_trigger_registered(
        &self,
        config: &WatchmanConfig,
    ) -> Result<bool, WorkingCopyStateError> {
        self.tree_state()?
            .is_watchman_trigger_registered(config)
            .await
            .map_err(|err| WorkingCopyStateError {
                message: "Failed to query watchman".to_string(),
                err: err.into(),
            })
    }
}

pub struct LocalWorkingCopyFactory {}

impl WorkingCopyFactory for LocalWorkingCopyFactory {
    fn init_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        operation_id: OperationId,
        workspace_name: WorkspaceNameBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        Ok(Box::new(LocalWorkingCopy::init(
            store,
            working_copy_path,
            state_path,
            operation_id,
            workspace_name,
            settings,
        )?))
    }

    fn load_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        Ok(Box::new(LocalWorkingCopy::load(
            store,
            working_copy_path,
            state_path,
            settings,
        )?))
    }
}

/// A working copy that's locked on disk. The lock is held until you call
/// `finish()` or `discard()`.
pub struct LockedLocalWorkingCopy {
    wc: LocalWorkingCopy,
    old_operation_id: OperationId,
    old_tree: MergedTree,
    tree_state_dirty: bool,
    new_workspace_name: Option<WorkspaceNameBuf>,
    _lock: FileLock,
}

#[async_trait]
impl LockedWorkingCopy for LockedLocalWorkingCopy {
    fn old_operation_id(&self) -> &OperationId {
        &self.old_operation_id
    }

    fn old_tree(&self) -> &MergedTree {
        &self.old_tree
    }

    async fn snapshot(
        &mut self,
        options: &SnapshotOptions,
    ) -> Result<(MergedTree, SnapshotStats), SnapshotError> {
        let tree_state = self.wc.tree_state_mut()?;
        let (is_dirty, stats) = tree_state.snapshot(options).await?;
        self.tree_state_dirty |= is_dirty;
        Ok((tree_state.current_tree().clone(), stats))
    }

    async fn check_out(&mut self, commit: &Commit) -> Result<CheckoutStats, CheckoutError> {
        // TODO: Write a "pending_checkout" file with the new TreeId so we can
        // continue an interrupted update if we find such a file.
        let new_tree = commit.tree();
        let tree_state = self.wc.tree_state_mut()?;
        if tree_state.tree.tree_ids_and_labels() != new_tree.tree_ids_and_labels() {
            let stats = tree_state.check_out(&new_tree)?;
            self.tree_state_dirty = true;
            Ok(stats)
        } else {
            Ok(CheckoutStats::default())
        }
    }

    fn rename_workspace(&mut self, new_name: WorkspaceNameBuf) {
        self.new_workspace_name = Some(new_name);
    }

    async fn reset(&mut self, commit: &Commit) -> Result<(), ResetError> {
        let new_tree = commit.tree();
        self.wc.tree_state_mut()?.reset(&new_tree).await?;
        self.tree_state_dirty = true;
        Ok(())
    }

    async fn recover(&mut self, commit: &Commit) -> Result<(), ResetError> {
        let new_tree = commit.tree();
        self.wc.tree_state_mut()?.recover(&new_tree).await?;
        self.tree_state_dirty = true;
        Ok(())
    }

    fn sparse_patterns(&self) -> Result<&[RepoPathBuf], WorkingCopyStateError> {
        self.wc.sparse_patterns()
    }

    async fn set_sparse_patterns(
        &mut self,
        new_sparse_patterns: Vec<RepoPathBuf>,
    ) -> Result<CheckoutStats, CheckoutError> {
        // TODO: Write a "pending_checkout" file with new sparse patterns so we can
        // continue an interrupted update if we find such a file.
        let stats = self
            .wc
            .tree_state_mut()?
            .set_sparse_patterns(new_sparse_patterns)?;
        self.tree_state_dirty = true;
        Ok(stats)
    }

    #[instrument(skip_all)]
    async fn finish(
        mut self: Box<Self>,
        operation_id: OperationId,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        assert!(
            self.tree_state_dirty
                || self.old_tree.tree_ids_and_labels() == self.wc.tree()?.tree_ids_and_labels()
        );
        if self.tree_state_dirty {
            self.wc
                .tree_state_mut()?
                .save()
                .map_err(|err| WorkingCopyStateError {
                    message: "Failed to write working copy state".to_string(),
                    err: Box::new(err),
                })?;
        }
        if self.old_operation_id != operation_id || self.new_workspace_name.is_some() {
            self.wc.checkout_state.operation_id = operation_id;
            if let Some(workspace_name) = self.new_workspace_name {
                self.wc.checkout_state.workspace_name = workspace_name;
            }
            self.wc.checkout_state.save(&self.wc.state_path)?;
        }
        // TODO: Clear the "pending_checkout" file here.
        Ok(Box::new(self.wc))
    }
}

impl LockedLocalWorkingCopy {
    pub fn reset_watchman(&mut self) -> Result<(), SnapshotError> {
        self.wc.tree_state_mut()?.reset_watchman();
        self.tree_state_dirty = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashset;

    use super::*;

    fn repo_path(value: &str) -> &RepoPath {
        RepoPath::from_internal_string(value).unwrap()
    }

    fn repo_path_component(value: &str) -> &RepoPathComponent {
        RepoPathComponent::new(value).unwrap()
    }

    fn new_state(size: u64) -> FileState {
        FileState {
            file_type: FileType::Normal {
                exec_bit: ExecBit(false),
            },
            mtime: MillisSinceEpoch(0),
            size,
            materialized_conflict_data: None,
        }
    }

    #[test]
    fn test_file_states_merge() {
        let new_static_entry = |path: &'static str, size| (repo_path(path), new_state(size));
        let new_owned_entry = |path: &str, size| (repo_path(path).to_owned(), new_state(size));
        let new_proto_entry = |path: &str, size| {
            file_state_entry_to_proto(repo_path(path).to_owned(), &new_state(size))
        };
        let data = vec![
            new_proto_entry("aa", 0),
            new_proto_entry("b#", 4), // '#' < '/'
            new_proto_entry("b/c", 1),
            new_proto_entry("b/d/e", 2),
            new_proto_entry("b/e", 3),
            new_proto_entry("bc", 5),
        ];
        let mut file_states = FileStatesMap::from_proto(data, false);

        let changed_file_states = vec![
            new_owned_entry("aa", 10),    // change
            new_owned_entry("b/d/f", 11), // add
            new_owned_entry("b/e", 12),   // change
            new_owned_entry("c", 13),     // add
        ];
        let deleted_files = hashset! {
            repo_path("b/c").to_owned(),
            repo_path("b#").to_owned(),
        };
        file_states.merge_in(changed_file_states, &deleted_files);
        assert_eq!(
            file_states.all().iter().collect_vec(),
            vec![
                new_static_entry("aa", 10),
                new_static_entry("b/d/e", 2),
                new_static_entry("b/d/f", 11),
                new_static_entry("b/e", 12),
                new_static_entry("bc", 5),
                new_static_entry("c", 13),
            ],
        );
    }

    #[test]
    fn test_file_states_lookup() {
        let new_proto_entry = |path: &str, size| {
            file_state_entry_to_proto(repo_path(path).to_owned(), &new_state(size))
        };
        let data = vec![
            new_proto_entry("aa", 0),
            new_proto_entry("b/c", 1),
            new_proto_entry("b/d/e", 2),
            new_proto_entry("b/e", 3),
            new_proto_entry("b#", 4), // '#' < '/'
            new_proto_entry("bc", 5),
        ];
        let file_states = FileStates::from_sorted(&data);

        assert_eq!(
            file_states.prefixed(repo_path("")).paths().collect_vec(),
            ["aa", "b/c", "b/d/e", "b/e", "b#", "bc"].map(repo_path)
        );
        assert!(file_states.prefixed(repo_path("a")).is_empty());
        assert_eq!(
            file_states.prefixed(repo_path("aa")).paths().collect_vec(),
            ["aa"].map(repo_path)
        );
        assert_eq!(
            file_states.prefixed(repo_path("b")).paths().collect_vec(),
            ["b/c", "b/d/e", "b/e"].map(repo_path)
        );
        assert_eq!(
            file_states.prefixed(repo_path("b/d")).paths().collect_vec(),
            ["b/d/e"].map(repo_path)
        );
        assert_eq!(
            file_states.prefixed(repo_path("b#")).paths().collect_vec(),
            ["b#"].map(repo_path)
        );
        assert_eq!(
            file_states.prefixed(repo_path("bc")).paths().collect_vec(),
            ["bc"].map(repo_path)
        );
        assert!(file_states.prefixed(repo_path("z")).is_empty());

        assert!(!file_states.contains_path(repo_path("a")));
        assert!(file_states.contains_path(repo_path("aa")));
        assert!(file_states.contains_path(repo_path("b/d/e")));
        assert!(!file_states.contains_path(repo_path("b/d")));
        assert!(file_states.contains_path(repo_path("b#")));
        assert!(file_states.contains_path(repo_path("bc")));
        assert!(!file_states.contains_path(repo_path("z")));

        assert_eq!(file_states.get(repo_path("a")), None);
        assert_eq!(file_states.get(repo_path("aa")), Some(new_state(0)));
        assert_eq!(file_states.get(repo_path("b/d/e")), Some(new_state(2)));
        assert_eq!(file_states.get(repo_path("bc")), Some(new_state(5)));
        assert_eq!(file_states.get(repo_path("z")), None);
    }

    #[test]
    fn test_file_states_lookup_at() {
        let new_proto_entry = |path: &str, size| {
            file_state_entry_to_proto(repo_path(path).to_owned(), &new_state(size))
        };
        let data = vec![
            new_proto_entry("b/c", 0),
            new_proto_entry("b/d/e", 1),
            new_proto_entry("b/d#", 2), // '#' < '/'
            new_proto_entry("b/e", 3),
            new_proto_entry("b#", 4), // '#' < '/'
        ];
        let file_states = FileStates::from_sorted(&data);

        // At root
        assert_eq!(
            file_states.get_at(RepoPath::root(), repo_path_component("b")),
            None
        );
        assert_eq!(
            file_states.get_at(RepoPath::root(), repo_path_component("b#")),
            Some(new_state(4))
        );

        // At prefixed dir
        let prefixed_states = file_states.prefixed_at(RepoPath::root(), repo_path_component("b"));
        assert_eq!(
            prefixed_states.paths().collect_vec(),
            ["b/c", "b/d/e", "b/d#", "b/e"].map(repo_path)
        );
        assert_eq!(
            prefixed_states.get_at(repo_path("b"), repo_path_component("c")),
            Some(new_state(0))
        );
        assert_eq!(
            prefixed_states.get_at(repo_path("b"), repo_path_component("d")),
            None
        );
        assert_eq!(
            prefixed_states.get_at(repo_path("b"), repo_path_component("d#")),
            Some(new_state(2))
        );

        // At nested prefixed dir
        let prefixed_states = prefixed_states.prefixed_at(repo_path("b"), repo_path_component("d"));
        assert_eq!(
            prefixed_states.paths().collect_vec(),
            ["b/d/e"].map(repo_path)
        );
        assert_eq!(
            prefixed_states.get_at(repo_path("b/d"), repo_path_component("e")),
            Some(new_state(1))
        );
        assert_eq!(
            prefixed_states.get_at(repo_path("b/d"), repo_path_component("#")),
            None
        );

        // At prefixed file
        let prefixed_states = file_states.prefixed_at(RepoPath::root(), repo_path_component("b#"));
        assert_eq!(prefixed_states.paths().collect_vec(), ["b#"].map(repo_path));
        assert_eq!(
            prefixed_states.get_at(repo_path("b#"), repo_path_component("#")),
            None
        );
    }
}
