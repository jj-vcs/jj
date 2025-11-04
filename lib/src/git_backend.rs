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

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::fs;
use std::io;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Command;
use std::process::ExitStatus;
use std::str::Utf8Error;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::SystemTime;

use async_trait::async_trait;
use futures::stream::BoxStream;
use gix::bstr::BString;
use gix::objs::CommitRefIter;
use gix::objs::WriteTo as _;
use itertools::Itertools as _;
use once_cell::sync::OnceCell as OnceLock;
use pollster::FutureExt as _;
use prost::Message as _;
use smallvec::SmallVec;
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

use crate::backend::Backend;
use crate::backend::BackendError;
use crate::backend::BackendInitError;
use crate::backend::BackendLoadError;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::Commit;
use crate::backend::CommitId;
use crate::backend::CopyHistory;
use crate::backend::CopyId;
use crate::backend::CopyRecord;
use crate::backend::FileId;
use crate::backend::MillisSinceEpoch;
use crate::backend::SecureSig;
use crate::backend::Signature;
use crate::backend::SigningFn;
use crate::backend::SymlinkId;
use crate::backend::Timestamp;
use crate::backend::Tree;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::backend::make_root_commit;
use crate::config::ConfigGetError;
use crate::file_util;
use crate::file_util::BadPathEncoding;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::git::GitSettings;
use crate::index::Index;
use crate::lock::FileLock;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::object_id::ObjectId;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::settings::UserSettings;
use crate::stacked_table::MutableTable;
use crate::stacked_table::ReadonlyTable;
use crate::stacked_table::TableSegment as _;
use crate::stacked_table::TableStore;
use crate::stacked_table::TableStoreError;

const HASH_LENGTH: usize = 20;
const CHANGE_ID_LENGTH: usize = 16;
/// Ref namespace used only for preventing GC.
const NO_GC_REF_NAMESPACE: &str = "refs/jj/keep/";

pub const JJ_TREES_COMMIT_HEADER: &str = "jj:trees";
pub const CHANGE_ID_COMMIT_HEADER: &str = "change-id";

#[derive(Debug, Error)]
pub enum GitBackendInitError {
    #[error("Failed to initialize git repository")]
    InitRepository(#[source] gix::init::Error),
    #[error("Failed to open git repository")]
    OpenRepository(#[source] gix::open::Error),
    #[error("Failed to encode git repository path")]
    EncodeRepositoryPath(#[source] BadPathEncoding),
    #[error(transparent)]
    Config(ConfigGetError),
    #[error(transparent)]
    Path(PathError),
}

impl From<Box<GitBackendInitError>> for BackendInitError {
    fn from(err: Box<GitBackendInitError>) -> Self {
        Self(err)
    }
}

#[derive(Debug, Error)]
pub enum GitBackendLoadError {
    #[error("Failed to open git repository")]
    OpenRepository(#[source] gix::open::Error),
    #[error("Failed to decode git repository path")]
    DecodeRepositoryPath(#[source] BadPathEncoding),
    #[error(transparent)]
    Config(ConfigGetError),
    #[error(transparent)]
    Path(PathError),
}

impl From<Box<GitBackendLoadError>> for BackendLoadError {
    fn from(err: Box<GitBackendLoadError>) -> Self {
        Self(err)
    }
}

/// `GitBackend`-specific error that may occur after the backend is loaded.
#[derive(Debug, Error)]
pub enum GitBackendError {
    #[error("Failed to read non-git metadata")]
    ReadMetadata(#[source] TableStoreError),
    #[error("Failed to write non-git metadata")]
    WriteMetadata(#[source] TableStoreError),
}

impl From<GitBackendError> for BackendError {
    fn from(err: GitBackendError) -> Self {
        Self::Other(err.into())
    }
}

#[derive(Debug, Error)]
pub enum GitGcError {
    #[error("Failed to run git gc command")]
    GcCommand(#[source] std::io::Error),
    #[error("git gc command exited with an error: {0}")]
    GcCommandErrorStatus(ExitStatus),
}

pub struct GitBackend {
    // While gix::Repository can be created from gix::ThreadSafeRepository, it's
    // cheaper to cache the thread-local instance behind a mutex than creating
    // one for each backend method call. Our GitBackend is most likely to be
    // used in a single-threaded context.
    base_repo: gix::ThreadSafeRepository,
    repo: Mutex<gix::Repository>,
    root_commit_id: CommitId,
    root_change_id: ChangeId,
    empty_tree_id: TreeId,
    shallow_root_ids: OnceLock<Vec<CommitId>>,
    extra_metadata_store: TableStore,
    cached_extra_metadata: Mutex<Option<Arc<ReadonlyTable>>>,
    git_executable: PathBuf,
    write_change_id_header: bool,
}

impl GitBackend {
    pub fn name() -> &'static str {
        "git"
    }

    fn new(
        base_repo: gix::ThreadSafeRepository,
        extra_metadata_store: TableStore,
        git_settings: GitSettings,
    ) -> Self {
        let repo = Mutex::new(base_repo.to_thread_local());
        let root_commit_id = CommitId::from_bytes(&[0; HASH_LENGTH]);
        let root_change_id = ChangeId::from_bytes(&[0; CHANGE_ID_LENGTH]);
        let empty_tree_id = TreeId::from_hex("4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        Self {
            base_repo,
            repo,
            root_commit_id,
            root_change_id,
            empty_tree_id,
            shallow_root_ids: OnceLock::new(),
            extra_metadata_store,
            cached_extra_metadata: Mutex::new(None),
            git_executable: git_settings.executable_path,
            write_change_id_header: git_settings.write_change_id_header,
        }
    }

    pub fn init_internal(
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Self, Box<GitBackendInitError>> {
        let git_repo_path = Path::new("git");
        let git_repo = gix::ThreadSafeRepository::init_opts(
            store_path.join(git_repo_path),
            gix::create::Kind::Bare,
            gix::create::Options::default(),
            gix_open_opts_from_settings(settings),
        )
        .map_err(GitBackendInitError::InitRepository)?;
        let git_settings =
            GitSettings::from_settings(settings).map_err(GitBackendInitError::Config)?;
        Self::init_with_repo(store_path, git_repo_path, git_repo, git_settings)
    }

    /// Initializes backend by creating a new Git repo at the specified
    /// workspace path. The workspace directory must exist.
    pub fn init_colocated(
        settings: &UserSettings,
        store_path: &Path,
        workspace_root: &Path,
    ) -> Result<Self, Box<GitBackendInitError>> {
        let canonical_workspace_root = {
            let path = store_path.join(workspace_root);
            dunce::canonicalize(&path)
                .context(&path)
                .map_err(GitBackendInitError::Path)?
        };
        let git_repo = gix::ThreadSafeRepository::init_opts(
            canonical_workspace_root,
            gix::create::Kind::WithWorktree,
            gix::create::Options::default(),
            gix_open_opts_from_settings(settings),
        )
        .map_err(GitBackendInitError::InitRepository)?;
        let git_repo_path = workspace_root.join(".git");
        let git_settings =
            GitSettings::from_settings(settings).map_err(GitBackendInitError::Config)?;
        Self::init_with_repo(store_path, &git_repo_path, git_repo, git_settings)
    }

    /// Initializes backend with an existing Git repo at the specified path.
    pub fn init_external(
        settings: &UserSettings,
        store_path: &Path,
        git_repo_path: &Path,
    ) -> Result<Self, Box<GitBackendInitError>> {
        let canonical_git_repo_path = {
            let path = store_path.join(git_repo_path);
            canonicalize_git_repo_path(&path)
                .context(&path)
                .map_err(GitBackendInitError::Path)?
        };
        let git_repo = gix::ThreadSafeRepository::open_opts(
            canonical_git_repo_path,
            gix_open_opts_from_settings(settings),
        )
        .map_err(GitBackendInitError::OpenRepository)?;
        let git_settings =
            GitSettings::from_settings(settings).map_err(GitBackendInitError::Config)?;
        Self::init_with_repo(store_path, git_repo_path, git_repo, git_settings)
    }

    fn init_with_repo(
        store_path: &Path,
        git_repo_path: &Path,
        repo: gix::ThreadSafeRepository,
        git_settings: GitSettings,
    ) -> Result<Self, Box<GitBackendInitError>> {
        let extra_path = store_path.join("extra");
        fs::create_dir(&extra_path)
            .context(&extra_path)
            .map_err(GitBackendInitError::Path)?;
        let target_path = store_path.join("git_target");
        let git_repo_path = if cfg!(windows) && git_repo_path.is_relative() {
            // When a repository is created in Windows, format the path with *forward
            // slashes* and not backwards slashes. This makes it possible to use the same
            // repository under Windows Subsystem for Linux.
            //
            // This only works for relative paths. If the path is absolute, there's not much
            // we can do, and it simply won't work inside and outside WSL at the same time.
            file_util::slash_path(git_repo_path)
        } else {
            git_repo_path.into()
        };
        let git_repo_path_bytes = file_util::path_to_bytes(&git_repo_path)
            .map_err(GitBackendInitError::EncodeRepositoryPath)?;
        fs::write(&target_path, git_repo_path_bytes)
            .context(&target_path)
            .map_err(GitBackendInitError::Path)?;
        let extra_metadata_store = TableStore::init(extra_path, HASH_LENGTH);
        Ok(Self::new(repo, extra_metadata_store, git_settings))
    }

    pub fn load(
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Self, Box<GitBackendLoadError>> {
        let git_repo_path = {
            let target_path = store_path.join("git_target");
            let git_repo_path_bytes = fs::read(&target_path)
                .context(&target_path)
                .map_err(GitBackendLoadError::Path)?;
            let git_repo_path = file_util::path_from_bytes(&git_repo_path_bytes)
                .map_err(GitBackendLoadError::DecodeRepositoryPath)?;
            let git_repo_path = store_path.join(git_repo_path);
            canonicalize_git_repo_path(&git_repo_path)
                .context(&git_repo_path)
                .map_err(GitBackendLoadError::Path)?
        };
        let repo = gix::ThreadSafeRepository::open_opts(
            git_repo_path,
            gix_open_opts_from_settings(settings),
        )
        .map_err(GitBackendLoadError::OpenRepository)?;
        let extra_metadata_store = TableStore::load(store_path.join("extra"), HASH_LENGTH);
        let git_settings =
            GitSettings::from_settings(settings).map_err(GitBackendLoadError::Config)?;
        Ok(Self::new(repo, extra_metadata_store, git_settings))
    }

    fn lock_git_repo(&self) -> MutexGuard<'_, gix::Repository> {
        self.repo.lock().unwrap()
    }

    /// Returns new thread-local instance to access to the underlying Git repo.
    pub fn git_repo(&self) -> gix::Repository {
        self.base_repo.to_thread_local()
    }

    /// Path to the `.git` directory or the repository itself if it's bare.
    pub fn git_repo_path(&self) -> &Path {
        self.base_repo.path()
    }

    /// Path to the working directory if the repository isn't bare.
    pub fn git_workdir(&self) -> Option<&Path> {
        self.base_repo.work_dir()
    }

    fn shallow_root_ids(&self, git_repo: &gix::Repository) -> BackendResult<&[CommitId]> {
        // The list of shallow roots is cached by gix, but it's still expensive
        // to stat file on every read_object() call. Refreshing shallow roots is
        // also bad for consistency reasons.
        self.shallow_root_ids
            .get_or_try_init(|| {
                let maybe_oids = git_repo
                    .shallow_commits()
                    .map_err(|err| BackendError::Other(err.into()))?;
                let commit_ids = maybe_oids.map_or(vec![], |oids| {
                    oids.iter()
                        .map(|oid| CommitId::from_bytes(oid.as_bytes()))
                        .collect()
                });
                Ok(commit_ids)
            })
            .map(AsRef::as_ref)
    }

    fn cached_extra_metadata_table(&self) -> BackendResult<Arc<ReadonlyTable>> {
        let mut locked_head = self.cached_extra_metadata.lock().unwrap();
        match locked_head.as_ref() {
            Some(head) => Ok(head.clone()),
            None => {
                let table = self
                    .extra_metadata_store
                    .get_head()
                    .map_err(GitBackendError::ReadMetadata)?;
                *locked_head = Some(table.clone());
                Ok(table)
            }
        }
    }

    fn read_extra_metadata_table_locked(&self) -> BackendResult<(Arc<ReadonlyTable>, FileLock)> {
        let table = self
            .extra_metadata_store
            .get_head_locked()
            .map_err(GitBackendError::ReadMetadata)?;
        Ok(table)
    }

    fn save_extra_metadata_table(
        &self,
        mut_table: MutableTable,
        _table_lock: &FileLock,
    ) -> BackendResult<()> {
        let table = self
            .extra_metadata_store
            .save_table(mut_table)
            .map_err(GitBackendError::WriteMetadata)?;
        // Since the parent table was the head, saved table are likely to be new head.
        // If it's not, cache will be reloaded when entry can't be found.
        *self.cached_extra_metadata.lock().unwrap() = Some(table);
        Ok(())
    }

    /// Imports the given commits and ancestors from the backing Git repo.
    ///
    /// The `head_ids` may contain commits that have already been imported, but
    /// the caller should filter them out to eliminate redundant I/O processing.
    #[tracing::instrument(skip(self, head_ids))]
    pub fn import_head_commits<'a>(
        &self,
        head_ids: impl IntoIterator<Item = &'a CommitId>,
    ) -> BackendResult<()> {
        let head_ids: HashSet<&CommitId> = head_ids
            .into_iter()
            .filter(|&id| *id != self.root_commit_id)
            .collect();
        if head_ids.is_empty() {
            return Ok(());
        }

        // Create no-gc ref even if known to the extras table. Concurrent GC
        // process might have deleted the no-gc ref.
        let locked_repo = self.lock_git_repo();
        locked_repo
            .edit_references(head_ids.iter().copied().map(to_no_gc_ref_update))
            .map_err(|err| BackendError::Other(Box::new(err)))?;

        // These commits are imported from Git. Make our change ids persist (otherwise
        // future write_commit() could reassign new change id.)
        tracing::debug!(
            heads_count = head_ids.len(),
            "import extra metadata entries"
        );
        let (table, table_lock) = self.read_extra_metadata_table_locked()?;
        let mut mut_table = table.start_mutation();
        import_extra_metadata_entries_from_heads(
            &locked_repo,
            &mut mut_table,
            &table_lock,
            &head_ids,
            self.shallow_root_ids(&locked_repo)?,
        )?;
        self.save_extra_metadata_table(mut_table, &table_lock)
    }

    fn read_file_sync(&self, id: &FileId) -> BackendResult<Vec<u8>> {
        let git_blob_id = validate_git_object_id(id)?;
        let locked_repo = self.lock_git_repo();
        let mut blob = locked_repo
            .find_object(git_blob_id)
            .map_err(|err| map_not_found_err(err, id))?
            .try_into_blob()
            .map_err(|err| to_read_object_err(err, id))?;
        Ok(blob.take_data())
    }

    fn new_diff_platform(&self) -> BackendResult<gix::diff::blob::Platform> {
        let attributes = gix::worktree::Stack::new(
            Path::new(""),
            gix::worktree::stack::State::AttributesStack(Default::default()),
            gix::worktree::glob::pattern::Case::Sensitive,
            Vec::new(),
            Vec::new(),
        );
        let filter = gix::diff::blob::Pipeline::new(
            Default::default(),
            gix::filter::plumbing::Pipeline::new(
                self.git_repo()
                    .command_context()
                    .map_err(|err| BackendError::Other(Box::new(err)))?,
                Default::default(),
            ),
            Vec::new(),
            Default::default(),
        );
        Ok(gix::diff::blob::Platform::new(
            Default::default(),
            filter,
            gix::diff::blob::pipeline::Mode::ToGit,
            attributes,
        ))
    }

    fn read_tree_for_commit<'repo>(
        &self,
        repo: &'repo gix::Repository,
        id: &CommitId,
    ) -> BackendResult<gix::Tree<'repo>> {
        let tree = self.read_commit(id).block_on()?.root_tree;
        // TODO(kfm): probably want to do something here if it is a merge
        let tree_id = tree.first().clone();
        let gix_id = validate_git_object_id(&tree_id)?;
        repo.find_object(gix_id)
            .map_err(|err| map_not_found_err(err, &tree_id))?
            .try_into_tree()
            .map_err(|err| to_read_object_err(err, &tree_id))
    }
}

/// Canonicalizes the given `path` except for the last `".git"` component.
///
/// The last path component matters when opening a Git repo without `core.bare`
/// config. This config is usually set, but the "repo" tool will set up such
/// repositories and symlinks. Opening such repo with fully-canonicalized path
/// would turn a colocated Git repo into a bare repo.
pub fn canonicalize_git_repo_path(path: &Path) -> io::Result<PathBuf> {
    if path.ends_with(".git") {
        let workdir = path.parent().unwrap();
        dunce::canonicalize(workdir).map(|dir| dir.join(".git"))
    } else {
        dunce::canonicalize(path)
    }
}

fn gix_open_opts_from_settings(settings: &UserSettings) -> gix::open::Options {
    let user_name = settings.user_name();
    let user_email = settings.user_email();
    gix::open::Options::default()
        .config_overrides([
            // Committer has to be configured to record reflog. Author isn't
            // needed, but let's copy the same values.
            format!("author.name={user_name}"),
            format!("author.email={user_email}"),
            format!("committer.name={user_name}"),
            format!("committer.email={user_email}"),
        ])
        // The git_target path should point the repository, not the working directory.
        .open_path_as_is(true)
        // Gitoxide recommends this when correctness is preferred
        .strict_config(true)
}

/// Parses the `jj:trees` header value if present, otherwise returns the
/// resolved tree ID from Git.
fn extract_root_tree_from_commit(commit: &gix::objs::CommitRef) -> Result<Merge<TreeId>, ()> {
    let Some(value) = commit.extra_headers().find(JJ_TREES_COMMIT_HEADER) else {
        let tree_id = TreeId::from_bytes(commit.tree().as_bytes());
        return Ok(Merge::resolved(tree_id));
    };

    let mut tree_ids = SmallVec::new();
    for hex in value.split(|b| *b == b' ') {
        let tree_id = TreeId::try_from_hex(hex).ok_or(())?;
        if tree_id.as_bytes().len() != HASH_LENGTH {
            return Err(());
        }
        tree_ids.push(tree_id);
    }
    // It is invalid to use `jj:trees` with a non-conflicted tree. If this were
    // allowed, it would be possible to construct a commit which appears to have
    // different contents depending on whether it is viewed using `jj` or `git`.
    if tree_ids.len() == 1 || tree_ids.len() % 2 == 0 {
        return Err(());
    }
    Ok(Merge::from_vec(tree_ids))
}

fn commit_from_git_without_root_parent(
    id: &CommitId,
    git_object: &gix::Object,
    is_shallow: bool,
) -> BackendResult<Commit> {
    let commit = git_object
        .try_to_commit_ref()
        .map_err(|err| to_read_object_err(err, id))?;

    // If the git header has a change-id field, we attempt to convert that to a
    // valid JJ Change Id
    let change_id = extract_change_id_from_commit(&commit)
        .unwrap_or_else(|| synthetic_change_id_from_git_commit_id(id));

    // shallow commits don't have parents their parents actually fetched, so we
    // discard them here
    // TODO: This causes issues when a shallow repository is deepened/unshallowed
    let parents = if is_shallow {
        vec![]
    } else {
        commit
            .parents()
            .map(|oid| CommitId::from_bytes(oid.as_bytes()))
            .collect_vec()
    };
    // Conflicted commits written before we started using the `jj:trees` header
    // (~March 2024) may have the root trees stored in the extra metadata table
    // instead. For such commits, we'll update the root tree later when we read the
    // extra metadata.
    let root_tree = extract_root_tree_from_commit(&commit)
        .map_err(|()| to_read_object_err("Invalid jj:trees header", id))?;
    // Use lossy conversion as commit message with "mojibake" is still better than
    // nothing.
    // TODO: what should we do with commit.encoding?
    let description = String::from_utf8_lossy(commit.message).into_owned();
    let author = signature_from_git(commit.author());
    let committer = signature_from_git(commit.committer());

    // If the commit is signed, extract both the signature and the signed data
    // (which is the commit buffer with the gpgsig header omitted).
    // We have to re-parse the raw commit data because gix CommitRef does not give
    // us the sogned data, only the signature.
    // Ideally, we could use try_to_commit_ref_iter at the beginning of this
    // function and extract everything from that. For now, this works
    let secure_sig = commit
        .extra_headers
        .iter()
        // gix does not recognize gpgsig-sha256, but prevent future footguns by checking for it too
        .any(|(k, _)| *k == "gpgsig" || *k == "gpgsig-sha256")
        .then(|| CommitRefIter::signature(&git_object.data))
        .transpose()
        .map_err(|err| to_read_object_err(err, id))?
        .flatten()
        .map(|(sig, data)| SecureSig {
            data: data.to_bstring().into(),
            sig: sig.into_owned().into(),
        });

    Ok(Commit {
        parents,
        predecessors: vec![],
        // If this commit has associated extra metadata, we may reset this later.
        root_tree,
        // TODO: store conflict labels
        conflict_labels: Merge::resolved(String::new()),
        change_id,
        description,
        author,
        committer,
        secure_sig,
    })
}

/// Extracts change id from commit headers.
pub fn extract_change_id_from_commit(commit: &gix::objs::CommitRef) -> Option<ChangeId> {
    commit
        .extra_headers()
        .find(CHANGE_ID_COMMIT_HEADER)
        .and_then(ChangeId::try_from_reverse_hex)
        .filter(|val| val.as_bytes().len() == CHANGE_ID_LENGTH)
}

/// Deterministically creates a change id based on the commit id
///
/// Used when we get a commit without a change id. The exact algorithm for the
/// computation should not be relied upon.
pub fn synthetic_change_id_from_git_commit_id(id: &CommitId) -> ChangeId {
    // We reverse the bits of the commit id to create the change id. We don't
    // want to use the first bytes unmodified because then it would be ambiguous
    // if a given hash prefix refers to the commit id or the change id. It would
    // have been enough to pick the last 16 bytes instead of the leading 16
    // bytes to address that. We also reverse the bits to make it less likely
    // that users depend on any relationship between the two ids.
    let bytes = id.as_bytes()[4..HASH_LENGTH]
        .iter()
        .rev()
        .map(|b| b.reverse_bits())
        .collect();
    ChangeId::new(bytes)
}

const EMPTY_STRING_PLACEHOLDER: &str = "JJ_EMPTY_STRING";

fn signature_from_git(signature: gix::actor::SignatureRef) -> Signature {
    let name = signature.name;
    let name = if name != EMPTY_STRING_PLACEHOLDER {
        String::from_utf8_lossy(name).into_owned()
    } else {
        "".to_string()
    };
    let email = signature.email;
    let email = if email != EMPTY_STRING_PLACEHOLDER {
        String::from_utf8_lossy(email).into_owned()
    } else {
        "".to_string()
    };
    let time = signature.time().unwrap_or_default();
    let timestamp = MillisSinceEpoch(time.seconds * 1000);
    let tz_offset = time.offset.div_euclid(60); // in minutes
    Signature {
        name,
        email,
        timestamp: Timestamp {
            timestamp,
            tz_offset,
        },
    }
}

fn signature_to_git(signature: &Signature) -> gix::actor::Signature {
    // git does not support empty names or emails
    let name = if !signature.name.is_empty() {
        &signature.name
    } else {
        EMPTY_STRING_PLACEHOLDER
    };
    let email = if !signature.email.is_empty() {
        &signature.email
    } else {
        EMPTY_STRING_PLACEHOLDER
    };
    let time = gix::date::Time::new(
        signature.timestamp.timestamp.0.div_euclid(1000),
        signature.timestamp.tz_offset * 60, // in seconds
    );
    gix::actor::Signature {
        name: name.into(),
        email: email.into(),
        time,
    }
}

fn serialize_extras(commit: &Commit) -> Vec<u8> {
    let mut proto = crate::protos::git_store::Commit {
        change_id: commit.change_id.to_bytes(),
        ..Default::default()
    };
    proto.uses_tree_conflict_format = true;
    if !commit.root_tree.is_resolved() {
        // This is done for the sake of jj versions <0.28 (before commit
        // f7b14be) being able to read the repo. At some point in the
        // future, we can stop doing it.
        proto.root_tree = commit.root_tree.iter().map(|r| r.to_bytes()).collect();
    }
    for predecessor in &commit.predecessors {
        proto.predecessors.push(predecessor.to_bytes());
    }
    proto.encode_to_vec()
}

fn deserialize_extras(commit: &mut Commit, bytes: &[u8]) {
    let proto = crate::protos::git_store::Commit::decode(bytes).unwrap();
    if !proto.change_id.is_empty() {
        commit.change_id = ChangeId::new(proto.change_id);
    }
    if commit.root_tree.is_resolved()
        && proto.uses_tree_conflict_format
        && !proto.root_tree.is_empty()
    {
        let merge_builder: MergeBuilder<_> = proto
            .root_tree
            .iter()
            .map(|id_bytes| TreeId::from_bytes(id_bytes))
            .collect();
        commit.root_tree = merge_builder.build();
    }
    for predecessor in &proto.predecessors {
        commit.predecessors.push(CommitId::from_bytes(predecessor));
    }
}

/// Returns `RefEdit` that will create a ref in `refs/jj/keep` if not exist.
/// Used for preventing GC of commits we create.
fn to_no_gc_ref_update(id: &CommitId) -> gix::refs::transaction::RefEdit {
    let name = format!("{NO_GC_REF_NAMESPACE}{id}");
    let new = gix::refs::Target::Object(gix::ObjectId::from_bytes_or_panic(id.as_bytes()));
    let expected = gix::refs::transaction::PreviousValue::ExistingMustMatch(new.clone());
    gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange {
                message: "used by jj".into(),
                ..Default::default()
            },
            expected,
            new,
        },
        name: name.try_into().unwrap(),
        deref: false,
    }
}

fn to_ref_deletion(git_ref: gix::refs::Reference) -> gix::refs::transaction::RefEdit {
    let expected = gix::refs::transaction::PreviousValue::ExistingMustMatch(git_ref.target);
    gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Delete {
            expected,
            log: gix::refs::transaction::RefLog::AndReference,
        },
        name: git_ref.name,
        deref: false,
    }
}

/// Recreates `refs/jj/keep` refs for the `new_heads`, and removes the other
/// unreachable and non-head refs.
fn recreate_no_gc_refs(
    git_repo: &gix::Repository,
    new_heads: impl IntoIterator<Item = CommitId>,
    keep_newer: SystemTime,
) -> BackendResult<()> {
    // Calculate diff between existing no-gc refs and new heads.
    let new_heads: HashSet<CommitId> = new_heads.into_iter().collect();
    let mut no_gc_refs_to_keep_count: usize = 0;
    let mut no_gc_refs_to_delete: Vec<gix::refs::Reference> = Vec::new();
    let git_references = git_repo
        .references()
        .map_err(|err| BackendError::Other(err.into()))?;
    let no_gc_refs_iter = git_references
        .prefixed(NO_GC_REF_NAMESPACE)
        .map_err(|err| BackendError::Other(err.into()))?;
    for git_ref in no_gc_refs_iter {
        let git_ref = git_ref.map_err(BackendError::Other)?.detach();
        let oid = git_ref.target.try_id().ok_or_else(|| {
            let name = git_ref.name.as_bstr();
            BackendError::Other(format!("Symbolic no-gc ref found: {name}").into())
        })?;
        let id = CommitId::from_bytes(oid.as_bytes());
        let name_good = git_ref.name.as_bstr()[NO_GC_REF_NAMESPACE.len()..] == id.hex();
        if new_heads.contains(&id) && name_good {
            no_gc_refs_to_keep_count += 1;
            continue;
        }
        // Check timestamp of loose ref, but this is still racy on re-import
        // because:
        // - existing packed ref won't be demoted to loose ref
        // - existing loose ref won't be touched
        //
        // TODO: might be better to switch to a dummy merge, where new no-gc ref
        // will always have a unique name. Doing that with the current
        // ref-per-head strategy would increase the number of the no-gc refs.
        // https://github.com/jj-vcs/jj/pull/2659#issuecomment-1837057782
        let loose_ref_path = git_repo.path().join(git_ref.name.to_path());
        if let Ok(metadata) = loose_ref_path.metadata() {
            let mtime = metadata.modified().expect("unsupported platform?");
            if mtime > keep_newer {
                tracing::trace!(?git_ref, "not deleting new");
                no_gc_refs_to_keep_count += 1;
                continue;
            }
        }
        // Also deletes no-gc ref of random name created by old jj.
        tracing::trace!(?git_ref, ?name_good, "will delete");
        no_gc_refs_to_delete.push(git_ref);
    }
    tracing::info!(
        new_heads_count = new_heads.len(),
        no_gc_refs_to_keep_count,
        no_gc_refs_to_delete_count = no_gc_refs_to_delete.len(),
        "collected reachable refs"
    );

    // It's slow to delete packed refs one by one, so update refs all at once.
    let ref_edits = itertools::chain(
        no_gc_refs_to_delete.into_iter().map(to_ref_deletion),
        new_heads.iter().map(to_no_gc_ref_update),
    );
    git_repo
        .edit_references(ref_edits)
        .map_err(|err| BackendError::Other(err.into()))?;

    Ok(())
}

fn run_git_gc(program: &OsStr, git_dir: &Path, keep_newer: SystemTime) -> Result<(), GitGcError> {
    let keep_newer = keep_newer
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default(); // underflow
    let mut git = Command::new(program);
    git.arg("--git-dir=.") // turn off discovery
        .arg("gc")
        .arg(format!("--prune=@{} +0000", keep_newer.as_secs()));
    // Don't specify it by GIT_DIR/--git-dir. On Windows, the path could be
    // canonicalized as UNC path, which wouldn't be supported by git.
    git.current_dir(git_dir);
    // TODO: pass output to UI layer instead of printing directly here
    tracing::info!(?git, "running git gc");
    let status = git.status().map_err(GitGcError::GcCommand)?;
    tracing::info!(?status, "git gc exited");
    if !status.success() {
        return Err(GitGcError::GcCommandErrorStatus(status));
    }
    Ok(())
}

fn validate_git_object_id(id: &impl ObjectId) -> BackendResult<gix::ObjectId> {
    if id.as_bytes().len() != HASH_LENGTH {
        return Err(BackendError::InvalidHashLength {
            expected: HASH_LENGTH,
            actual: id.as_bytes().len(),
            object_type: id.object_type(),
            hash: id.hex(),
        });
    }
    Ok(gix::ObjectId::from_bytes_or_panic(id.as_bytes()))
}

fn map_not_found_err(err: gix::object::find::existing::Error, id: &impl ObjectId) -> BackendError {
    if matches!(err, gix::object::find::existing::Error::NotFound { .. }) {
        BackendError::ObjectNotFound {
            object_type: id.object_type(),
            hash: id.hex(),
            source: Box::new(err),
        }
    } else {
        to_read_object_err(err, id)
    }
}

fn to_read_object_err(
    err: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    id: &impl ObjectId,
) -> BackendError {
    BackendError::ReadObject {
        object_type: id.object_type(),
        hash: id.hex(),
        source: err.into(),
    }
}

fn to_invalid_utf8_err(source: Utf8Error, id: &impl ObjectId) -> BackendError {
    BackendError::InvalidUtf8 {
        object_type: id.object_type(),
        hash: id.hex(),
        source,
    }
}

fn import_extra_metadata_entries_from_heads(
    git_repo: &gix::Repository,
    mut_table: &mut MutableTable,
    _table_lock: &FileLock,
    head_ids: &HashSet<&CommitId>,
    shallow_roots: &[CommitId],
) -> BackendResult<()> {
    let mut work_ids = head_ids
        .iter()
        .filter(|&id| mut_table.get_value(id.as_bytes()).is_none())
        .map(|&id| id.clone())
        .collect_vec();
    while let Some(id) = work_ids.pop() {
        let git_object = git_repo
            .find_object(validate_git_object_id(&id)?)
            .map_err(|err| map_not_found_err(err, &id))?;
        let is_shallow = shallow_roots.contains(&id);
        // TODO(#1624): Should we read the root tree here and check if it has a
        // `.jjconflict-...` entries? That could happen if the user used `git` to e.g.
        // change the description of a commit with tree-level conflicts.
        let commit = commit_from_git_without_root_parent(&id, &git_object, is_shallow)?;
        mut_table.add_entry(id.to_bytes(), serialize_extras(&commit));
        work_ids.extend(
            commit
                .parents
                .into_iter()
                .filter(|id| mut_table.get_value(id.as_bytes()).is_none()),
        );
    }
    Ok(())
}

impl Debug for GitBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("GitBackend")
            .field("path", &self.git_repo_path())
            .finish()
    }
}

#[async_trait]
impl Backend for GitBackend {
    fn name(&self) -> &str {
        Self::name()
    }

    fn commit_id_length(&self) -> usize {
        HASH_LENGTH
    }

    fn change_id_length(&self) -> usize {
        CHANGE_ID_LENGTH
    }

    fn root_commit_id(&self) -> &CommitId {
        &self.root_commit_id
    }

    fn root_change_id(&self) -> &ChangeId {
        &self.root_change_id
    }

    fn empty_tree_id(&self) -> &TreeId {
        &self.empty_tree_id
    }

    fn concurrency(&self) -> usize {
        1
    }

    async fn read_file(
        &self,
        _path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>> {
        let data = self.read_file_sync(id)?;
        Ok(Box::pin(Cursor::new(data)))
    }

    async fn write_file(
        &self,
        _path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> BackendResult<FileId> {
        let mut bytes = Vec::new();
        contents.read_to_end(&mut bytes).await.unwrap();
        let locked_repo = self.lock_git_repo();
        let oid = locked_repo
            .write_blob(bytes)
            .map_err(|err| BackendError::WriteObject {
                object_type: "file",
                source: Box::new(err),
            })?;
        Ok(FileId::new(oid.as_bytes().to_vec()))
    }

    async fn read_symlink(&self, _path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        let git_blob_id = validate_git_object_id(id)?;
        let locked_repo = self.lock_git_repo();
        let mut blob = locked_repo
            .find_object(git_blob_id)
            .map_err(|err| map_not_found_err(err, id))?
            .try_into_blob()
            .map_err(|err| to_read_object_err(err, id))?;
        let target = String::from_utf8(blob.take_data())
            .map_err(|err| to_invalid_utf8_err(err.utf8_error(), id))?;
        Ok(target)
    }

    async fn write_symlink(&self, _path: &RepoPath, target: &str) -> BackendResult<SymlinkId> {
        let locked_repo = self.lock_git_repo();
        let oid =
            locked_repo
                .write_blob(target.as_bytes())
                .map_err(|err| BackendError::WriteObject {
                    object_type: "symlink",
                    source: Box::new(err),
                })?;
        Ok(SymlinkId::new(oid.as_bytes().to_vec()))
    }

    async fn read_copy(&self, _id: &CopyId) -> BackendResult<CopyHistory> {
        Err(BackendError::Unsupported(
            "The Git backend doesn't support tracked copies yet".to_string(),
        ))
    }

    async fn write_copy(&self, _contents: &CopyHistory) -> BackendResult<CopyId> {
        Err(BackendError::Unsupported(
            "The Git backend doesn't support tracked copies yet".to_string(),
        ))
    }

    async fn get_related_copies(&self, _copy_id: &CopyId) -> BackendResult<Vec<CopyHistory>> {
        Err(BackendError::Unsupported(
            "The Git backend doesn't support tracked copies yet".to_string(),
        ))
    }

    async fn read_tree(&self, _path: &RepoPath, id: &TreeId) -> BackendResult<Tree> {
        if id == &self.empty_tree_id {
            return Ok(Tree::default());
        }
        let git_tree_id = validate_git_object_id(id)?;

        let locked_repo = self.lock_git_repo();
        let git_tree = locked_repo
            .find_object(git_tree_id)
            .map_err(|err| map_not_found_err(err, id))?
            .try_into_tree()
            .map_err(|err| to_read_object_err(err, id))?;
        let mut entries: Vec<_> = git_tree
            .iter()
            .map(|entry| -> BackendResult<_> {
                let entry = entry.map_err(|err| to_read_object_err(err, id))?;
                let name = RepoPathComponentBuf::new(
                    str::from_utf8(entry.filename()).map_err(|err| to_invalid_utf8_err(err, id))?,
                )
                .unwrap();
                let value = match entry.mode().kind() {
                    gix::object::tree::EntryKind::Tree => {
                        let id = TreeId::from_bytes(entry.oid().as_bytes());
                        TreeValue::Tree(id)
                    }
                    gix::object::tree::EntryKind::Blob => {
                        let id = FileId::from_bytes(entry.oid().as_bytes());
                        TreeValue::File {
                            id,
                            executable: false,
                            copy_id: CopyId::placeholder(),
                        }
                    }
                    gix::object::tree::EntryKind::BlobExecutable => {
                        let id = FileId::from_bytes(entry.oid().as_bytes());
                        TreeValue::File {
                            id,
                            executable: true,
                            copy_id: CopyId::placeholder(),
                        }
                    }
                    gix::object::tree::EntryKind::Link => {
                        let id = SymlinkId::from_bytes(entry.oid().as_bytes());
                        TreeValue::Symlink(id)
                    }
                    gix::object::tree::EntryKind::Commit => {
                        let id = CommitId::from_bytes(entry.oid().as_bytes());
                        TreeValue::GitSubmodule(id)
                    }
                };
                Ok((name, value))
            })
            .try_collect()?;
        // While Git tree entries are sorted, the rule is slightly different.
        // Directory names are sorted as if they had trailing "/".
        if !entries.is_sorted_by_key(|(name, _)| name) {
            entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        }
        Ok(Tree::from_sorted_entries(entries))
    }

    async fn write_tree(&self, _path: &RepoPath, contents: &Tree) -> BackendResult<TreeId> {
        // Tree entries to be written must be sorted by Entry::filename(), which
        // is slightly different from the order of our backend::Tree.
        let entries = contents
            .entries()
            .map(|entry| {
                let filename = BString::from(entry.name().as_internal_str());
                match entry.value() {
                    TreeValue::File {
                        id,
                        executable: false,
                        copy_id: _, // TODO: Use the value
                    } => gix::objs::tree::Entry {
                        mode: gix::object::tree::EntryKind::Blob.into(),
                        filename,
                        oid: gix::ObjectId::from_bytes_or_panic(id.as_bytes()),
                    },
                    TreeValue::File {
                        id,
                        executable: true,
                        copy_id: _, // TODO: Use the value
                    } => gix::objs::tree::Entry {
                        mode: gix::object::tree::EntryKind::BlobExecutable.into(),
                        filename,
                        oid: gix::ObjectId::from_bytes_or_panic(id.as_bytes()),
                    },
                    TreeValue::Symlink(id) => gix::objs::tree::Entry {
                        mode: gix::object::tree::EntryKind::Link.into(),
                        filename,
                        oid: gix::ObjectId::from_bytes_or_panic(id.as_bytes()),
                    },
                    TreeValue::Tree(id) => gix::objs::tree::Entry {
                        mode: gix::object::tree::EntryKind::Tree.into(),
                        filename,
                        oid: gix::ObjectId::from_bytes_or_panic(id.as_bytes()),
                    },
                    TreeValue::GitSubmodule(id) => gix::objs::tree::Entry {
                        mode: gix::object::tree::EntryKind::Commit.into(),
                        filename,
                        oid: gix::ObjectId::from_bytes_or_panic(id.as_bytes()),
                    },
                }
            })
            .sorted_unstable()
            .collect();
        let locked_repo = self.lock_git_repo();
        let oid = locked_repo
            .write_object(gix::objs::Tree { entries })
            .map_err(|err| BackendError::WriteObject {
                object_type: "tree",
                source: Box::new(err),
            })?;
        Ok(TreeId::from_bytes(oid.as_bytes()))
    }

    #[tracing::instrument(skip(self))]
    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit> {
        if *id == self.root_commit_id {
            return Ok(make_root_commit(
                self.root_change_id().clone(),
                self.empty_tree_id.clone(),
            ));
        }
        let git_commit_id = validate_git_object_id(id)?;

        let mut commit = {
            let locked_repo = self.lock_git_repo();
            let git_object = locked_repo
                .find_object(git_commit_id)
                .map_err(|err| map_not_found_err(err, id))?;
            let is_shallow = self.shallow_root_ids(&locked_repo)?.contains(id);
            commit_from_git_without_root_parent(id, &git_object, is_shallow)?
        };
        if commit.parents.is_empty() {
            commit.parents.push(self.root_commit_id.clone());
        };

        let table = self.cached_extra_metadata_table()?;
        if let Some(extras) = table.get_value(id.as_bytes()) {
            deserialize_extras(&mut commit, extras);
        } else {
            // TODO: Remove this hack and map to ObjectNotFound error if we're sure that
            // there are no reachable ancestor commits without extras metadata. Git commits
            // imported by jj < 0.8.0 might not have extras (#924).
            // https://github.com/jj-vcs/jj/issues/2343
            tracing::info!("unimported Git commit found");
            self.import_head_commits([id])?;
            let table = self.cached_extra_metadata_table()?;
            let extras = table.get_value(id.as_bytes()).unwrap();
            deserialize_extras(&mut commit, extras);
        }
        Ok(commit)
    }

    async fn write_commit(
        &self,
        mut contents: Commit,
        mut sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<(CommitId, Commit)> {
        assert!(contents.secure_sig.is_none(), "commit.secure_sig was set");

        let locked_repo = self.lock_git_repo();
        let tree_ids = &contents.root_tree;
        let git_tree_id = match tree_ids.as_resolved() {
            Some(tree_id) => validate_git_object_id(tree_id)?,
            None => write_tree_conflict(&locked_repo, tree_ids)?,
        };
        let author = signature_to_git(&contents.author);
        let mut committer = signature_to_git(&contents.committer);
        let message = &contents.description;
        if contents.parents.is_empty() {
            return Err(BackendError::Other(
                "Cannot write a commit with no parents".into(),
            ));
        }
        let mut parents = SmallVec::new();
        for parent_id in &contents.parents {
            if *parent_id == self.root_commit_id {
                // Git doesn't have a root commit, so if the parent is the root commit, we don't
                // add it to the list of parents to write in the Git commit. We also check that
                // there are no other parents since Git cannot represent a merge between a root
                // commit and another commit.
                if contents.parents.len() > 1 {
                    return Err(BackendError::Unsupported(
                        "The Git backend does not support creating merge commits with the root \
                         commit as one of the parents."
                            .to_owned(),
                    ));
                }
            } else {
                parents.push(validate_git_object_id(parent_id)?);
            }
        }
        let mut extra_headers: Vec<(BString, BString)> = vec![];
        if !tree_ids.is_resolved() {
            let value = tree_ids.iter().map(|id| id.hex()).join(" ");
            extra_headers.push((JJ_TREES_COMMIT_HEADER.into(), value.into()));
        }
        if self.write_change_id_header {
            extra_headers.push((
                CHANGE_ID_COMMIT_HEADER.into(),
                contents.change_id.reverse_hex().into(),
            ));
        }

        let extras = serialize_extras(&contents);

        // If two writers write commits of the same id with different metadata, they
        // will both succeed and the metadata entries will be "merged" later. Since
        // metadata entry is keyed by the commit id, one of the entries would be lost.
        // To prevent such race condition locally, we extend the scope covered by the
        // table lock. This is still racy if multiple machines are involved and the
        // repository is rsync-ed.
        let (table, table_lock) = self.read_extra_metadata_table_locked()?;
        let id = loop {
            let mut commit = gix::objs::Commit {
                message: message.to_owned().into(),
                tree: git_tree_id,
                author: author.clone(),
                committer: committer.clone(),
                encoding: None,
                parents: parents.clone(),
                extra_headers: extra_headers.clone(),
            };

            if let Some(sign) = &mut sign_with {
                // we don't use gix pool, but at least use their heuristic
                let mut data = Vec::with_capacity(512);
                commit.write_to(&mut data).unwrap();

                let sig = sign(&data).map_err(|err| BackendError::WriteObject {
                    object_type: "commit",
                    source: Box::new(err),
                })?;
                commit
                    .extra_headers
                    .push(("gpgsig".into(), sig.clone().into()));
                contents.secure_sig = Some(SecureSig { data, sig });
            }

            let git_id =
                locked_repo
                    .write_object(&commit)
                    .map_err(|err| BackendError::WriteObject {
                        object_type: "commit",
                        source: Box::new(err),
                    })?;

            match table.get_value(git_id.as_bytes()) {
                Some(existing_extras) if existing_extras != extras => {
                    // It's possible a commit already exists with the same
                    // commit id but different change id. Adjust the timestamp
                    // until this is no longer the case.
                    //
                    // For example, this can happen when rebasing duplicate
                    // commits, https://github.com/jj-vcs/jj/issues/694.
                    //
                    // `jj` resets the committer timestamp to the current
                    // timestamp whenever it rewrites a commit. So, it's
                    // unlikely for the timestamp to be 0 even if the original
                    // commit had its timestamp set to 0. Moreover, we test that
                    // a commit with a negative timestamp can still be written
                    // and read back by `jj`.
                    committer.time.seconds -= 1;
                }
                _ => break CommitId::from_bytes(git_id.as_bytes()),
            }
        };

        // Everything up to this point had no permanent effect on the repo except
        // GC-able objects
        locked_repo
            .edit_reference(to_no_gc_ref_update(&id))
            .map_err(|err| BackendError::Other(Box::new(err)))?;

        // Update the signature to match the one that was actually written to the object
        // store
        contents.committer.timestamp.timestamp = MillisSinceEpoch(committer.time.seconds * 1000);
        let mut mut_table = table.start_mutation();
        mut_table.add_entry(id.to_bytes(), extras);
        self.save_extra_metadata_table(mut_table, &table_lock)?;
        Ok((id, contents))
    }

    fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root_id: &CommitId,
        head_id: &CommitId,
    ) -> BackendResult<BoxStream<'_, BackendResult<CopyRecord>>> {
        let repo = self.git_repo();
        let root_tree = self.read_tree_for_commit(&repo, root_id)?;
        let head_tree = self.read_tree_for_commit(&repo, head_id)?;

        let change_to_copy_record =
            |change: gix::object::tree::diff::Change| -> BackendResult<Option<CopyRecord>> {
                let gix::object::tree::diff::Change::Rewrite {
                    source_location,
                    source_entry_mode,
                    source_id,
                    entry_mode: dest_entry_mode,
                    location: dest_location,
                    ..
                } = change
                else {
                    return Ok(None);
                };
                // TODO: Renamed symlinks cannot be returned because CopyRecord
                // expects `source_file: FileId`.
                if !source_entry_mode.is_blob() || !dest_entry_mode.is_blob() {
                    return Ok(None);
                }

                let source = str::from_utf8(source_location)
                    .map_err(|err| to_invalid_utf8_err(err, root_id))?;
                let dest = str::from_utf8(dest_location)
                    .map_err(|err| to_invalid_utf8_err(err, head_id))?;

                let target = RepoPathBuf::from_internal_string(dest).unwrap();
                if !paths.is_none_or(|paths| paths.contains(&target)) {
                    return Ok(None);
                }

                Ok(Some(CopyRecord {
                    target,
                    target_commit: head_id.clone(),
                    source: RepoPathBuf::from_internal_string(source).unwrap(),
                    source_file: FileId::from_bytes(source_id.as_bytes()),
                    source_commit: root_id.clone(),
                }))
            };

        let mut records: Vec<BackendResult<CopyRecord>> = Vec::new();
        root_tree
            .changes()
            .map_err(|err| BackendError::Other(err.into()))?
            .options(|opts| {
                opts.track_path().track_rewrites(Some(gix::diff::Rewrites {
                    copies: Some(gix::diff::rewrites::Copies {
                        source: gix::diff::rewrites::CopySource::FromSetOfModifiedFiles,
                        percentage: Some(0.5),
                    }),
                    percentage: Some(0.5),
                    limit: 1000,
                    track_empty: false,
                }));
            })
            .for_each_to_obtain_tree_with_cache(
                &head_tree,
                &mut self.new_diff_platform()?,
                |change| -> BackendResult<_> {
                    match change_to_copy_record(change) {
                        Ok(None) => {}
                        Ok(Some(change)) => records.push(Ok(change)),
                        Err(err) => records.push(Err(err)),
                    }
                    Ok(gix::object::tree::diff::Action::Continue)
                },
            )
            .map_err(|err| BackendError::Other(err.into()))?;
        Ok(Box::pin(futures::stream::iter(records)))
    }

    #[tracing::instrument(skip(self, index))]
    fn gc(&self, index: &dyn Index, keep_newer: SystemTime) -> BackendResult<()> {
        let git_repo = self.lock_git_repo();
        let new_heads = index
            .all_heads_for_gc()
            .map_err(|err| BackendError::Other(err.into()))?
            .filter(|id| *id != self.root_commit_id);
        recreate_no_gc_refs(&git_repo, new_heads, keep_newer)?;
        // TODO: remove unreachable entries from extras table if segment file
        // mtime <= keep_newer? (it won't be consistent with no-gc refs
        // preserved by the keep_newer timestamp though)
        // TODO: remove unreachable extras table segments
        run_git_gc(
            self.git_executable.as_ref(),
            self.git_repo_path(),
            keep_newer,
        )
        .map_err(|err| BackendError::Other(err.into()))?;
        // Since "git gc" will move loose refs into packed refs, in-memory
        // packed-refs cache should be invalidated without relying on mtime.
        git_repo.refs.force_refresh_packed_buffer().ok();
        Ok(())
    }
}

/// Write a tree conflict as a special tree with `.jjconflict-base-N` and
/// `.jjconflict-base-N` subtrees. This ensure that the parts are not GC'd.
fn write_tree_conflict(
    repo: &gix::Repository,
    conflict: &Merge<TreeId>,
) -> BackendResult<gix::ObjectId> {
    // Tree entries to be written must be sorted by Entry::filename().
    let mut entries = itertools::chain(
        conflict
            .removes()
            .enumerate()
            .map(|(i, tree_id)| (format!(".jjconflict-base-{i}"), tree_id)),
        conflict
            .adds()
            .enumerate()
            .map(|(i, tree_id)| (format!(".jjconflict-side-{i}"), tree_id)),
    )
    .map(|(name, tree_id)| gix::objs::tree::Entry {
        mode: gix::object::tree::EntryKind::Tree.into(),
        filename: name.into(),
        oid: gix::ObjectId::from_bytes_or_panic(tree_id.as_bytes()),
    })
    .collect_vec();
    let readme_id = repo
        .write_blob(
            r#"This commit was made by jj, https://jj-vcs.dev/.
The commit contains file conflicts, and therefore looks wrong when used with plain
Git or other tools that are unfamiliar with jj.

The .jjconflict-* directories represent the different inputs to the conflict.
For details, see
https://docs.jj-vcs.dev/prerelease/git-compatibility/#format-mapping-details

If you see this file in your working copy, it probably means that you used a
regular `git` command to check out a conflicted commit. Use `jj abandon` to
recover.
"#,
        )
        .map_err(|err| {
            BackendError::Other(format!("Failed to write README for conflict tree: {err}").into())
        })?
        .detach();
    entries.push(gix::objs::tree::Entry {
        mode: gix::object::tree::EntryKind::Blob.into(),
        filename: "README".into(),
        oid: readme_id,
    });
    entries.sort_unstable();
    let id = repo
        .write_object(gix::objs::Tree { entries })
        .map_err(|err| BackendError::WriteObject {
            object_type: "tree",
            source: Box::new(err),
        })?;
    Ok(id.detach())
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use gix::date::parse::TimeBuf;
    use gix::objs::CommitRef;
    use indoc::indoc;
    use pollster::FutureExt as _;

    use super::*;
    use crate::config::StackedConfig;
    use crate::content_hash::blake2b_hash;
    use crate::hex_util;
    use crate::tests::new_temp_dir;

    const GIT_USER: &str = "Someone";
    const GIT_EMAIL: &str = "someone@example.com";

    fn git_config() -> Vec<bstr::BString> {
        vec![
            format!("user.name = {GIT_USER}").into(),
            format!("user.email = {GIT_EMAIL}").into(),
            "init.defaultBranch = master".into(),
        ]
    }

    fn open_options() -> gix::open::Options {
        gix::open::Options::isolated()
            .config_overrides(git_config())
            .strict_config(true)
    }

    fn git_init(directory: impl AsRef<Path>) -> gix::Repository {
        gix::ThreadSafeRepository::init_opts(
            directory,
            gix::create::Kind::WithWorktree,
            gix::create::Options::default(),
            open_options(),
        )
        .unwrap()
        .to_thread_local()
    }

    #[test]
    fn read_plain_git_commit() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(git_repo_path);

        // Add a commit with some files in
        let blob1 = git_repo.write_blob(b"content1").unwrap().detach();
        let blob2 = git_repo.write_blob(b"normal").unwrap().detach();
        let mut dir_tree_editor = git_repo.empty_tree().edit().unwrap();
        dir_tree_editor
            .upsert("normal", gix::object::tree::EntryKind::Blob, blob1)
            .unwrap();
        dir_tree_editor
            .upsert("symlink", gix::object::tree::EntryKind::Link, blob2)
            .unwrap();
        let dir_tree_id = dir_tree_editor.write().unwrap().detach();
        let mut root_tree_builder = git_repo.empty_tree().edit().unwrap();
        root_tree_builder
            .upsert("dir", gix::object::tree::EntryKind::Tree, dir_tree_id)
            .unwrap();
        let root_tree_id = root_tree_builder.write().unwrap().detach();
        let git_author = gix::actor::Signature {
            name: "git author".into(),
            email: "git.author@example.com".into(),
            time: gix::date::Time::new(1000, 60 * 60),
        };
        let git_committer = gix::actor::Signature {
            name: "git committer".into(),
            email: "git.committer@example.com".into(),
            time: gix::date::Time::new(2000, -480 * 60),
        };
        let git_commit_id = git_repo
            .commit_as(
                git_committer.to_ref(&mut TimeBuf::default()),
                git_author.to_ref(&mut TimeBuf::default()),
                "refs/heads/dummy",
                "git commit message",
                root_tree_id,
                [] as [gix::ObjectId; 0],
            )
            .unwrap()
            .detach();
        git_repo
            .find_reference("refs/heads/dummy")
            .unwrap()
            .delete()
            .unwrap();
        let commit_id = CommitId::from_hex("efdcea5ca4b3658149f899ca7feee6876d077263");
        // The change id is the leading reverse bits of the commit id
        let change_id = ChangeId::from_hex("c64ee0b6e16777fe53991f9281a6cd25");
        // Check that the git commit above got the hash we expect
        assert_eq!(
            git_commit_id.as_bytes(),
            commit_id.as_bytes(),
            "{git_commit_id:?} vs {commit_id:?}"
        );

        // Add an empty commit on top
        let git_commit_id2 = git_repo
            .commit_as(
                git_committer.to_ref(&mut TimeBuf::default()),
                git_author.to_ref(&mut TimeBuf::default()),
                "refs/heads/dummy2",
                "git commit message 2",
                root_tree_id,
                [git_commit_id],
            )
            .unwrap()
            .detach();
        git_repo
            .find_reference("refs/heads/dummy2")
            .unwrap()
            .delete()
            .unwrap();
        let commit_id2 = CommitId::from_bytes(git_commit_id2.as_bytes());

        let backend = GitBackend::init_external(&settings, store_path, git_repo.path()).unwrap();

        // Import the head commit and its ancestors
        backend.import_head_commits([&commit_id2]).unwrap();
        // Ref should be created only for the head commit
        let git_refs = backend
            .git_repo()
            .references()
            .unwrap()
            .prefixed("refs/jj/keep/")
            .unwrap()
            .map(|git_ref| git_ref.unwrap().id().detach())
            .collect_vec();
        assert_eq!(git_refs, vec![git_commit_id2]);

        let commit = backend.read_commit(&commit_id).block_on().unwrap();
        assert_eq!(&commit.change_id, &change_id);
        assert_eq!(commit.parents, vec![CommitId::from_bytes(&[0; 20])]);
        assert_eq!(commit.predecessors, vec![]);
        assert_eq!(
            commit.root_tree,
            Merge::resolved(TreeId::from_bytes(root_tree_id.as_bytes()))
        );
        assert_eq!(commit.description, "git commit message");
        assert_eq!(commit.author.name, "git author");
        assert_eq!(commit.author.email, "git.author@example.com");
        assert_eq!(
            commit.author.timestamp.timestamp,
            MillisSinceEpoch(1000 * 1000)
        );
        assert_eq!(commit.author.timestamp.tz_offset, 60);
        assert_eq!(commit.committer.name, "git committer");
        assert_eq!(commit.committer.email, "git.committer@example.com");
        assert_eq!(
            commit.committer.timestamp.timestamp,
            MillisSinceEpoch(2000 * 1000)
        );
        assert_eq!(commit.committer.timestamp.tz_offset, -480);

        let root_tree = backend
            .read_tree(
                RepoPath::root(),
                &TreeId::from_bytes(root_tree_id.as_bytes()),
            )
            .block_on()
            .unwrap();
        let mut root_entries = root_tree.entries();
        let dir = root_entries.next().unwrap();
        assert_eq!(root_entries.next(), None);
        assert_eq!(dir.name().as_internal_str(), "dir");
        assert_eq!(
            dir.value(),
            &TreeValue::Tree(TreeId::from_bytes(dir_tree_id.as_bytes()))
        );

        let dir_tree = backend
            .read_tree(
                RepoPath::from_internal_string("dir").unwrap(),
                &TreeId::from_bytes(dir_tree_id.as_bytes()),
            )
            .block_on()
            .unwrap();
        let mut entries = dir_tree.entries();
        let file = entries.next().unwrap();
        let symlink = entries.next().unwrap();
        assert_eq!(entries.next(), None);
        assert_eq!(file.name().as_internal_str(), "normal");
        assert_eq!(
            file.value(),
            &TreeValue::File {
                id: FileId::from_bytes(blob1.as_bytes()),
                executable: false,
                copy_id: CopyId::placeholder(),
            }
        );
        assert_eq!(symlink.name().as_internal_str(), "symlink");
        assert_eq!(
            symlink.value(),
            &TreeValue::Symlink(SymlinkId::from_bytes(blob2.as_bytes()))
        );

        let commit2 = backend.read_commit(&commit_id2).block_on().unwrap();
        assert_eq!(commit2.parents, vec![commit_id.clone()]);
        assert_eq!(commit.predecessors, vec![]);
        assert_eq!(
            commit.root_tree,
            Merge::resolved(TreeId::from_bytes(root_tree_id.as_bytes()))
        );
    }

    #[test]
    fn read_git_commit_without_importing() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(&git_repo_path);

        let signature = gix::actor::Signature {
            name: GIT_USER.into(),
            email: GIT_EMAIL.into(),
            time: gix::date::Time::now_utc(),
        };
        let empty_tree_id =
            gix::ObjectId::from_hex(b"4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap();
        let git_commit_id = git_repo
            .commit_as(
                signature.to_ref(&mut TimeBuf::default()),
                signature.to_ref(&mut TimeBuf::default()),
                "refs/heads/main",
                "git commit message",
                empty_tree_id,
                [] as [gix::ObjectId; 0],
            )
            .unwrap();

        let backend = GitBackend::init_external(&settings, store_path, git_repo.path()).unwrap();

        // read_commit() without import_head_commits() works as of now. This might be
        // changed later.
        assert!(
            backend
                .read_commit(&CommitId::from_bytes(git_commit_id.as_bytes()))
                .block_on()
                .is_ok()
        );
        assert!(
            backend
                .cached_extra_metadata_table()
                .unwrap()
                .get_value(git_commit_id.as_bytes())
                .is_some(),
            "extra metadata should have been be created"
        );
    }

    #[test]
    fn read_signed_git_commit() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(git_repo_path);

        let signature = gix::actor::Signature {
            name: GIT_USER.into(),
            email: GIT_EMAIL.into(),
            time: gix::date::Time::now_utc(),
        };
        let empty_tree_id =
            gix::ObjectId::from_hex(b"4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap();

        let secure_sig =
            "here are some ASCII bytes to be used as a test signature\n\ndefinitely not PGP\n";

        let mut commit = gix::objs::Commit {
            tree: empty_tree_id,
            parents: smallvec::SmallVec::new(),
            author: signature.clone(),
            committer: signature.clone(),
            encoding: None,
            message: "git commit message".into(),
            extra_headers: Vec::new(),
        };

        let mut commit_buf = Vec::new();
        commit.write_to(&mut commit_buf).unwrap();
        let commit_str = str::from_utf8(&commit_buf).unwrap();

        commit
            .extra_headers
            .push(("gpgsig".into(), secure_sig.into()));

        let git_commit_id = git_repo.write_object(&commit).unwrap();

        let backend = GitBackend::init_external(&settings, store_path, git_repo.path()).unwrap();

        let commit = backend
            .read_commit(&CommitId::from_bytes(git_commit_id.as_bytes()))
            .block_on()
            .unwrap();

        let sig = commit.secure_sig.expect("failed to read the signature");

        // converting to string for nicer assert diff
        assert_eq!(str::from_utf8(&sig.sig).unwrap(), secure_sig);
        assert_eq!(str::from_utf8(&sig.data).unwrap(), commit_str);
    }

    #[test]
    fn change_id_parsing() {
        let id = |commit_object_bytes: &[u8]| {
            extract_change_id_from_commit(&CommitRef::from_bytes(commit_object_bytes).unwrap())
        };

        let commit_with_id = indoc! {b"
            tree 126799bf8058d1b5c531e93079f4fe79733920dd
            parent bd50783bdf38406dd6143475cd1a3c27938db2ee
            author JJ Fan <jjfan@example.com> 1757112665 -0700
            committer JJ Fan <jjfan@example.com> 1757359886 -0700
            extra-header blah
            change-id lkonztmnvsxytrwkxpvuutrmompwylqq

            test-commit
        "};
        insta::assert_compact_debug_snapshot!(
            id(commit_with_id),
            @r#"Some(ChangeId("efbc06dc4721683f2a45568dbda31e99"))"#
        );

        let commit_without_id = indoc! {b"
            tree 126799bf8058d1b5c531e93079f4fe79733920dd
            parent bd50783bdf38406dd6143475cd1a3c27938db2ee
            author JJ Fan <jjfan@example.com> 1757112665 -0700
            committer JJ Fan <jjfan@example.com> 1757359886 -0700
            extra-header blah

            no id in header
        "};
        insta::assert_compact_debug_snapshot!(
            id(commit_without_id),
            @"None"
        );

        let commit = indoc! {b"
            tree 126799bf8058d1b5c531e93079f4fe79733920dd
            parent bd50783bdf38406dd6143475cd1a3c27938db2ee
            author JJ Fan <jjfan@example.com> 1757112665 -0700
            committer JJ Fan <jjfan@example.com> 1757359886 -0700
            change-id lkonztmnvsxytrwkxpvuutrmompwylqq
            extra-header blah
            change-id abcabcabcabcabcabcabcabcabcabcab

            valid change id first
        "};
        insta::assert_compact_debug_snapshot!(
            id(commit),
            @r#"Some(ChangeId("efbc06dc4721683f2a45568dbda31e99"))"#
        );

        // We only look at the first change id if multiple are present, so this should
        // error
        let commit = indoc! {b"
            tree 126799bf8058d1b5c531e93079f4fe79733920dd
            parent bd50783bdf38406dd6143475cd1a3c27938db2ee
            author JJ Fan <jjfan@example.com> 1757112665 -0700
            committer JJ Fan <jjfan@example.com> 1757359886 -0700
            change-id abcabcabcabcabcabcabcabcabcabcab
            extra-header blah
            change-id lkonztmnvsxytrwkxpvuutrmompwylqq

            valid change id first
        "};
        insta::assert_compact_debug_snapshot!(
            id(commit),
            @"None"
        );
    }

    #[test]
    fn round_trip_change_id_via_git_header() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();

        let store_path = temp_dir.path().join("store");
        fs::create_dir(&store_path).unwrap();
        let empty_store_path = temp_dir.path().join("empty_store");
        fs::create_dir(&empty_store_path).unwrap();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(git_repo_path);

        let backend = GitBackend::init_external(&settings, &store_path, git_repo.path()).unwrap();
        let original_change_id = ChangeId::from_hex("1111eeee1111eeee1111eeee1111eeee");
        let commit = Commit {
            parents: vec![backend.root_commit_id().clone()],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: original_change_id.clone(),
            description: "initial".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let (initial_commit_id, _init_commit) =
            backend.write_commit(commit, None).block_on().unwrap();
        let commit = backend.read_commit(&initial_commit_id).block_on().unwrap();
        assert_eq!(
            commit.change_id, original_change_id,
            "The change-id header did not roundtrip"
        );

        // Because of how change ids are also persisted in extra proto files,
        // initialize a new store without those files, but reuse the same git
        // storage. This change-id must be derived from the git commit header.
        let no_extra_backend =
            GitBackend::init_external(&settings, &empty_store_path, git_repo.path()).unwrap();
        let no_extra_commit = no_extra_backend
            .read_commit(&initial_commit_id)
            .block_on()
            .unwrap();

        assert_eq!(
            no_extra_commit.change_id, original_change_id,
            "The change-id header did not roundtrip"
        );
    }

    #[test]
    fn read_empty_string_placeholder() {
        let git_signature1 = gix::actor::Signature {
            name: EMPTY_STRING_PLACEHOLDER.into(),
            email: "git.author@example.com".into(),
            time: gix::date::Time::new(1000, 60 * 60),
        };
        let signature1 = signature_from_git(git_signature1.to_ref(&mut TimeBuf::default()));
        assert!(signature1.name.is_empty());
        assert_eq!(signature1.email, "git.author@example.com");
        let git_signature2 = gix::actor::Signature {
            name: "git committer".into(),
            email: EMPTY_STRING_PLACEHOLDER.into(),
            time: gix::date::Time::new(2000, -480 * 60),
        };
        let signature2 = signature_from_git(git_signature2.to_ref(&mut TimeBuf::default()));
        assert_eq!(signature2.name, "git committer");
        assert!(signature2.email.is_empty());
    }

    #[test]
    fn write_empty_string_placeholder() {
        let signature1 = Signature {
            name: "".to_string(),
            email: "someone@example.com".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        };
        let git_signature1 = signature_to_git(&signature1);
        assert_eq!(git_signature1.name, EMPTY_STRING_PLACEHOLDER);
        assert_eq!(git_signature1.email, "someone@example.com");
        let signature2 = Signature {
            name: "Someone".to_string(),
            email: "".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        };
        let git_signature2 = signature_to_git(&signature2);
        assert_eq!(git_signature2.name, "Someone");
        assert_eq!(git_signature2.email, EMPTY_STRING_PLACEHOLDER);
    }

    /// Test that parents get written correctly
    #[test]
    fn git_commit_parents() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(&git_repo_path);

        let backend = GitBackend::init_external(&settings, store_path, git_repo.path()).unwrap();
        let mut commit = Commit {
            parents: vec![],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::from_hex("abc123"),
            description: "".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let write_commit = |commit: Commit| -> BackendResult<(CommitId, Commit)> {
            backend.write_commit(commit, None).block_on()
        };

        // No parents
        commit.parents = vec![];
        assert_matches!(
            write_commit(commit.clone()),
            Err(BackendError::Other(err)) if err.to_string().contains("no parents")
        );

        // Only root commit as parent
        commit.parents = vec![backend.root_commit_id().clone()];
        let first_id = write_commit(commit.clone()).unwrap().0;
        let first_commit = backend.read_commit(&first_id).block_on().unwrap();
        assert_eq!(first_commit, commit);
        let first_git_commit = git_repo.find_commit(git_id(&first_id)).unwrap();
        assert!(first_git_commit.parent_ids().collect_vec().is_empty());

        // Only non-root commit as parent
        commit.parents = vec![first_id.clone()];
        let second_id = write_commit(commit.clone()).unwrap().0;
        let second_commit = backend.read_commit(&second_id).block_on().unwrap();
        assert_eq!(second_commit, commit);
        let second_git_commit = git_repo.find_commit(git_id(&second_id)).unwrap();
        assert_eq!(
            second_git_commit.parent_ids().collect_vec(),
            vec![git_id(&first_id)]
        );

        // Merge commit
        commit.parents = vec![first_id.clone(), second_id.clone()];
        let merge_id = write_commit(commit.clone()).unwrap().0;
        let merge_commit = backend.read_commit(&merge_id).block_on().unwrap();
        assert_eq!(merge_commit, commit);
        let merge_git_commit = git_repo.find_commit(git_id(&merge_id)).unwrap();
        assert_eq!(
            merge_git_commit.parent_ids().collect_vec(),
            vec![git_id(&first_id), git_id(&second_id)]
        );

        // Merge commit with root as one parent
        commit.parents = vec![first_id, backend.root_commit_id().clone()];
        assert_matches!(
            write_commit(commit),
            Err(BackendError::Unsupported(message)) if message.contains("root commit")
        );
    }

    #[test]
    fn write_tree_conflicts() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();
        let git_repo_path = temp_dir.path().join("git");
        let git_repo = git_init(&git_repo_path);

        let backend = GitBackend::init_external(&settings, store_path, git_repo.path()).unwrap();
        let create_tree = |i| {
            let blob_id = git_repo.write_blob(format!("content {i}")).unwrap();
            let mut tree_builder = git_repo.empty_tree().edit().unwrap();
            tree_builder
                .upsert(
                    format!("file{i}"),
                    gix::object::tree::EntryKind::Blob,
                    blob_id,
                )
                .unwrap();
            TreeId::from_bytes(tree_builder.write().unwrap().as_bytes())
        };

        let root_tree = Merge::from_removes_adds(
            vec![create_tree(0), create_tree(1)],
            vec![create_tree(2), create_tree(3), create_tree(4)],
        );
        let mut commit = Commit {
            parents: vec![backend.root_commit_id().clone()],
            predecessors: vec![],
            root_tree: root_tree.clone(),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::from_hex("abc123"),
            description: "".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let write_commit = |commit: Commit| -> BackendResult<(CommitId, Commit)> {
            backend.write_commit(commit, None).block_on()
        };

        // When writing a tree-level conflict, the root tree on the git side has the
        // individual trees as subtrees.
        let read_commit_id = write_commit(commit.clone()).unwrap().0;
        let read_commit = backend.read_commit(&read_commit_id).block_on().unwrap();
        assert_eq!(read_commit, commit);
        let git_commit = git_repo
            .find_commit(gix::ObjectId::from_bytes_or_panic(
                read_commit_id.as_bytes(),
            ))
            .unwrap();
        let git_tree = git_repo.find_tree(git_commit.tree_id().unwrap()).unwrap();
        assert!(
            git_tree
                .iter()
                .map(Result::unwrap)
                .filter(|entry| entry.filename() != b"README")
                .all(|entry| entry.mode().value() == 0o040000)
        );
        let mut iter = git_tree.iter().map(Result::unwrap);
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b".jjconflict-base-0");
        assert_eq!(
            entry.id().as_bytes(),
            root_tree.get_remove(0).unwrap().as_bytes()
        );
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b".jjconflict-base-1");
        assert_eq!(
            entry.id().as_bytes(),
            root_tree.get_remove(1).unwrap().as_bytes()
        );
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b".jjconflict-side-0");
        assert_eq!(
            entry.id().as_bytes(),
            root_tree.get_add(0).unwrap().as_bytes()
        );
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b".jjconflict-side-1");
        assert_eq!(
            entry.id().as_bytes(),
            root_tree.get_add(1).unwrap().as_bytes()
        );
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b".jjconflict-side-2");
        assert_eq!(
            entry.id().as_bytes(),
            root_tree.get_add(2).unwrap().as_bytes()
        );
        let entry = iter.next().unwrap();
        assert_eq!(entry.filename(), b"README");
        assert_eq!(entry.mode().value(), 0o100644);
        assert!(iter.next().is_none());

        // When writing a single tree using the new format, it's represented by a
        // regular git tree.
        commit.root_tree = Merge::resolved(create_tree(5));
        let read_commit_id = write_commit(commit.clone()).unwrap().0;
        let read_commit = backend.read_commit(&read_commit_id).block_on().unwrap();
        assert_eq!(read_commit, commit);
        let git_commit = git_repo
            .find_commit(gix::ObjectId::from_bytes_or_panic(
                read_commit_id.as_bytes(),
            ))
            .unwrap();
        assert_eq!(
            Merge::resolved(TreeId::from_bytes(git_commit.tree_id().unwrap().as_bytes())),
            commit.root_tree
        );
    }

    #[test]
    fn commit_has_ref() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let backend = GitBackend::init_internal(&settings, temp_dir.path()).unwrap();
        let git_repo = backend.git_repo();
        let signature = Signature {
            name: "Someone".to_string(),
            email: "someone@example.com".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        };
        let commit = Commit {
            parents: vec![backend.root_commit_id().clone()],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::new(vec![42; 16]),
            description: "initial".to_string(),
            author: signature.clone(),
            committer: signature,
            secure_sig: None,
        };
        let commit_id = backend.write_commit(commit, None).block_on().unwrap().0;
        let git_refs = git_repo.references().unwrap();
        let git_ref_ids: Vec<_> = git_refs
            .prefixed("refs/jj/keep/")
            .unwrap()
            .map(|x| x.unwrap().id().detach())
            .collect();
        assert!(git_ref_ids.iter().any(|id| *id == git_id(&commit_id)));

        // Concurrently-running GC deletes the ref, leaving the extra metadata.
        for git_ref in git_refs.prefixed("refs/jj/keep/").unwrap() {
            git_ref.unwrap().delete().unwrap();
        }
        // Re-imported commit should have new ref.
        backend.import_head_commits([&commit_id]).unwrap();
        let git_refs = git_repo.references().unwrap();
        let git_ref_ids: Vec<_> = git_refs
            .prefixed("refs/jj/keep/")
            .unwrap()
            .map(|x| x.unwrap().id().detach())
            .collect();
        assert!(git_ref_ids.iter().any(|id| *id == git_id(&commit_id)));
    }

    #[test]
    fn import_head_commits_duplicates() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let backend = GitBackend::init_internal(&settings, temp_dir.path()).unwrap();
        let git_repo = backend.git_repo();

        let signature = gix::actor::Signature {
            name: GIT_USER.into(),
            email: GIT_EMAIL.into(),
            time: gix::date::Time::now_utc(),
        };
        let empty_tree_id =
            gix::ObjectId::from_hex(b"4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap();
        let git_commit_id = git_repo
            .commit_as(
                signature.to_ref(&mut TimeBuf::default()),
                signature.to_ref(&mut TimeBuf::default()),
                "refs/heads/main",
                "git commit message",
                empty_tree_id,
                [] as [gix::ObjectId; 0],
            )
            .unwrap()
            .detach();
        let commit_id = CommitId::from_bytes(git_commit_id.as_bytes());

        // Ref creation shouldn't fail because of duplicated head ids.
        backend
            .import_head_commits([&commit_id, &commit_id])
            .unwrap();
        assert!(
            git_repo
                .references()
                .unwrap()
                .prefixed("refs/jj/keep/")
                .unwrap()
                .any(|git_ref| git_ref.unwrap().id().detach() == git_commit_id)
        );
    }

    #[test]
    fn overlapping_git_commit_id() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let backend = GitBackend::init_internal(&settings, temp_dir.path()).unwrap();
        let commit1 = Commit {
            parents: vec![backend.root_commit_id().clone()],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::from_hex("7f0a7ce70354b22efcccf7bf144017c4"),
            description: "initial".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let write_commit = |commit: Commit| -> BackendResult<(CommitId, Commit)> {
            backend.write_commit(commit, None).block_on()
        };

        let (commit_id1, mut commit2) = write_commit(commit1).unwrap();
        commit2.predecessors.push(commit_id1.clone());
        // `write_commit` should prevent the ids from being the same by changing the
        // committer timestamp of the commit it actually writes.
        let (commit_id2, mut actual_commit2) = write_commit(commit2.clone()).unwrap();
        // The returned matches the ID
        assert_eq!(
            backend.read_commit(&commit_id2).block_on().unwrap(),
            actual_commit2
        );
        assert_ne!(commit_id2, commit_id1);
        // The committer timestamp should differ
        assert_ne!(
            actual_commit2.committer.timestamp.timestamp,
            commit2.committer.timestamp.timestamp
        );
        // The rest of the commit should be the same
        actual_commit2.committer.timestamp.timestamp = commit2.committer.timestamp.timestamp;
        assert_eq!(actual_commit2, commit2);
    }

    #[test]
    fn write_signed_commit() {
        let settings = user_settings();
        let temp_dir = new_temp_dir();
        let backend = GitBackend::init_internal(&settings, temp_dir.path()).unwrap();

        let commit = Commit {
            parents: vec![backend.root_commit_id().clone()],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::new(vec![42; 16]),
            description: "initial".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let mut signer = |data: &_| {
            let hash: String = hex_util::encode_hex(&blake2b_hash(data));
            Ok(format!("test sig\nhash={hash}\n").into_bytes())
        };

        let (id, commit) = backend
            .write_commit(commit, Some(&mut signer as &mut SigningFn))
            .block_on()
            .unwrap();

        let git_repo = backend.git_repo();
        let obj = git_repo
            .find_object(gix::ObjectId::from_bytes_or_panic(id.as_bytes()))
            .unwrap();
        insta::assert_snapshot!(str::from_utf8(&obj.data).unwrap(), @r"
        tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
        author Someone <someone@example.com> 0 +0000
        committer Someone <someone@example.com> 0 +0000
        change-id xpxpxpxpxpxpxpxpxpxpxpxpxpxpxpxp
        gpgsig test sig
         hash=03feb0caccbacce2e7b7bca67f4c82292dd487e669ed8a813120c9f82d3fd0801420a1f5d05e1393abfe4e9fc662399ec4a9a1898c5f1e547e0044a52bd4bd29

        initial
        ");

        let returned_sig = commit.secure_sig.expect("failed to return the signature");

        let commit = backend.read_commit(&id).block_on().unwrap();

        let sig = commit.secure_sig.expect("failed to read the signature");
        assert_eq!(&sig, &returned_sig);

        insta::assert_snapshot!(str::from_utf8(&sig.sig).unwrap(), @r"
        test sig
        hash=03feb0caccbacce2e7b7bca67f4c82292dd487e669ed8a813120c9f82d3fd0801420a1f5d05e1393abfe4e9fc662399ec4a9a1898c5f1e547e0044a52bd4bd29
        ");
        insta::assert_snapshot!(str::from_utf8(&sig.data).unwrap(), @r"
        tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904
        author Someone <someone@example.com> 0 +0000
        committer Someone <someone@example.com> 0 +0000
        change-id xpxpxpxpxpxpxpxpxpxpxpxpxpxpxpxp

        initial
        ");
    }

    fn git_id(commit_id: &CommitId) -> gix::ObjectId {
        gix::ObjectId::from_bytes_or_panic(commit_id.as_bytes())
    }

    fn create_signature() -> Signature {
        Signature {
            name: GIT_USER.to_string(),
            email: GIT_EMAIL.to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        }
    }

    // Not using testutils::user_settings() because there is a dependency cycle
    // 'jj_lib (1) -> testutils -> jj_lib (2)' which creates another distinct
    // UserSettings type. testutils returns jj_lib (2)'s UserSettings, whereas
    // our UserSettings type comes from jj_lib (1).
    fn user_settings() -> UserSettings {
        let config = StackedConfig::with_defaults();
        UserSettings::from_config(config).unwrap()
    }
}
