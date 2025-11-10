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

use std::any::Any;
use std::fmt::Debug;
use std::pin::Pin;
use std::slice;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::TimeZone as _;
use futures::stream::BoxStream;
use thiserror::Error;
use tokio::io::AsyncRead;

use crate::content_hash::ContentHash;
use crate::hex_util;
use crate::index::Index;
use crate::merge::Merge;
use crate::object_id::ObjectId as _;
use crate::object_id::id_type;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;
use crate::repo_path::RepoPathComponentBuf;
use crate::signing::SignResult;

id_type!(
    /// Identifier for a [`Commit`] based on its content. When a commit is
    /// rewritten, its `CommitId` changes.
    pub CommitId { hex() }
);
id_type!(
    /// Stable identifier for a [`Commit`]. Unlike the `CommitId`, the `ChangeId`
    /// follows the commit and is not updated when the commit is rewritten.
    pub ChangeId { reverse_hex() }
);
id_type!(pub TreeId { hex() });
id_type!(pub FileId { hex() });
id_type!(pub SymlinkId { hex() });
id_type!(pub CopyId { hex() });

impl ChangeId {
    /// Parses the given "reverse" hex string into a `ChangeId`.
    pub fn try_from_reverse_hex(hex: impl AsRef<[u8]>) -> Option<Self> {
        hex_util::decode_reverse_hex(hex).map(Self)
    }

    /// Returns the hex string representation of this ID, which uses `z-k`
    /// "digits" instead of `0-9a-f`.
    pub fn reverse_hex(&self) -> String {
        hex_util::encode_reverse_hex(&self.0)
    }
}

impl CopyId {
    /// Returns a placeholder copy id to be used when we don't have a real copy
    /// id yet.
    // TODO: Delete this
    pub fn placeholder() -> Self {
        Self::new(vec![])
    }
}

#[derive(Debug, Error)]
#[error("Out-of-range date")]
pub struct TimestampOutOfRange;

#[derive(ContentHash, Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct MillisSinceEpoch(pub i64);

#[derive(ContentHash, Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct Timestamp {
    pub timestamp: MillisSinceEpoch,
    // time zone offset in minutes
    pub tz_offset: i32,
}

impl Timestamp {
    pub fn now() -> Self {
        Self::from_datetime(chrono::offset::Local::now())
    }

    pub fn from_datetime<Tz: chrono::TimeZone<Offset = chrono::offset::FixedOffset>>(
        datetime: chrono::DateTime<Tz>,
    ) -> Self {
        Self {
            timestamp: MillisSinceEpoch(datetime.timestamp_millis()),
            tz_offset: datetime.offset().local_minus_utc() / 60,
        }
    }

    pub fn to_datetime(
        &self,
    ) -> Result<chrono::DateTime<chrono::FixedOffset>, TimestampOutOfRange> {
        let utc = match chrono::Utc.timestamp_opt(
            self.timestamp.0.div_euclid(1000),
            (self.timestamp.0.rem_euclid(1000)) as u32 * 1000000,
        ) {
            chrono::LocalResult::None => {
                return Err(TimestampOutOfRange);
            }
            chrono::LocalResult::Single(x) => x,
            chrono::LocalResult::Ambiguous(y, _z) => y,
        };

        Ok(utc.with_timezone(
            &chrono::FixedOffset::east_opt(self.tz_offset * 60)
                .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap()),
        ))
    }
}

impl serde::Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // TODO: test is_human_readable() to use raw format?
        let t = self.to_datetime().map_err(serde::ser::Error::custom)?;
        t.serialize(serializer)
    }
}

/// Represents a [`Commit`] signature.
#[derive(ContentHash, Debug, PartialEq, Eq, Clone, serde::Serialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub timestamp: Timestamp,
}

/// Represents a cryptographically signed [`Commit`] signature.
#[derive(ContentHash, Debug, PartialEq, Eq, Clone)]
pub struct SecureSig {
    pub data: Vec<u8>,
    pub sig: Vec<u8>,
}

pub type SigningFn<'a> = dyn FnMut(&[u8]) -> SignResult<Vec<u8>> + Send + 'a;

#[derive(ContentHash, Debug, PartialEq, Eq, Clone, serde::Serialize)]
pub struct Commit {
    pub parents: Vec<CommitId>,
    // TODO: delete commit.predecessors when we can assume that most commits are
    // tracked by op.commit_predecessors. (in jj 0.42 or so?)
    #[serde(skip)] // deprecated
    pub predecessors: Vec<CommitId>,
    #[serde(skip)] // TODO: should be exposed?
    pub root_tree: Merge<TreeId>,
    pub change_id: ChangeId,
    pub description: String,
    pub author: Signature,
    pub committer: Signature,
    #[serde(skip)] // raw data wouldn't be useful
    pub secure_sig: Option<SecureSig>,
}

/// An individual copy event, from file A -> B.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CopyRecord {
    /// The destination of the copy, B.
    pub target: RepoPathBuf,
    /// The CommitId where the copy took place.
    pub target_commit: CommitId,
    /// The source path a target was copied from.
    ///
    /// It is not required that the source path is different than the target
    /// path. A custom backend may choose to represent 'rollbacks' as copies
    /// from a file unto itself, from a specific prior commit.
    pub source: RepoPathBuf,
    pub source_file: FileId,
    /// The source commit the target was copied from. Backends may use this
    /// field to implement 'integration' logic, where a source may be
    /// periodically merged into a target, similar to a branch, but the
    /// branching occurs at the file level rather than the repository level. It
    /// also follows naturally that any copy source targeted to a specific
    /// commit should avoid copy propagation on rebasing, which is desirable
    /// for 'fork' style copies.
    ///
    /// It is required that the commit id is an ancestor of the commit with
    /// which this copy source is associated.
    pub source_commit: CommitId,
}

/// Describes the copy history of a file. The copy object is unchanged when a
/// file is modified.
#[derive(ContentHash, Debug, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub struct CopyHistory {
    /// The file's current path.
    pub current_path: RepoPathBuf,
    /// IDs of the files that became the current incarnation of this file.
    ///
    /// A newly created file has no parents. A regular copy or rename has one
    /// parent. A merge of multiple files has multiple parents.
    pub parents: Vec<CopyId>,
    /// An optional piece of data to give the Copy object a different ID. May be
    /// randomly generated. This allows a commit to say that a file was replaced
    /// by a new incarnation of it, indicating a logically distinct file
    /// taking the place of the previous file at the path.
    pub salt: Vec<u8>,
}

/// Error that may occur during backend initialization.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct BackendInitError(pub Box<dyn std::error::Error + Send + Sync>);

/// Error that may occur during backend loading.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct BackendLoadError(pub Box<dyn std::error::Error + Send + Sync>);

/// Commit-backend error that may occur after the backend is loaded.
#[derive(Debug, Error)]
pub enum BackendError {
    #[error(
        "Invalid hash length for object of type {object_type} (expected {expected} bytes, got \
         {actual} bytes): {hash}"
    )]
    InvalidHashLength {
        expected: usize,
        actual: usize,
        object_type: String,
        hash: String,
    },
    #[error("Invalid UTF-8 for object {hash} of type {object_type}")]
    InvalidUtf8 {
        object_type: String,
        hash: String,
        source: std::str::Utf8Error,
    },
    #[error("Object {hash} of type {object_type} not found")]
    ObjectNotFound {
        object_type: String,
        hash: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Error when reading object {hash} of type {object_type}")]
    ReadObject {
        object_type: String,
        hash: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Access denied to read object {hash} of type {object_type}")]
    ReadAccessDenied {
        object_type: String,
        hash: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(
        "Error when reading file content for file {path} with id {id}",
        path = path.as_internal_file_string()
    )]
    ReadFile {
        path: RepoPathBuf,
        id: FileId,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Could not write object of type {object_type}")]
    WriteObject {
        object_type: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
    /// A valid operation attempted, but failed because it isn't supported by
    /// the particular backend.
    #[error("{0}")]
    Unsupported(String),
}

pub type BackendResult<T> = Result<T, BackendError>;

#[derive(ContentHash, Debug, PartialEq, Eq, Clone, Hash)]
pub enum TreeValue {
    // TODO: When there's a CopyId here, the copy object's path must match
    // the path identified by the tree.
    File {
        id: FileId,
        executable: bool,
        copy_id: CopyId,
    },
    Symlink(SymlinkId),
    Tree(TreeId),
    GitSubmodule(CommitId),
}

impl TreeValue {
    pub fn hex(&self) -> String {
        match self {
            Self::File { id, .. } => id.hex(),
            Self::Symlink(id) => id.hex(),
            Self::Tree(id) => id.hex(),
            Self::GitSubmodule(id) => id.hex(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TreeEntry<'a> {
    name: &'a RepoPathComponent,
    value: &'a TreeValue,
}

impl<'a> TreeEntry<'a> {
    pub fn new(name: &'a RepoPathComponent, value: &'a TreeValue) -> Self {
        Self { name, value }
    }

    pub fn name(&self) -> &'a RepoPathComponent {
        self.name
    }

    pub fn value(&self) -> &'a TreeValue {
        self.value
    }
}

pub struct TreeEntriesNonRecursiveIterator<'a> {
    iter: slice::Iter<'a, (RepoPathComponentBuf, TreeValue)>,
}

impl<'a> Iterator for TreeEntriesNonRecursiveIterator<'a> {
    type Item = TreeEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|(name, value)| TreeEntry { name, value })
    }
}

#[derive(ContentHash, Default, PartialEq, Eq, Debug, Clone)]
pub struct Tree {
    entries: Vec<(RepoPathComponentBuf, TreeValue)>,
}

impl Tree {
    pub fn from_sorted_entries(entries: Vec<(RepoPathComponentBuf, TreeValue)>) -> Self {
        debug_assert!(entries.is_sorted_by(|(a, _), (b, _)| a < b));
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &RepoPathComponent> {
        self.entries.iter().map(|(name, _)| name.as_ref())
    }

    pub fn entries(&self) -> TreeEntriesNonRecursiveIterator<'_> {
        TreeEntriesNonRecursiveIterator {
            iter: self.entries.iter(),
        }
    }

    pub fn entry(&self, name: &RepoPathComponent) -> Option<TreeEntry<'_>> {
        let index = self
            .entries
            .binary_search_by_key(&name, |(name, _)| name)
            .ok()?;
        let (name, value) = &self.entries[index];
        Some(TreeEntry { name, value })
    }

    pub fn value(&self, name: &RepoPathComponent) -> Option<&TreeValue> {
        self.entry(name).map(|entry| entry.value)
    }
}

pub fn make_root_commit(root_change_id: ChangeId, empty_tree_id: TreeId) -> Commit {
    let timestamp = Timestamp {
        timestamp: MillisSinceEpoch(0),
        tz_offset: 0,
    };
    let signature = Signature {
        name: String::new(),
        email: String::new(),
        timestamp,
    };
    Commit {
        parents: vec![],
        predecessors: vec![],
        root_tree: Merge::resolved(empty_tree_id),
        change_id: root_change_id,
        description: String::new(),
        author: signature.clone(),
        committer: signature,
        secure_sig: None,
    }
}

/// Defines the interface for commit backends.
#[async_trait]
pub trait Backend: Any + Send + Sync + Debug {
    /// A unique name that identifies this backend. Written to
    /// `.jj/repo/store/type` when the repo is created.
    fn name(&self) -> &str;

    /// The length of commit IDs in bytes.
    fn commit_id_length(&self) -> usize;

    /// The length of change IDs in bytes.
    fn change_id_length(&self) -> usize;

    fn root_commit_id(&self) -> &CommitId;

    fn root_change_id(&self) -> &ChangeId;

    fn empty_tree_id(&self) -> &TreeId;

    /// An estimate of how many concurrent requests this backend handles well. A
    /// local backend like the Git backend (at until it supports partial clones)
    /// may want to set this to 1. A cloud-backed backend may want to set it to
    /// 100 or so.
    ///
    /// It is not guaranteed that at most this number of concurrent requests are
    /// sent.
    fn concurrency(&self) -> usize;

    async fn read_file(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>>;

    async fn write_file(
        &self,
        path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> BackendResult<FileId>;

    async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> BackendResult<String>;

    async fn write_symlink(&self, path: &RepoPath, target: &str) -> BackendResult<SymlinkId>;

    /// Read the specified `CopyHistory` object.
    ///
    /// Backends that don't support copy tracking may return
    /// `BackendError::Unsupported`.
    async fn read_copy(&self, id: &CopyId) -> BackendResult<CopyHistory>;

    /// Write the `CopyHistory` object and return its ID.
    ///
    /// Backends that don't support copy tracking may return
    /// `BackendError::Unsupported`.
    async fn write_copy(&self, copy: &CopyHistory) -> BackendResult<CopyId>;

    /// Find all copy histories that are related to the specified one. This is
    /// defined as those that are ancestors of the given specified one, plus
    /// their descendants. Children must be returned before parents.
    ///
    /// It is valid (but wasteful) to include other copy histories, such as
    /// siblings, or even completely unrelated copy histories.
    ///
    /// Backends that don't support copy tracking may return
    /// `BackendError::Unsupported`.
    async fn get_related_copies(&self, copy_id: &CopyId) -> BackendResult<Vec<CopyHistory>>;

    async fn read_tree(&self, path: &RepoPath, id: &TreeId) -> BackendResult<Tree>;

    async fn write_tree(&self, path: &RepoPath, contents: &Tree) -> BackendResult<TreeId>;

    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit>;

    /// Writes a commit and returns its ID and the commit itself. The commit
    /// should contain the data that was actually written, which may differ
    /// from the data passed in. For example, the backend may change the
    /// committer name to an authenticated user's name, or the backend's
    /// timestamps may have less precision than the millisecond precision in
    /// `Commit`.
    ///
    /// The `sign_with` parameter could contain a function to cryptographically
    /// sign some binary representation of the commit.
    /// If the backend supports it, it could call it and store the result in
    /// an implementation specific fashion, and both `read_commit` and the
    /// return of `write_commit` should read it back as the `secure_sig`
    /// field.
    async fn write_commit(
        &self,
        contents: Commit,
        sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<(CommitId, Commit)>;

    /// Get copy records for the dag range `root..head`.  If `paths` is None
    /// include all paths, otherwise restrict to only `paths`.
    ///
    /// The exact order these are returned is unspecified, but it is guaranteed
    /// to be reverse-topological. That is, for any two copy records with
    /// different commit ids A and B, if A is an ancestor of B, A is streamed
    /// after B.
    ///
    /// Streaming by design to better support large backends which may have very
    /// large single-file histories. This also allows more iterative algorithms
    /// like blame/annotate to short-circuit after a point without wasting
    /// unnecessary resources.
    fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> BackendResult<BoxStream<'_, BackendResult<CopyRecord>>>;

    /// Perform garbage collection.
    ///
    /// All commits found in the `index` won't be removed. In addition to that,
    /// objects created after `keep_newer` will be preserved. This mitigates a
    /// risk of deleting new commits created concurrently by another process.
    fn gc(&self, index: &dyn Index, keep_newer: SystemTime) -> BackendResult<()>;
}

impl dyn Backend {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: Backend>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}
