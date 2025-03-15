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

#![allow(missing_docs)]

use std::borrow::Borrow;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::default::Default;
use std::fs::File;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;

use bstr::BStr;
use bstr::BString;
use futures::StreamExt as _;
use itertools::Itertools as _;
use pollster::FutureExt as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::backend::TreeValue;
use crate::commit::Commit;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::git_backend::GitBackend;
use crate::git_subprocess::GitSubprocessContext;
use crate::git_subprocess::GitSubprocessError;
use crate::matchers::EverythingMatcher;
use crate::merged_tree::MergedTree;
use crate::merged_tree::TreeDiffEntry;
use crate::object_id::ObjectId as _;
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
use crate::refs::BookmarkPushUpdate;
use crate::repo::MutableRepo;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::revset::RevsetExpression;
use crate::settings::GitSettings;
use crate::store::Store;
use crate::str_util::StringPattern;
use crate::view::View;

/// Reserved remote name for the backing Git repo.
pub const REMOTE_NAME_FOR_LOCAL_GIT_REPO: &RemoteName = RemoteName::new("git");
/// Git ref prefix that would conflict with the reserved "git" remote.
pub const RESERVED_REMOTE_REF_NAMESPACE: &str = "refs/remotes/git/";
/// Ref name used as a placeholder to unset HEAD without a commit.
const UNBORN_ROOT_REF_NAME: &str = "refs/jj/root";
/// Dummy file to be added to the index to indicate that the user is editing a
/// commit with a conflict that isn't represented in the Git index.
const INDEX_DUMMY_CONFLICT_FILE: &str = ".jj-do-not-resolve-this-conflict";

#[derive(Debug, Error)]
pub enum GitRemoteNameError {
    #[error(
        "Git remote named '{name}' is reserved for local Git repository",
        name = REMOTE_NAME_FOR_LOCAL_GIT_REPO.as_symbol()
    )]
    ReservedForLocalGitRepo,
    #[error("Git remotes with slashes are incompatible with jj: {}", .0.as_symbol())]
    WithSlash(RemoteNameBuf),
}

fn validate_remote_name(name: &RemoteName) -> Result<(), GitRemoteNameError> {
    if name == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        Err(GitRemoteNameError::ReservedForLocalGitRepo)
    } else if name.as_str().contains("/") {
        Err(GitRemoteNameError::WithSlash(name.to_owned()))
    } else {
        Ok(())
    }
}

/// Type of Git ref to be imported or exported.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitRefKind {
    Bookmark,
    Tag,
}

/// Stats from a git push
#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
        RefSpec {
            forced: true,
            source: Some(source.into()),
            destination: destination.into(),
        }
    }

    fn delete(destination: impl Into<String>) -> Self {
        // We don't force push on branch deletion
        RefSpec {
            forced: false,
            source: None,
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

/// Helper struct that matches a refspec with its expected location in the
/// remote it's being pushed to
pub(crate) struct RefToPush<'a> {
    pub(crate) refspec: &'a RefSpec,
    pub(crate) expected_location: Option<&'a CommitId>,
}

impl<'a> RefToPush<'a> {
    fn new(
        refspec: &'a RefSpec,
        expected_locations: &'a HashMap<&GitRefName, Option<&CommitId>>,
    ) -> Self {
        let expected_location = *expected_locations
            .get(GitRefName::new(&refspec.destination))
            .expect(
                "The refspecs and the expected locations were both constructed from the same \
                 source of truth. This means the lookup should always work.",
            );

        RefToPush {
            refspec,
            expected_location,
        }
    }

    pub(crate) fn to_git_lease(&self) -> String {
        format!(
            "{}:{}",
            self.refspec.destination,
            self.expected_location
                .map(|x| x.to_string())
                .as_deref()
                .unwrap_or("")
        )
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
    } else if let Some(remote_and_name) = full_name.as_str().strip_prefix("refs/remotes/") {
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
                Some(format!("refs/remotes/{remote}/{name}").into())
            }
        }
        GitRefKind::Tag => {
            (remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO).then(|| format!("refs/tags/{name}").into())
        }
    }
}

#[derive(Debug, Error)]
#[error("The repo is not backed by a Git repo")]
pub struct UnexpectedGitBackendError;

/// Returns the underlying `GitBackend` implementation.
pub fn get_git_backend(store: &Store) -> Result<&GitBackend, UnexpectedGitBackendError> {
    store
        .backend_impl()
        .downcast_ref()
        .ok_or(UnexpectedGitBackendError)
}

/// Returns new thread-local instance to access to the underlying Git repo.
pub fn get_git_repo(store: &Store) -> Result<gix::Repository, UnexpectedGitBackendError> {
    get_git_backend(store).map(|backend| backend.git_repo())
}

/// Checks if `git_ref` points to a Git commit object, and returns its id.
///
/// If the ref points to the previously `known_target` (i.e. unchanged), this
/// should be faster than `git_ref.into_fully_peeled_id()`.
fn resolve_git_ref_to_commit_id(
    git_ref: &gix::Reference,
    known_target: &RefTarget,
) -> Option<CommitId> {
    let mut peeling_ref = Cow::Borrowed(git_ref);

    // Try fast path if we have a candidate id which is known to be a commit object.
    if let Some(id) = known_target.as_normal() {
        let raw_ref = &git_ref.inner;
        if matches!(raw_ref.target.try_id(), Some(oid) if oid.as_bytes() == id.as_bytes()) {
            return Some(id.clone());
        }
        if matches!(raw_ref.peeled, Some(oid) if oid.as_bytes() == id.as_bytes()) {
            // Perhaps an annotated tag stored in packed-refs file, and pointing to the
            // already known target commit.
            return Some(id.clone());
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
                if oid.as_bytes() == id.as_bytes() {
                    // An annotated tag pointing to the already known target commit.
                    return Some(id.clone());
                }
                // Unknown id. Recurse from the current state. A tag may point to
                // non-commit object.
                peeling_ref.to_mut().inner.target = gix::refs::Target::Object(oid.detach());
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
    is_commit.then(|| CommitId::from_bytes(peeled_id.as_bytes()))
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
    Backend(BackendError),
    #[error(transparent)]
    Git(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitImportError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        GitImportError::Git(source.into())
    }
}

/// Describes changes made by `import_refs()` or `fetch()`.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct GitImportStats {
    /// Commits superseded by newly imported commits.
    pub abandoned_commits: Vec<CommitId>,
    /// Remote bookmark `(symbol, (old_remote_ref, new_target))`s to be merged
    /// in to the local bookmarks, sorted by `symbol`.
    pub changed_remote_bookmarks: Vec<(RemoteRefSymbolBuf, (RemoteRef, RefTarget))>,
    /// Remote tag `(symbol, (old_remote_ref, new_target))`s to be merged in to
    /// the local tags, sorted by `symbol`.
    pub changed_remote_tags: Vec<(RemoteRefSymbolBuf, (RemoteRef, RefTarget))>,
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
    /// Remote bookmark `(symbol, (old_remote_ref, new_target))`s to be merged
    /// in to the local bookmarks, sorted by `symbol`.
    changed_remote_bookmarks: Vec<(RemoteRefSymbolBuf, (RemoteRef, RefTarget))>,
    /// Remote tag `(symbol, (old_remote_ref, new_target))`s to be merged in to
    /// the local tags, sorted by `symbol`.
    changed_remote_tags: Vec<(RemoteRefSymbolBuf, (RemoteRef, RefTarget))>,
    /// Git ref names that couldn't be imported, sorted by name.
    failed_ref_names: Vec<BString>,
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// This function detects conflicts (if both Git and JJ modified a bookmark) and
/// records them in JJ's view.
pub fn import_refs(
    mut_repo: &mut MutableRepo,
    git_settings: &GitSettings,
) -> Result<GitImportStats, GitImportError> {
    import_some_refs(mut_repo, git_settings, |_, _| true)
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// Only bookmarks and tags whose remote symbol pass the filter will be
/// considered for addition, update, or deletion.
pub fn import_some_refs(
    mut_repo: &mut MutableRepo,
    git_settings: &GitSettings,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> Result<GitImportStats, GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store)?;
    let git_repo = git_backend.git_repo();

    let RefsToImport {
        changed_git_refs,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    } = diff_refs_to_import(mut_repo.view(), &git_repo, git_ref_filter)?;

    // Bulk-import all reachable Git commits to the backend to reduce overhead
    // of table merging and ref updates.
    //
    // changed_remote_bookmarks/tags might contain new_targets that are not in
    // changed_git_refs, but such targets should have already been imported to
    // the backend.
    let index = mut_repo.index();
    let missing_head_ids = changed_git_refs
        .iter()
        .flat_map(|(_, new_target)| new_target.added_ids())
        .filter(|&id| !index.has_id(id));
    let heads_imported = git_backend.import_head_commits(missing_head_ids).is_ok();

    // Import new remote heads
    let mut head_commits = Vec::new();
    let get_commit = |id| {
        // If bulk-import failed, try again to find bad head or ref.
        if !heads_imported && !index.has_id(id) {
            git_backend.import_head_commits([id])?;
        }
        store.get_commit(id)
    };
    for (symbol, (_, new_target)) in
        itertools::chain(&changed_remote_bookmarks, &changed_remote_tags)
    {
        for id in new_target.added_ids() {
            let commit = get_commit(id).map_err(|err| GitImportError::MissingRefAncestor {
                symbol: symbol.clone(),
                err,
            })?;
            head_commits.push(commit);
        }
    }
    // It's unlikely the imported commits were missing, but I/O-related error
    // can still occur.
    mut_repo
        .add_heads(&head_commits)
        .map_err(GitImportError::Backend)?;

    // Apply the change that happened in git since last time we imported refs.
    for (full_name, new_target) in changed_git_refs {
        mut_repo.set_git_ref_target(&full_name, new_target);
    }
    for (symbol, (old_remote_ref, new_target)) in &changed_remote_bookmarks {
        let symbol = symbol.as_ref();
        let base_target = old_remote_ref.tracked_target();
        let new_remote_ref = RemoteRef {
            target: new_target.clone(),
            state: if old_remote_ref.is_present() {
                old_remote_ref.state
            } else {
                default_remote_ref_state_for(GitRefKind::Bookmark, symbol, git_settings)
            },
        };
        if new_remote_ref.is_tracked() {
            mut_repo.merge_local_bookmark(symbol.name, base_target, &new_remote_ref.target);
        }
        // Remote-tracking branch is the last known state of the branch in the remote.
        // It shouldn't diverge even if we had inconsistent view.
        mut_repo.set_remote_bookmark(symbol, new_remote_ref);
    }
    for (symbol, (old_remote_ref, new_target)) in &changed_remote_tags {
        let symbol = symbol.as_ref();
        let base_target = old_remote_ref.tracked_target();
        let new_remote_ref = RemoteRef {
            target: new_target.clone(),
            state: if old_remote_ref.is_present() {
                old_remote_ref.state
            } else {
                default_remote_ref_state_for(GitRefKind::Tag, symbol, git_settings)
            },
        };
        if new_remote_ref.is_tracked() {
            mut_repo.merge_tag(symbol.name, base_target, &new_remote_ref.target);
        }
        // TODO: If we add Git-tracking tag, it will be updated here.
    }

    let abandoned_commits = if git_settings.abandon_unreachable_commits {
        abandon_unreachable_commits(mut_repo, &changed_remote_bookmarks, &changed_remote_tags)
            .map_err(GitImportError::Backend)?
    } else {
        vec![]
    };
    let stats = GitImportStats {
        abandoned_commits,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    };
    Ok(stats)
}

/// Finds commits that used to be reachable in git that no longer are reachable.
/// Those commits will be recorded as abandoned in the `MutableRepo`.
fn abandon_unreachable_commits(
    mut_repo: &mut MutableRepo,
    changed_remote_bookmarks: &[(RemoteRefSymbolBuf, (RemoteRef, RefTarget))],
    changed_remote_tags: &[(RemoteRefSymbolBuf, (RemoteRef, RefTarget))],
) -> BackendResult<Vec<CommitId>> {
    let hidable_git_heads = itertools::chain(changed_remote_bookmarks, changed_remote_tags)
        .flat_map(|(_, (old_remote_ref, _))| old_remote_ref.target.added_ids())
        .cloned()
        .collect_vec();
    if hidable_git_heads.is_empty() {
        return Ok(vec![]);
    }
    let pinned_expression = RevsetExpression::union_all(&[
        // Local refs are usually visible, no need to filter out hidden
        RevsetExpression::commits(pinned_commit_ids(mut_repo.view())),
        RevsetExpression::commits(remotely_pinned_commit_ids(mut_repo.view()))
            // Hidden remote branches should not contribute to pinning
            .intersection(&RevsetExpression::visible_heads().ancestors()),
        RevsetExpression::root(),
    ]);
    let abandoned_expression = pinned_expression
        .range(&RevsetExpression::commits(hidable_git_heads))
        // Don't include already-abandoned commits in GitImportStats
        .intersection(&RevsetExpression::visible_heads().ancestors());
    let abandoned_commit_ids: Vec<_> = abandoned_expression
        .evaluate(mut_repo)
        .map_err(|err| err.into_backend_error())?
        .iter()
        .try_collect()
        .map_err(|err| err.into_backend_error())?;
    for id in &abandoned_commit_ids {
        let commit = mut_repo.store().get_commit(id)?;
        mut_repo.record_abandoned_commit(&commit);
    }
    Ok(abandoned_commit_ids)
}

/// Calculates diff of git refs to be imported.
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
    // TODO: migrate tags to the remote view, and don't destructure &RemoteRef
    let mut known_remote_bookmarks = view
        .all_remote_bookmarks()
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Bookmark, symbol))
        .map(|(symbol, remote_ref)| (RemoteRefKey(symbol), (&remote_ref.target, remote_ref.state)))
        .collect();
    // TODO: compare to tags stored in the "git" remote view. Since tags should
    // never be moved locally in jj, we can consider local tags as merge base.
    let mut known_remote_tags = view
        .tags()
        .iter()
        .map(|(name, target)| {
            let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
            let state = RemoteRefState::Tracked;
            (symbol, (target, state))
        })
        .filter(|&(symbol, _)| git_ref_filter(GitRefKind::Tag, symbol))
        .map(|(symbol, remote_ref)| (RemoteRefKey(symbol), remote_ref))
        .collect();

    let mut changed_git_refs = Vec::new();
    let mut changed_remote_bookmarks = Vec::new();
    let mut changed_remote_tags = Vec::new();
    let mut failed_ref_names = Vec::new();
    let actual = git_repo.references().map_err(GitImportError::from_git)?;
    collect_changed_refs_to_import(
        actual.local_branches().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut known_remote_bookmarks,
        &mut changed_git_refs,
        &mut changed_remote_bookmarks,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    collect_changed_refs_to_import(
        actual.remote_branches().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut known_remote_bookmarks,
        &mut changed_git_refs,
        &mut changed_remote_bookmarks,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    collect_changed_refs_to_import(
        actual.tags().map_err(GitImportError::from_git)?,
        &mut known_git_refs,
        &mut known_remote_tags,
        &mut changed_git_refs,
        &mut changed_remote_tags,
        &mut failed_ref_names,
        &git_ref_filter,
    )?;
    for full_name in known_git_refs.into_keys() {
        changed_git_refs.push((full_name.to_owned(), RefTarget::absent()));
    }
    for (RemoteRefKey(symbol), (old_target, old_state)) in known_remote_bookmarks {
        let old_remote_ref = RemoteRef {
            target: old_target.clone(),
            state: old_state,
        };
        changed_remote_bookmarks.push((symbol.to_owned(), (old_remote_ref, RefTarget::absent())));
    }
    for (RemoteRefKey(symbol), (old_target, old_state)) in known_remote_tags {
        let old_remote_ref = RemoteRef {
            target: old_target.clone(),
            state: old_state,
        };
        changed_remote_tags.push((symbol.to_owned(), (old_remote_ref, RefTarget::absent())));
    }

    // Stabilize merge order and output.
    changed_git_refs.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));
    changed_remote_bookmarks.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    changed_remote_tags.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    failed_ref_names.sort_unstable();
    Ok(RefsToImport {
        changed_git_refs,
        changed_remote_bookmarks,
        changed_remote_tags,
        failed_ref_names,
    })
}

fn collect_changed_refs_to_import(
    actual_git_refs: gix::reference::iter::Iter<'_>,
    known_git_refs: &mut HashMap<&GitRefName, &RefTarget>,
    known_remote_refs: &mut HashMap<RemoteRefKey<'_>, (&RefTarget, RemoteRefState)>,
    changed_git_refs: &mut Vec<(GitRefNameBuf, RefTarget)>,
    changed_remote_refs: &mut Vec<(RemoteRefSymbolBuf, (RemoteRef, RefTarget))>,
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
        let Some(id) = resolve_git_ref_to_commit_id(&git_ref, old_git_target) else {
            // Skip (or remove existing) invalid refs.
            continue;
        };
        let new_target = RefTarget::normal(id);
        known_git_refs.remove(full_name);
        if new_target != *old_git_target {
            changed_git_refs.push((full_name.to_owned(), new_target.clone()));
        }
        // TODO: Make it configurable which remotes are publishing and update public
        // heads here.
        let (old_remote_target, old_remote_state) = known_remote_refs
            .remove(&symbol)
            .unwrap_or_else(|| (RefTarget::absent_ref(), RemoteRefState::New));
        if new_target != *old_remote_target {
            let old_remote_ref = RemoteRef {
                target: old_remote_target.clone(),
                state: old_remote_state,
            };
            changed_remote_refs.push((symbol.to_owned(), (old_remote_ref, new_target)));
        }
    }
    Ok(())
}

fn default_remote_ref_state_for(
    kind: GitRefKind,
    symbol: RemoteRefSymbol<'_>,
    git_settings: &GitSettings,
) -> RemoteRefState {
    match kind {
        GitRefKind::Bookmark => {
            if symbol.remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO || git_settings.auto_local_bookmark {
                RemoteRefState::Tracked
            } else {
                RemoteRefState::New
            }
        }
        GitRefKind::Tag => RemoteRefState::Tracked,
    }
}

/// Commits referenced by local branches or tags.
///
/// On `import_refs()`, this is similar to collecting commits referenced by
/// `view.git_refs()`. Main difference is that local branches can be moved by
/// tracking remotes, and such mutation isn't applied to `view.git_refs()` yet.
fn pinned_commit_ids(view: &View) -> Vec<CommitId> {
    itertools::chain(
        view.local_bookmarks().map(|(_, target)| target),
        view.tags().values(),
    )
    .flat_map(|target| target.added_ids())
    .cloned()
    .collect()
}

/// Commits referenced by untracked remote branches including hidden ones.
///
/// Tracked remote branches aren't included because they should have been merged
/// into the local counterparts, and the changes pulled from one remote should
/// propagate to the other remotes on later push. OTOH, untracked remote
/// branches are considered independent refs.
fn remotely_pinned_commit_ids(view: &View) -> Vec<CommitId> {
    view.all_remote_bookmarks()
        .filter(|(_, remote_ref)| !remote_ref.is_tracked())
        .map(|(_, remote_ref)| &remote_ref.target)
        .flat_map(|target| target.added_ids())
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
pub fn import_head(mut_repo: &mut MutableRepo) -> Result<(), GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store)?;
    let git_repo = git_backend.git_repo();

    let old_git_head = mut_repo.view().git_head();
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
        if !index.has_id(head_id) {
            git_backend.import_head_commits([head_id]).map_err(|err| {
                GitImportError::MissingHeadTarget {
                    id: head_id.clone(),
                    err,
                }
            })?;
        }
        // It's unlikely the imported commits were missing, but I/O-related
        // error can still occur.
        store
            .get_commit(head_id)
            .and_then(|commit| mut_repo.add_head(&commit))
            .map_err(GitImportError::Backend)?;
    }

    mut_repo.set_git_head_target(RefTarget::resolved(new_git_head_id));
    Ok(())
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
        GitExportError::Git(source.into())
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
    FailedToDelete(#[source] Box<gix::reference::edit::Error>),
    /// Failed to set the ref in the Git repo
    #[error("Failed to set")]
    FailedToSet(#[source] Box<gix::reference::edit::Error>),
}

/// Describes changes made by [`export_refs()`].
#[derive(Debug)]
pub struct GitExportStats {
    /// Remote bookmarks that couldn't be exported, sorted by `symbol`.
    pub failed_bookmarks: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
}

#[derive(Debug)]
struct RefsToExport {
    /// Remote bookmark `(symbol, (old_oid, new_oid))`s to update, sorted by
    /// `symbol`.
    bookmarks_to_update: Vec<(RemoteRefSymbolBuf, (Option<gix::ObjectId>, gix::ObjectId))>,
    /// Remote bookmark `(symbol, old_oid)`s to delete, sorted by `symbol`.
    ///
    /// Deletion has to be exported first to avoid conflict with new bookmarks
    /// on file-system.
    bookmarks_to_delete: Vec<(RemoteRefSymbolBuf, gix::ObjectId)>,
    /// Remote bookmarks that couldn't be exported, sorted by `symbol`.
    failed_bookmarks: Vec<(RemoteRefSymbolBuf, FailedRefExportReason)>,
}

/// Export changes to bookmarks made in the Jujutsu repo compared to our last
/// seen view of the Git repo in `mut_repo.view().git_refs()`.
///
/// We ignore changed bookmarks that are conflicted (were also changed in the
/// Git repo compared to our last remembered view of the Git repo). These will
/// be marked conflicted by the next `jj git import`.
///
/// We do not export tags and other refs at the moment, since these aren't
/// supposed to be modified by JJ. For them, the Git state is considered
/// authoritative.
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

    let git_repo = get_git_repo(mut_repo.store())?;

    let RefsToExport {
        bookmarks_to_update,
        bookmarks_to_delete,
        mut failed_bookmarks,
    } = diff_refs_to_export(
        mut_repo.view(),
        mut_repo.store().root_commit_id(),
        &git_ref_filter,
    );

    // TODO: Also check other worktrees' HEAD.
    if let Ok(head_ref) = git_repo.find_reference("HEAD") {
        let target_name = head_ref.target().try_name().map(|name| name.to_owned());
        if let Some((GitRefKind::Bookmark, symbol)) = target_name
            .as_ref()
            .and_then(|name| str::from_utf8(name.as_bstr()).ok())
            .and_then(|name| parse_git_ref(name.as_ref()))
        {
            let old_target = head_ref.inner.target.clone();
            let current_oid = match head_ref.into_fully_peeled_id() {
                Ok(id) => Some(id.detach()),
                Err(gix::reference::peel::Error::ToId(
                    gix::refs::peel::to_id::Error::FollowToObject(
                        gix::refs::peel::to_object::Error::Follow(
                            gix::refs::file::find::existing::Error::NotFound { .. },
                        ),
                    ),
                )) => None, // Unborn ref should be considered absent
                Err(err) => return Err(GitExportError::from_git(err)),
            };
            let new_oid = if let Some((_old_oid, new_oid)) = get(&bookmarks_to_update, symbol) {
                Some(new_oid)
            } else if get(&bookmarks_to_delete, symbol).is_some() {
                None
            } else {
                current_oid.as_ref()
            };
            if new_oid != current_oid.as_ref() {
                update_git_head(
                    &git_repo,
                    gix::refs::transaction::PreviousValue::MustExistAndMatch(old_target),
                    current_oid,
                )
                .map_err(GitExportError::from_git)?;
            }
        }
    }
    for (symbol, old_oid) in bookmarks_to_delete {
        let Some(git_ref_name) = to_git_ref_name(GitRefKind::Bookmark, symbol.as_ref()) else {
            failed_bookmarks.push((symbol, FailedRefExportReason::InvalidGitName));
            continue;
        };
        if let Err(reason) = delete_git_ref(&git_repo, &git_ref_name, &old_oid) {
            failed_bookmarks.push((symbol, reason));
        } else {
            let new_target = RefTarget::absent();
            mut_repo.set_git_ref_target(&git_ref_name, new_target);
        }
    }
    for (symbol, (old_oid, new_oid)) in bookmarks_to_update {
        let Some(git_ref_name) = to_git_ref_name(GitRefKind::Bookmark, symbol.as_ref()) else {
            failed_bookmarks.push((symbol, FailedRefExportReason::InvalidGitName));
            continue;
        };
        if let Err(reason) = update_git_ref(&git_repo, &git_ref_name, old_oid, new_oid) {
            failed_bookmarks.push((symbol, reason));
        } else {
            let new_target = RefTarget::normal(CommitId::from_bytes(new_oid.as_bytes()));
            mut_repo.set_git_ref_target(&git_ref_name, new_target);
        }
    }

    // Stabilize output, allow binary search.
    failed_bookmarks.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));

    copy_exportable_local_bookmarks_to_remote_view(
        mut_repo,
        REMOTE_NAME_FOR_LOCAL_GIT_REPO,
        |name| {
            let symbol = name.to_remote_symbol(REMOTE_NAME_FOR_LOCAL_GIT_REPO);
            git_ref_filter(GitRefKind::Bookmark, symbol) && get(&failed_bookmarks, symbol).is_none()
        },
    );

    Ok(GitExportStats { failed_bookmarks })
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
            (!new_target.has_conflict() && old_target != new_target).then_some((name, new_target))
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

/// Calculates diff of bookmarks to be exported.
fn diff_refs_to_export(
    view: &View,
    root_commit_id: &CommitId,
    git_ref_filter: impl Fn(GitRefKind, RemoteRefSymbol<'_>) -> bool,
) -> RefsToExport {
    // Local targets will be copied to the "git" remote if successfully exported. So
    // the local bookmarks are considered to be the new "git" remote bookmarks.
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
        .map(|(symbol, new_target)| (symbol, (RefTarget::absent_ref(), new_target)))
        .collect();
    let known_git_refs = view
        .git_refs()
        .iter()
        .map(|(full_name, target)| {
            let (kind, symbol) =
                parse_git_ref(full_name).expect("stored git ref should be parsable");
            ((kind, symbol), target)
        })
        .filter(|&((kind, symbol), _)| {
            // There are two situations where remote bookmarks get out of sync:
            // 1. `jj bookmark forget --include-remotes`
            // 2. `jj op undo`/`restore` in colocated repo
            kind == GitRefKind::Bookmark && git_ref_filter(kind, symbol)
        });
    for ((_kind, symbol), target) in known_git_refs {
        all_bookmark_targets
            .entry(symbol)
            .and_modify(|(old_target, _)| *old_target = target)
            .or_insert((target, RefTarget::absent_ref()));
    }

    let mut bookmarks_to_update = Vec::new();
    let mut bookmarks_to_delete = Vec::new();
    let mut failed_bookmarks = Vec::new();
    let root_commit_target = RefTarget::normal(root_commit_id.clone());
    for (symbol, (old_target, new_target)) in all_bookmark_targets {
        if new_target == old_target {
            continue;
        }
        if *new_target == root_commit_target {
            // Git doesn't have a root commit
            failed_bookmarks.push((symbol.to_owned(), FailedRefExportReason::OnRootCommit));
            continue;
        }
        let old_oid = if let Some(id) = old_target.as_normal() {
            Some(gix::ObjectId::from_bytes_or_panic(id.as_bytes()))
        } else if old_target.has_conflict() {
            // The old git ref should only be a conflict if there were concurrent import
            // operations while the value changed. Don't overwrite these values.
            failed_bookmarks.push((symbol.to_owned(), FailedRefExportReason::ConflictedOldState));
            continue;
        } else {
            assert!(old_target.is_absent());
            None
        };
        if let Some(id) = new_target.as_normal() {
            let new_oid = gix::ObjectId::from_bytes_or_panic(id.as_bytes());
            bookmarks_to_update.push((symbol.to_owned(), (old_oid, new_oid)));
        } else if new_target.has_conflict() {
            // Skip conflicts and leave the old value in git_refs
            continue;
        } else {
            assert!(new_target.is_absent());
            bookmarks_to_delete.push((symbol.to_owned(), old_oid.unwrap()));
        }
    }

    // Stabilize export order and output, allow binary search.
    bookmarks_to_update.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    bookmarks_to_delete.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    failed_bookmarks.sort_unstable_by(|(sym1, _), (sym2, _)| sym1.cmp(sym2));
    RefsToExport {
        bookmarks_to_update,
        bookmarks_to_delete,
        failed_bookmarks,
    }
}

fn delete_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &GitRefName,
    old_oid: &gix::oid,
) -> Result<(), FailedRefExportReason> {
    if let Ok(git_ref) = git_repo.find_reference(git_ref_name.as_str()) {
        if git_ref.inner.target.try_id() == Some(old_oid) {
            // The ref has not been updated by git, so go ahead and delete it
            git_ref
                .delete()
                .map_err(|err| FailedRefExportReason::FailedToDelete(err.into()))?;
        } else {
            // The ref was updated by git
            return Err(FailedRefExportReason::DeletedInJjModifiedInGit);
        }
    } else {
        // The ref is already deleted
    }
    Ok(())
}

fn update_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &GitRefName,
    old_oid: Option<gix::ObjectId>,
    new_oid: gix::ObjectId,
) -> Result<(), FailedRefExportReason> {
    match old_oid {
        None => {
            if let Ok(git_repo_ref) = git_repo.find_reference(git_ref_name.as_str()) {
                // The ref was added in jj and in git. We're good if and only if git
                // pointed it to our desired target.
                if git_repo_ref.inner.target.try_id() != Some(&new_oid) {
                    return Err(FailedRefExportReason::AddedInJjAddedInGit);
                }
            } else {
                // The ref was added in jj but still doesn't exist in git, so add it
                git_repo
                    .reference(
                        git_ref_name.as_str(),
                        new_oid,
                        gix::refs::transaction::PreviousValue::MustNotExist,
                        "export from jj",
                    )
                    .map_err(|err| FailedRefExportReason::FailedToSet(err.into()))?;
            }
        }
        Some(old_oid) => {
            // The ref was modified in jj. We can use gix API for updating under a lock.
            if let Err(err) = git_repo.reference(
                git_ref_name.as_str(),
                new_oid,
                gix::refs::transaction::PreviousValue::MustExistAndMatch(old_oid.into()),
                "export from jj",
            ) {
                // The reference was probably updated in git
                if let Ok(git_repo_ref) = git_repo.find_reference(git_ref_name.as_str()) {
                    // We still consider this a success if it was updated to our desired target
                    if git_repo_ref.inner.target.try_id() != Some(&new_oid) {
                        return Err(FailedRefExportReason::FailedToSet(err.into()));
                    }
                } else {
                    // The reference was deleted in git and moved in jj
                    return Err(FailedRefExportReason::ModifiedInJjDeletedInGit);
                }
            } else {
                // Successfully updated from old_oid to new_oid (unchanged in
                // git)
            }
        }
    }
    Ok(())
}

/// Ensures Git HEAD is detached and pointing to the `new_oid`. If `new_oid`
/// is `None` (meaning absent), dummy placeholder ref will be set.
fn update_git_head(
    git_repo: &gix::Repository,
    expected_ref: gix::refs::transaction::PreviousValue,
    new_oid: Option<gix::ObjectId>,
) -> Result<(), gix::reference::edit::Error> {
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
pub enum GitResetHeadError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Git(Box<dyn std::error::Error + Send + Sync>),
    #[error("Failed to update Git HEAD ref")]
    UpdateHeadRef(#[source] Box<gix::reference::edit::Error>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitResetHeadError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        GitResetHeadError::Git(source.into())
    }
}

/// Sets Git HEAD to the parent of the given working-copy commit and resets
/// the Git index.
pub fn reset_head(mut_repo: &mut MutableRepo, wc_commit: &Commit) -> Result<(), GitResetHeadError> {
    let git_repo = get_git_repo(mut_repo.store())?;

    let first_parent_id = &wc_commit.parent_ids()[0];
    let new_head_target = if first_parent_id != mut_repo.store().root_commit_id() {
        RefTarget::normal(first_parent_id.clone())
    } else {
        RefTarget::absent()
    };

    // If the first parent of the working copy has changed, reset the Git HEAD.
    let old_head_target = mut_repo.git_head();
    if old_head_target != new_head_target {
        let expected_ref = if let Some(id) = old_head_target.as_normal() {
            // We have to check the actual HEAD state because we don't record a
            // symbolic ref as such.
            let actual_head = git_repo.head().map_err(GitResetHeadError::from_git)?;
            if actual_head.is_detached() {
                let id = gix::ObjectId::from_bytes_or_panic(id.as_bytes());
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
        let new_oid = new_head_target
            .as_normal()
            .map(|id| gix::ObjectId::from_bytes_or_panic(id.as_bytes()));
        update_git_head(&git_repo, expected_ref, new_oid)
            .map_err(|err| GitResetHeadError::UpdateHeadRef(err.into()))?;
        mut_repo.set_git_head_target(new_head_target);
    }

    // If there is an ongoing operation (merge, rebase, etc.), we need to clean it
    // up.
    //
    // TODO: Polish and upstream this to `gix`.
    if git_repo.state().is_some() {
        // Based on the files `git2::Repository::cleanup_state` deletes; when
        // upstreaming this logic should probably become more elaborate to match
        // `git(1)` behaviour.
        const STATE_FILE_NAMES: &[&str] = &[
            "MERGE_HEAD",
            "MERGE_MODE",
            "MERGE_MSG",
            "REVERT_HEAD",
            "CHERRY_PICK_HEAD",
            "BISECT_LOG",
        ];
        const STATE_DIR_NAMES: &[&str] = &["rebase-merge", "rebase-apply", "sequencer"];
        let handle_err = |err: PathError| match err.error.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(GitResetHeadError::from_git(err)),
        };
        for file_name in STATE_FILE_NAMES {
            let path = git_repo.path().join(file_name);
            std::fs::remove_file(&path)
                .context(&path)
                .or_else(handle_err)?;
        }
        for dir_name in STATE_DIR_NAMES {
            let path = git_repo.path().join(dir_name);
            std::fs::remove_dir_all(&path)
                .context(&path)
                .or_else(handle_err)?;
        }
    }

    let parent_tree = wc_commit.parent_tree(mut_repo)?;

    // Use the merged parent tree as the Git index, allowing `git diff` to show the
    // same changes as `jj diff`. If the merged parent tree has conflicts, then the
    // Git index will also be conflicted.
    let mut index = if let Some(tree) = parent_tree.as_merge().as_resolved() {
        if tree.id() == mut_repo.store().empty_tree_id() {
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
                .index_from_tree(&gix::ObjectId::from_bytes_or_panic(tree.id().as_bytes()))
                .map_err(GitResetHeadError::from_git)?
        }
    } else {
        build_index_from_merged_tree(&git_repo, parent_tree.clone())?
    };

    let wc_tree = wc_commit.tree()?;
    update_intent_to_add_impl(&mut index, &parent_tree, &wc_tree, git_repo.object_hash())
        .block_on()?;

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
        .map_err(GitResetHeadError::from_git)?;

    Ok(())
}

fn build_index_from_merged_tree(
    git_repo: &gix::Repository,
    merged_tree: MergedTree,
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
                TreeValue::File { id, executable } => {
                    if *executable {
                        (id.as_bytes(), gix::index::entry::Mode::FILE_EXECUTABLE)
                    } else {
                        (id.as_bytes(), gix::index::entry::Mode::FILE)
                    }
                }
                TreeValue::Symlink(id) => (id.as_bytes(), gix::index::entry::Mode::SYMLINK),
                TreeValue::Tree(_) => {
                    // This case is only possible if there is a file-directory conflict, since
                    // `MergedTree::entries` handles the recursion otherwise. We only materialize a
                    // file in the working copy for file-directory conflicts, so we don't add the
                    // tree to the index here either.
                    return;
                }
                TreeValue::GitSubmodule(id) => (id.as_bytes(), gix::index::entry::Mode::COMMIT),
                TreeValue::Conflict(_) => panic!("unexpected merged tree entry: {entry:?}"),
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

    for (path, entry) in merged_tree.entries() {
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
pub fn update_intent_to_add(
    repo: &dyn Repo,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
) -> Result<(), GitResetHeadError> {
    let git_repo = get_git_repo(repo.store())?;
    let mut index = git_repo
        .index_or_empty()
        .map_err(GitResetHeadError::from_git)?;
    let mut_index = Arc::make_mut(&mut index);
    update_intent_to_add_impl(mut_index, old_tree, new_tree, git_repo.object_hash()).block_on()?;
    debug_assert!(mut_index.verify_entries().is_ok());
    mut_index
        .write(gix::index::write::Options::default())
        .map_err(GitResetHeadError::from_git)?;

    Ok(())
}

async fn update_intent_to_add_impl(
    index: &mut gix::index::File,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
    hash_kind: gix::hash::Kind,
) -> BackendResult<()> {
    let mut diff_stream = old_tree.diff_stream(new_tree, &EverythingMatcher);
    let mut added_paths = vec![];
    let mut removed_paths = HashSet::new();
    while let Some(TreeDiffEntry { path, values }) = diff_stream.next().await {
        let (before, after) = values?;
        if before.is_absent() {
            let executable = match after.as_normal() {
                Some(TreeValue::File { id: _, executable }) => *executable,
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
        } else if after.is_absent() {
            removed_paths.insert(BString::from(path.into_internal_string()));
        }
    }

    if added_paths.is_empty() && removed_paths.is_empty() {
        return Ok(());
    }

    for (path, executable) in added_paths {
        // We have checked that the index doesn't have this entry
        index.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            gix::ObjectId::empty_blob(hash_kind),
            gix::index::entry::Flags::INTENT_TO_ADD | gix::index::entry::Flags::EXTENDED,
            if executable {
                gix::index::entry::Mode::FILE_EXECUTABLE
            } else {
                gix::index::entry::Mode::FILE
            },
            path.as_ref(),
        );
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
    InternalGitError(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitRemoteManagementError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        GitRemoteManagementError::InternalGitError(source.into())
    }
}

/// Determine, by its name, if a remote refers to the special local-only "git"
/// remote that is used in the Git backend.
///
/// This function always returns false if the "git" feature is not enabled.
pub fn is_special_git_remote(remote: &RemoteName) -> bool {
    remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO
}

fn default_fetch_refspec(remote: &RemoteName) -> String {
    format!(
        "+refs/heads/*:refs/remotes/{remote}/*",
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
fn save_git_config(config: &gix::config::File) -> std::io::Result<()> {
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
    config: &mut gix::config::File<'static>,
    remote_name: &RemoteName,
    remote: &mut gix::Remote,
) -> Result<(), GitRemoteManagementError> {
    // Work around the gitoxide remote management bug
    // <https://github.com/GitoxideLabs/gitoxide/issues/1951> by adding
    // an empty section.
    //
    // Note that this will produce useless empty sections if we ever
    // support remote configuration keys other than `fetch` and `url`.
    config
        .new_section(
            "remote",
            Some(Cow::Owned(BString::from(remote_name.as_str()))),
        )
        .map_err(GitRemoteManagementError::from_git)?;
    remote
        .save_as_to(remote_name.as_str(), config)
        .map_err(GitRemoteManagementError::from_git)?;
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
                .any(|branch_remote_name| **branch_remote_name == remote_name.as_str())
            {
                return None;
            }
            if remote_values.len() > 1
                || push_remote_values.len() > 1
                || section.value_names().any(|name| {
                    !name.eq_ignore_ascii_case(b"remote") && !name.eq_ignore_ascii_case(b"merge")
                })
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
            .set(
                "remote"
                    .try_into()
                    .expect("'remote' to be a valid value name"),
                BStr::new(new_remote_name.as_str()),
            );
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
                !name.eq_ignore_ascii_case(b"url") && !name.eq_ignore_ascii_case(b"fetch")
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
    let names = git_repo
        .remote_names()
        .into_iter()
        // exclude empty [remote "<name>"] section
        .filter(|name| git_repo.try_find_remote(name.as_ref()).is_some())
        // ignore non-UTF-8 remote names which we don't support
        .filter_map(|name| String::from_utf8(name.into_owned().into()).ok())
        .map(RemoteNameBuf::from)
        .collect();
    Ok(names)
}

pub fn add_remote(
    store: &Store,
    remote_name: &RemoteName,
    url: &str,
) -> Result<(), GitRemoteManagementError> {
    let git_repo = get_git_repo(store)?;

    validate_remote_name(remote_name)?;

    if git_repo.try_find_remote(remote_name.as_str()).is_some() {
        return Err(GitRemoteManagementError::RemoteAlreadyExists(
            remote_name.to_owned(),
        ));
    }

    let mut remote = git_repo
        .remote_at(url)
        .map_err(GitRemoteManagementError::from_git)?
        .with_refspecs(
            [default_fetch_refspec(remote_name).as_bytes()],
            gix::remote::Direction::Fetch,
        )
        .expect("default refspec to be valid");

    let mut config = git_repo.config_snapshot().clone();
    save_remote(&mut config, remote_name, &mut remote)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    Ok(())
}

pub fn remove_remote(
    mut_repo: &mut MutableRepo,
    remote_name: &RemoteName,
) -> Result<(), GitRemoteManagementError> {
    let mut git_repo = get_git_repo(mut_repo.store())?;

    if git_repo.try_find_remote(remote_name.as_str()).is_none() {
        return Err(GitRemoteManagementError::NoSuchRemote(
            remote_name.to_owned(),
        ));
    };

    let mut config = git_repo.config_snapshot().clone();
    remove_remote_git_branch_config_sections(&mut config, remote_name)?;
    remove_remote_git_config_sections(&mut config, remote_name)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    remove_remote_git_refs(&mut git_repo, remote_name)
        .map_err(GitRemoteManagementError::from_git)?;

    if remote_name != REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        remove_remote_refs(mut_repo, remote_name);
    }

    Ok(())
}

fn remove_remote_git_refs(
    git_repo: &mut gix::Repository,
    remote: &RemoteName,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let edits: Vec<_> = git_repo
        .references()?
        .prefixed(format!("refs/remotes/{remote}/", remote = remote.as_str()))?
        .map_ok(remove_ref)
        .try_collect()?;
    git_repo.edit_references(edits)?;
    Ok(())
}

fn remove_remote_refs(mut_repo: &mut MutableRepo, remote: &RemoteName) {
    mut_repo.remove_remote(remote);
    let prefix = format!("refs/remotes/{remote}/", remote = remote.as_str());
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

    let Some(result) = git_repo.try_find_remote(old_remote_name.as_str()) else {
        return Err(GitRemoteManagementError::NoSuchRemote(
            old_remote_name.to_owned(),
        ));
    };
    let mut remote = result.map_err(GitRemoteManagementError::from_git)?;

    if git_repo.try_find_remote(new_remote_name.as_str()).is_some() {
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
            ))
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

    rename_remote_git_refs(&mut git_repo, old_remote_name, new_remote_name)
        .map_err(GitRemoteManagementError::from_git)?;

    if old_remote_name != REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        rename_remote_refs(mut_repo, old_remote_name, new_remote_name);
    }

    Ok(())
}

fn rename_remote_git_refs(
    git_repo: &mut gix::Repository,
    old_remote_name: &RemoteName,
    new_remote_name: &RemoteName,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let old_prefix = format!("refs/remotes/{}/", old_remote_name.as_str());
    let new_prefix = format!("refs/remotes/{}/", new_remote_name.as_str());
    let ref_log_message = BString::from(format!(
        "renamed remote {old_remote_name} to {new_remote_name}",
        old_remote_name = old_remote_name.as_symbol(),
        new_remote_name = new_remote_name.as_symbol(),
    ));

    let edits: Vec<_> = git_repo
        .references()?
        .prefixed(old_prefix.clone())?
        .map_ok(|old_ref| {
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
        })
        .flatten_ok()
        .try_collect()?;
    git_repo.edit_references(edits)?;
    Ok(())
}

/// Set the `url` to be used when fetching data from a remote.
///
/// Shim for the missing `gix::Remote::fetch_url` API.
///
/// **TODO:** Upstream an implementation of this to `gix`.
fn gix_remote_with_fetch_url<Url, E>(
    remote: gix::Remote,
    url: Url,
) -> Result<gix::Remote, gix::remote::init::Error>
where
    Url: TryInto<gix::Url, Error = E>,
    gix::url::parse::Error: From<E>,
{
    let mut new_remote = remote.repo().remote_at(url)?;
    // Copy the existing data from `remote`.
    //
    // We don’t copy the push URL, as there does not seem to be any way to reliably
    // detect whether one is present with the current API, and `jj git remote
    // set-url` refuses to work with them anyway.
    new_remote = new_remote.with_fetch_tags(remote.fetch_tags());
    for direction in [gix::remote::Direction::Fetch, gix::remote::Direction::Push] {
        new_remote
            .replace_refspecs(
                remote
                    .refspecs(direction)
                    .iter()
                    .map(|refspec| refspec.to_ref().to_bstring()),
                direction,
            )
            .expect("existing refspecs to be valid");
    }
    Ok(new_remote)
}

pub fn set_remote_url(
    store: &Store,
    remote_name: &RemoteName,
    new_remote_url: &str,
) -> Result<(), GitRemoteManagementError> {
    let git_repo = get_git_repo(store)?;

    validate_remote_name(remote_name)?;

    let Some(result) = git_repo.try_find_remote_without_url_rewrite(remote_name.as_str()) else {
        return Err(GitRemoteManagementError::NoSuchRemote(
            remote_name.to_owned(),
        ));
    };
    let mut remote = result.map_err(GitRemoteManagementError::from_git)?;

    if remote.url(gix::remote::Direction::Push) != remote.url(gix::remote::Direction::Fetch) {
        return Err(GitRemoteManagementError::NonstandardConfiguration(
            remote_name.to_owned(),
        ));
    }

    remote = gix_remote_with_fetch_url(remote, new_remote_url)
        .map_err(GitRemoteManagementError::from_git)?;

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
    let prefix = format!("refs/remotes/{}/", old_remote_name.as_str());
    let git_refs = mut_repo
        .view()
        .git_refs()
        .iter()
        .filter_map(|(old, target)| {
            old.as_str().strip_prefix(&prefix).map(|p| {
                let new: GitRefNameBuf =
                    format!("refs/remotes/{}/{p}", new_remote_name.as_str()).into();
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
    #[error(
        "Invalid branch pattern provided. When fetching, branch names and globs may not contain the characters `{chars}`",
        chars = INVALID_REFSPEC_CHARS.iter().join("`, `")
    )]
    InvalidBranchPattern(StringPattern),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error(transparent)]
    Subprocess(#[from] GitSubprocessError),
}

// TODO: If Git2 implementation is removed, this can be replaced with
// UnexpectedGitBackendError.
#[derive(Debug, Error)]
pub enum GitFetchPrepareError {
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

struct FetchedBranches {
    remote: RemoteNameBuf,
    branches: Vec<StringPattern>,
}

/// Helper struct to execute multiple `git fetch` operations
pub struct GitFetch<'a> {
    mut_repo: &'a mut MutableRepo,
    fetch_impl: GitFetchImpl<'a>,
    git_settings: &'a GitSettings,
    fetched: Vec<FetchedBranches>,
}

impl<'a> GitFetch<'a> {
    pub fn new(
        mut_repo: &'a mut MutableRepo,
        git_settings: &'a GitSettings,
    ) -> Result<Self, GitFetchPrepareError> {
        let fetch_impl = GitFetchImpl::new(mut_repo.store(), git_settings)?;
        Ok(GitFetch {
            mut_repo,
            fetch_impl,
            git_settings,
            fetched: vec![],
        })
    }

    /// Perform a `git fetch` on the local git repo, updating the
    /// remote-tracking branches in the git repo.
    ///
    /// Keeps track of the {branch_names, remote_name} pair the refs can be
    /// subsequently imported into the `jj` repo by calling `import_refs()`.
    #[tracing::instrument(skip(self, callbacks))]
    pub fn fetch(
        &mut self,
        remote_name: &RemoteName,
        branch_names: &[StringPattern],
        callbacks: RemoteCallbacks<'_>,
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitFetchError> {
        validate_remote_name(remote_name)?;
        self.fetch_impl
            .fetch(remote_name, branch_names, callbacks, depth)?;
        self.fetched.push(FetchedBranches {
            remote: remote_name.to_owned(),
            branches: branch_names.to_vec(),
        });
        Ok(())
    }

    /// Queries remote for the default branch name.
    #[tracing::instrument(skip(self, callbacks))]
    pub fn get_default_branch(
        &self,
        remote_name: &RemoteName,
        callbacks: RemoteCallbacks<'_>,
    ) -> Result<Option<RefNameBuf>, GitFetchError> {
        self.fetch_impl.get_default_branch(remote_name, callbacks)
    }

    /// Import the previously fetched remote-tracking branches into the jj repo
    /// and update jj's local branches. We also import local tags since remote
    /// tags should have been merged by Git.
    ///
    /// Clears all yet-to-be-imported {branch_names, remote_name} pairs after
    /// the import. If `fetch()` has not been called since the last time
    /// `import_refs()` was called then this will be a no-op.
    #[tracing::instrument(skip(self))]
    pub fn import_refs(&mut self) -> Result<GitImportStats, GitImportError> {
        tracing::debug!("import_refs");
        let import_stats =
            import_some_refs(
                self.mut_repo,
                self.git_settings,
                |kind, symbol| match kind {
                    GitRefKind::Bookmark => self
                        .fetched
                        .iter()
                        .filter(|fetched| fetched.remote == symbol.remote)
                        .any(|fetched| {
                            fetched
                                .branches
                                .iter()
                                .any(|pattern| pattern.matches(symbol.name.as_str()))
                        }),
                    GitRefKind::Tag => true,
                },
            )?;

        self.fetched.clear();

        Ok(import_stats)
    }
}

fn expand_fetch_refspecs(
    remote: &RemoteName,
    branch_names: &[StringPattern],
) -> Result<Vec<RefSpec>, GitFetchError> {
    branch_names
        .iter()
        .map(|pattern| {
            pattern
                .to_glob()
                .filter(
                    /* This triggered by non-glob `*`s in addition to INVALID_REFSPEC_CHARS
                     * because `to_glob()` escapes such `*`s as `[*]`. */
                    |glob| !glob.contains(INVALID_REFSPEC_CHARS),
                )
                .map(|glob| {
                    RefSpec::forced(
                        format!("refs/heads/{glob}"),
                        format!("refs/remotes/{remote}/{glob}", remote = remote.as_str()),
                    )
                })
                .ok_or_else(|| GitFetchError::InvalidBranchPattern(pattern.clone()))
        })
        .collect()
}

enum GitFetchImpl<'a> {
    Subprocess {
        git_repo: Box<gix::Repository>,
        git_ctx: GitSubprocessContext<'a>,
    },
}

impl<'a> GitFetchImpl<'a> {
    fn new(store: &Store, git_settings: &'a GitSettings) -> Result<Self, GitFetchPrepareError> {
        let git_backend = get_git_backend(store)?;
        let git_repo = Box::new(git_backend.git_repo());
        let git_ctx =
            GitSubprocessContext::from_git_backend(git_backend, &git_settings.executable_path);
        Ok(GitFetchImpl::Subprocess { git_repo, git_ctx })
    }

    fn fetch(
        &self,
        remote_name: &RemoteName,
        branch_names: &[StringPattern],
        callbacks: RemoteCallbacks<'_>,
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitFetchError> {
        match self {
            GitFetchImpl::Subprocess { git_repo, git_ctx } => subprocess_fetch(
                git_repo,
                git_ctx,
                remote_name,
                branch_names,
                callbacks,
                depth,
            ),
        }
    }

    fn get_default_branch(
        &self,
        remote_name: &RemoteName,
        callbacks: RemoteCallbacks<'_>,
    ) -> Result<Option<RefNameBuf>, GitFetchError> {
        match self {
            GitFetchImpl::Subprocess { git_repo, git_ctx } => {
                subprocess_get_default_branch(git_repo, git_ctx, remote_name, callbacks)
            }
        }
    }
}

fn subprocess_fetch(
    git_repo: &gix::Repository,
    git_ctx: &GitSubprocessContext,
    remote_name: &RemoteName,
    branch_names: &[StringPattern],
    mut callbacks: RemoteCallbacks<'_>,
    depth: Option<NonZeroU32>,
) -> Result<(), GitFetchError> {
    // check the remote exists
    if git_repo.try_find_remote(remote_name.as_str()).is_none() {
        return Err(GitFetchError::NoSuchRemote(remote_name.to_owned()));
    }
    // At this point, we are only updating Git's remote tracking branches, not the
    // local branches.
    let mut remaining_refspecs: Vec<_> = expand_fetch_refspecs(remote_name, branch_names)?;
    if remaining_refspecs.is_empty() {
        // Don't fall back to the base refspecs.
        return Ok(());
    }

    let mut branches_to_prune = Vec::new();
    // git unfortunately errors out if one of the many refspecs is not found
    //
    // our approach is to filter out failures and retry,
    // until either all have failed or an attempt has succeeded
    //
    // even more unfortunately, git errors out one refspec at a time,
    // meaning that the below cycle runs in O(#failed refspecs)
    while let Some(failing_refspec) =
        git_ctx.spawn_fetch(remote_name, &remaining_refspecs, &mut callbacks, depth)?
    {
        tracing::debug!(failing_refspec, "failed to fetch ref");
        remaining_refspecs.retain(|r| r.source.as_ref() != Some(&failing_refspec));

        if let Some(branch_name) = failing_refspec.strip_prefix("refs/heads/") {
            branches_to_prune.push(format!(
                "{remote_name}/{branch_name}",
                remote_name = remote_name.as_str()
            ));
        }
    }

    // Even if git fetch has --prune, if a branch is not found it will not be
    // pruned on fetch
    git_ctx.spawn_branch_prune(&branches_to_prune)?;
    Ok(())
}

fn subprocess_get_default_branch(
    git_repo: &gix::Repository,
    git_ctx: &GitSubprocessContext,
    remote_name: &RemoteName,
    _callbacks: RemoteCallbacks<'_>,
) -> Result<Option<RefNameBuf>, GitFetchError> {
    if git_repo.try_find_remote(remote_name.as_str()).is_none() {
        return Err(GitFetchError::NoSuchRemote(remote_name.to_owned()));
    }
    let default_branch = git_ctx.spawn_remote_show(remote_name)?;
    tracing::debug!(?default_branch);
    Ok(default_branch)
}

#[derive(Error, Debug)]
pub enum GitPushError {
    #[error("No git remote named '{}'", .0.as_symbol())]
    NoSuchRemote(RemoteNameBuf),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error(transparent)]
    Subprocess(#[from] GitSubprocessError),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

#[derive(Clone, Debug)]
pub struct GitBranchPushTargets {
    pub branch_updates: Vec<(RefNameBuf, BookmarkPushUpdate)>,
}

pub struct GitRefUpdate {
    pub qualified_name: GitRefNameBuf,
    /// Expected position on the remote or None if we expect the ref to not
    /// exist on the remote
    ///
    /// This is sourced from the local remote-tracking branch.
    pub expected_current_target: Option<CommitId>,
    pub new_target: Option<CommitId>,
}

/// Pushes the specified branches and updates the repo view accordingly.
pub fn push_branches(
    mut_repo: &mut MutableRepo,
    git_settings: &GitSettings,
    remote: &RemoteName,
    targets: &GitBranchPushTargets,
    callbacks: RemoteCallbacks<'_>,
) -> Result<GitPushStats, GitPushError> {
    validate_remote_name(remote)?;

    let ref_updates = targets
        .branch_updates
        .iter()
        .map(|(name, update)| GitRefUpdate {
            qualified_name: format!("refs/heads/{name}", name = name.as_str()).into(),
            expected_current_target: update.old_target.clone(),
            new_target: update.new_target.clone(),
        })
        .collect_vec();

    let push_stats = push_updates(mut_repo, git_settings, remote, &ref_updates, callbacks)?;
    tracing::debug!(?push_stats);

    // TODO: add support for partially pushed refs? we could update the view
    // excluding rejected refs, but the transaction would be aborted anyway
    // if we returned an Err.
    if push_stats.all_ok() {
        for (name, update) in &targets.branch_updates {
            let git_ref_name: GitRefNameBuf = format!(
                "refs/remotes/{remote}/{name}",
                remote = remote.as_str(),
                name = name.as_str()
            )
            .into();
            let new_remote_ref = RemoteRef {
                target: RefTarget::resolved(update.new_target.clone()),
                state: RemoteRefState::Tracked,
            };
            mut_repo.set_git_ref_target(&git_ref_name, new_remote_ref.target.clone());
            mut_repo.set_remote_bookmark(name.to_remote_symbol(remote), new_remote_ref);
        }
    }

    Ok(push_stats)
}

/// Pushes the specified Git refs without updating the repo view.
pub fn push_updates(
    repo: &dyn Repo,
    git_settings: &GitSettings,
    remote_name: &RemoteName,
    updates: &[GitRefUpdate],
    callbacks: RemoteCallbacks<'_>,
) -> Result<GitPushStats, GitPushError> {
    let mut qualified_remote_refs_expected_locations = HashMap::new();
    let mut refspecs = vec![];
    for update in updates {
        qualified_remote_refs_expected_locations.insert(
            update.qualified_name.as_ref(),
            update.expected_current_target.as_ref(),
        );
        if let Some(new_target) = &update.new_target {
            // We always force-push. We use the push_negotiation callback in
            // `push_refs` to check that the refs did not unexpectedly move on
            // the remote.
            refspecs.push(RefSpec::forced(new_target.hex(), &update.qualified_name));
        } else {
            // Prefixing this with `+` to force-push or not should make no
            // difference. The push negotiation happens regardless, and wouldn't
            // allow creating a branch if it's not a fast-forward.
            refspecs.push(RefSpec::delete(&update.qualified_name));
        }
    }
    // TODO(ilyagr): `push_refs`, or parts of it, should probably be inlined. This
    // requires adjusting some tests.

    let git_backend = get_git_backend(repo.store())?;
    let git_repo = git_backend.git_repo();
    let git_ctx =
        GitSubprocessContext::from_git_backend(git_backend, &git_settings.executable_path);
    subprocess_push_refs(
        &git_repo,
        &git_ctx,
        remote_name,
        &qualified_remote_refs_expected_locations,
        &refspecs,
        callbacks,
    )
}

fn subprocess_push_refs(
    git_repo: &gix::Repository,
    git_ctx: &GitSubprocessContext,
    remote_name: &RemoteName,
    qualified_remote_refs_expected_locations: &HashMap<&GitRefName, Option<&CommitId>>,
    refspecs: &[RefSpec],
    mut callbacks: RemoteCallbacks<'_>,
) -> Result<GitPushStats, GitPushError> {
    // check the remote exists
    if git_repo.try_find_remote(remote_name.as_str()).is_none() {
        return Err(GitPushError::NoSuchRemote(remote_name.to_owned()));
    }

    let refs_to_push: Vec<RefToPush> = refspecs
        .iter()
        .map(|full_refspec| RefToPush::new(full_refspec, qualified_remote_refs_expected_locations))
        .collect();

    let mut push_stats = git_ctx.spawn_push(remote_name, &refs_to_push, &mut callbacks)?;
    push_stats.pushed.sort();
    push_stats.rejected.sort();
    push_stats.remote_rejected.sort();
    Ok(push_stats)
}

#[non_exhaustive]
#[derive(Default)]
#[expect(clippy::type_complexity)]
pub struct RemoteCallbacks<'a> {
    pub progress: Option<&'a mut dyn FnMut(&Progress)>,
    pub sideband_progress: Option<&'a mut dyn FnMut(&[u8])>,
    pub get_ssh_keys: Option<&'a mut dyn FnMut(&str) -> Vec<PathBuf>>,
    pub get_password: Option<&'a mut dyn FnMut(&str, &str) -> Option<String>>,
    pub get_username_password: Option<&'a mut dyn FnMut(&str) -> Option<(String, String)>>,
}

#[derive(Clone, Debug)]
pub struct Progress {
    /// `Some` iff data transfer is currently in progress
    pub bytes_downloaded: Option<u64>,
    pub overall: f32,
}
