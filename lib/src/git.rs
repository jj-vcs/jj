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

//! Git interoperability functions.
//!
//! # Reference handling
//!
//! * `jj git import` applies the diff between the backing Git repo and jj's
//!   `git_refs`, and updates jj's `remotes` view.
//! * `jj git export` copies jj's `remotes` view back to the Git repo. If a ref
//!   in the Git repo has been updated since the last import, the ref isn't
//!   exported.
//! * `jj op restore` never rolls back `git_refs`. This means the restored state
//!   can be exported to Git but not re-imported.

#![expect(missing_docs)]

use std::borrow::Borrow;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::default::Default;
use std::ffi::OsString;
use std::fs::File;
use std::iter;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bstr::BStr;
use bstr::BString;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use gix::error::ResultExt as _;
use gix::refs::transaction::RefEdit;
use gix::refspec::Instruction;
use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::TreeValue;
use crate::commit::Commit;
use crate::config::ConfigGetError;
use crate::git_backend::GitBackend;
pub use crate::git_remote::GitProgress;
pub use crate::git_remote::GitRemoteCallback;
pub use crate::git_remote::GitSidebandLineTerminator;
use crate::index::IndexError;
use crate::matchers::EverythingMatcher;
use crate::merge::Diff;
use crate::merged_tree::MergedTree;
use crate::merged_tree::TreeDiffEntry;
use crate::object_id::ObjectId as _;
use crate::op_store::ABSENT_REF_TARGET;
use crate::op_store::ABSENT_REMOTE_REF;
use crate::op_store::RefTarget;
use crate::op_store::RefTargetOptionExt as _;
use crate::op_store::RemoteRef;
use crate::op_store::RemoteRefState;
use crate::ref_name::GitRefName;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefName;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteNameBuf;
use crate::ref_name::RemoteRefSymbol;
use crate::ref_name::RemoteRefSymbolBuf;
use crate::ref_name::WorkspaceName;
use crate::repo::MutableRepo;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetStreamExt as _;
use crate::settings::UserSettings;
use crate::store::Store;
use crate::str_util::StringExpression;
use crate::str_util::StringMatcher;
use crate::str_util::StringPattern;
use crate::view::View;

/// Reserved remote name for the backing Git repo.
pub const REMOTE_NAME_FOR_LOCAL_GIT_REPO: &RemoteName = RemoteName::new("git");
/// Git ref prefix that would conflict with the reserved "git" remote.
pub const RESERVED_REMOTE_REF_NAMESPACE: &str = "refs/remotes/git/";
/// Git ref prefix where remote bookmarks are stored.
const REMOTE_BOOKMARK_REF_NAMESPACE: &str = "refs/remotes/";
/// Git ref prefix where remote refs will be temporarily fetched.
const FETCH_BOOKMARK_REF_NAMESPACE: &str = "refs/jj/remote-bookmarks/";
/// Git ref prefix where remote tags will be temporarily fetched.
const REMOTE_TAG_REF_NAMESPACE: &str = "refs/jj/remote-tags/";
/// Ref name used as a placeholder to unset HEAD without a commit.
///
/// This is not a normal branch ref, and is deliberately left unborn: HEAD is
/// pointed at it symbolically while the ref itself is kept nonexistent. That
/// is how jj represents "HEAD has no commit yet".
const UNBORN_ROOT_REF_NAME: &str = "refs/jj/root";
/// Dummy file to be added to the index to indicate that the user is editing a
/// commit with a conflict that isn't represented in the Git index.
const INDEX_DUMMY_CONFLICT_FILE: &str = ".jj-do-not-resolve-this-conflict";

#[derive(Clone, Debug)]
pub struct GitSettings {
    pub abandon_unreachable_commits: bool,
    pub record_synthetic_predecessors: bool,
    pub write_change_id_header: bool,
}

impl GitSettings {
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            abandon_unreachable_commits: settings.get_bool("git.abandon-unreachable-commits")?,
            record_synthetic_predecessors: settings
                .get_bool("git.record-synthetic-predecessors")?,
            write_change_id_header: settings.get("git.write-change-id-header")?,
        })
    }
}

/// Configuration for native Git remote operations.
#[derive(Clone, Debug, Default)]
pub struct GitRemoteOptions {
    /// Environment for authentication helpers, SSH, and other configured commands.
    /// These overrides apply to this operation without changing process state.
    pub environment: HashMap<OsString, OsString>,
}

#[derive(Debug, Error)]
pub enum GitRemoteNameError {
    #[error(
        "Git remote named '{name}' is reserved for local Git repository",
        name = REMOTE_NAME_FOR_LOCAL_GIT_REPO.as_symbol()
    )]
    ReservedForLocalGitRepo,
    #[error("Git remotes with slashes are incompatible with jj: {}", .0.as_symbol())]
    WithSlash(RemoteNameBuf),
    #[error("Invalid Git remote name")]
    InvalidName(#[from] gix::Error),
}

fn validate_remote_name(name: &RemoteName) -> Result<(), GitRemoteNameError> {
    gix::remote::name::validated(name.as_str())?;
    if name == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        Err(GitRemoteNameError::ReservedForLocalGitRepo)
    } else if name.as_str().contains('/') {
        Err(GitRemoteNameError::WithSlash(name.to_owned()))
    } else {
        Ok(())
    }
}

/// Converts [`CommitId`] of valid length to [`gix::oid`].
fn oid_from_commit_id(id: &CommitId) -> &gix::oid {
    gix::oid::from_bytes_unchecked(id.as_bytes())
}

/// Converts [`CommitId`] of valid length to [`gix::ObjectId`].
fn owned_oid_from_commit_id(id: &CommitId) -> gix::ObjectId {
    gix::ObjectId::from_bytes_or_panic(id.as_bytes())
}

/// Type of Git ref to be imported or exported.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitRefKind {
    Bookmark,
    Tag,
}

/// Stats from a git push
#[derive(Debug, Default)]
pub struct GitPushStats {
    /// reference accepted by the remote
    pub pushed: Vec<GitRefNameBuf>,
    /// rejected reference, due to lease failure, with an optional reason
    pub rejected: Vec<(GitRefNameBuf, Option<String>)>,
    /// reference rejected by the remote, with an optional reason
    pub remote_rejected: Vec<(GitRefNameBuf, Option<String>)>,
}

impl GitPushStats {
    pub fn all_ok(&self) -> bool {
        self.rejected.is_empty() && self.remote_rejected.is_empty()
    }
}

/// Newtype to look up `HashMap` entry by key of shorter lifetime.
///
/// https://users.rust-lang.org/t/unexpected-lifetime-issue-with-hashmap-remove/113961/6
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct RemoteRefKey<'a>(RemoteRefSymbol<'a>);

impl<'a: 'b, 'b> Borrow<RemoteRefSymbol<'b>> for RemoteRefKey<'a> {
    fn borrow(&self) -> &RemoteRefSymbol<'b> {
        &self.0
    }
}

/// Representation of a Git refspec
///
/// It is often the case that we need only parts of the refspec,
/// Passing strings around and repeatedly parsing them is sub-optimal, confusing
/// and error prone
#[derive(Debug, Hash, PartialEq, Eq)]
pub(crate) struct RefSpec {
    forced: bool,
    // Source and destination may be fully-qualified ref name, glob pattern, or
    // object ID. The GitRefNameBuf type shouldn't be used.
    source: Option<String>,
    destination: String,
}

impl RefSpec {
    fn forced(source: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            forced: true,
            source: Some(source.into()),
            destination: destination.into(),
        }
    }

    pub(crate) fn to_git_format(&self) -> String {
        format!(
            "{}{}",
            if self.forced { "+" } else { "" },
            self.to_git_format_not_forced()
        )
    }

    /// Format git refspec without the leading force flag '+'
    ///
    /// When independently setting --force-with-lease, having the
    /// leading flag overrides the lease, so we need to print it
    /// without it
    pub(crate) fn to_git_format_not_forced(&self) -> String {
        if let Some(s) = &self.source {
            format!("{}:{}", s, self.destination)
        } else {
            format!(":{}", self.destination)
        }
    }
}

/// Representation of a negative Git refspec
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct NegativeRefSpec {
    source: String,
}

impl NegativeRefSpec {
    fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }

    pub(crate) fn to_git_format(&self) -> String {
        format!("^{}", self.source)
    }
}

/// Translates Git ref name to jj's `name@remote` symbol. Returns `None` if the
/// ref cannot be represented in jj.
pub fn parse_git_ref(full_name: &GitRefName) -> Option<(GitRefKind, RemoteRefSymbol<'_>)> {
    if let Some(name) = full_name.as_str().strip_prefix("refs/heads/") {
        // Git CLI says 'HEAD' is not a valid branch name
        if name == "HEAD" {
            return None;
        }
        let name = RefName::new(name);
        let remote = REMOTE_NAME_FOR_LOCAL_GIT_REPO;
        Some((GitRefKind::Bookmark, RemoteRefSymbol { name, remote }))
    } else if let Some(remote_and_name) = full_name
        .as_str()
        .strip_prefix(REMOTE_BOOKMARK_REF_NAMESPACE)
    {
        let (remote, name) = remote_and_name.split_once('/')?;
        // "refs/remotes/origin/HEAD" isn't a real remote-tracking branch
        if remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO || name == "HEAD" {
            return None;
        }
        let name = RefName::new(name);
        let remote = RemoteName::new(remote);
        Some((GitRefKind::Bookmark, RemoteRefSymbol { name, remote }))
    } else if let Some(name) = full_name.as_str().strip_prefix("refs/tags/") {
        let name = RefName::new(name);
        let remote = REMOTE_NAME_FOR_LOCAL_GIT_REPO;
        Some((GitRefKind::Tag, RemoteRefSymbol { name, remote }))
    } else {
        None
    }
}

fn parse_fetched_ref(full_name: &GitRefName) -> Option<(GitRefKind, RemoteRefSymbol<'_>)> {
    let (kind, remote_and_name) = [
        (GitRefKind::Bookmark, FETCH_BOOKMARK_REF_NAMESPACE),
        (GitRefKind::Tag, REMOTE_TAG_REF_NAMESPACE),
    ]
    .into_iter()
    .find_map(|(kind, prefix)| Some((kind, full_name.as_str().strip_prefix(prefix)?)))?;
    let (remote, name) = remote_and_name.split_once('/')?;
    if remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        return None;
    }
    let name = RefName::new(name);
    let remote = RemoteName::new(remote);
    Some((kind, RemoteRefSymbol { name, remote }))
}

fn to_git_ref_name(kind: GitRefKind, symbol: RemoteRefSymbol<'_>) -> Option<GitRefNameBuf> {
    let RemoteRefSymbol { name, remote } = symbol;
    let name = name.as_str();
    let remote = remote.as_str();
    if name.is_empty() || remote.is_empty() {
        return None;
    }
    match kind {
        GitRefKind::Bookmark => {
            if name == "HEAD" {
                return None;
            }
            if remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
                Some(format!("refs/heads/{name}").into())
            } else {
                Some(format!("{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/{name}").into())
            }
        }
        GitRefKind::Tag => {
            // Only local tags are mapped. Remote tags don't exist in Git world.
            (remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO).then(|| format!("refs/tags/{name}").into())
        }
    }
}

fn to_git_or_remote_tag_ref_name(symbol: RemoteRefSymbol<'_>) -> GitRefNameBuf {
    let RemoteRefSymbol { name, remote } = symbol;
    let name = name.as_str();
    let remote = remote.as_str();
    if remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        format!("refs/tags/{name}").into()
    } else {
        format!("{REMOTE_TAG_REF_NAMESPACE}{remote}/{name}").into()
    }
}

#[derive(Debug, Error)]
#[error("The repo is not backed by a Git repo")]
pub struct UnexpectedGitBackendError;

/// Returns the underlying `GitBackend` implementation.
pub fn get_git_backend(store: &Store) -> Result<&GitBackend, UnexpectedGitBackendError> {
    store.backend_impl().ok_or(UnexpectedGitBackendError)
}

/// Returns new thread-local instance to access to the underlying Git repo.
pub fn get_git_repo(store: &Store) -> Result<gix::Repository, UnexpectedGitBackendError> {
    get_git_backend(store).map(|backend| backend.git_repo())
}

/// Checks if `git_ref` points to a Git commit object, and returns its id.
///
/// If the ref points to the previously `known_commit_oid` (i.e. unchanged),
/// this should be faster than `git_ref.into_fully_peeled_id()`.
fn resolve_git_ref_to_commit_id(
    git_ref: &gix::Reference,
    known_commit_oid: Option<&gix::oid>,
) -> Option<gix::ObjectId> {
    let mut peeling_ref = Cow::Borrowed(git_ref);

    // Try fast path if we have a candidate id which is known to be a commit object.
    if let Some(known_oid) = known_commit_oid {
        let raw_ref = &git_ref.inner;
        if let Some(oid) = raw_ref.target.try_id()
            && oid == known_oid
        {
            return Some(oid.to_owned());
        }
        if let Some(oid) = raw_ref.peeled
            && oid == known_oid
        {
            // Perhaps an annotated tag stored in packed-refs file, and pointing to the
            // already known target commit.
            return Some(oid);
        }
        // A tag (according to ref name.) Try to peel one more level. This is slightly
        // faster than recurse into into_fully_peeled_id(). If we recorded a tag oid, we
        // could skip this at all.
        if raw_ref.peeled.is_none() && git_ref.name().as_bstr().starts_with(b"refs/tags/") {
            let maybe_tag = git_ref
                .try_id()
                .and_then(|id| id.object().ok())
                .and_then(|object| object.try_into_tag().ok());
            if let Some(oid) = maybe_tag.as_ref().and_then(|tag| tag.target_id().ok()) {
                let oid = oid.detach();
                if oid == known_oid {
                    // An annotated tag pointing to the already known target commit.
                    return Some(oid);
                }
                // Unknown id. Recurse from the current state. A tag may point to
                // non-commit object.
                peeling_ref.to_mut().inner.target = gix::refs::Target::Object(oid);
            }
        }
    }

    // Alternatively, we might want to inline the first half of the peeling
    // loop. into_fully_peeled_id() looks up the target object to see if it's
    // a tag or not, and we need to check if it's a commit object.
    let peeled_id = peeling_ref.into_owned().into_fully_peeled_id().ok()?;
    let is_commit = peeled_id
        .object()
        .is_ok_and(|object| object.kind.is_commit());
    is_commit.then_some(peeled_id.detach())
}

#[derive(Error, Debug)]
pub enum GitImportError {
    #[error("Failed to read Git HEAD target commit {id}")]
    MissingHeadTarget {
        id: CommitId,
        #[source]
        err: BackendError,
    },
    #[error("Ancestor of Git ref {symbol} is missing")]
    MissingRefAncestor {
        symbol: RemoteRefSymbolBuf,
        #[source]
        err: BackendError,
    },
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    RevsetEvaluation(#[from] RevsetEvaluationError),
    #[error(transparent)]
    Git(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitImportError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Git(source.into())
    }
}

/// Options for [`import_refs()`].
#[derive(Debug)]
pub struct GitImportOptions {
    /// Whether to abandon commits that became unreachable in Git.
    pub abandon_unreachable_commits: bool,
    /// Whether to generate synthetic predecessors for imported commits.
    pub record_synthetic_predecessors: bool,
    /// Per-remote patterns whether to track bookmarks automatically.
    pub remote_auto_track_bookmarks: HashMap<RemoteNameBuf, StringMatcher>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitImportRefUpdate {
    pub symbol: RemoteRefSymbolBuf,
    pub old_remote_ref: RemoteRef,
    pub new_target: RefTarget,
}

impl GitImportRefUpdate {
    pub fn new(
        symbol: RemoteRefSymbolBuf,
        old_remote_ref: RemoteRef,
        new_target: RefTarget,
    ) -> Self {
        Self {
            symbol,
            old_remote_ref,
            new_target,
        }
    }
}

/// Describes changes made by `import_refs()` or `fetch()`.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct GitImportStats {
    /// Commits that are no longer reachable nor rewritten to the new commits.
    pub abandoned_commits: Vec<Commit>,
    /// Commits that have been rewritten to the new commits.
    pub rewritten_commit_ids: HashSet<CommitId>,
    /// Remote bookmark updates to be merged in to the local bookmarks, sorted
    /// by `symbol`.
    pub changed_remote_bookmarks: Vec<GitImportRefUpdate>,
    /// Remote tag updates to be merged in to the local tags, sorted by
    /// `symbol`.
    pub changed_remote_tags: Vec<GitImportRefUpdate>,
    /// Git ref names that couldn't be imported, sorted by name.
    ///
    /// This list doesn't include refs that are supposed to be ignored, such as
    /// refs pointing to non-commit objects.
    pub failed_ref_names: Vec<BString>,
}

#[derive(Debug)]
struct RefsToImport {
    /// Git ref `(full_name, new_target)`s to be copied to the view, sorted by
    /// `full_name`.
    changed_git_refs: Vec<(GitRefNameBuf, RefTarget)>,
    /// Remote bookmark updates to be merged in to the local bookmarks, sorted
    /// by `symbol`.
    changed_remote_bookmarks: Vec<GitImportRefUpdate>,
    /// Remote tag updates to be merged in to the local tags, sorted by
    /// `symbol`.
    changed_remote_tags: Vec<GitImportRefUpdate>,
    /// Git ref names that couldn't be imported, sorted by name.
    failed_ref_names: Vec<BString>,
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// This function detects conflicts (if both Git and JJ modified a bookmark) and
/// records them in JJ's view.
pub async fn import_refs(
    mut_repo: &mut MutableRepo,
    options: &GitImportOptions,
) -> Result<GitImportStats, GitImportError> {
    import_some_refs(mut_repo, options, |_, _| true).await
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// Only bookmarks and tags whose remote symbol pass the filter will be
/// considered for addition, update, or deletion.
pub async fn import_some_refs(
    mut_repo: &mut MutableRepo,
    options: &GitImportOptions,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<GitImportStats, GitImportError> {
    let git_repo = get_git_repo(mut_repo.store())?;

    // Allocate views for new remotes configured externally. There may be
    // remotes with no refs, but the user might still want to "track" absent
    // remote refs.
    for remote_name in iter_remote_names(&git_repo) {
        mut_repo.ensure_remote(&remote_name);
    }

    // Exclude real remote tags, which should never be updated by Git.
    let refs_to_import = diff_refs_to_import(mut_repo.view(), &git_repo, git_ref_filter)?;
    import_refs_inner(mut_repo, refs_to_import, options).await
}

async fn import_refs_inner(
    mut_repo: &mut MutableRepo,
    refs_to_import: RefsToImport,
    options: &GitImportOptions,
) -> Result<GitImportStats, GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store).expect("backend type should have been tested");

    let RefsToImport {
        changed_git_refs,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    } = refs_to_import;

    let iter_changed_refs = || itertools::chain(&changed_remote_bookmarks, &changed_remote_tags);
    // List of changed old/new ref heads, which may include duplicates.
    let (old_referenced_heads, new_referenced_heads) = {
        let mut old_heads = Vec::new();
        let mut new_heads = Vec::new();
        for update in iter_changed_refs() {
            old_heads.extend(update.old_remote_ref.target.present_adds().cloned());
            new_heads.extend(update.new_target.present_adds().cloned());
        }
        (old_heads, new_heads)
    };
    let old_visible_heads = mut_repo.view().heads().iter().cloned().collect_vec();

    // Bulk-import all reachable Git commits to the backend to reduce overhead
    // of table merging and ref updates.
    //
    // changed_git_refs aren't respected because changed_remote_bookmarks/tags
    // should include all heads that will become reachable in jj.
    let index = mut_repo.index();
    let missing_head_ids: Vec<&CommitId> = stream::iter(&new_referenced_heads)
        .map(async move |id| (id, index.has_id(id).await))
        .buffered(mut_repo.store().concurrency())
        .filter_map(async move |m| match m {
            (id, Ok(false)) => Some(Ok(id)),
            (_, Ok(true)) => None,
            (_, Err(err)) => Some(Err(GitImportError::Index(err))),
        })
        .try_collect()
        .await?;
    let heads_imported = git_backend.import_head_commits(missing_head_ids).is_ok();

    // Import new remote heads
    let mut head_commits = Vec::new();
    let get_commit = async |id: &CommitId, symbol: &RemoteRefSymbolBuf| {
        let missing_ref_err = |err| GitImportError::MissingRefAncestor {
            symbol: symbol.clone(),
            err,
        };
        // If bulk-import failed, try again to find bad head or ref.
        if !heads_imported && !index.has_id(id).await? {
            git_backend
                .import_head_commits([id])
                .map_err(missing_ref_err)?;
        }
        store.get_commit_async(id).await.map_err(missing_ref_err)
    };
    // Uses iter_changed_refs() instead of new_referenced_heads to report error
    // with ref name.
    for update in iter_changed_refs() {
        for id in update.new_target.present_adds() {
            let commit = get_commit(id, &update.symbol).await?;
            head_commits.push(commit);
        }
    }
    // It's unlikely the imported commits were missing, but I/O-related error
    // can still occur.
    let imported_commits = mut_repo.index_commits(&head_commits).await?;
    mut_repo.add_heads(&head_commits).await?;

    // Apply the change that happened in git since last time we imported refs.
    for (full_name, new_target) in changed_git_refs {
        mut_repo.set_git_ref_target(&full_name, new_target);
    }
    for update in &changed_remote_bookmarks {
        let symbol = update.symbol.as_ref();
        let base_target = update.old_remote_ref.tracked_target();
        let new_remote_ref = RemoteRef {
            target: update.new_target.clone(),
            state: if update.old_remote_ref != ABSENT_REMOTE_REF {
                update.old_remote_ref.state
            } else {
                default_remote_ref_state_for(GitRefKind::Bookmark, symbol, options)
            },
        };
        if new_remote_ref.is_tracked() {
            mut_repo
                .merge_local_bookmark(symbol.name, base_target, &new_remote_ref.target)
                .await?;
        }
        // Remote-tracking branch is the last known state of the branch in the remote.
        // It shouldn't diverge even if we had inconsistent view.
        mut_repo.set_remote_bookmark(symbol, new_remote_ref);
    }
    for update in &changed_remote_tags {
        let symbol = update.symbol.as_ref();
        let base_target = update.old_remote_ref.tracked_target();
        let new_remote_ref = RemoteRef {
            target: update.new_target.clone(),
            state: if update.old_remote_ref != ABSENT_REMOTE_REF {
                update.old_remote_ref.state
            } else {
                default_remote_ref_state_for(GitRefKind::Tag, symbol, options)
            },
        };
        if new_remote_ref.is_tracked() {
            mut_repo
                .merge_local_tag(symbol.name, base_target, &new_remote_ref.target)
                .await?;
        }
        // Remote-tracking tag is the last known state of the tag in the remote.
        // It shouldn't diverge even if we had inconsistent view.
        mut_repo.set_remote_tag(symbol, new_remote_ref);
    }

    let any_old_referenced = !old_referenced_heads.is_empty();
    let any_new_referenced = !new_referenced_heads.is_empty();
    let old_visible_heads = RevsetExpression::commits(old_visible_heads);
    let old_referenced_heads = RevsetExpression::commits(old_referenced_heads);
    let new_referenced_heads = RevsetExpression::commits(new_referenced_heads);
    let mut abandoned_commits = if options.abandon_unreachable_commits && any_old_referenced {
        abandon_unreachable_commits(mut_repo, &old_referenced_heads).await?
    } else {
        vec![]
    };
    let rewritten_commit_ids = if options.record_synthetic_predecessors && any_new_referenced {
        record_synthetic_predecessors(
            mut_repo,
            &old_visible_heads,
            Diff::new(&old_referenced_heads, &new_referenced_heads),
            &imported_commits,
            // TODO: Maybe enable rewriting unconditionally? This should be more
            // reliable than reachability-based heuristic.
            options.abandon_unreachable_commits,
        )
        .await?
    } else {
        HashSet::new()
    };
    abandoned_commits.retain(|commit| !rewritten_commit_ids.contains(commit.id()));
    let stats = GitImportStats {
        abandoned_commits,
        rewritten_commit_ids,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    };
    Ok(stats)
}

/// Finds commits that used to be reachable in git that no longer are reachable.
/// Those commits will be recorded as abandoned in the `MutableRepo`.
async fn abandon_unreachable_commits(
    mut_repo: &mut MutableRepo,
    hidable_git_heads: &Arc<ResolvedRevsetExpression>,
) -> Result<Vec<Commit>, GitImportError> {
    let pinned_expression = RevsetExpression::union_all(&[
        // Local refs are usually visible, no need to filter out hidden
        RevsetExpression::commits(pinned_commit_ids(mut_repo.view())),
        RevsetExpression::commits(remotely_pinned_commit_ids(mut_repo.view()))
            // Hidden remote refs should not contribute to pinning
            .intersection(&RevsetExpression::visible_heads().ancestors()),
        RevsetExpression::root(),
    ]);
    let abandoned_expression = pinned_expression
        .range(hidable_git_heads)
        // Don't include already-abandoned commits in GitImportStats
        .intersection(&RevsetExpression::visible_heads().ancestors());
    let abandoned_commits: Vec<_> = abandoned_expression
        .evaluate(mut_repo)?
        .stream()
        .commits(mut_repo.store())
        .try_collect()
        .await?;
    for commit in &abandoned_commits {
        mut_repo.record_abandoned_commit(commit);
    }
    Ok(abandoned_commits)
}

/// Deduces predecessors of `old_visible_heads..new_referenced_heads` based on
/// change IDs, records synthetic predecessors, and updates parent mappings.
///
/// The `imported_commits` should exclude any pre-existing commits, including
/// those that were previously hidden.
///
/// Returns old commit IDs that have been mapped to the new commits.
async fn record_synthetic_predecessors(
    mut_repo: &mut MutableRepo,
    old_visible_heads: &Arc<ResolvedRevsetExpression>,
    Diff {
        before: old_referenced_heads,
        after: new_referenced_heads,
    }: Diff<&Arc<ResolvedRevsetExpression>>,
    imported_commits: &[Commit],
    rewrite_commits: bool,
) -> Result<HashSet<CommitId>, GitImportError> {
    let build_change_to_commit_ids_map = async |expr: Arc<ResolvedRevsetExpression>| {
        let mut change_to_commit_ids: HashMap<ChangeId, Vec<CommitId>> = HashMap::new();
        let mut stream = expr.evaluate(mut_repo)?.commit_change_ids();
        while let Some((commit_id, change_id)) = stream.try_next().await? {
            let commit_ids = change_to_commit_ids.entry(change_id).or_default();
            commit_ids.push(commit_id);
        }
        Ok::<_, GitImportError>(change_to_commit_ids)
    };
    let old_referenced_change_to_commit_ids =
        build_change_to_commit_ids_map(new_referenced_heads.range(old_referenced_heads)).await?;
    let new_referenced_change_to_commit_ids =
        build_change_to_commit_ids_map(old_visible_heads.range(new_referenced_heads)).await?;
    let imported_commit_ids: HashSet<_> = imported_commits.iter().map(Commit::id).collect();
    let rewritable_commit_ids: HashSet<_> = if rewrite_commits {
        // Similar to old_referenced_change_to_commit_ids, but doesn't include
        // previously abandoned commits, which shouldn't be rewritten again.
        new_referenced_heads
            .range(old_referenced_heads)
            .intersection(&old_visible_heads.ancestors())
            .evaluate(mut_repo)?
            .stream()
            .try_collect()
            .await?
    } else {
        HashSet::new()
    };

    let mut rewritten_commit_ids = HashSet::new();
    for (change_id, new_commit_ids) in &new_referenced_change_to_commit_ids {
        let predecessor_id: Option<CommitId>;
        let rewrite_source_ids: &[CommitId];
        if let Some(old_commit_ids) = old_referenced_change_to_commit_ids.get(change_id) {
            // Pick the latest one if previously diverged. Divergence isn't
            // usually resolved by "squashing" the commits.
            predecessor_id = Some(old_commit_ids[0].clone());
            rewrite_source_ids = old_commit_ids;
        } else {
            // Record as newly created commit
            predecessor_id = None;
            rewrite_source_ids = &[];
        }
        // Predecessors are recorded only for newly imported commits to prevent
        // cycles in the evolution graph. While this restriction can be lifted
        // later, note that the existing predecessor chain may be more detailed
        // than "imported from Git" if the original commits were created locally.
        for new_commit_id in new_commit_ids
            .iter()
            .filter(|&id| imported_commit_ids.contains(id))
        {
            mut_repo.set_predecessors(new_commit_id.clone(), predecessor_id.as_slice().to_vec());
        }
        let rewrite_source_ids = rewrite_source_ids
            .iter()
            .filter(|id| rewritable_commit_ids.contains(id));
        if let [new_commit_id] = &**new_commit_ids {
            for old_commit_id in rewrite_source_ids {
                mut_repo.set_rewritten_commit(old_commit_id.clone(), new_commit_id.clone());
                rewritten_commit_ids.insert(old_commit_id.clone());
            }
        } else {
            for old_commit_id in rewrite_source_ids {
                mut_repo
                    .set_divergent_rewrite(old_commit_id.clone(), new_commit_ids.iter().cloned());
                rewritten_commit_ids.insert(old_commit_id.clone());
            }
        }
    }

    Ok(rewritten_commit_ids)
}

/// Calculates diff of local and remote Git refs to import.
fn diff_refs_to_import(
    view: &View,
    git_repo: &gix::Repository,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<RefsToImport, GitImportError> {
    let mut known_git_refs = view
        .git_refs()
        .iter()
        .filter_map(|(full_name, target)| {
            // TODO: or clean up invalid ref in case it was stored due to historical bug?
            let (kind, symbol) =
                parse_git_ref(full_name).expect("stored git ref should be parsable");
            git_ref_filter(kind, symbol).then_some((full_name.as_ref(), target))
        })
        .collect();

    let mut changed_git_refs = Vec::new();
    let mut failed_ref_names = Vec::new();
    let actual = git_repo.references().map_err(GitImportError::from_git)?;
    collect_changed_git_refs_to_import(
        actual.local_branches().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut changed_git_refs,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    collect_changed_git_refs_to_import(
        actual.remote_branches().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut changed_git_refs,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    collect_changed_git_refs_to_import(
        actual.tags().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut changed_git_refs,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    for full_name in known_git_refs.into_keys() {
        changed_git_refs.push((full_name.to_owned(), RefTarget::absent()));
    }

    let mut changed_remote_bookmarks = Vec::new();
    let mut changed_remote_tags = Vec::new();
    for (full_name, new_target) in &changed_git_refs {
        let (kind, symbol) = parse_git_ref(full_name).expect("should be parsed before");
        let (changes, old_remote_ref) = match kind {
            GitRefKind::Bookmark => (
                &mut changed_remote_bookmarks,
                view.get_remote_bookmark(symbol),
            ),
            GitRefKind::Tag => (&mut changed_remote_tags, view.get_remote_tag(symbol)),
        };
        if *new_target != old_remote_ref.target {
            changes.push(GitImportRefUpdate::new(
                symbol.to_owned(),
                old_remote_ref.clone(),
                new_target.clone(),
            ));
        }
    }

    // Stabilize merge order and output.
    changed_git_refs.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));
    changed_remote_bookmarks
        .sort_unstable_by(|update1, update2| update1.symbol.cmp(&update2.symbol));
    changed_remote_tags.sort_unstable_by(|update1, update2| update1.symbol.cmp(&update2.symbol));
    failed_ref_names.sort_unstable();
    Ok(RefsToImport {
        changed_git_refs,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    })
}

/// Calculates diff of fetched refs to import.
fn diff_fetched_refs_to_import(
    view: &View,
    git_repo: &gix::Repository,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<RefsToImport, GitImportError> {
    let mut known_remote_bookmarks = view
        .all_remote_bookmarks()
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Bookmark, symbol))
        .map(|(symbol, remote_ref)| (RemoteRefKey(symbol), remote_ref))
        .collect();
    let mut known_remote_tags = view
        .all_remote_tags()
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Tag, symbol))
        .map(|(symbol, remote_ref)| (RemoteRefKey(symbol), remote_ref))
        .collect();

    // TODO: Refactor git_ref_filter in a way that uninteresting refs don't have
    // to be scanned. For example, if the caller imports bookmark changes from a
    // specific remote, we only need to walk refs/jj/remote-{kind}/{remote}.
    let mut changed_remote_bookmarks = Vec::new();
    let mut changed_remote_tags = Vec::new();
    let mut failed_ref_names = Vec::new();
    let actual = git_repo.references().map_err(GitImportError::from_git)?;
    collect_changed_fetched_refs_to_import(
        actual
            .prefixed(FETCH_BOOKMARK_REF_NAMESPACE)
            .map_err(GitImportError::from_git)?,
        &mut known_remote_bookmarks,
        &mut changed_remote_bookmarks,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    collect_changed_fetched_refs_to_import(
        actual
            .prefixed(REMOTE_TAG_REF_NAMESPACE)
            .map_err(GitImportError::from_git)?,
        &mut known_remote_tags,
        &mut changed_remote_tags,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    for (RemoteRefKey(symbol), old) in known_remote_bookmarks {
        if old.is_present() {
            changed_remote_bookmarks.push(GitImportRefUpdate::new(
                symbol.to_owned(),
                old.clone(),
                RefTarget::absent(),
            ));
        }
    }
    for (RemoteRefKey(symbol), old) in known_remote_tags {
        if old.is_present() {
            changed_remote_tags.push(GitImportRefUpdate::new(
                symbol.to_owned(),
                old.clone(),
                RefTarget::absent(),
            ));
        }
    }

    // Stabilize merge order and output.
    changed_remote_bookmarks
        .sort_unstable_by(|update1, update2| update1.symbol.cmp(&update2.symbol));
    changed_remote_tags.sort_unstable_by(|update1, update2| update1.symbol.cmp(&update2.symbol));
    failed_ref_names.sort_unstable();
    Ok(RefsToImport {
        changed_git_refs: vec![],
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    })
}

fn collect_changed_git_refs_to_import(
    actual_git_refs: gix::reference::iter::Iter,
    known_git_refs: &mut HashMap<&GitRefName, &RefTarget>,
    changed_git_refs: &mut Vec<(GitRefNameBuf, RefTarget)>,
    failed_ref_names: &mut Vec<BString>,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<(), GitImportError> {
    for git_ref in actual_git_refs {
        let git_ref = git_ref.map_err(GitImportError::from_git)?;
        let full_name_bytes = git_ref.name().as_bstr();
        let Ok(full_name) = str::from_utf8(full_name_bytes) else {
            // Non-utf8 refs cannot be imported.
            failed_ref_names.push(full_name_bytes.to_owned());
            continue;
        };
        if full_name.starts_with(RESERVED_REMOTE_REF_NAMESPACE) {
            failed_ref_names.push(full_name_bytes.to_owned());
            continue;
        }
        let full_name = GitRefName::new(full_name);
        let Some((kind, symbol)) = parse_git_ref(full_name) else {
            // Skip special refs such as refs/remotes/*/HEAD.
            continue;
        };
        if !git_ref_filter(kind, symbol) {
            continue;
        }
        let old_git_target = known_git_refs.get(full_name).copied().flatten();
        let old_git_oid = old_git_target.as_normal().map(oid_from_commit_id);
        let Some(oid) = resolve_git_ref_to_commit_id(&git_ref, old_git_oid) else {
            // Skip (or remove existing) invalid refs.
            continue;
        };
        let new_target = RefTarget::normal(CommitId::from_bytes(oid.as_bytes()));
        known_git_refs.remove(full_name);
        if new_target != *old_git_target {
            changed_git_refs.push((full_name.to_owned(), new_target));
        }
    }
    Ok(())
}

/// Similar to [`collect_changed_git_refs_to_import()`], but compares remote
/// refs. Fetched refs should be managed solely by jj.
fn collect_changed_fetched_refs_to_import(
    actual_git_refs: gix::reference::iter::Iter,
    known_remote_refs: &mut HashMap<RemoteRefKey<'_>, &RemoteRef>,
    changed_remote_refs: &mut Vec<GitImportRefUpdate>,
    failed_ref_names: &mut Vec<BString>,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<(), GitImportError> {
    for git_ref in actual_git_refs {
        let git_ref = git_ref.map_err(GitImportError::from_git)?;
        let full_name_bytes = git_ref.name().as_bstr();
        let Ok(full_name) = str::from_utf8(full_name_bytes) else {
            // Non-utf8 refs cannot be imported.
            failed_ref_names.push(full_name_bytes.to_owned());
            continue;
        };
        let full_name = GitRefName::new(full_name);
        let Some((kind, symbol)) = parse_fetched_ref(full_name) else {
            // Skip invalid ref names.
            continue;
        };
        if !git_ref_filter(kind, symbol) {
            continue;
        }
        let old_remote_ref = known_remote_refs
            .get(&symbol)
            .copied()
            .unwrap_or(&ABSENT_REMOTE_REF);
        let old_git_oid = old_remote_ref.target.as_normal().map(oid_from_commit_id);
        let Some(oid) = resolve_git_ref_to_commit_id(&git_ref, old_git_oid) else {
            // Skip (or remove existing) invalid refs.
            continue;
        };
        let new_target = RefTarget::normal(CommitId::from_bytes(oid.as_bytes()));
        known_remote_refs.remove(&symbol);
        if new_target != old_remote_ref.target {
            changed_remote_refs.push(GitImportRefUpdate::new(
                symbol.to_owned(),
                old_remote_ref.clone(),
                new_target,
            ));
        }
    }
    Ok(())
}

fn default_remote_ref_state_for(
    kind: GitRefKind,
    symbol: RemoteRefSymbol<'_>,
    options: &GitImportOptions,
) -> RemoteRefState {
    match kind {
        GitRefKind::Bookmark => {
            if symbol.remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO
                || options
                    .remote_auto_track_bookmarks
                    .get(symbol.remote)
                    .is_some_and(|matcher| matcher.is_match(symbol.name.as_str()))
            {
                RemoteRefState::Tracked
            } else {
                RemoteRefState::New
            }
        }
        // TODO: add option to not track tags by default?
        GitRefKind::Tag => RemoteRefState::Tracked,
    }
}

/// Commits referenced by local branches or tags.
///
/// On `import_refs()`, this is similar to collecting commits referenced by
/// `view.git_refs()`. Main difference is that local branches can be moved by
/// tracking remotes, and such mutation isn't applied to `view.git_refs()` yet.
fn pinned_commit_ids(view: &View) -> Vec<CommitId> {
    itertools::chain(view.local_bookmarks(), view.local_tags())
        .flat_map(|(_, target)| target.present_adds())
        .cloned()
        .collect()
}

/// Commits referenced by untracked remote bookmarks/tags including hidden ones.
///
/// Tracked remote refs aren't included because they should have been merged
/// into the local counterparts, and the changes pulled from one remote should
/// propagate to the other remotes on later push. OTOH, untracked remote refs
/// are considered independent refs.
fn remotely_pinned_commit_ids(view: &View) -> Vec<CommitId> {
    itertools::chain(view.all_remote_bookmarks(), view.all_remote_tags())
        .filter(|(_, remote_ref)| !remote_ref.is_tracked())
        .map(|(_, remote_ref)| &remote_ref.target)
        .flat_map(|target| target.present_adds())
        .cloned()
        .collect()
}

/// Imports HEAD from the underlying Git repo.
///
/// Unlike `import_refs()`, the old HEAD branch is not abandoned because HEAD
/// move doesn't always mean the old HEAD branch has been rewritten.
///
/// Unlike `reset_head()`, this function doesn't move the working-copy commit to
/// the child of the new HEAD revision.
pub async fn import_head(
    mut_repo: &mut MutableRepo,
    workspace_name: &WorkspaceName,
    workspace_root: &Path,
) -> Result<(), GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store)?;
    let git_repo = git_backend
        .open_git_repo_at_workdir(workspace_root)
        .map_err(GitImportError::from_git)?;

    let old_git_head = mut_repo.view().git_head(workspace_name);
    let new_git_head_id = if let Ok(oid) = git_repo.head_id() {
        Some(CommitId::from_bytes(oid.as_bytes()))
    } else {
        None
    };
    if old_git_head.as_resolved() == Some(&new_git_head_id) {
        return Ok(());
    }

    // Import new head
    if let Some(head_id) = &new_git_head_id {
        let index = mut_repo.index();
        if !index.has_id(head_id).await? {
            git_backend.import_head_commits([head_id]).map_err(|err| {
                GitImportError::MissingHeadTarget {
                    id: head_id.clone(),
                    err,
                }
            })?;
        }
        // It's unlikely the imported commits were missing, but I/O-related
        // error can still occur.
        let commit = store.get_commit_async(head_id).await?;
        mut_repo.add_head(&commit).await?;
    }

    mut_repo.set_git_head_target(workspace_name, RefTarget::resolved(new_git_head_id));
    Ok(())
}

/// Imports the HEAD commit without updating the view.
pub async fn import_head_commit(
    mut_repo: &mut MutableRepo,
) -> Result<Option<Commit>, GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store)?;
    let git_repo = git_backend.git_repo();

    let Ok(oid) = git_repo.head_id() else {
        return Ok(None);
    };
    let head_id = CommitId::from_bytes(oid.as_bytes());

    let index = mut_repo.index();
    if !index.has_id(&head_id).await? {
        git_backend.import_head_commits([&head_id]).map_err(|err| {
            GitImportError::MissingHeadTarget {
                id: head_id.clone(),
                err,
            }
        })?;
    }
    // It's unlikely the imported commits were missing, but I/O-related error
    // can still occur.
    let commit = store.get_commit_async(&head_id).await?;
    mut_repo.add_head(&commit).await?;
    Ok(Some(commit))
}

#[derive(Error, Debug)]
pub enum GitExportError {
    #[error(transparent)]
    Git(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitExportError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Git(source.into())
    }
}

/// The reason we failed to export a ref to Git.
#[derive(Debug, Error)]
pub enum FailedRefExportReason {
    /// The name is not allowed in Git.
    #[error("Name is not allowed in Git")]
    InvalidGitName,
    /// The ref was in a conflicted state from the last import. A re-import
    /// should fix it.
    #[error("Ref was in a conflicted state from the last import")]
    ConflictedOldState,
    /// The ref points to the root commit, which Git doesn't have.
    #[error("Ref cannot point to the root commit in Git")]
    OnRootCommit,
    /// We wanted to delete it, but it had been modified in Git.
    #[error("Deleted ref had been modified in Git")]
    DeletedInJjModifiedInGit,
    /// We wanted to add it, but Git had added it with a different target
    #[error("Added ref had been added with a different target in Git")]
    AddedInJjAddedInGit,
    /// We wanted to modify it, but Git had deleted it
    #[error("Modified ref had been deleted in Git")]
    ModifiedInJjDeletedInGit,
    /// Failed to delete the ref from the Git repo
    #[error("Failed to delete")]
    FailedToDelete(#[source] gix::Error),
    /// Failed to set the ref in the Git repo
    #[error("Failed to set")]
    FailedToSet(#[source] gix::Error),
}

/// Describes changes made by [`export_refs()`].
#[derive(Debug)]
pub struct GitExportStats {
    /// Remote bookmarks that couldn't be exported, sorted by `symbol`.
    pub failed_bookmarks: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
    /// Remote tags that couldn't be exported, sorted by `symbol`.
    ///
    /// Since Git doesn't have remote tags, this list only contains `@git` tags.
    pub failed_tags: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
}

#[derive(Debug)]
struct AllRefsToExport {
    bookmarks: RefsToExport,
    tags: RefsToExport,
}

#[derive(Debug)]
struct RefsToExport {
    /// Remote `(symbol, (old_oid, new_oid))`s to update, sorted by `symbol`.
    to_update: Vec<(RemoteRefSymbolBuf, (Option<gix::ObjectId>, gix::ObjectId))>,
    /// Remote `(symbol, old_oid)`s to delete, sorted by `symbol`.
    ///
    /// Deletion has to be exported first to avoid conflict with new refs on
    /// file-system.
    to_delete: Vec<(RemoteRefSymbolBuf, gix::ObjectId)>,
    /// Remote refs that couldn't be exported, sorted by `symbol`.
    failed: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
}

/// Export changes to bookmarks and tags made in the Jujutsu repo compared to
/// our last seen view of the Git repo in `mut_repo.view().git_refs()`.
///
/// We ignore changed refs that are conflicted (were also changed in the Git
/// repo compared to our last remembered view of the Git repo). These will be
/// marked conflicted by the next `jj git import`.
///
/// New/updated tags are exported as Git lightweight tags.
pub fn export_refs(mut_repo: &mut MutableRepo) -> Result<GitExportStats, GitExportError> {
    export_some_refs(mut_repo, |_, _| true)
}

pub fn export_some_refs(
    mut_repo: &mut MutableRepo,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<GitExportStats, GitExportError> {
    fn get<'a, V>(map: &'a [(RemoteRefSymbolBuf, V)], key: RemoteRefSymbol<'_>) -> Option<&'a V> {
        debug_assert!(map.is_sorted_by_key(|(k, _)| k));
        let index = map.binary_search_by_key(&key, |(k, _)| k.as_ref()).ok()?;
        let (_, value) = &map[index];
        Some(value)
    }

    let AllRefsToExport { bookmarks, tags } = diff_refs_to_export(
        mut_repo.view(),
        mut_repo.store().root_commit_id(),
        &git_ref_filter,
    );

    let check_and_detach_head = |git_repo: &gix::Repository| -> Result<(), GitExportError> {
        let Ok(head_ref) = git_repo.find_reference("HEAD") else {
            return Ok(());
        };
        let old_target = head_ref.inner.target;
        if let Some(target_name) = old_target.try_name()
            && let Ok(target_name_str) = str::from_utf8(target_name.as_bstr())
            && let Some((kind, symbol)) = parse_git_ref(target_name_str.as_ref())
        {
            // Unborn ref is mapped to None (= absent)
            let maybe_target_ref = git_repo
                .try_find_reference(target_name)
                .map_err(GitExportError::from_git)?;
            let current_oid = maybe_target_ref
                .map(|target_ref| target_ref.into_fully_peeled_id())
                .transpose()
                .map_err(GitExportError::from_git)?
                .map(|id| id.detach());
            let refs = match kind {
                GitRefKind::Bookmark => &bookmarks,
                GitRefKind::Tag => &tags,
            };
            let new_oid = if let Some((_old_oid, new_oid)) = get(&refs.to_update, symbol) {
                Some(new_oid)
            } else if get(&refs.to_delete, symbol).is_some() {
                None
            } else {
                current_oid.as_ref()
            };
            if new_oid != current_oid.as_ref() {
                update_git_head(
                    git_repo,
                    gix::refs::transaction::PreviousValue::MustExistAndMatch(old_target),
                    current_oid,
                )
                .map_err(GitExportError::from_git)?;
            }
        }
        Ok(())
    };

    let git_repo = get_git_repo(mut_repo.store())?;

    check_and_detach_head(&git_repo)?;
    for worktree in git_repo.worktrees().map_err(GitExportError::from_git)? {
        if let Ok(worktree_repo) = worktree.into_repo_with_possibly_inaccessible_worktree() {
            check_and_detach_head(&worktree_repo)?;
        }
    }

    let failed_bookmarks = export_refs_to_git(mut_repo, &git_repo, GitRefKind::Bookmark, bookmarks);
    let failed_tags = export_refs_to_git(mut_repo, &git_repo, GitRefKind::Tag, tags);

    copy_exportable_local_bookmarks_to_remote_view(
        mut_repo,
        REMOTE_NAME_FOR_LOCAL_GIT_REPO,
        |name| {
            let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
            git_ref_filter(GitRefKind::Bookmark, symbol) && get(&failed_bookmarks, symbol).is_none()
        },
    );
    copy_exportable_local_tags_to_remote_view(mut_repo, REMOTE_NAME_FOR_LOCAL_GIT_REPO, |name| {
        let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
        git_ref_filter(GitRefKind::Tag, symbol) && get(&failed_tags, symbol).is_none()
    });

    Ok(GitExportStats {
        failed_bookmarks,
        failed_tags,
    })
}

fn export_refs_to_git(
    mut_repo: &mut MutableRepo,
    git_repo: &gix::Repository,
    kind: GitRefKind,
    refs: RefsToExport,
) -> Vec<(RemoteRefSymbolBuf, FailedRefExportReason)> {
    let mut failed = refs.failed;

    let to_delete = delete_git_refs(git_repo, kind, refs.to_delete);
    failed.extend(to_delete.failed);

    {
        let edits = to_delete
            .to_apply
            .iter()
            .filter_map(|(.., edit)| edit.clone());
        if git_repo.edit_references(edits).is_ok() {
            for (_, git_ref_name, _) in &to_delete.to_apply {
                mut_repo.set_git_ref_target(git_ref_name, RefTarget::absent());
            }
        } else {
            // Fall back to deleting one by one
            for (symbol, git_ref_name, edit) in &to_delete.to_apply {
                if let Some(edit) = edit
                    && let Err(reason) = git_repo.edit_reference(edit.clone())
                {
                    failed.push((
                        symbol.clone(),
                        FailedRefExportReason::FailedToDelete(reason),
                    ));
                } else {
                    mut_repo.set_git_ref_target(git_ref_name, RefTarget::absent());
                }
            }
        }
    }

    // TODO: batch updates similarly to deletes
    for (symbol, (old_commit_oid, new_commit_oid)) in refs.to_update {
        let Some(git_ref_name) = to_git_ref_name(kind, symbol.as_ref()) else {
            failed.push((symbol, FailedRefExportReason::InvalidGitName));
            continue;
        };
        let new_ref_oid = match kind {
            GitRefKind::Bookmark => None,
            // Copy existing tag ref, which may point to annotated tag object.
            GitRefKind::Tag => {
                let remote_matcher = StringMatcher::all();
                find_git_tag_oid_to_copy(
                    mut_repo.view(),
                    git_repo,
                    &symbol.name,
                    &remote_matcher,
                    &new_commit_oid,
                )
            }
        };
        if let Err(reason) = update_git_ref(
            git_repo,
            &git_ref_name,
            old_commit_oid,
            new_commit_oid,
            new_ref_oid,
        ) {
            failed.push((symbol, reason));
        } else {
            let new_target = RefTarget::normal(CommitId::from_bytes(new_commit_oid.as_bytes()));
            mut_repo.set_git_ref_target(&git_ref_name, new_target);
        }
    }

    // Stabilize output, allow binary search.
    failed.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));
    failed
}

fn copy_exportable_local_bookmarks_to_remote_view(
    mut_repo: &mut MutableRepo,
    remote: &RemoteName,
    name_filter: impl Fn(&RefName) -> bool,
) {
    let new_local_bookmarks = mut_repo
        .view()
        .local_remote_bookmarks(remote)
        .filter_map(|(name, targets)| {
            // TODO: filter out untracked bookmarks (if we add support for untracked @git
            // bookmarks)
            let old_target = &targets.remote_ref.target;
            let new_target = targets.local_target;
            (new_target.is_resolved() && old_target != new_target).then_some((name, new_target))
        })
        .filter(|&(name, _)| name_filter(name))
        .map(|(name, new_target)| (name.to_owned(), new_target.clone()))
        .collect_vec();
    for (name, new_target) in new_local_bookmarks {
        let new_remote_ref = RemoteRef {
            target: new_target,
            state: RemoteRefState::Tracked,
        };
        mut_repo.set_remote_bookmark(name.to_remote_symbol(remote), new_remote_ref);
    }
}

fn copy_exportable_local_tags_to_remote_view(
    mut_repo: &mut MutableRepo,
    remote: &RemoteName,
    name_filter: impl Fn(&RefName) -> bool,
) {
    let new_local_tags = mut_repo
        .view()
        .local_remote_tags(remote)
        .filter_map(|(name, targets)| {
            // TODO: filter out untracked tags (if we add support for untracked @git tags)
            let old_target = &targets.remote_ref.target;
            let new_target = targets.local_target;
            (new_target.is_resolved() && old_target != new_target).then_some((name, new_target))
        })
        .filter(|&(name, _)| name_filter(name))
        .map(|(name, new_target)| (name.to_owned(), new_target.clone()))
        .collect_vec();
    for (name, new_target) in new_local_tags {
        let new_remote_ref = RemoteRef {
            target: new_target,
            state: RemoteRefState::Tracked,
        };
        mut_repo.set_remote_tag(name.to_remote_symbol(remote), new_remote_ref);
    }
}

/// Calculates diff of bookmarks and tags to be exported.
fn diff_refs_to_export(
    view: &View,
    root_commit_id: &CommitId,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> AllRefsToExport {
    // Local targets will be copied to the "git" remote if successfully exported. So
    // the local refs are considered to be the new "git" remote refs.
    let mut all_bookmark_targets: HashMap<RemoteRefSymbol, (&RefTarget, &RefTarget)> =
        itertools::chain(
            view.local_bookmarks().map(|(name, target)| {
                let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
                (symbol, target)
            }),
            view.all_remote_bookmarks()
                .filter(|&(symbol, _)| symbol.remote != REMOTE_NAME_FOR_LOCAL_GIT_REPO)
                .map(|(symbol, remote_ref)| (symbol, &remote_ref.target)),
        )
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Bookmark, symbol))
        .map(|(symbol, new_target)| (symbol, (&ABSENT_REF_TARGET, new_target)))
        .collect();
    // Remote tags aren't included because Git has no such concept.
    let mut all_tag_targets: HashMap<RemoteRefSymbol, (&RefTarget, &RefTarget)> = view
        .local_tags()
        .map(|(name, target)| {
            let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
            (symbol, target)
        })
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Tag, symbol))
        .map(|(symbol, new_target)| (symbol, (&ABSENT_REF_TARGET, new_target)))
        .collect();
    let known_git_refs = view
        .git_refs()
        .iter()
        .map(|(full_name, target)| {
            let (kind, symbol) =
                parse_git_ref(full_name).expect("stored git ref should be parsable");
            ((kind, symbol), target)
        })
        // There are two situations where remote refs get out of sync:
        // 1. `jj bookmark forget --include-remotes`
        // 2. `jj op revert`/`restore` in colocated repo
        .filter(|&((kind, symbol), _)| git_ref_filter(kind, symbol));
    for ((kind, symbol), target) in known_git_refs {
        let ref_targets = match kind {
            GitRefKind::Bookmark => &mut all_bookmark_targets,
            GitRefKind::Tag => &mut all_tag_targets,
        };
        ref_targets
            .entry(symbol)
            .and_modify(|(old_target, _)| *old_target = target)
            .or_insert((target, &ABSENT_REF_TARGET));
    }

    let root_commit_target = RefTarget::normal(root_commit_id.clone());
    let bookmarks = collect_changed_refs_to_export(&all_bookmark_targets, &root_commit_target);
    let tags = collect_changed_refs_to_export(&all_tag_targets, &root_commit_target);
    AllRefsToExport { bookmarks, tags }
}

fn collect_changed_refs_to_export(
    old_new_ref_targets: &HashMap<RemoteRefSymbol, (&RefTarget, &RefTarget)>,
    root_commit_target: &RefTarget,
) -> RefsToExport {
    let mut to_update = Vec::new();
    let mut to_delete = Vec::new();
    let mut failed = Vec::new();
    for (&symbol, &(old_target, new_target)) in old_new_ref_targets {
        if new_target == old_target {
            continue;
        }
        if new_target == root_commit_target {
            // Git doesn't have a root commit
            failed.push((symbol.to_owned(), FailedRefExportReason::OnRootCommit));
            continue;
        }
        let old_oid = if let Some(id) = old_target.as_normal() {
            Some(owned_oid_from_commit_id(id))
        } else if !old_target.is_resolved() {
            // The old git ref should only be a conflict if there were concurrent import
            // operations while the value changed. Don't overwrite these values.
            failed.push((symbol.to_owned(), FailedRefExportReason::ConflictedOldState));
            continue;
        } else {
            assert!(old_target.is_absent());
            None
        };
        if let Some(id) = new_target.as_normal() {
            let new_oid = owned_oid_from_commit_id(id);
            to_update.push((symbol.to_owned(), (old_oid, new_oid)));
        } else if !new_target.is_resolved() {
            // Skip conflicts and leave the old value in git_refs
            continue;
        } else {
            assert!(new_target.is_absent());
            to_delete.push((symbol.to_owned(), old_oid.unwrap()));
        }
    }

    // Stabilize export order and output, allow binary search.
    to_update.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    to_delete.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    failed.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    RefsToExport {
        to_update,
        to_delete,
        failed,
    }
}

/// Looks up tracked remote tag refs and returns the ref target object ID if
/// peeled to the given commit ID.
fn find_git_tag_oid_to_copy(
    view: &View,
    git_repo: &gix::Repository,
    name: &RefName,
    remote_matcher: &StringMatcher,
    commit_oid: &gix::oid,
) -> Option<gix::ObjectId> {
    // Filter candidates by tag name and known commit id first
    view.remote_tags_matching(&StringMatcher::exact(name), remote_matcher)
        .filter(|(_, remote_ref)| {
            let maybe_id = remote_ref.tracked_target().as_normal();
            maybe_id.is_some_and(|id| id.as_bytes() == commit_oid.as_bytes())
        })
        // Query existing Git ref and tag object
        .filter_map(|(symbol, _)| {
            let git_ref_name = to_git_or_remote_tag_ref_name(symbol);
            git_repo.find_reference(git_ref_name.as_str()).ok()
        })
        // This usually holds because remote tags are managed by jj, but jj's
        // view may be updated independently by undo/redo commands.
        .filter(|git_ref| {
            resolve_git_ref_to_commit_id(git_ref, Some(commit_oid)).as_deref() == Some(commit_oid)
        })
        .find_map(|git_ref| git_ref.inner.target.try_into_id().ok())
}

struct EditsForDelete {
    failed: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
    to_apply: Vec<(RemoteRefSymbolBuf, GitRefNameBuf, Option<RefEdit>)>,
}

/// Gather [`RefEdit`]s to delete.
fn delete_git_refs(
    git_repo: &gix::Repository,
    kind: GitRefKind,
    refs: Vec<(RemoteRefSymbolBuf, gix::ObjectId)>,
) -> EditsForDelete {
    let mut failed = vec![];

    let mut to_apply = vec![];

    for (symbol, old_oid) in refs {
        let Some(git_ref_name) = to_git_ref_name(kind, symbol.as_ref()) else {
            failed.push((symbol, FailedRefExportReason::InvalidGitName));
            continue;
        };
        let git_ref = match git_repo.try_find_reference(git_ref_name.as_str()) {
            Err(err) => {
                failed.push((symbol, FailedRefExportReason::FailedToDelete(err)));
                continue;
            }
            Ok(None) => {
                // The ref is already deleted in git
                to_apply.push((symbol, git_ref_name, None));
                continue;
            }
            Ok(Some(git_ref)) => git_ref,
        };
        if resolve_git_ref_to_commit_id(&git_ref, Some(&old_oid)).as_deref() == Some(&old_oid) {
            // The ref has not been updated by git, so go ahead and delete it
            let edit = remove_ref(git_ref.clone());
            to_apply.push((symbol, git_ref_name, Some(edit)));
        } else {
            // The ref was updated by git
            failed.push((symbol, FailedRefExportReason::DeletedInJjModifiedInGit));
        }
    }

    EditsForDelete { failed, to_apply }
}

/// Creates new ref pointing to `new_ref_oid` or (peeled) `new_commit_oid`.
fn create_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &GitRefName,
    new_commit_oid: gix::ObjectId,
    new_ref_oid: Option<gix::ObjectId>,
) -> Result<(), FailedRefExportReason> {
    let new_oid = new_ref_oid.unwrap_or(new_commit_oid);
    let constraint = gix::refs::transaction::PreviousValue::MustNotExist;
    let Err(set_err) =
        git_repo.reference(git_ref_name.as_str(), new_oid, constraint, "export from jj")
    else {
        // The ref was added in jj but still doesn't exist in git
        return Ok(());
    };
    let Some(git_ref) = git_repo
        .try_find_reference(git_ref_name.as_str())
        .map_err(FailedRefExportReason::FailedToSet)?
    else {
        return Err(FailedRefExportReason::FailedToSet(set_err));
    };
    // The ref was added in jj and in git. We're good if and only if git
    // pointed it to our desired target.
    if resolve_git_ref_to_commit_id(&git_ref, None) == Some(new_commit_oid) {
        Ok(())
    } else {
        Err(FailedRefExportReason::AddedInJjAddedInGit)
    }
}

/// Updates existing ref to point to `new_ref_oid` or (peeled) `new_commit_oid`.
fn move_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &GitRefName,
    old_commit_oid: gix::ObjectId,
    new_commit_oid: gix::ObjectId,
    new_ref_oid: Option<gix::ObjectId>,
) -> Result<(), FailedRefExportReason> {
    let new_oid = new_ref_oid.unwrap_or(new_commit_oid);
    let constraint =
        gix::refs::transaction::PreviousValue::MustExistAndMatch(old_commit_oid.into());
    let Err(set_err) =
        git_repo.reference(git_ref_name.as_str(), new_oid, constraint, "export from jj")
    else {
        // Successfully updated from old_oid to new_oid (unchanged in git)
        return Ok(());
    };
    // The reference was probably updated in git
    let Some(git_ref) = git_repo
        .try_find_reference(git_ref_name.as_str())
        .map_err(FailedRefExportReason::FailedToSet)?
    else {
        // The reference was deleted in git and moved in jj
        return Err(FailedRefExportReason::ModifiedInJjDeletedInGit);
    };
    // We still consider this a success if it was updated to our desired target
    let git_commit_oid = resolve_git_ref_to_commit_id(&git_ref, Some(&old_commit_oid));
    if git_commit_oid == Some(new_commit_oid) {
        Ok(())
    } else if git_commit_oid == Some(old_commit_oid) {
        // The reference would point to annotated tag, try again
        let constraint =
            gix::refs::transaction::PreviousValue::MustExistAndMatch(git_ref.inner.target);
        git_repo
            .reference(git_ref_name.as_str(), new_oid, constraint, "export from jj")
            .map_err(FailedRefExportReason::FailedToSet)?;
        Ok(())
    } else {
        Err(FailedRefExportReason::FailedToSet(set_err))
    }
}

fn update_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &GitRefName,
    old_commit_oid: Option<gix::ObjectId>,
    new_commit_oid: gix::ObjectId,
    new_ref_oid: Option<gix::ObjectId>,
) -> Result<(), FailedRefExportReason> {
    match old_commit_oid {
        None => create_git_ref(git_repo, git_ref_name, new_commit_oid, new_ref_oid),
        Some(old_oid) => move_git_ref(git_repo, git_ref_name, old_oid, new_commit_oid, new_ref_oid),
    }
}

/// Ensures Git HEAD is detached and pointing to the `new_oid`. If `new_oid`
/// is `None` (meaning absent), dummy placeholder ref will be set.
fn update_git_head(
    git_repo: &gix::Repository,
    expected_ref: gix::refs::transaction::PreviousValue,
    new_oid: Option<gix::ObjectId>,
) -> gix::Result<()> {
    let mut ref_edits = Vec::new();
    let new_target = if let Some(oid) = new_oid {
        gix::refs::Target::Object(oid)
    } else {
        // Can't detach HEAD without a commit. Use placeholder ref to nullify
        // the HEAD. The placeholder ref isn't a normal branch ref. Git CLI
        // appears to deal with that, and can move the placeholder ref. So we
        // need to ensure that the ref doesn't exist.
        ref_edits.push(gix::refs::transaction::RefEdit {
            change: gix::refs::transaction::Change::Delete {
                expected: gix::refs::transaction::PreviousValue::Any,
                log: gix::refs::transaction::RefLog::AndReference,
            },
            name: UNBORN_ROOT_REF_NAME.try_into().unwrap(),
            deref: false,
        });
        gix::refs::Target::Symbolic(UNBORN_ROOT_REF_NAME.try_into().unwrap())
    };
    ref_edits.push(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange {
                message: "export from jj".into(),
                ..Default::default()
            },
            expected: expected_ref,
            new: new_target,
        },
        name: "HEAD".try_into().unwrap(),
        deref: false,
    });
    git_repo.edit_references(ref_edits)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum GitCreateWorktreeError {
    #[error("A .git file or directory already exists in the Git worktree destination")]
    ExistingGitLink,
    #[error("Failed to create Git worktree")]
    Git(#[from] gix::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

/// Creates a Git worktree at `destination` to back a jj workspace.
///
/// Nothing is checked out: the worktree is created with HEAD unborn, so jj
/// remains responsible for the working-copy contents. `destination` may
/// already hold a populated jj workspace, as it does when colocation is
/// enabled after the fact.
///
/// For a new workspace, the subsequent HEAD export (see [`reset_head()`])
/// points HEAD at the parent of the new working-copy commit. Only the `.git`
/// gitlink and worktree bookkeeping under `.git/worktrees/` are created here.
pub fn create_worktree(store: &Store, destination: &Path) -> Result<(), GitCreateWorktreeError> {
    if std::fs::symlink_metadata(destination.join(".git")).is_ok() {
        return Err(GitCreateWorktreeError::ExistingGitLink);
    }
    let mut git_repo = get_git_repo(store)?;
    // Keep the repository and its workspaces portable when moved together.
    git_repo
        .config_snapshot_mut()
        .set_raw_value("worktree.useRelativePaths", "true")?;
    git_repo.register_worktree(
        destination,
        gix::worktree::add::Head::Unborn(UNBORN_ROOT_REF_NAME.try_into().expect("valid ref name")),
        &AtomicBool::default(),
    )?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum GitUnlinkWorktreeError {
    #[error("Failed to unlink Git worktree")]
    Git(#[from] gix::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

/// Disconnects the linked Git worktree at `worktree_path` from the repository.
///
/// Returns `false` if there was no linked Git worktree to disconnect.
///
/// The worktree directory and its contents are left in place: only the `.git`
/// gitlink and the selected worktree's bookkeeping under `.git/worktrees/` are
/// removed. Other worktree registrations are retained even if their checkout
/// directories are missing. This is the inverse of [`create_worktree()`].
///
/// The gitlink is removed before the bookkeeping, so an error means either
/// that nothing was done, or that the worktree is already disconnected and
/// only stale metadata remains. Callers may treat these errors as non-fatal.
pub fn unlink_worktree(
    store: &Store,
    worktree_path: &Path,
) -> Result<bool, GitUnlinkWorktreeError> {
    let git_repo = get_git_repo(store)?;
    let target = match git_repo.prepare_remove_worktree(worktree_path) {
        Ok(target) => target,
        Err(err)
            if matches!(
                err.downcast_any_ref::<gix::worktree::remove::Error>(),
                Some(
                    gix::worktree::remove::Error::NotFound { .. }
                        | gix::worktree::remove::Error::MainWorktree { .. }
                )
            ) =>
        {
            return Ok(false);
        }
        Err(err) => return Err(err.into()),
    };
    target.unlink(gix::worktree::remove::Force::Never)?;
    Ok(true)
}

#[derive(Debug, Error)]
pub enum GitResetHeadError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Git(Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to update Git HEAD ref")]
    UpdateHeadRef(#[source] gix::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitResetHeadError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Git(source.into())
    }
}

/// Sets Git HEAD to the parent of the given working-copy commit and resets
/// the Git index.
pub async fn reset_head(
    mut_repo: &mut MutableRepo,
    workspace_name: &WorkspaceName,
    workspace_root: &Path,
    wc_commit: &Commit,
) -> Result<(), GitResetHeadError> {
    let git_backend = get_git_backend(mut_repo.store())?;
    let git_repo = git_backend
        .open_git_repo_at_workdir(workspace_root)
        .map_err(GitResetHeadError::from_git)?;

    let first_parent_id = &wc_commit.parent_ids()[0];
    let new_head_target = if first_parent_id != mut_repo.store().root_commit_id() {
        RefTarget::normal(first_parent_id.clone())
    } else {
        RefTarget::absent()
    };

    // If the first parent of the working copy has changed, reset the Git HEAD.
    let old_head_target = mut_repo.git_head(workspace_name);
    if *old_head_target != new_head_target {
        let expected_ref = if let Some(id) = old_head_target.as_normal() {
            // We have to check the actual HEAD state because we don't record a
            // symbolic ref as such.
            let actual_head = git_repo.head().map_err(GitResetHeadError::from_git)?;
            if actual_head.is_detached() {
                let id = owned_oid_from_commit_id(id);
                gix::refs::transaction::PreviousValue::MustExistAndMatch(id.into())
            } else {
                // Just overwrite symbolic ref, which is unusual. Alternatively,
                // maybe we can test the target ref by issuing noop edit.
                gix::refs::transaction::PreviousValue::MustExist
            }
        } else {
            // Just overwrite if unborn (or conflict), which is also unusual.
            gix::refs::transaction::PreviousValue::MustExist
        };
        let new_oid = new_head_target.as_normal().map(owned_oid_from_commit_id);
        update_git_head(&git_repo, expected_ref, new_oid)
            .map_err(GitResetHeadError::UpdateHeadRef)?;
        mut_repo.set_git_head_target(workspace_name, new_head_target);
    }

    // If there is an ongoing operation (merge, rebase, etc.), we need to clean it
    // up.
    if git_repo.state().is_some() {
        git_repo
            .clear_operation_state()
            .map_err(GitResetHeadError::from_git)?;
    }

    reset_index(mut_repo, &git_repo, wc_commit).await
}

async fn reset_index(
    repo: &dyn Repo,
    git_repo: &gix::Repository,
    wc_commit: &Commit,
) -> Result<(), GitResetHeadError> {
    let parent_tree = wc_commit.parent_tree(repo.index()).await?;
    // Use the merged parent tree as the Git index, allowing `git diff` to show the
    // same changes as `jj diff`. If the merged parent tree has conflicts, then the
    // Git index will also be conflicted.
    let mut index = if let Some(tree_id) = parent_tree.tree_ids().as_resolved() {
        if tree_id == repo.store().empty_tree_id() {
            // If the tree is empty, gix can fail to load the object (since Git doesn't
            // require the empty tree to actually be present in the object database), so we
            // just use an empty index directly.
            gix::index::File::from_state(
                gix::index::State::new(git_repo.object_hash()),
                git_repo.index_path(),
            )
        } else {
            // If the parent tree is resolved, we can use gix's `index_from_tree` method.
            // This is more efficient than iterating over the tree and adding each entry.
            git_repo
                .index_from_tree(&gix::ObjectId::from_bytes_or_panic(tree_id.as_bytes()))
                .map_err(GitResetHeadError::from_git)?
        }
    } else {
        build_index_from_merged_tree(git_repo, &parent_tree).await?
    };

    let wc_tree = wc_commit.tree();
    update_intent_to_add_impl(git_repo, &mut index, &parent_tree, &wc_tree).await?;

    // Match entries in the new index with entries in the old index, and copy stat
    // information if the entry didn't change.
    if let Some(old_index) = git_repo.try_index().map_err(GitResetHeadError::from_git)? {
        index
            .entries_mut_with_paths()
            .merge_join_by(old_index.entries(), |(entry, path), old_entry| {
                gix::index::Entry::cmp_filepaths(path, old_entry.path(&old_index))
                    .then_with(|| entry.stage().cmp(&old_entry.stage()))
            })
            .filter_map(|merged| merged.both())
            .map(|((entry, _), old_entry)| (entry, old_entry))
            .filter(|(entry, old_entry)| entry.id == old_entry.id && entry.mode == old_entry.mode)
            .for_each(|(entry, old_entry)| entry.stat = old_entry.stat);
    }

    debug_assert!(index.verify_entries().is_ok());

    index
        .write(gix::index::write::Options::default())
        .map_err(GitResetHeadError::from_git)
}

async fn build_index_from_merged_tree(
    git_repo: &gix::Repository,
    merged_tree: &MergedTree,
) -> Result<gix::index::File, GitResetHeadError> {
    let mut index = gix::index::File::from_state(
        gix::index::State::new(git_repo.object_hash()),
        git_repo.index_path(),
    );

    let mut push_index_entry =
        |path: &RepoPath, maybe_entry: &Option<TreeValue>, stage: gix::index::entry::Stage| {
            let Some(entry) = maybe_entry else {
                return;
            };

            let (id, mode) = match entry {
                TreeValue::File {
                    id,
                    executable,
                    copy_id: _,
                } => {
                    if *executable {
                        (id.as_bytes(), gix::index::entry::Mode::FILE_EXECUTABLE)
                    } else {
                        (id.as_bytes(), gix::index::entry::Mode::FILE)
                    }
                }
                TreeValue::Symlink(id) => (id.as_bytes(), gix::index::entry::Mode::SYMLINK),
                TreeValue::Tree(_) => {
                    // This case is only possible if there is a file-directory conflict, since
                    // `MergedTree::entries` handles the recursion otherwise. We only
                    // materialize a file in the working copy for file-directory conflicts, so we
                    // don't add the tree to the index here either.
                    return;
                }
                TreeValue::GitSubmodule(id) => (id.as_bytes(), gix::index::entry::Mode::COMMIT),
            };

            let path = BStr::new(path.as_internal_file_string());

            // It is safe to push the entry because we ensure that we only add each path to
            // a stage once, and we sort the entries after we finish adding them.
            index.dangerously_push_entry(
                gix::index::entry::Stat::default(),
                gix::ObjectId::from_bytes_or_panic(id),
                gix::index::entry::Flags::from_stage(stage),
                mode,
                path,
            );
        };

    let mut has_many_sided_conflict = false;

    let mut entries = merged_tree.entries();
    while let Some((path, entry)) = entries.next().await {
        let entry = entry?;
        if let Some(resolved) = entry.as_resolved() {
            push_index_entry(&path, resolved, gix::index::entry::Stage::Unconflicted);
            continue;
        }

        let conflict = entry.simplify();
        if let [left, base, right] = conflict.as_slice() {
            // 2-sided conflicts can be represented in the Git index
            push_index_entry(&path, left, gix::index::entry::Stage::Ours);
            push_index_entry(&path, base, gix::index::entry::Stage::Base);
            push_index_entry(&path, right, gix::index::entry::Stage::Theirs);
        } else {
            // We can't represent many-sided conflicts in the Git index, so just add the
            // first side as staged. This is preferable to adding the first 2 sides as a
            // conflict, since some tools rely on being able to resolve conflicts using the
            // index, which could lead to an incorrect conflict resolution if the index
            // didn't contain all of the conflict sides. Instead, we add a dummy conflict of
            // a file named ".jj-do-not-resolve-this-conflict" to prevent the user from
            // accidentally committing the conflict markers.
            has_many_sided_conflict = true;
            push_index_entry(
                &path,
                conflict.first(),
                gix::index::entry::Stage::Unconflicted,
            );
        }
    }

    // Required after `dangerously_push_entry` for correctness. We use do a lookup
    // in the index after this, so it must be sorted before we do the lookup.
    index.sort_entries();

    // If the conflict had an unrepresentable conflict and the dummy file path isn't
    // already added in the index, add a dummy file as a conflict.
    if has_many_sided_conflict
        && index
            .entry_index_by_path(INDEX_DUMMY_CONFLICT_FILE.into())
            .is_err()
    {
        let file_blob = git_repo
            .write_blob(
                b"The working copy commit contains conflicts which cannot be resolved using Git.\n",
            )
            .map_err(GitResetHeadError::from_git)?;
        index.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            file_blob.detach(),
            gix::index::entry::Flags::from_stage(gix::index::entry::Stage::Ours),
            gix::index::entry::Mode::FILE,
            INDEX_DUMMY_CONFLICT_FILE.into(),
        );
        // We need to sort again for correctness before writing the index file since we
        // added a new entry.
        index.sort_entries();
    }

    Ok(index)
}

/// Diff `old_tree` to `new_tree` and mark added files as intent-to-add in the
/// Git index. Also removes current intent-to-add entries in the index if they
/// were removed in the diff.
///
/// Should be called when the diff between the working-copy commit and its
/// parent(s) has changed.
pub async fn update_intent_to_add(
    repo: &dyn Repo,
    workspace_root: &Path,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
) -> Result<(), GitResetHeadError> {
    let git_backend = get_git_backend(repo.store())?;
    let git_repo = git_backend
        .open_git_repo_at_workdir(workspace_root)
        .map_err(GitResetHeadError::from_git)?;
    let mut index = git_repo
        .index_or_empty()
        .map_err(GitResetHeadError::from_git)?;
    let mut_index = Arc::make_mut(&mut index);
    update_intent_to_add_impl(&git_repo, mut_index, old_tree, new_tree).await?;
    debug_assert!(mut_index.verify_entries().is_ok());
    mut_index
        .write(gix::index::write::Options::default())
        .map_err(GitResetHeadError::from_git)?;

    Ok(())
}

async fn update_intent_to_add_impl(
    git_repo: &gix::Repository,
    index: &mut gix::index::File,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
) -> Result<(), GitResetHeadError> {
    let mut diff_stream = old_tree.diff(new_tree, &EverythingMatcher);
    let mut added_paths = vec![];
    let mut removed_paths = HashSet::new();
    while let Some(TreeDiffEntry { path, values }) = diff_stream.next().await {
        let values = values?;
        if values.before.is_absent() {
            let executable = match values.after.as_normal() {
                Some(TreeValue::File {
                    id: _,
                    executable,
                    copy_id: _,
                }) => *executable,
                Some(TreeValue::Symlink(_)) => false,
                _ => {
                    continue;
                }
            };
            if index
                .entry_index_by_path(BStr::new(path.as_internal_file_string()))
                .is_err()
            {
                added_paths.push((BString::from(path.into_internal_string()), executable));
            }
        } else if values.after.is_absent() {
            removed_paths.insert(BString::from(path.into_internal_string()));
        }
    }

    if added_paths.is_empty() && removed_paths.is_empty() {
        return Ok(());
    }

    if !added_paths.is_empty() {
        // We need to write the empty blob, otherwise `jj util gc` will report an error.
        let empty_blob = git_repo
            .write_blob(b"")
            .map_err(GitResetHeadError::from_git)?
            .detach();
        for (path, executable) in added_paths {
            // We have checked that the index doesn't have this entry
            index.dangerously_push_entry(
                gix::index::entry::Stat::default(),
                empty_blob,
                gix::index::entry::Flags::INTENT_TO_ADD | gix::index::entry::Flags::EXTENDED,
                if executable {
                    gix::index::entry::Mode::FILE_EXECUTABLE
                } else {
                    gix::index::entry::Mode::FILE
                },
                path.as_ref(),
            );
        }
    }
    if !removed_paths.is_empty() {
        index.remove_entries(|_size, path, entry| {
            entry
                .flags
                .contains(gix::index::entry::Flags::INTENT_TO_ADD)
                && removed_paths.contains(path)
        });
    }

    index.sort_entries();

    Ok(())
}

#[derive(Debug, Error)]
pub enum GitRemoteManagementError {
    #[error("No git remote named '{}'", .0.as_symbol())]
    NoSuchRemote(RemoteNameBuf),
    #[error("Git remote named '{}' already exists", .0.as_symbol())]
    RemoteAlreadyExists(RemoteNameBuf),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error("Git remote named '{}' has nonstandard configuration", .0.as_symbol())]
    NonstandardConfiguration(RemoteNameBuf),
    #[error("Error saving Git configuration")]
    GitConfigSaveError(#[source] std::io::Error),
    #[error("Unexpected Git error when managing remotes")]
    InternalGitError(#[from] gix::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
    #[error(transparent)]
    RefExpansionError(#[from] GitRefExpansionError),
}

fn default_fetch_refspec(remote: &RemoteName) -> String {
    format!(
        "+refs/heads/*:{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/*",
        remote = remote.as_str()
    )
}

fn add_ref(
    name: gix::refs::FullName,
    target: gix::refs::Target,
    message: BString,
) -> gix::refs::transaction::RefEdit {
    gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange {
                mode: gix::refs::transaction::RefLog::AndReference,
                force_create_reflog: false,
                message,
            },
            expected: gix::refs::transaction::PreviousValue::MustNotExist,
            new: target,
        },
        name,
        deref: false,
    }
}

fn remove_ref(reference: gix::Reference) -> gix::refs::transaction::RefEdit {
    gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Delete {
            expected: gix::refs::transaction::PreviousValue::MustExistAndMatch(
                reference.target().into_owned(),
            ),
            log: gix::refs::transaction::RefLog::AndReference,
        },
        name: reference.name().to_owned(),
        deref: false,
    }
}

/// Save an edited [`gix::config::File`] to its original location on disk.
///
/// Note that the resulting configuration changes are *not* persisted to the
/// originating [`gix::Repository`]! The repository must be reloaded with the
/// new configuration if necessary.
pub fn save_git_config(config: &gix::config::File) -> std::io::Result<()> {
    let mut config_file = File::create(
        config
            .meta()
            .path
            .as_ref()
            .expect("Git repository to have a config file"),
    )?;
    config.write_to_filter(&mut config_file, |section| section.meta() == config.meta())
}

fn save_remote(
    config: &mut gix::config::File,
    remote_name: &RemoteName,
    remote: &mut gix::Remote,
) -> Result<(), GitRemoteManagementError> {
    remote.save_as_to(remote_name.as_str(), config)?;
    Ok(())
}

fn git_config_branch_section_ids_by_remote(
    config: &gix::config::File,
    remote_name: &RemoteName,
) -> Result<Vec<gix::config::file::SectionId>, GitRemoteManagementError> {
    config
        .sections_by_name("branch")
        .into_iter()
        .flatten()
        .filter_map(|section| {
            let remote_values = section.values("remote");
            let push_remote_values = section.values("pushRemote");
            if !remote_values
                .iter()
                .chain(push_remote_values.iter())
                .any(|branch_remote_name| branch_remote_name == remote_name.as_str())
            {
                return None;
            }
            // https://github.com/jj-vcs/jj/issues/6984#issuecomment-3073761797
            let is_supported_key = |name: &str| -> bool {
                name.eq_ignore_ascii_case("remote")
                    || name.eq_ignore_ascii_case("merge")
                    || name.eq_ignore_ascii_case("rebase")
            };
            if remote_values.len() > 1
                || push_remote_values.len() > 1
                || !section.value_names().all(|name| is_supported_key(&name))
            {
                return Some(Err(GitRemoteManagementError::NonstandardConfiguration(
                    remote_name.to_owned(),
                )));
            }
            Some(Ok(section.id()))
        })
        .collect()
}

fn rename_remote_in_git_branch_config_sections(
    config: &mut gix::config::File,
    old_remote_name: &RemoteName,
    new_remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    for id in git_config_branch_section_ids_by_remote(config, old_remote_name)? {
        config
            .section_mut_by_id(id)
            .expect("found section to exist")
            .set("remote", new_remote_name.as_str())
            .expect("'remote' to be a valid value name");
    }
    Ok(())
}

fn remove_remote_git_branch_config_sections(
    config: &mut gix::config::File,
    remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    for id in git_config_branch_section_ids_by_remote(config, remote_name)? {
        config
            .remove_section_by_id(id)
            .expect("removed section to exist");
    }
    Ok(())
}

fn remove_remote_git_config_sections(
    config: &mut gix::config::File,
    remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    let section_ids_to_remove: Vec<_> = config
        .sections_by_name("remote")
        .into_iter()
        .flatten()
        .filter(|section| {
            section.header().subsection_name() == Some(BStr::new(remote_name.as_str()))
        })
        .map(|section| {
            if section.value_names().any(|name| {
                !name.eq_ignore_ascii_case("url")
                    && !name.eq_ignore_ascii_case("fetch")
                    && !name.eq_ignore_ascii_case("tagOpt")
            }) {
                return Err(GitRemoteManagementError::NonstandardConfiguration(
                    remote_name.to_owned(),
                ));
            }
            Ok(section.id())
        })
        .try_collect()?;
    for id in section_ids_to_remove {
        config
            .remove_section_by_id(id)
            .expect("removed section to exist");
    }
    Ok(())
}

/// Returns a sorted list of configured remote names.
pub fn get_all_remote_names(
    store: &Store,
) -> Result<Vec<RemoteNameBuf>, UnexpectedGitBackendError> {
    let git_repo = get_git_repo(store)?;
    Ok(iter_remote_names(&git_repo).collect())
}

fn iter_remote_names(git_repo: &gix::Repository) -> impl Iterator<Item = RemoteNameBuf> {
    git_repo
        .remote_names()
        .into_iter()
        // ignore non-UTF-8 remote names which we don't support
        .filter_map(|name| String::from_utf8(name.into()).ok())
        .map(RemoteNameBuf::from)
        // exclude empty [remote "<name>"] section
        .filter(|name| try_find_active_remote_inner(git_repo, name).is_some())
}

/// Finds a configured remote with the given `name`. Returns `None` if it
/// doesn't exist or has no fetch or push URLs.
pub fn try_find_active_remote<'a>(
    git_repo: &'a gix::Repository,
    name: &RemoteName,
) -> Result<Option<gix::Remote<'a>>, GitRemoteManagementError> {
    try_find_active_remote_inner(git_repo, name)
        .transpose()
        .map_err(GitRemoteManagementError::InternalGitError)
}

fn try_find_active_remote_inner<'a>(
    git_repo: &'a gix::Repository,
    name: &RemoteName,
) -> Option<gix::Result<gix::Remote<'a>>> {
    // Since gix v0.86.0, try_find_remote() no longer filters out remotes
    // without configured URLs.
    git_repo
        .try_find_remote(name.as_str())
        .filter(|result| match result {
            Ok(remote) => {
                remote.url(gix::remote::Direction::Fetch).is_some()
                    || remote.url(gix::remote::Direction::Push).is_some()
            }
            Err(_) => true,
        })
}

pub fn add_remote(
    mut_repo: &mut MutableRepo,
    remote_name: &RemoteName,
    url: &str,
    push_url: Option<&str>,
) -> Result<(), GitRemoteManagementError> {
    let git_repo = get_git_repo(mut_repo.store())?;

    validate_remote_name(remote_name)?;

    if try_find_active_remote_inner(&git_repo, remote_name).is_some() {
        return Err(GitRemoteManagementError::RemoteAlreadyExists(
            remote_name.to_owned(),
        ));
    }

    let mut remote = git_repo
        .remote_at(url)?
        .with_refspecs(
            [default_fetch_refspec(remote_name).as_bytes()],
            gix::remote::Direction::Fetch,
        )
        .expect("default refspec to be valid");

    if let Some(push_url) = push_url {
        remote = remote.with_push_url(push_url)?;
    }

    let mut config = git_repo.config_snapshot().clone();
    save_remote(&mut config, remote_name, &mut remote)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    mut_repo.ensure_remote(remote_name);

    Ok(())
}

pub fn remove_remote(
    mut_repo: &mut MutableRepo,
    remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    let mut git_repo = get_git_repo(mut_repo.store())?;

    if try_find_active_remote_inner(&git_repo, remote_name).is_none() {
        return Err(GitRemoteManagementError::NoSuchRemote(
            remote_name.to_owned(),
        ));
    }

    let mut config = git_repo.config_snapshot().clone();
    remove_remote_git_branch_config_sections(&mut config, remote_name)?;
    remove_remote_git_config_sections(&mut config, remote_name)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    remove_remote_git_refs(&mut git_repo, remote_name)?;

    if remote_name != REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        remove_remote_refs(mut_repo, remote_name);
    }

    Ok(())
}

fn remove_remote_git_refs(git_repo: &mut gix::Repository, remote: &RemoteName) -> gix::Result<()> {
    let bookmark_prefix = format!(
        "{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/",
        remote = remote.as_str()
    );
    let fetch_bookmark_prefix = format!(
        "{FETCH_BOOKMARK_REF_NAMESPACE}{remote}/",
        remote = remote.as_str()
    );
    let tag_prefix = format!(
        "{REMOTE_TAG_REF_NAMESPACE}{remote}/",
        remote = remote.as_str()
    );
    let refs = git_repo.references()?;
    let edits: Vec<_> = itertools::chain!(
        refs.prefixed(bookmark_prefix.as_str())?.map_ok(remove_ref),
        refs.prefixed(fetch_bookmark_prefix.as_str())?
            .map_ok(remove_ref),
        refs.prefixed(tag_prefix.as_str())?.map_ok(remove_ref),
    )
    .try_collect()?;
    git_repo.edit_references(edits)?;
    Ok(())
}

fn remove_remote_refs(mut_repo: &mut MutableRepo, remote: &RemoteName) {
    mut_repo.remove_remote(remote);
    let prefix = format!(
        "{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/",
        remote = remote.as_str()
    );
    let git_refs_to_delete = mut_repo
        .view()
        .git_refs()
        .keys()
        .filter(|&r| r.as_str().starts_with(&prefix))
        .cloned()
        .collect_vec();
    for git_ref in git_refs_to_delete {
        mut_repo.set_git_ref_target(&git_ref, RefTarget::absent());
    }
}

pub fn rename_remote(
    mut_repo: &mut MutableRepo,
    old_remote_name: &RemoteName,
    new_remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    let mut git_repo = get_git_repo(mut_repo.store())?;

    validate_remote_name(new_remote_name)?;

    let mut remote = try_find_active_remote(&git_repo, old_remote_name)?
        .ok_or_else(|| GitRemoteManagementError::NoSuchRemote(old_remote_name.to_owned()))?;

    if try_find_active_remote_inner(&git_repo, new_remote_name).is_some() {
        return Err(GitRemoteManagementError::RemoteAlreadyExists(
            new_remote_name.to_owned(),
        ));
    }

    match (
        remote.refspecs(gix::remote::Direction::Fetch),
        remote.refspecs(gix::remote::Direction::Push),
    ) {
        ([refspec], [])
            if refspec.to_ref().to_bstring()
                == default_fetch_refspec(old_remote_name).as_bytes() => {}
        _ => {
            return Err(GitRemoteManagementError::NonstandardConfiguration(
                old_remote_name.to_owned(),
            ));
        }
    }

    remote
        .replace_refspecs(
            [default_fetch_refspec(new_remote_name).as_bytes()],
            gix::remote::Direction::Fetch,
        )
        .expect("default refspec to be valid");

    let mut config = git_repo.config_snapshot().clone();
    save_remote(&mut config, new_remote_name, &mut remote)?;
    rename_remote_in_git_branch_config_sections(&mut config, old_remote_name, new_remote_name)?;
    remove_remote_git_config_sections(&mut config, old_remote_name)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    rename_remote_git_refs(&mut git_repo, old_remote_name, new_remote_name)?;

    if old_remote_name != REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        rename_remote_refs(mut_repo, old_remote_name, new_remote_name);
    }

    Ok(())
}

fn rename_remote_git_refs(
    git_repo: &mut gix::Repository,
    old_remote_name: &RemoteName,
    new_remote_name: &RemoteName,
) -> gix::Result<()> {
    let to_prefixes = |namespace: &str| {
        (
            format!("{namespace}{remote}/", remote = old_remote_name.as_str()),
            format!("{namespace}{remote}/", remote = new_remote_name.as_str()),
        )
    };
    let to_rename_edits = {
        let ref_log_message = BString::from(format!(
            "renamed remote {old_remote_name} to {new_remote_name}",
            old_remote_name = old_remote_name.as_symbol(),
            new_remote_name = new_remote_name.as_symbol(),
        ));
        move |old_prefix: &str, new_prefix: &str, old_ref: gix::Reference| {
            let new_name = BString::new(
                [
                    new_prefix.as_bytes(),
                    &old_ref.name().as_bstr()[old_prefix.len()..],
                ]
                .concat(),
            );
            [
                add_ref(
                    new_name.try_into().expect("new ref name to be valid"),
                    old_ref.target().into_owned(),
                    ref_log_message.clone(),
                ),
                remove_ref(old_ref),
            ]
        }
    };

    let (old_bookmark_prefix, new_bookmark_prefix) = to_prefixes(REMOTE_BOOKMARK_REF_NAMESPACE);
    let (old_fetch_bookmark_prefix, new_fetch_bookmark_prefix) =
        to_prefixes(FETCH_BOOKMARK_REF_NAMESPACE);
    let (old_tag_prefix, new_tag_prefix) = to_prefixes(REMOTE_TAG_REF_NAMESPACE);
    let refs = git_repo.references()?;
    let edits: Vec<_> = itertools::chain!(
        refs.prefixed(old_bookmark_prefix.as_str())?
            .map_ok(|old_ref| to_rename_edits(&old_bookmark_prefix, &new_bookmark_prefix, old_ref)),
        refs.prefixed(old_fetch_bookmark_prefix.as_str())?
            .map_ok(|old_ref| to_rename_edits(
                &old_fetch_bookmark_prefix,
                &new_fetch_bookmark_prefix,
                old_ref
            )),
        refs.prefixed(old_tag_prefix.as_str())?
            .map_ok(|old_ref| to_rename_edits(&old_tag_prefix, &new_tag_prefix, old_ref)),
    )
    .flatten_ok()
    .try_collect()?;
    git_repo.edit_references(edits)?;
    Ok(())
}

/// Sets the new URLs on the remote. If a URL of given kind is not provided, it
/// is not changed. I.e. it is not possible to remove a fetch/push URL from a
/// remote using this method.
pub fn set_remote_urls(
    store: &Store,
    remote_name: &RemoteName,
    new_url: Option<&str>,
    new_push_url: Option<&str>,
) -> Result<(), GitRemoteManagementError> {
    // quick sanity check
    if new_url.is_none() && new_push_url.is_none() {
        return Ok(());
    }

    let git_repo = get_git_repo(store)?;

    validate_remote_name(remote_name)?;

    let Some(result) = git_repo.try_find_remote_without_url_rewrite(remote_name.as_str()) else {
        return Err(GitRemoteManagementError::NoSuchRemote(
            remote_name.to_owned(),
        ));
    };
    let mut remote = result?;

    if let Some(url) = new_url {
        remote = remote.with_url(url)?;
    }

    if let Some(url) = new_push_url {
        remote = remote.with_push_url(url)?;
    }

    let mut config = git_repo.config_snapshot().clone();
    save_remote(&mut config, remote_name, &mut remote)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    Ok(())
}

fn rename_remote_refs(
    mut_repo: &mut MutableRepo,
    old_remote_name: &RemoteName,
    new_remote_name: &RemoteName,
) {
    mut_repo.rename_remote(old_remote_name.as_ref(), new_remote_name.as_ref());
    let prefix = format!(
        "{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/",
        remote = old_remote_name.as_str()
    );
    let git_refs = mut_repo
        .view()
        .git_refs()
        .iter()
        .filter_map(|(old, target)| {
            old.as_str().strip_prefix(&prefix).map(|p| {
                let new: GitRefNameBuf = format!(
                    "{REMOTE_BOOKMARK_REF_NAMESPACE}{remote}/{p}",
                    remote = new_remote_name.as_str()
                )
                .into();
                (old.clone(), new, target.clone())
            })
        })
        .collect_vec();
    for (old, new, target) in git_refs {
        mut_repo.set_git_ref_target(&old, RefTarget::absent());
        mut_repo.set_git_ref_target(&new, target);
    }
}

const INVALID_REFSPEC_CHARS: [char; 5] = [':', '^', '?', '[', ']'];

#[derive(Error, Debug)]
pub enum GitFetchError {
    #[error("No git remote named '{}'", .0.as_symbol())]
    NoSuchRemote(RemoteNameBuf),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error(transparent)]
    RefExpansion(#[from] GitRefExpansionError),
    #[error("Failed to update refs: {}", .0.iter().map(|n| n.as_symbol()).join(", "))]
    RejectedUpdates(Vec<GitRefNameBuf>),
    #[error("Git fetch failed")]
    Git(#[from] gix::Error),
}

#[derive(Error, Debug)]
pub enum GitDefaultRefspecError {
    #[error("No git remote named '{}'", .0.as_symbol())]
    NoSuchRemote(RemoteNameBuf),
    #[error("Invalid configuration for remote `{}`", .0.as_symbol())]
    InvalidRemoteConfiguration(RemoteNameBuf, #[source] gix::Error),
}

struct FetchedRefs {
    remote: RemoteNameBuf,
    bookmark_matcher: StringMatcher,
    tag_matcher: StringMatcher,
}

/// Name patterns that will be transformed to Git refspecs.
#[derive(Clone, Debug)]
pub struct GitFetchRefExpression {
    /// Matches bookmark or branch names.
    pub bookmark: StringExpression,
    /// Matches tag names.
    ///
    /// Tags matching this expression will be fetched as "remote tags" and
    /// merged with tracking local tags. This is different from `git fetch`,
    /// which would directly update local tags.
    pub tag: StringExpression,
}

#[derive(Error, Debug)]
pub enum GitRefExpansionError {
    #[error(transparent)]
    Expression(#[from] GitRefExpressionError),
    #[error(
        "Invalid branch pattern provided. When fetching, branch names and globs may not contain the characters `{chars}`",
        chars = INVALID_REFSPEC_CHARS.iter().join("`, `")
    )]
    InvalidBranchPattern(StringPattern),
}

/// Expand a list of branch string patterns to refspecs to fetch
fn expand_fetch_refspecs(
    remote: &RemoteName,
    expr: &GitFetchRefExpression,
) -> Result<(Vec<RefSpec>, Vec<NegativeRefSpec>), GitRefExpansionError> {
    let (positive_bookmarks, negative_bookmarks) =
        split_into_positive_negative_patterns(&expr.bookmark)?;
    let (positive_tags, negative_tags) = split_into_positive_negative_patterns(&expr.tag)?;

    let refspecs = itertools::chain(
        positive_bookmarks
            .iter()
            .map(|&pattern| pattern_to_refspec_glob(pattern))
            .map_ok(|glob| {
                RefSpec::forced(
                    format!("refs/heads/{glob}"),
                    format!(
                        "{FETCH_BOOKMARK_REF_NAMESPACE}{remote}/{glob}",
                        remote = remote.as_str(),
                    ),
                )
            }),
        positive_tags
            .iter()
            .map(|&pattern| pattern_to_refspec_glob(pattern))
            .map_ok(|glob| {
                RefSpec::forced(
                    format!("refs/tags/{glob}"),
                    format!(
                        "{REMOTE_TAG_REF_NAMESPACE}{remote}/{glob}",
                        remote = remote.as_str()
                    ),
                )
            }),
    )
    .try_collect()?;

    let negative_refspecs = itertools::chain(
        negative_bookmarks
            .iter()
            .map(|&pattern| pattern_to_refspec_glob(pattern))
            .map_ok(|glob| NegativeRefSpec::new(format!("refs/heads/{glob}"))),
        negative_tags
            .iter()
            .map(|&pattern| pattern_to_refspec_glob(pattern))
            .map_ok(|glob| NegativeRefSpec::new(format!("refs/tags/{glob}"))),
    )
    .try_collect()?;

    Ok((refspecs, negative_refspecs))
}

fn pattern_to_refspec_glob(pattern: &StringPattern) -> Result<Cow<'_, str>, GitRefExpansionError> {
    pattern
        .to_glob()
        // This triggered by non-glob `*`s in addition to INVALID_REFSPEC_CHARS
        // because `to_glob()` escapes such `*`s as `[*]`.
        .filter(|glob| !glob.contains(INVALID_REFSPEC_CHARS))
        .ok_or_else(|| GitRefExpansionError::InvalidBranchPattern(pattern.clone()))
}

#[derive(Debug, Error)]
pub enum GitRefExpressionError {
    #[error("Cannot use `~` in sub expression")]
    NestedNotIn,
    #[error("Cannot use `&` in sub expression")]
    NestedIntersection,
    #[error("Cannot use `&` for positive expressions")]
    PositiveIntersection,
}

/// Splits string matcher expression into Git-compatible `(positive | ...) &
/// ~(negative | ...)` form.
fn split_into_positive_negative_patterns(
    expr: &StringExpression,
) -> Result<(Vec<&StringPattern>, Vec<&StringPattern>), GitRefExpressionError> {
    static ALL: StringPattern = StringPattern::all();

    // Outer expression is considered an intersection of
    // - zero or one union of positive expressions
    // - zero or more unions of negative expressions
    // e.g.
    // - `a`                (1+)
    // - `~a&~b`            (1-, 1-)
    // - `(a|b)&~(c|d)&~e`  (2+, 2-, 1-)
    //
    // No negation nor intersection is allowed under union or not-in nodes.
    // - `a|~b`             (incompatible with Git refspecs)
    // - `~(~a&~b)`         (equivalent to `a|b`, but unsupported)
    // - `(a&~b)&(~c&~d)`   (equivalent to `a&~b&~c&~d`, but unsupported)

    fn visit_positive<'a>(
        expr: &'a StringExpression,
        positives: &mut Vec<&'a StringPattern>,
        negatives: &mut Vec<&'a StringPattern>,
    ) -> Result<(), GitRefExpressionError> {
        match expr {
            StringExpression::Pattern(pattern) => {
                positives.push(pattern);
                Ok(())
            }
            StringExpression::NotIn(complement) => {
                positives.push(&ALL);
                visit_negative(complement, negatives)
            }
            StringExpression::Union(expr1, expr2) => visit_union(expr1, expr2, positives),
            StringExpression::Intersection(expr1, expr2) => {
                match (expr1.as_ref(), expr2.as_ref()) {
                    (other, StringExpression::NotIn(complement))
                    | (StringExpression::NotIn(complement), other) => {
                        visit_positive(other, positives, negatives)?;
                        visit_negative(complement, negatives)
                    }
                    _ => Err(GitRefExpressionError::PositiveIntersection),
                }
            }
        }
    }

    fn visit_negative<'a>(
        expr: &'a StringExpression,
        negatives: &mut Vec<&'a StringPattern>,
    ) -> Result<(), GitRefExpressionError> {
        match expr {
            StringExpression::Pattern(pattern) => {
                negatives.push(pattern);
                Ok(())
            }
            StringExpression::NotIn(_) => Err(GitRefExpressionError::NestedNotIn),
            StringExpression::Union(expr1, expr2) => visit_union(expr1, expr2, negatives),
            StringExpression::Intersection(_, _) => Err(GitRefExpressionError::NestedIntersection),
        }
    }

    fn visit_union<'a>(
        expr1: &'a StringExpression,
        expr2: &'a StringExpression,
        patterns: &mut Vec<&'a StringPattern>,
    ) -> Result<(), GitRefExpressionError> {
        visit_union_sub(expr1, patterns)?;
        visit_union_sub(expr2, patterns)
    }

    fn visit_union_sub<'a>(
        expr: &'a StringExpression,
        patterns: &mut Vec<&'a StringPattern>,
    ) -> Result<(), GitRefExpressionError> {
        match expr {
            StringExpression::Pattern(pattern) => {
                patterns.push(pattern);
                Ok(())
            }
            StringExpression::NotIn(_) => Err(GitRefExpressionError::NestedNotIn),
            StringExpression::Union(expr1, expr2) => visit_union(expr1, expr2, patterns),
            StringExpression::Intersection(_, _) => Err(GitRefExpressionError::NestedIntersection),
        }
    }

    let mut positives = Vec::new();
    let mut negatives = Vec::new();
    visit_positive(expr, &mut positives, &mut negatives)?;
    // Don't generate uninteresting patterns for `~*` (= none). `x~*`, `~(x|*)`,
    // etc. aren't special-cased because `x` may be Git-incompatible pattern.
    if positives.iter().all(|pattern| pattern.is_all())
        && !negatives.is_empty()
        && negatives.iter().all(|pattern| pattern.is_all())
    {
        Ok((vec![], vec![]))
    } else {
        Ok((positives, negatives))
    }
}

/// A list of fetch refspecs configured within a remote that were ignored during
/// an expansion. Callers should consider displaying these in the UI as
/// appropriate.
#[derive(Debug)]
#[must_use = "warnings should be surfaced in the UI"]
pub struct IgnoredRefspecs(pub Vec<IgnoredRefspec>);

/// A fetch refspec configured within a remote that was ignored during
/// expansion.
#[derive(Debug)]
pub struct IgnoredRefspec {
    /// The ignored refspec
    pub refspec: BString,
    /// The reason why it was ignored
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FetchRefSpecKind {
    Positive,
    Negative,
}

/// Loads the remote's fetch branch or bookmark patterns from Git config.
pub fn load_default_fetch_bookmarks(
    remote_name: &RemoteName,
    git_repo: &gix::Repository,
) -> Result<(IgnoredRefspecs, StringExpression), GitDefaultRefspecError> {
    let remote = try_find_active_remote_inner(git_repo, remote_name)
        .ok_or_else(|| GitDefaultRefspecError::NoSuchRemote(remote_name.to_owned()))?
        .map_err(|e| {
            GitDefaultRefspecError::InvalidRemoteConfiguration(remote_name.to_owned(), e)
        })?;

    let remote_refspecs = remote.refspecs(gix::remote::Direction::Fetch);
    let mut ignored_refspecs = Vec::with_capacity(remote_refspecs.len());
    let mut positive_bookmarks = Vec::with_capacity(remote_refspecs.len());
    let mut negative_bookmarks = Vec::new();
    for refspec in remote_refspecs {
        let refspec = refspec.to_ref();
        match parse_fetch_refspec(remote_name, refspec) {
            Ok((FetchRefSpecKind::Positive, bookmark)) => {
                positive_bookmarks.push(StringExpression::pattern(bookmark));
            }
            Ok((FetchRefSpecKind::Negative, bookmark)) => {
                negative_bookmarks.push(StringExpression::pattern(bookmark));
            }
            Err(reason) => {
                let refspec = refspec.to_bstring();
                ignored_refspecs.push(IgnoredRefspec { refspec, reason });
            }
        }
    }

    let mut bookmark_expr = StringExpression::union_all(positive_bookmarks);
    // Avoid double negation `~~*` when no negative patterns are provided.
    if !negative_bookmarks.is_empty() {
        bookmark_expr =
            bookmark_expr.intersection(StringExpression::union_all(negative_bookmarks).negated());
    }

    Ok((IgnoredRefspecs(ignored_refspecs), bookmark_expr))
}

fn parse_fetch_refspec(
    remote_name: &RemoteName,
    refspec: gix::refspec::RefSpecRef<'_>,
) -> Result<(FetchRefSpecKind, StringPattern), &'static str> {
    let ensure_utf8 = |s| str::from_utf8(s).map_err(|_| "invalid UTF-8");

    let (src, positive_dst) = match refspec.instruction() {
        Instruction::Push(_) => panic!("push refspec should be filtered out by caller"),
        Instruction::Fetch(fetch) => match fetch {
            gix::refspec::instruction::Fetch::Only { src: _ } => {
                return Err("fetch-only refspecs are not supported");
            }
            gix::refspec::instruction::Fetch::AndUpdate {
                src,
                dst,
                allow_non_fast_forward,
            } => {
                if !allow_non_fast_forward {
                    return Err("non-forced refspecs are not supported");
                }
                (ensure_utf8(src)?, Some(ensure_utf8(dst)?))
            }
            gix::refspec::instruction::Fetch::Exclude { src } => (ensure_utf8(src)?, None),
        },
    };

    let src_branch = src
        .strip_prefix("refs/heads/")
        .ok_or("only refs/heads/ is supported for refspec sources")?;
    let branch = StringPattern::glob(src_branch).map_err(|_| "invalid pattern")?;

    if let Some(dst) = positive_dst {
        let dst_without_prefix = dst
            .strip_prefix(REMOTE_BOOKMARK_REF_NAMESPACE)
            .ok_or("only refs/remotes/ is supported for fetch destinations")?;
        let dst_branch = dst_without_prefix
            .strip_prefix(remote_name.as_str())
            .and_then(|d| d.strip_prefix("/"))
            .ok_or("remote renaming not supported")?;
        if src_branch != dst_branch {
            return Err("renaming is not supported");
        }
        Ok((FetchRefSpecKind::Positive, branch))
    } else {
        Ok((FetchRefSpecKind::Negative, branch))
    }
}

/// Helper struct to fetch repeatedly from Git remotes.
pub struct GitFetch<'a> {
    mut_repo: &'a mut MutableRepo,
    git_repo: Box<gix::Repository>,
    remote_options: GitRemoteOptions,
    import_options: &'a GitImportOptions,
    fetched: Vec<FetchedRefs>,
}

impl<'a> GitFetch<'a> {
    pub fn new(
        mut_repo: &'a mut MutableRepo,
        remote_options: GitRemoteOptions,
        import_options: &'a GitImportOptions,
    ) -> Result<Self, UnexpectedGitBackendError> {
        let git_backend = get_git_backend(mut_repo.store())?;
        let git_repo = Box::new(git_backend.git_repo());
        Ok(GitFetch {
            mut_repo,
            git_repo,
            remote_options,
            import_options,
            fetched: vec![],
        })
    }

    /// Fetch into the backing Git repository, updating its remote-tracking refs.
    ///
    /// Keeps track of the {branch_names, remote_name} pair the refs can be
    /// subsequently imported into the `jj` repo by calling `import_refs()`.
    #[tracing::instrument(skip(self, callback))]
    pub fn fetch(
        &mut self,
        remote_name: &RemoteName,
        expr: &GitFetchRefExpression,
        callback: &mut dyn GitRemoteCallback,
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitFetchError> {
        validate_remote_name(remote_name)?;

        try_find_active_remote_inner(&self.git_repo, remote_name)
            .ok_or_else(|| GitFetchError::NoSuchRemote(remote_name.to_owned()))??;
        let (positive, negative) = expand_fetch_refspecs(remote_name, expr)?;
        if positive.is_empty() {
            return Ok(());
        }
        let refspecs = positive
            .iter()
            .map(RefSpec::to_git_format)
            .chain(negative.iter().map(NegativeRefSpec::to_git_format))
            .collect_vec();
        let repository = self.git_repo.as_ref().clone().into_sync();
        let environment = self.remote_options.environment.clone();
        let outcome = crate::git_remote::run_with_progress(
            callback,
            move |progress, sideband, interrupt| {
                let repository = repository.to_thread_local();
                let mut remote = repository
                    .find_remote(remote_name.as_str())?
                    .with_fetch_tags(gix::remote::fetch::Tags::None)
                    .with_command_environment(environment);
                remote.replace_refspecs(
                    refspecs.iter().map(String::as_str),
                    gix::remote::Direction::Fetch,
                )?;
                remote.fetch_with_remote_progress(
                    gix::remote::fetch::Options {
                        prune: true,
                        missing_refs_allowed: true,
                        shallow: depth
                            .map(gix::remote::fetch::Shallow::DepthAtRemote)
                            .unwrap_or_default(),
                        ..Default::default()
                    },
                    progress,
                    |is_error, bytes| sideband.send(is_error, bytes),
                    interrupt,
                )
            },
        )?;
        let updates = match outcome.status {
            gix::remote::fetch::Status::Change { update_refs, .. }
            | gix::remote::fetch::Status::NoPackReceived { update_refs, .. } => update_refs,
        };
        let rejected = updates
            .updates
            .iter()
            .zip(&outcome.ref_map.mappings)
            .filter(|(update, _)| {
                matches!(
                    update.mode,
                    gix::remote::fetch::refs::update::Mode::RejectedSourceObjectNotFound { .. }
                        | gix::remote::fetch::refs::update::Mode::RejectedTagUpdate
                        | gix::remote::fetch::refs::update::Mode::RejectedNonFastForward
                        | gix::remote::fetch::refs::update::Mode::RejectedToReplaceWithUnborn
                        | gix::remote::fetch::refs::update::Mode::RejectedCurrentlyCheckedOut { .. }
                )
            })
            .filter_map(|(_, mapping)| mapping.local.as_ref())
            .map(|name| GitRefNameBuf::from(String::from_utf8_lossy(name).into_owned()))
            .collect_vec();
        if !rejected.is_empty() {
            return Err(GitFetchError::RejectedUpdates(rejected));
        }

        self.fetched.push(FetchedRefs {
            remote: remote_name.to_owned(),
            bookmark_matcher: expr.bookmark.to_matcher(),
            tag_matcher: expr.tag.to_matcher(),
        });
        Ok(())
    }

    /// Queries remote for the default branch name.
    #[tracing::instrument(skip(self))]
    pub fn get_default_branch(
        &self,
        remote_name: &RemoteName,
    ) -> Result<Option<RefNameBuf>, GitFetchError> {
        let mut remote = try_find_active_remote_inner(&self.git_repo, remote_name)
            .ok_or_else(|| GitFetchError::NoSuchRemote(remote_name.to_owned()))??
            .with_fetch_tags(gix::remote::fetch::Tags::None)
            .with_command_environment(self.remote_options.environment.clone());
        remote.replace_refspecs(
            ["HEAD", "refs/heads/*:refs/heads/*"],
            gix::remote::Direction::Fetch,
        )?;
        let (map, _) = remote.ref_map(gix::progress::Discard, Default::default())?;
        let mut candidates = map.remote_head_candidates();
        let candidate = candidates.next().filter(|_| candidates.next().is_none());
        let default_branch = candidate
            .and_then(|reference| reference.unpack().0.strip_prefix(b"refs/heads/"))
            .map(|name| std::str::from_utf8(name).or_error().map(RefNameBuf::from))
            .transpose()?;
        tracing::debug!(?default_branch);
        Ok(default_branch)
    }

    /// Import the previously fetched remote-tracking branches and tags into the
    /// jj repo and update jj's local bookmarks and tags.
    ///
    /// Clears all yet-to-be-imported {branch/tag_names, remote_name} pairs
    /// after the import. If `fetch()` has not been called since the last time
    /// `import_refs()` was called then this will be a no-op.
    #[tracing::instrument(skip(self))]
    pub async fn import_refs(&mut self) -> Result<GitImportStats, GitImportError> {
        tracing::debug!("import_refs");
        let refs_to_import =
            diff_fetched_refs_to_import(self.mut_repo.view(), &self.git_repo, |kind, symbol| {
                match kind {
                    GitRefKind::Bookmark => self
                        .fetched
                        .iter()
                        .filter(|fetched| fetched.remote == symbol.remote)
                        .any(|fetched| fetched.bookmark_matcher.is_match(symbol.name.as_str())),
                    GitRefKind::Tag => self
                        .fetched
                        .iter()
                        .filter(|fetched| fetched.remote == symbol.remote)
                        .any(|fetched| fetched.tag_matcher.is_match(symbol.name.as_str())),
                }
            })?;
        let import_stats =
            import_refs_inner(self.mut_repo, refs_to_import, self.import_options).await?;

        self.fetched.clear();

        Ok(import_stats)
    }
}

#[derive(Error, Debug)]
pub enum GitPushError {
    #[error("No git remote named '{}'", .0.as_symbol())]
    NoSuchRemote(RemoteNameBuf),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error("Git push failed")]
    Git(#[from] gix::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

#[derive(Clone, Debug, Default)]
pub struct GitPushRefTargets {
    /// Bookmark or branch `(name, [expected_target, new_target])`s to push.
    pub bookmarks: Vec<(RefNameBuf, Diff<Option<CommitId>>)>,
    /// Tag `(name, [expected_target, new_target])`s to push.
    pub tags: Vec<(RefNameBuf, Diff<Option<CommitId>>)>,
}

pub struct GitRefUpdate {
    pub qualified_name: GitRefNameBuf,
    /// Expected position on the remote and new position to push.
    ///
    /// The expected position is sourced from the local remote-tracking branch.
    /// This should be `None` if we expect the ref to not exist on the remote.
    pub targets: Diff<Option<gix::ObjectId>>,
}

/// Miscellaneous options for Git push command.
#[derive(Clone, Debug, Default)]
pub struct GitPushOptions {
    /// `--push-option` arguments.
    pub remote_push_options: Vec<String>,
}

/// Pushes the specified refs and updates the repo view accordingly.
pub fn push_refs(
    mut_repo: &mut MutableRepo,
    remote_options: GitRemoteOptions,
    remote: &RemoteName,
    targets: &GitPushRefTargets,
    callback: &mut dyn GitRemoteCallback,
    options: &GitPushOptions,
) -> Result<GitPushStats, GitPushError> {
    validate_remote_name(remote)?;

    let git_repo = get_git_repo(mut_repo.store())?;
    let to_tag_target = |name: &RefName, remote: &RemoteName, id: &CommitId| {
        let remote_matcher = StringMatcher::exact(remote);
        let oid = owned_oid_from_commit_id(id);
        find_git_tag_oid_to_copy(mut_repo.view(), &git_repo, name, &remote_matcher, &oid)
            .unwrap_or(oid)
    };
    let ref_updates = itertools::chain(
        targets.bookmarks.iter().map(|(name, update)| GitRefUpdate {
            qualified_name: format!("refs/heads/{name}", name = name.as_str()).into(),
            targets: update
                .as_ref()
                .map(|id| id.as_ref().map(owned_oid_from_commit_id)),
        }),
        targets.tags.iter().map(|(name, update)| GitRefUpdate {
            qualified_name: format!("refs/tags/{name}", name = name.as_str()).into(),
            targets: Diff {
                before: update
                    .before
                    .as_ref()
                    .map(|id| to_tag_target(name, remote, id)),
                after: update
                    .after
                    .as_ref()
                    .map(|id| to_tag_target(name, REMOTE_NAME_FOR_LOCAL_GIT_REPO, id)),
            },
        }),
    )
    .collect_vec();

    let push_stats = push_updates(
        mut_repo,
        remote_options,
        remote,
        &ref_updates,
        callback,
        options,
    )?;
    tracing::debug!(?push_stats);

    let pushed: HashSet<&GitRefName> = push_stats.pushed.iter().map(AsRef::as_ref).collect();
    let pushed_bookmark_updates = || {
        iter::zip(&targets.bookmarks, &ref_updates[..targets.bookmarks.len()])
            .filter(|(_, ref_update)| pushed.contains(&*ref_update.qualified_name))
            .map(|((name, update), ref_update)| (&**name, update, ref_update))
    };
    let pushed_tag_updates = || {
        iter::zip(&targets.tags, &ref_updates[targets.bookmarks.len()..])
            .filter(|(_, ref_update)| pushed.contains(&*ref_update.qualified_name))
            .map(|((name, update), ref_update)| (&**name, update, ref_update))
    };

    // Update fetched refs to reduce path conflicts on subsequent fetches and
    // allow looking up annotated tag oid without fetching. Since these refs
    // should never be imported without fetching from the remote, update failure
    // isn't a hard error.
    for (name, _, ref_update) in pushed_bookmark_updates() {
        let symbol = name.to_remote_symbol(remote);
        let edit = to_fetched_ref_update(GitRefKind::Bookmark, symbol, ref_update.targets.after);
        if let Err(err) = git_repo.edit_reference(edit) {
            tracing::warn!(?symbol, ?err, "failed to update fetched bookmark ref");
        }
    }
    for (name, _, ref_update) in pushed_tag_updates() {
        let symbol = name.to_remote_symbol(remote);
        let edit = to_fetched_ref_update(GitRefKind::Tag, symbol, ref_update.targets.after);
        if let Err(err) = git_repo.edit_reference(edit) {
            tracing::warn!(?symbol, ?err, "failed to update remote tag ref");
        }
    }

    for (name, update, _) in pushed_bookmark_updates() {
        let new_remote_ref = RemoteRef {
            target: RefTarget::resolved(update.after.clone()),
            state: RemoteRefState::Tracked,
        };
        mut_repo.set_remote_bookmark(name.to_remote_symbol(remote), new_remote_ref);
    }
    for (name, update, _) in pushed_tag_updates() {
        let new_remote_ref = RemoteRef {
            target: RefTarget::resolved(update.after.clone()),
            state: RemoteRefState::Tracked,
        };
        mut_repo.set_remote_tag(name.to_remote_symbol(remote), new_remote_ref);
    }

    Ok(push_stats)
}

/// Pushes the specified Git refs without updating the repo view.
pub fn push_updates(
    repo: &dyn Repo,
    remote_options: GitRemoteOptions,
    remote_name: &RemoteName,
    updates: &[GitRefUpdate],
    callback: &mut dyn GitRemoteCallback,
    options: &GitPushOptions,
) -> Result<GitPushStats, GitPushError> {
    use gix::push::Rejection;
    use gix::push::Status;

    let git_repo = get_git_repo(repo.store())?;
    try_find_active_remote_inner(&git_repo, remote_name)
        .ok_or_else(|| GitPushError::NoSuchRemote(remote_name.to_owned()))??;
    let git_repo = git_repo.into_sync();
    let remote_name = remote_name.to_owned();
    let updates = updates
        .iter()
        .map(|update| {
            Ok(gix::push::Update {
                destination: gix::refs::FullName::try_from(update.qualified_name.as_str())
                    .or_error()?,
                new_id: update.targets.after,
                lease: gix::push::Lease::MustMatch(update.targets.before),
                force: false,
            })
        })
        .collect::<gix::Result<Vec<_>>>()?;
    let push_options = options
        .remote_push_options
        .iter()
        .map(|option| BString::from(option.as_str()))
        .collect_vec();
    let outcomes = crate::git_remote::run_with_progress(
        callback,
        move |mut progress, sideband, interrupt| {
            use gix::progress::NestedProgress as _;

            let git_repo = git_repo.to_thread_local();
            let remote = git_repo
                .find_remote(remote_name.as_str())?
                .with_command_environment(remote_options.environment);
            let urls = remote
                .urls(gix::remote::Direction::Push)
                .cloned()
                .collect_vec();
            let mut outcomes = Vec::with_capacity(urls.len());
            for url in urls {
                let remote = remote.clone().with_push_url_without_url_rewrite(url)?;
                outcomes.push(remote.push(
                    updates.clone(),
                    gix::push::Options {
                        push_options: &push_options,
                        ..Default::default()
                    },
                    progress.add_child("push"),
                    |is_error, bytes| sideband.send(is_error, bytes),
                    interrupt,
                )?);
            }
            Ok(outcomes)
        },
    )?;
    let mut stats = GitPushStats::default();
    for update in outcomes.into_iter().flat_map(|outcome| outcome.updates) {
        let name = GitRefNameBuf::from(
            String::from_utf8(update.update.destination.as_bstr().to_vec()).or_error()?,
        );
        let reason = match update.status {
            Status::UpToDate | Status::Updated { .. } | Status::WouldUpdate { .. } => {
                stats.pushed.push(name);
                continue;
            }
            Status::Rejected(Rejection::Server(reason)) => {
                stats
                    .remote_rejected
                    .push((name, Some(String::from_utf8_lossy(&reason).into_owned())));
                continue;
            }
            Status::Rejected(Rejection::StaleLease) => "stale info",
            Status::Rejected(Rejection::AlreadyExists) => "already exists",
            Status::Rejected(Rejection::FetchFirst) => "fetch first",
            Status::Rejected(Rejection::NeedsForce) => "needs force",
            Status::Rejected(Rejection::NonFastForward) => "non-fast-forward",
            Status::Rejected(Rejection::NonCommitBranch) => "new tip is not a commit",
            Status::Rejected(Rejection::Atomic) => "atomic push failed",
        };
        stats.rejected.push((name, Some(reason.to_owned())));
    }
    // A ref is fully pushed only when every configured push destination accepted it.
    stats.pushed.retain(|name| {
        !stats
            .rejected
            .iter()
            .chain(&stats.remote_rejected)
            .any(|(rejected, _)| rejected == name)
    });
    stats.pushed.sort();
    stats.pushed.dedup();
    stats.rejected.sort();
    stats.rejected.dedup();
    stats.remote_rejected.sort();
    stats.remote_rejected.dedup();
    Ok(stats)
}

/// Constructs `RefEdit` to update pushed ref in the fetch namespace.
fn to_fetched_ref_update(
    kind: GitRefKind,
    symbol: RemoteRefSymbol<'_>,
    new_oid: Option<gix::ObjectId>,
) -> gix::refs::transaction::RefEdit {
    // No constraint on existing ref because fetched ref shouldn't be moved
    // externally, and should always point to the actual remote ref.
    let expected = gix::refs::transaction::PreviousValue::Any;
    let change = match new_oid {
        Some(oid) => gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange::default(),
            expected,
            new: oid.into(),
        },
        None => gix::refs::transaction::Change::Delete {
            expected,
            log: gix::refs::transaction::RefLog::AndReference,
        },
    };
    let namespace = match kind {
        GitRefKind::Bookmark => FETCH_BOOKMARK_REF_NAMESPACE,
        GitRefKind::Tag => REMOTE_TAG_REF_NAMESPACE,
    };
    let name = format!(
        "{namespace}{remote}/{name}",
        remote = symbol.remote.as_str(),
        name = symbol.name.as_str()
    );
    gix::refs::transaction::RefEdit {
        change,
        name: name.try_into().expect("pushed ref name should be valid"),
        deref: false,
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::revset;
    use crate::revset::RevsetDiagnostics;

    #[test]
    fn test_split_positive_negative_patterns() {
        fn split(text: &str) -> (Vec<StringPattern>, Vec<StringPattern>) {
            try_split(text).unwrap()
        }

        fn try_split(
            text: &str,
        ) -> Result<(Vec<StringPattern>, Vec<StringPattern>), GitRefExpressionError> {
            let mut diagnostics = RevsetDiagnostics::new();
            let expr = revset::parse_string_expression(&mut diagnostics, text).unwrap();
            let (positives, negatives) = split_into_positive_negative_patterns(&expr)?;
            Ok((
                positives.into_iter().cloned().collect(),
                negatives.into_iter().cloned().collect(),
            ))
        }

        insta::assert_compact_debug_snapshot!(
            split("a"),
            @r#"([Exact("a")], [])"#);
        insta::assert_compact_debug_snapshot!(
            split("~a"),
            @r#"([Substring("")], [Exact("a")])"#);
        insta::assert_compact_debug_snapshot!(
            split("~a~b"),
            @r#"([Substring("")], [Exact("a"), Exact("b")])"#);
        insta::assert_compact_debug_snapshot!(
            split("~(a|b)"),
            @r#"([Substring("")], [Exact("a"), Exact("b")])"#);
        insta::assert_compact_debug_snapshot!(
            split("a|b"),
            @r#"([Exact("a"), Exact("b")], [])"#);
        insta::assert_compact_debug_snapshot!(
            split("(a|b)&~c"),
            @r#"([Exact("a"), Exact("b")], [Exact("c")])"#);
        insta::assert_compact_debug_snapshot!(
            split("~a&b"),
            @r#"([Exact("b")], [Exact("a")])"#);
        insta::assert_compact_debug_snapshot!(
            split("a&~b&~c"),
            @r#"([Exact("a")], [Exact("b"), Exact("c")])"#);
        insta::assert_compact_debug_snapshot!(
            split("~a&b&~c"),
            @r#"([Exact("b")], [Exact("a"), Exact("c")])"#);
        insta::assert_compact_debug_snapshot!(
            split("a&~(b|c)"),
            @r#"([Exact("a")], [Exact("b"), Exact("c")])"#);
        insta::assert_compact_debug_snapshot!(
            split("((a|b)|c)&~(d|(e|f))"),
            @r#"([Exact("a"), Exact("b"), Exact("c")], [Exact("d"), Exact("e"), Exact("f")])"#);
        assert_matches!(
            try_split("a&b"),
            Err(GitRefExpressionError::PositiveIntersection)
        );
        assert_matches!(try_split("a|~b"), Err(GitRefExpressionError::NestedNotIn));
        assert_matches!(
            try_split("a&~(b&~c)"),
            Err(GitRefExpressionError::NestedIntersection)
        );
        assert_matches!(
            try_split("(a|b)&c"),
            Err(GitRefExpressionError::PositiveIntersection)
        );
        assert_matches!(
            try_split("(a&~b)&(~c&~d)"),
            Err(GitRefExpressionError::PositiveIntersection)
        );
        assert_matches!(try_split("a&~~b"), Err(GitRefExpressionError::NestedNotIn));
        assert_matches!(
            try_split("a&~b|c&~d"),
            Err(GitRefExpressionError::NestedIntersection)
        );

        // `~*` should generate empty patterns. `a~*` and `~(a|*)` don't because
        // `a` may be incompatible with Git refspecs.
        insta::assert_compact_debug_snapshot!(
            split("*"),
            @r#"([Glob(GlobPattern("*"))], [])"#);
        insta::assert_compact_debug_snapshot!(
            split("~*"),
            @"([], [])");
        insta::assert_compact_debug_snapshot!(
            split("a~*"),
            @r#"([Exact("a")], [Glob(GlobPattern("*"))])"#);
        insta::assert_compact_debug_snapshot!(
            split("~(a|*)"),
            @r#"([Substring("")], [Exact("a"), Glob(GlobPattern("*"))])"#);
    }
}
