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

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::default::Default;
use std::fmt;
use std::fs::File;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str;

use bstr::BStr;
use bstr::BString;
use itertools::Itertools;
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
use crate::index::Index;
use crate::merged_tree::MergedTree;
use crate::object_id::ObjectId;
use crate::op_store::RefTarget;
use crate::op_store::RefTargetOptionExt;
use crate::op_store::RemoteRef;
use crate::op_store::RemoteRefState;
use crate::refs;
use crate::refs::BookmarkPushUpdate;
use crate::refs::RemoteRefSymbol;
use crate::refs::RemoteRefSymbolBuf;
use crate::repo::MutableRepo;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::revset::RevsetExpression;
use crate::settings::GitSettings;
use crate::store::Store;
use crate::str_util::StringPattern;
use crate::view::View;

/// Reserved remote name for the backing Git repo.
pub const REMOTE_NAME_FOR_LOCAL_GIT_REPO: &str = "git";
/// Ref name used as a placeholder to unset HEAD without a commit.
const UNBORN_ROOT_REF_NAME: &str = "refs/jj/root";
/// Dummy file to be added to the index to indicate that the user is editing a
/// commit with a conflict that isn't represented in the Git index.
const INDEX_DUMMY_CONFLICT_FILE: &str = ".jj-do-not-resolve-this-conflict";

#[derive(Debug, Error)]
pub enum GitRemoteNameError {
    #[error(
        "Git remote named '{name}' is reserved for local Git repository",
        name = REMOTE_NAME_FOR_LOCAL_GIT_REPO
    )]
    ReservedForLocalGitRepo,
    #[error("Git remotes with slashes are incompatible with jj: {0}")]
    WithSlash(String),
}

fn validate_remote_name(name: &str) -> Result<(), GitRemoteNameError> {
    if name == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
        Err(GitRemoteNameError::ReservedForLocalGitRepo)
    } else if name.contains("/") {
        Err(GitRemoteNameError::WithSlash(name.to_owned()))
    } else {
        Ok(())
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash, Debug)]
pub enum RefName {
    LocalBranch(String),
    RemoteBranch(RemoteRefSymbolBuf),
    Tag(String),
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefName::LocalBranch(name) => write!(f, "{name}"),
            RefName::RemoteBranch(symbol) => write!(f, "{symbol}"),
            RefName::Tag(name) => write!(f, "{name}"),
        }
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
    fn new(refspec: &'a RefSpec, expected_locations: &'a HashMap<&str, Option<&CommitId>>) -> Self {
        let expected_location = *expected_locations.get(refspec.destination.as_str()).expect(
            "The refspecs and the expected locations were both constructed from the same source \
             of truth. This means the lookup should always work.",
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

pub fn parse_git_ref(ref_name: &str) -> Option<RefName> {
    if let Some(branch_name) = ref_name.strip_prefix("refs/heads/") {
        // Git CLI says 'HEAD' is not a valid branch name
        (branch_name != "HEAD").then(|| RefName::LocalBranch(branch_name.to_string()))
    } else if let Some(remote_and_branch) = ref_name.strip_prefix("refs/remotes/") {
        remote_and_branch
            .split_once('/')
            // "refs/remotes/origin/HEAD" isn't a real remote-tracking branch
            .filter(|&(_, name)| name != "HEAD")
            .map(|(remote, name)| {
                RefName::RemoteBranch(RemoteRefSymbol { name, remote }.to_owned())
            })
    } else {
        ref_name
            .strip_prefix("refs/tags/")
            .map(|tag_name| RefName::Tag(tag_name.to_string()))
    }
}

fn to_git_ref_name(parsed_ref: &RefName) -> Option<String> {
    match parsed_ref {
        RefName::LocalBranch(branch) => {
            (!branch.is_empty() && branch != "HEAD").then(|| format!("refs/heads/{branch}"))
        }
        RefName::RemoteBranch(RemoteRefSymbolBuf { name, remote }) => {
            (!name.is_empty() && name != "HEAD").then(|| format!("refs/remotes/{remote}/{name}"))
        }
        RefName::Tag(tag) => Some(format!("refs/tags/{tag}")),
    }
}

fn to_remote_branch<'a>(parsed_ref: &'a RefName, remote_name: &str) -> Option<&'a str> {
    match parsed_ref {
        RefName::RemoteBranch(RemoteRefSymbolBuf { name, remote }) => {
            (remote == remote_name).then_some(name)
        }
        RefName::LocalBranch(..) | RefName::Tag(..) => None,
    }
}

/// Returns true if the `parsed_ref` won't be imported because its remote name
/// is reserved.
///
/// Use this as a negative `git_ref_filter` to be passed in to
/// `import_some_refs()`.
pub fn is_reserved_git_remote_ref(parsed_ref: &RefName) -> bool {
    to_remote_branch(parsed_ref, REMOTE_NAME_FOR_LOCAL_GIT_REPO).is_some()
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
    #[error("Ancestor of Git ref {ref_name} is missing")]
    MissingRefAncestor {
        ref_name: String,
        #[source]
        err: BackendError,
    },
    #[error(
        "Git remote named '{name}' is reserved for local Git repository",
        name = REMOTE_NAME_FOR_LOCAL_GIT_REPO
    )]
    RemoteReservedForLocalGitRepo,
    #[error("Unexpected backend error when importing refs")]
    InternalBackend(#[source] BackendError),
    #[error("Unexpected git error when importing refs")]
    InternalGitError(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

impl GitImportError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        GitImportError::InternalGitError(source.into())
    }
}

/// Describes changes made by `import_refs()` or `fetch()`.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct GitImportStats {
    /// Commits superseded by newly imported commits.
    pub abandoned_commits: Vec<CommitId>,
    /// Remote `(ref_name, (old_remote_ref, new_target))`s to be merged in to
    /// the local refs.
    pub changed_remote_refs: BTreeMap<RefName, (RemoteRef, RefTarget)>,
}

#[derive(Debug)]
struct RefsToImport {
    /// Git ref `(full_name, new_target)`s to be copied to the view.
    changed_git_refs: Vec<(String, RefTarget)>,
    /// Remote `(ref_name, (old_remote_ref, new_target))`s to be merged in to
    /// the local refs.
    changed_remote_refs: BTreeMap<RefName, (RemoteRef, RefTarget)>,
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// This function detects conflicts (if both Git and JJ modified a bookmark) and
/// records them in JJ's view.
pub fn import_refs(
    mut_repo: &mut MutableRepo,
    git_settings: &GitSettings,
) -> Result<GitImportStats, GitImportError> {
    import_some_refs(mut_repo, git_settings, |_| true)
}

/// Reflect changes made in the underlying Git repo in the Jujutsu repo.
///
/// Only branches whose git full reference name pass the filter will be
/// considered for addition, update, or deletion.
pub fn import_some_refs(
    mut_repo: &mut MutableRepo,
    git_settings: &GitSettings,
    git_ref_filter: impl Fn(&RefName) -> bool,
) -> Result<GitImportStats, GitImportError> {
    let store = mut_repo.store();
    let git_backend = get_git_backend(store)?;
    let git_repo = git_backend.git_repo();

    let RefsToImport {
        changed_git_refs,
        changed_remote_refs,
    } = diff_refs_to_import(mut_repo.view(), &git_repo, git_ref_filter)?;

    // Bulk-import all reachable Git commits to the backend to reduce overhead
    // of table merging and ref updates.
    //
    // changed_remote_refs might contain new_targets that are not in
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
    for (ref_name, (_, new_target)) in &changed_remote_refs {
        for id in new_target.added_ids() {
            let commit = get_commit(id).map_err(|err| GitImportError::MissingRefAncestor {
                ref_name: ref_name.to_string(),
                err,
            })?;
            head_commits.push(commit);
        }
    }
    // It's unlikely the imported commits were missing, but I/O-related error
    // can still occur.
    mut_repo
        .add_heads(&head_commits)
        .map_err(GitImportError::InternalBackend)?;

    // Apply the change that happened in git since last time we imported refs.
    for (full_name, new_target) in changed_git_refs {
        mut_repo.set_git_ref_target(&full_name, new_target);
    }
    for (ref_name, (old_remote_ref, new_target)) in &changed_remote_refs {
        let base_target = old_remote_ref.tracking_target();
        let new_remote_ref = RemoteRef {
            target: new_target.clone(),
            state: if old_remote_ref.is_present() {
                old_remote_ref.state
            } else {
                default_remote_ref_state_for(ref_name, git_settings)
            },
        };
        match ref_name {
            RefName::LocalBranch(name) => {
                let symbol = RemoteRefSymbol {
                    name,
                    remote: REMOTE_NAME_FOR_LOCAL_GIT_REPO,
                };
                if new_remote_ref.is_tracking() {
                    mut_repo.merge_local_bookmark(symbol.name, base_target, &new_remote_ref.target);
                }
                // Update Git-tracking branch like the other remote branches.
                mut_repo.set_remote_bookmark(symbol, new_remote_ref);
            }
            RefName::RemoteBranch(symbol) => {
                let symbol = symbol.as_ref();
                if new_remote_ref.is_tracking() {
                    mut_repo.merge_local_bookmark(symbol.name, base_target, &new_remote_ref.target);
                }
                // Remote-tracking branch is the last known state of the branch in the remote.
                // It shouldn't diverge even if we had inconsistent view.
                mut_repo.set_remote_bookmark(symbol, new_remote_ref);
            }
            RefName::Tag(name) => {
                if new_remote_ref.is_tracking() {
                    mut_repo.merge_tag(name, base_target, &new_remote_ref.target);
                }
                // TODO: If we add Git-tracking tag, it will be updated here.
            }
        }
    }

    let abandoned_commits = if git_settings.abandon_unreachable_commits {
        abandon_unreachable_commits(mut_repo, &changed_remote_refs)
            .map_err(GitImportError::InternalBackend)?
    } else {
        vec![]
    };
    let stats = GitImportStats {
        abandoned_commits,
        changed_remote_refs,
    };
    Ok(stats)
}

/// Finds commits that used to be reachable in git that no longer are reachable.
/// Those commits will be recorded as abandoned in the `MutableRepo`.
fn abandon_unreachable_commits(
    mut_repo: &mut MutableRepo,
    changed_remote_refs: &BTreeMap<RefName, (RemoteRef, RefTarget)>,
) -> BackendResult<Vec<CommitId>> {
    let hidable_git_heads = changed_remote_refs
        .values()
        .flat_map(|(old_remote_ref, _)| old_remote_ref.target.added_ids())
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
        .map_err(|err| err.expect_backend_error())?
        .iter()
        .try_collect()
        .map_err(|err| err.expect_backend_error())?;
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
    git_ref_filter: impl Fn(&RefName) -> bool,
) -> Result<RefsToImport, GitImportError> {
    let mut known_git_refs: HashMap<&str, &RefTarget> = view
        .git_refs()
        .iter()
        .filter_map(|(full_name, target)| {
            // TODO: or clean up invalid ref in case it was stored due to historical bug?
            let ref_name = parse_git_ref(full_name).expect("stored git ref should be parsable");
            git_ref_filter(&ref_name).then_some((full_name.as_ref(), target))
        })
        .collect();
    // TODO: migrate tags to the remote view, and don't destructure &RemoteRef
    let mut known_remote_refs: HashMap<RefName, (&RefTarget, RemoteRefState)> = itertools::chain(
        view.all_remote_bookmarks().map(|(symbol, remote_ref)| {
            // TODO: want to abstract local ref as "git" tracking remote, but
            // we'll probably need to refactor the git_ref_filter API first.
            let ref_name = if symbol.remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO {
                RefName::LocalBranch(symbol.name.to_owned())
            } else {
                RefName::RemoteBranch(symbol.to_owned())
            };
            let RemoteRef { target, state } = remote_ref;
            (ref_name, (target, *state))
        }),
        // TODO: compare to tags stored in the "git" remote view. Since tags should never
        // be moved locally in jj, we can consider local tags as merge base.
        view.tags().iter().map(|(name, target)| {
            let ref_name = RefName::Tag(name.to_owned());
            (ref_name, (target, RemoteRefState::Tracking))
        }),
    )
    .filter(|(ref_name, _)| git_ref_filter(ref_name))
    .collect();

    let mut changed_git_refs = Vec::new();
    let mut changed_remote_refs = BTreeMap::new();
    let git_references = git_repo.references().map_err(GitImportError::from_git)?;
    let chain_git_refs_iters = || -> Result<_, gix::reference::iter::init::Error> {
        // Exclude uninteresting directories such as refs/jj/keep.
        Ok(itertools::chain!(
            git_references.local_branches()?,
            git_references.remote_branches()?,
            git_references.tags()?,
        ))
    };
    for git_ref in chain_git_refs_iters().map_err(GitImportError::from_git)? {
        let git_ref = git_ref.map_err(GitImportError::from_git)?;
        let Ok(full_name) = str::from_utf8(git_ref.name().as_bstr()) else {
            // Skip non-utf8 refs.
            continue;
        };
        let Some(ref_name) = parse_git_ref(full_name) else {
            // Skip other refs (such as notes) and symbolic refs.
            continue;
        };
        if !git_ref_filter(&ref_name) {
            continue;
        }
        if is_reserved_git_remote_ref(&ref_name) {
            return Err(GitImportError::RemoteReservedForLocalGitRepo);
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
            .remove(&ref_name)
            .unwrap_or_else(|| (RefTarget::absent_ref(), RemoteRefState::New));
        if new_target != *old_remote_target {
            let old_remote_ref = RemoteRef {
                target: old_remote_target.clone(),
                state: old_remote_state,
            };
            changed_remote_refs.insert(ref_name, (old_remote_ref, new_target));
        }
    }
    for full_name in known_git_refs.into_keys() {
        changed_git_refs.push((full_name.to_owned(), RefTarget::absent()));
    }
    for (ref_name, (old_target, old_state)) in known_remote_refs {
        let old_remote_ref = RemoteRef {
            target: old_target.clone(),
            state: old_state,
        };
        changed_remote_refs.insert(ref_name, (old_remote_ref, RefTarget::absent()));
    }
    Ok(RefsToImport {
        changed_git_refs,
        changed_remote_refs,
    })
}

fn default_remote_ref_state_for(ref_name: &RefName, git_settings: &GitSettings) -> RemoteRefState {
    match ref_name {
        // LocalBranch means Git-tracking branch
        RefName::LocalBranch(_) | RefName::Tag(_) => RemoteRefState::Tracking,
        RefName::RemoteBranch(_) => {
            if git_settings.auto_local_bookmark {
                RemoteRefState::Tracking
            } else {
                RemoteRefState::New
            }
        }
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
        .filter(|(_, remote_ref)| !remote_ref.is_tracking())
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
            .map_err(GitImportError::InternalBackend)?;
    }

    mut_repo.set_git_head_target(RefTarget::resolved(new_git_head_id));
    Ok(())
}

#[derive(Error, Debug)]
pub enum GitExportError {
    #[error("Git error")]
    InternalGitError(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
    #[error(transparent)]
    Backend(#[from] BackendError),
}

impl GitExportError {
    fn from_git(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        GitExportError::InternalGitError(source.into())
    }
}

/// A ref we failed to export to Git, along with the reason it failed.
#[derive(Debug)]
pub struct FailedRefExport {
    pub name: RefName,
    pub reason: FailedRefExportReason,
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
    /// The branch points to the root commit, which Git doesn't have
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

#[derive(Debug)]
struct RefsToExport {
    branches_to_update: BTreeMap<RefName, (Option<gix::ObjectId>, gix::ObjectId)>,
    branches_to_delete: BTreeMap<RefName, gix::ObjectId>,
    failed_branches: HashMap<RefName, FailedRefExportReason>,
}

/// Export changes to branches made in the Jujutsu repo compared to our last
/// seen view of the Git repo in `mut_repo.view().git_refs()`. Returns a list of
/// refs that failed to export.
///
/// We ignore changed branches that are conflicted (were also changed in the Git
/// repo compared to our last remembered view of the Git repo). These will be
/// marked conflicted by the next `jj git import`.
///
/// We do not export tags and other refs at the moment, since these aren't
/// supposed to be modified by JJ. For them, the Git state is considered
/// authoritative.
pub fn export_refs(mut_repo: &mut MutableRepo) -> Result<Vec<FailedRefExport>, GitExportError> {
    export_some_refs(mut_repo, |_| true)
}

pub fn export_some_refs(
    mut_repo: &mut MutableRepo,
    git_ref_filter: impl Fn(&RefName) -> bool,
) -> Result<Vec<FailedRefExport>, GitExportError> {
    let git_repo = get_git_repo(mut_repo.store())?;

    let RefsToExport {
        branches_to_update,
        branches_to_delete,
        mut failed_branches,
    } = diff_refs_to_export(
        mut_repo.view(),
        mut_repo.store().root_commit_id(),
        &git_ref_filter,
    );

    // TODO: Also check other worktrees' HEAD.
    if let Ok(head_ref) = git_repo.find_reference("HEAD") {
        if let Some(parsed_ref) = head_ref
            .target()
            .try_name()
            .and_then(|name| str::from_utf8(name.as_bstr()).ok())
            .and_then(parse_git_ref)
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
            let new_oid = if let Some((_old_oid, new_oid)) = branches_to_update.get(&parsed_ref) {
                Some(new_oid)
            } else if branches_to_delete.contains_key(&parsed_ref) {
                None
            } else {
                current_oid.as_ref()
            };
            if new_oid != current_oid.as_ref() {
                update_git_head(
                    &git_repo,
                    gix::refs::transaction::PreviousValue::MustExistAndMatch(old_target),
                    current_oid,
                )?;
            }
        }
    }
    for (parsed_ref_name, old_oid) in branches_to_delete {
        let Some(git_ref_name) = to_git_ref_name(&parsed_ref_name) else {
            failed_branches.insert(parsed_ref_name, FailedRefExportReason::InvalidGitName);
            continue;
        };
        if let Err(reason) = delete_git_ref(&git_repo, &git_ref_name, &old_oid) {
            failed_branches.insert(parsed_ref_name, reason);
        } else {
            let new_target = RefTarget::absent();
            mut_repo.set_git_ref_target(&git_ref_name, new_target);
        }
    }
    for (parsed_ref_name, (old_oid, new_oid)) in branches_to_update {
        let Some(git_ref_name) = to_git_ref_name(&parsed_ref_name) else {
            failed_branches.insert(parsed_ref_name, FailedRefExportReason::InvalidGitName);
            continue;
        };
        if let Err(reason) = update_git_ref(&git_repo, &git_ref_name, old_oid, new_oid) {
            failed_branches.insert(parsed_ref_name, reason);
        } else {
            let new_target = RefTarget::normal(CommitId::from_bytes(new_oid.as_bytes()));
            mut_repo.set_git_ref_target(&git_ref_name, new_target);
        }
    }

    copy_exportable_local_branches_to_remote_view(
        mut_repo,
        REMOTE_NAME_FOR_LOCAL_GIT_REPO,
        |ref_name| git_ref_filter(ref_name) && !failed_branches.contains_key(ref_name),
    );

    let failed_branches = failed_branches
        .into_iter()
        .map(|(name, reason)| FailedRefExport { name, reason })
        .sorted_unstable_by(|a, b| a.name.cmp(&b.name))
        .collect();
    Ok(failed_branches)
}

fn copy_exportable_local_branches_to_remote_view(
    mut_repo: &mut MutableRepo,
    remote: &str,
    git_ref_filter: impl Fn(&RefName) -> bool,
) {
    let new_local_branches = mut_repo
        .view()
        .local_remote_bookmarks(remote)
        .filter_map(|(branch, targets)| {
            // TODO: filter out untracked branches (if we add support for untracked @git
            // branches)
            let old_target = &targets.remote_ref.target;
            let new_target = targets.local_target;
            (!new_target.has_conflict() && old_target != new_target).then_some((branch, new_target))
        })
        .filter(|&(branch, _)| git_ref_filter(&RefName::LocalBranch(branch.to_owned())))
        .map(|(branch, new_target)| (branch.to_owned(), new_target.clone()))
        .collect_vec();
    for (ref name, new_target) in new_local_branches {
        let symbol = RemoteRefSymbol { name, remote };
        let new_remote_ref = RemoteRef {
            target: new_target,
            state: RemoteRefState::Tracking,
        };
        mut_repo.set_remote_bookmark(symbol, new_remote_ref);
    }
}

/// Calculates diff of branches to be exported.
fn diff_refs_to_export(
    view: &View,
    root_commit_id: &CommitId,
    git_ref_filter: impl Fn(&RefName) -> bool,
) -> RefsToExport {
    // Local targets will be copied to the "git" remote if successfully exported. So
    // the local branches are considered to be the new "git" remote branches.
    let mut all_branch_targets: HashMap<RefName, (&RefTarget, &RefTarget)> = itertools::chain(
        view.local_bookmarks()
            .map(|(branch, target)| (RefName::LocalBranch(branch.to_owned()), target)),
        view.all_remote_bookmarks()
            .filter(|&(symbol, _)| symbol.remote != REMOTE_NAME_FOR_LOCAL_GIT_REPO)
            .map(|(symbol, remote_ref)| {
                let ref_name = RefName::RemoteBranch(symbol.to_owned());
                (ref_name, &remote_ref.target)
            }),
    )
    .map(|(ref_name, new_target)| (ref_name, (RefTarget::absent_ref(), new_target)))
    .filter(|(ref_name, _)| git_ref_filter(ref_name))
    .collect();
    let known_git_refs = view
        .git_refs()
        .iter()
        .map(|(full_name, target)| {
            let ref_name = parse_git_ref(full_name).expect("stored git ref should be parsable");
            (ref_name, target)
        })
        .filter(|(ref_name, _)| {
            // There are two situations where remote-tracking branches get out of sync:
            // 1. `jj branch forget`
            // 2. `jj op undo`/`restore` in colocated repo
            matches!(ref_name, RefName::LocalBranch(_) | RefName::RemoteBranch(_))
        })
        .filter(|(ref_name, _)| git_ref_filter(ref_name));
    for (ref_name, target) in known_git_refs {
        all_branch_targets
            .entry(ref_name)
            .and_modify(|(old_target, _)| *old_target = target)
            .or_insert((target, RefTarget::absent_ref()));
    }

    let mut branches_to_update = BTreeMap::new();
    let mut branches_to_delete = BTreeMap::new();
    let mut failed_branches = HashMap::new();
    let root_commit_target = RefTarget::normal(root_commit_id.clone());
    for (ref_name, (old_target, new_target)) in all_branch_targets {
        if new_target == old_target {
            continue;
        }
        if *new_target == root_commit_target {
            // Git doesn't have a root commit
            failed_branches.insert(ref_name, FailedRefExportReason::OnRootCommit);
            continue;
        }
        let old_oid = if let Some(id) = old_target.as_normal() {
            Some(gix::ObjectId::from_bytes_or_panic(id.as_bytes()))
        } else if old_target.has_conflict() {
            // The old git ref should only be a conflict if there were concurrent import
            // operations while the value changed. Don't overwrite these values.
            failed_branches.insert(ref_name, FailedRefExportReason::ConflictedOldState);
            continue;
        } else {
            assert!(old_target.is_absent());
            None
        };
        if let Some(id) = new_target.as_normal() {
            let new_oid = gix::ObjectId::from_bytes_or_panic(id.as_bytes());
            branches_to_update.insert(ref_name, (old_oid, new_oid));
        } else if new_target.has_conflict() {
            // Skip conflicts and leave the old value in git_refs
            continue;
        } else {
            assert!(new_target.is_absent());
            branches_to_delete.insert(ref_name, old_oid.unwrap());
        }
    }

    RefsToExport {
        branches_to_update,
        branches_to_delete,
        failed_branches,
    }
}

fn delete_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &str,
    old_oid: &gix::oid,
) -> Result<(), FailedRefExportReason> {
    if let Ok(git_ref) = git_repo.find_reference(git_ref_name) {
        if git_ref.inner.target.try_id() == Some(old_oid) {
            // The branch has not been updated by git, so go ahead and delete it
            git_ref
                .delete()
                .map_err(|err| FailedRefExportReason::FailedToDelete(err.into()))?;
        } else {
            // The branch was updated by git
            return Err(FailedRefExportReason::DeletedInJjModifiedInGit);
        }
    } else {
        // The branch is already deleted
    }
    Ok(())
}

fn update_git_ref(
    git_repo: &gix::Repository,
    git_ref_name: &str,
    old_oid: Option<gix::ObjectId>,
    new_oid: gix::ObjectId,
) -> Result<(), FailedRefExportReason> {
    match old_oid {
        None => {
            if let Ok(git_repo_ref) = git_repo.find_reference(git_ref_name) {
                // The branch was added in jj and in git. We're good if and only if git
                // pointed it to our desired target.
                if git_repo_ref.inner.target.try_id() != Some(&new_oid) {
                    return Err(FailedRefExportReason::AddedInJjAddedInGit);
                }
            } else {
                // The branch was added in jj but still doesn't exist in git, so add it
                git_repo
                    .reference(
                        git_ref_name,
                        new_oid,
                        gix::refs::transaction::PreviousValue::MustNotExist,
                        "export from jj",
                    )
                    .map_err(|err| FailedRefExportReason::FailedToSet(err.into()))?;
            }
        }
        Some(old_oid) => {
            // The branch was modified in jj. We can use gix API for updating under a lock.
            if let Err(err) = git_repo.reference(
                git_ref_name,
                new_oid,
                gix::refs::transaction::PreviousValue::MustExistAndMatch(old_oid.into()),
                "export from jj",
            ) {
                // The reference was probably updated in git
                if let Ok(git_repo_ref) = git_repo.find_reference(git_ref_name) {
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
) -> Result<(), GitExportError> {
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
    git_repo
        .edit_references(ref_edits)
        .map_err(GitExportError::from_git)?;
    Ok(())
}

/// Sets Git HEAD to the parent of the given working-copy commit and resets
/// the Git index.
pub fn reset_head(mut_repo: &mut MutableRepo, wc_commit: &Commit) -> Result<(), GitExportError> {
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
            let actual_head = git_repo.head().map_err(GitExportError::from_git)?;
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
        update_git_head(&git_repo, expected_ref, new_oid)?;
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
            _ => Err(GitExportError::from_git(err)),
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
                .map_err(GitExportError::from_git)?
        }
    } else {
        build_index_from_merged_tree(&git_repo, parent_tree)?
    };

    // Match entries in the new index with entries in the old index, and copy stat
    // information if the entry didn't change.
    if let Some(old_index) = git_repo.try_index().map_err(GitExportError::from_git)? {
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
        .map_err(GitExportError::from_git)?;

    Ok(())
}

fn build_index_from_merged_tree(
    git_repo: &gix::Repository,
    merged_tree: MergedTree,
) -> Result<gix::index::File, GitExportError> {
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
            .map_err(GitExportError::from_git)?;
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

#[derive(Debug, Error)]
pub enum GitRemoteManagementError {
    #[error("No git remote named '{0}'")]
    NoSuchRemote(String),
    #[error("Git remote named '{0}' already exists")]
    RemoteAlreadyExists(String),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error("Git remote named '{0}' has nonstandard configuration")]
    NonstandardConfiguration(String),
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

fn is_remote_not_found_err(err: &git2::Error) -> bool {
    matches!(
        (err.class(), err.code()),
        (
            git2::ErrorClass::Config,
            git2::ErrorCode::NotFound | git2::ErrorCode::InvalidSpec
        )
    )
}

/// Determine, by its name, if a remote refers to the special local-only "git"
/// remote that is used in the Git backend.
///
/// This function always returns false if the "git" feature is not enabled.
pub fn is_special_git_remote(remote: &str) -> bool {
    remote == REMOTE_NAME_FOR_LOCAL_GIT_REPO
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

fn git_config_branch_section_ids_by_remote(
    config: &gix::config::File,
    remote_name: &str,
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
                .any(|branch_remote_name| **branch_remote_name == remote_name.as_bytes())
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
    old_remote_name: &str,
    new_remote_name: &str,
) -> Result<(), GitRemoteManagementError> {
    for id in git_config_branch_section_ids_by_remote(config, old_remote_name)? {
        config
            .section_mut_by_id(id)
            .expect("found section to exist")
            .set(
                "remote"
                    .try_into()
                    .expect("'remote' to be a valid value name"),
                BStr::new(new_remote_name),
            );
    }
    Ok(())
}

fn remove_remote_git_branch_config_sections(
    config: &mut gix::config::File,
    remote_name: &str,
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
    remote_name: &str,
) -> Result<(), GitRemoteManagementError> {
    let section_ids_to_remove = config
        .sections_by_name("remote")
        .into_iter()
        .flatten()
        .filter(|section| section.header().subsection_name() == Some(BStr::new(remote_name)))
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
        .collect::<Result<Vec<_>, _>>()?
        .into_iter();
    for id in section_ids_to_remove {
        config
            .remove_section_by_id(id)
            .expect("removed section to exist");
    }
    Ok(())
}

/// Returns a sorted list of configured remote names.
pub fn get_all_remote_names(store: &Store) -> Result<Vec<String>, UnexpectedGitBackendError> {
    let git_repo = get_git_repo(store)?;
    let names = git_repo
        .remote_names()
        .into_iter()
        // exclude empty [remote "<name>"] section
        .filter(|name| git_repo.try_find_remote(name.as_ref()).is_some())
        // ignore non-UTF-8 remote names which we don't support
        .filter_map(|name| String::from_utf8(name.into_owned().into()).ok())
        .collect();
    Ok(names)
}

pub fn add_remote(
    store: &Store,
    remote_name: &str,
    url: &str,
) -> Result<(), GitRemoteManagementError> {
    let git_repo = get_git_repo(store)?;

    validate_remote_name(remote_name)?;

    if git_repo.try_find_remote(remote_name).is_some() {
        return Err(GitRemoteManagementError::RemoteAlreadyExists(
            remote_name.to_owned(),
        ));
    }

    let mut remote = git_repo
        .remote_at(url)
        .map_err(GitRemoteManagementError::from_git)?
        .with_refspecs(
            [format!("+refs/heads/*:refs/remotes/{remote_name}/*").as_bytes()],
            gix::remote::Direction::Fetch,
        )
        .expect("default refspec to be valid");

    let mut config = git_repo.config_snapshot().clone();
    remote
        .save_as_to(remote_name, &mut config)
        .map_err(GitRemoteManagementError::from_git)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    Ok(())
}

pub fn remove_remote(
    mut_repo: &mut MutableRepo,
    remote_name: &str,
) -> Result<(), GitRemoteManagementError> {
    let mut git_repo = get_git_repo(mut_repo.store())?;

    if git_repo.try_find_remote(remote_name).is_none() {
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
    remote_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    git_repo.edit_references(
        git_repo
            .references()?
            .prefixed(format!("refs/remotes/{remote_name}/"))?
            .map_ok(remove_ref)
            .collect::<Result<Vec<_>, _>>()?,
    )?;
    Ok(())
}

fn remove_remote_refs(mut_repo: &mut MutableRepo, remote_name: &str) {
    mut_repo.remove_remote(remote_name);
    let prefix = format!("refs/remotes/{remote_name}/");
    let git_refs_to_delete = mut_repo
        .view()
        .git_refs()
        .keys()
        .filter(|&r| r.starts_with(&prefix))
        .cloned()
        .collect_vec();
    for git_ref in git_refs_to_delete {
        mut_repo.set_git_ref_target(&git_ref, RefTarget::absent());
    }
}

pub fn rename_remote(
    mut_repo: &mut MutableRepo,
    old_remote_name: &str,
    new_remote_name: &str,
) -> Result<(), GitRemoteManagementError> {
    let mut git_repo = get_git_repo(mut_repo.store())?;

    validate_remote_name(new_remote_name)?;

    let Some(result) = git_repo.try_find_remote(old_remote_name) else {
        return Err(GitRemoteManagementError::NoSuchRemote(
            old_remote_name.to_owned(),
        ));
    };
    let mut remote = result.map_err(GitRemoteManagementError::from_git)?;

    if git_repo.try_find_remote(new_remote_name).is_some() {
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
                == format!("+refs/heads/*:refs/remotes/{old_remote_name}/*").as_bytes() => {}
        _ => {
            return Err(GitRemoteManagementError::NonstandardConfiguration(
                old_remote_name.to_owned(),
            ))
        }
    }

    remote
        .replace_refspecs(
            [format!("+refs/heads/*:refs/remotes/{new_remote_name}/*").as_bytes()],
            gix::remote::Direction::Fetch,
        )
        .expect("default refspec to be valid");

    let mut config = git_repo.config_snapshot().clone();
    remote
        .save_as_to(new_remote_name, &mut config)
        .map_err(GitRemoteManagementError::from_git)?;
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
    old_remote_name: &str,
    new_remote_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let old_prefix = format!("refs/remotes/{old_remote_name}/");
    let new_prefix = format!("refs/remotes/{new_remote_name}/");
    let ref_log_message = BString::from(format!(
        "renamed remote {old_remote_name} to {new_remote_name}"
    ));

    git_repo.edit_references(
        git_repo
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
            .collect::<Result<Vec<_>, _>>()?,
    )?;
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
    remote_name: &str,
    new_remote_url: &str,
) -> Result<(), GitRemoteManagementError> {
    let git_repo = get_git_repo(store)?;

    validate_remote_name(remote_name)?;

    let Some(result) = git_repo.try_find_remote_without_url_rewrite(remote_name) else {
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
    remote
        .save_as_to(remote_name, &mut config)
        .map_err(GitRemoteManagementError::from_git)?;
    save_git_config(&config).map_err(GitRemoteManagementError::GitConfigSaveError)?;

    Ok(())
}

fn rename_remote_refs(mut_repo: &mut MutableRepo, old_remote_name: &str, new_remote_name: &str) {
    mut_repo.rename_remote(old_remote_name, new_remote_name);
    let prefix = format!("refs/remotes/{old_remote_name}/");
    let git_refs = mut_repo
        .view()
        .git_refs()
        .iter()
        .filter_map(|(r, target)| {
            r.strip_prefix(&prefix).map(|p| {
                (
                    r.clone(),
                    format!("refs/remotes/{new_remote_name}/{p}"),
                    target.clone(),
                )
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
    #[error("No git remote named '{0}'")]
    NoSuchRemote(String),
    #[error(
        "Invalid branch pattern provided. When fetching, branch names and globs may not contain the characters `{chars}`",
        chars = INVALID_REFSPEC_CHARS.iter().join("`, `")
    )]
    InvalidBranchPattern(StringPattern),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    // TODO: I'm sure there are other errors possible, such as transport-level errors.
    #[error("Unexpected git error when fetching")]
    InternalGitError(#[from] git2::Error),
    #[error(transparent)]
    Subprocess(#[from] GitSubprocessError),
}

// TODO: If Git2 implementation is removed, this can be replaced with
// UnexpectedGitBackendError.
#[derive(Debug, Error)]
pub enum GitFetchPrepareError {
    #[error(transparent)]
    Git2(#[from] git2::Error),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

fn git2_fetch_options(
    mut callbacks: RemoteCallbacks<'_>,
    depth: Option<NonZeroU32>,
) -> git2::FetchOptions<'_> {
    let mut proxy_options = git2::ProxyOptions::new();
    proxy_options.auto();

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.proxy_options(proxy_options);
    // git2 doesn't provide API to set "no-progress" protocol option. If
    // sideband callback were enabled, remote progress messages would be written
    // no matter if the process was attached to a tty or not.
    if callbacks.progress.is_none() {
        callbacks.sideband_progress = None;
    }
    fetch_options.remote_callbacks(callbacks.into_git());
    if let Some(depth) = depth {
        fetch_options.depth(depth.get().try_into().unwrap_or(i32::MAX));
    }

    fetch_options
}

struct FetchedBranches {
    remote: String,
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
        remote_name: &str,
        branch_names: &[StringPattern],
        callbacks: RemoteCallbacks<'_>,
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitFetchError> {
        validate_remote_name(remote_name)?;
        self.fetch_impl
            .fetch(remote_name, branch_names, callbacks, depth)?;
        self.fetched.push(FetchedBranches {
            remote: remote_name.to_string(),
            branches: branch_names.to_vec(),
        });
        Ok(())
    }

    /// Queries remote for the default branch name.
    #[tracing::instrument(skip(self, callbacks))]
    pub fn get_default_branch(
        &self,
        remote_name: &str,
        callbacks: RemoteCallbacks<'_>,
    ) -> Result<Option<String>, GitFetchError> {
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
                |ref_name| match ref_name {
                    RefName::LocalBranch(_) => false,
                    RefName::Tag(_) => true,
                    RefName::RemoteBranch(symbol) => self.fetched.iter().any(|fetched| {
                        if fetched.remote != symbol.remote {
                            return false;
                        }

                        fetched
                            .branches
                            .iter()
                            .any(|pattern| pattern.matches(&symbol.name))
                    }),
                },
            )?;

        self.fetched.clear();

        Ok(import_stats)
    }
}

fn expand_fetch_refspecs(
    remote_name: &str,
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
                        format!("refs/remotes/{remote_name}/{glob}"),
                    )
                })
                .ok_or_else(|| GitFetchError::InvalidBranchPattern(pattern.clone()))
        })
        .collect()
}

enum GitFetchImpl<'a> {
    Git2 {
        git_repo: git2::Repository,
    },
    Subprocess {
        git_repo: Box<gix::Repository>,
        git_ctx: GitSubprocessContext<'a>,
    },
}

impl<'a> GitFetchImpl<'a> {
    fn new(store: &Store, git_settings: &'a GitSettings) -> Result<Self, GitFetchPrepareError> {
        let git_backend = get_git_backend(store)?;
        if git_settings.subprocess {
            let git_repo = Box::new(git_backend.git_repo());
            let git_ctx =
                GitSubprocessContext::from_git_backend(git_backend, &git_settings.executable_path);
            Ok(GitFetchImpl::Subprocess { git_repo, git_ctx })
        } else {
            let git_repo = git_backend.open_git_repo()?;
            Ok(GitFetchImpl::Git2 { git_repo })
        }
    }

    fn fetch(
        &self,
        remote_name: &str,
        branch_names: &[StringPattern],
        callbacks: RemoteCallbacks<'_>,
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitFetchError> {
        match self {
            GitFetchImpl::Git2 { git_repo } => {
                git2_fetch(git_repo, remote_name, branch_names, callbacks, depth)
            }
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
        remote_name: &str,
        callbacks: RemoteCallbacks<'_>,
    ) -> Result<Option<String>, GitFetchError> {
        match self {
            GitFetchImpl::Git2 { git_repo } => {
                git2_get_default_branch(git_repo, remote_name, callbacks)
            }
            GitFetchImpl::Subprocess { git_repo, git_ctx } => {
                subprocess_get_default_branch(git_repo, git_ctx, remote_name, callbacks)
            }
        }
    }
}

fn git2_fetch(
    git_repo: &git2::Repository,
    remote_name: &str,
    branch_names: &[StringPattern],
    callbacks: RemoteCallbacks<'_>,
    depth: Option<NonZeroU32>,
) -> Result<(), GitFetchError> {
    let mut remote = git_repo.find_remote(remote_name).map_err(|err| {
        if is_remote_not_found_err(&err) {
            GitFetchError::NoSuchRemote(remote_name.to_string())
        } else {
            GitFetchError::InternalGitError(err)
        }
    })?;
    // At this point, we are only updating Git's remote tracking branches, not the
    // local branches.
    let refspecs: Vec<String> = expand_fetch_refspecs(remote_name, branch_names)?
        .iter()
        .map(|refspec| refspec.to_git_format())
        .collect();

    if refspecs.is_empty() {
        // Don't fall back to the base refspecs.
        return Ok(());
    }

    tracing::debug!("remote.download");
    remote.download(&refspecs, Some(&mut git2_fetch_options(callbacks, depth)))?;
    tracing::debug!("remote.prune");
    remote.prune(None)?;
    tracing::debug!("remote.update_tips");
    remote.update_tips(
        None,
        git2::RemoteUpdateFlags::empty(),
        git2::AutotagOption::Unspecified,
        None,
    )?;
    tracing::debug!("remote.disconnect");
    remote.disconnect()?;
    Ok(())
}

fn git2_get_default_branch(
    git_repo: &git2::Repository,
    remote_name: &str,
    callbacks: RemoteCallbacks<'_>,
) -> Result<Option<String>, GitFetchError> {
    let mut remote = git_repo.find_remote(remote_name).map_err(|err| {
        if is_remote_not_found_err(&err) {
            GitFetchError::NoSuchRemote(remote_name.to_string())
        } else {
            GitFetchError::InternalGitError(err)
        }
    })?;
    // Unlike .download(), connect_auth() returns RAII object.
    tracing::debug!("remote.connect");
    let connection = {
        let mut proxy_options = git2::ProxyOptions::new();
        proxy_options.auto();
        remote.connect_auth(
            git2::Direction::Fetch,
            Some(callbacks.into_git()),
            Some(proxy_options),
        )?
    };
    let mut default_branch = None;
    tracing::debug!("remote.default_branch");
    if let Ok(default_ref_buf) = connection.default_branch() {
        if let Some(default_ref) = default_ref_buf.as_str() {
            // LocalBranch here is the local branch on the remote, so it's really the remote
            // branch
            if let Some(RefName::LocalBranch(branch_name)) = parse_git_ref(default_ref) {
                tracing::debug!(default_branch = branch_name);
                default_branch = Some(branch_name);
            }
        }
    }
    Ok(default_branch)
}

fn subprocess_fetch(
    git_repo: &gix::Repository,
    git_ctx: &GitSubprocessContext,
    remote_name: &str,
    branch_names: &[StringPattern],
    mut callbacks: RemoteCallbacks<'_>,
    depth: Option<NonZeroU32>,
) -> Result<(), GitFetchError> {
    // check the remote exists
    if git_repo.try_find_remote(remote_name).is_none() {
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
        remaining_refspecs.retain(|r| r.source.as_ref() != Some(&failing_refspec));

        if let Some(branch_name) = failing_refspec.strip_prefix("refs/heads/") {
            branches_to_prune.push(format!("{remote_name}/{branch_name}"));
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
    remote_name: &str,
    _callbacks: RemoteCallbacks<'_>,
) -> Result<Option<String>, GitFetchError> {
    if git_repo.try_find_remote(remote_name).is_none() {
        return Err(GitFetchError::NoSuchRemote(remote_name.to_owned()));
    }
    let default_branch = git_ctx.spawn_remote_show(remote_name)?;
    tracing::debug!(default_branch = default_branch);
    Ok(default_branch)
}

#[derive(Error, Debug)]
pub enum GitPushError {
    #[error("No git remote named '{0}'")]
    NoSuchRemote(String),
    #[error(transparent)]
    RemoteName(#[from] GitRemoteNameError),
    #[error("Refs in unexpected location: {0:?}")]
    RefInUnexpectedLocation(Vec<String>),
    #[error("Remote rejected the update of some refs (do you have permission to push to {0:?}?)")]
    RefUpdateRejected(Vec<String>),
    // TODO: I'm sure there are other errors possible, such as transport-level errors,
    // and errors caused by the remote rejecting the push.
    #[error("Unexpected git error when pushing")]
    InternalGitError(#[from] git2::Error),
    #[error(transparent)]
    Subprocess(#[from] GitSubprocessError),
    #[error(transparent)]
    UnexpectedBackend(#[from] UnexpectedGitBackendError),
}

#[derive(Clone, Debug)]
pub struct GitBranchPushTargets {
    pub branch_updates: Vec<(String, BookmarkPushUpdate)>,
}

pub struct GitRefUpdate {
    pub qualified_name: String,
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
    remote: &str,
    targets: &GitBranchPushTargets,
    callbacks: RemoteCallbacks<'_>,
) -> Result<(), GitPushError> {
    validate_remote_name(remote)?;

    let ref_updates = targets
        .branch_updates
        .iter()
        .map(|(name, update)| GitRefUpdate {
            qualified_name: format!("refs/heads/{name}"),
            expected_current_target: update.old_target.clone(),
            new_target: update.new_target.clone(),
        })
        .collect_vec();
    push_updates(mut_repo, git_settings, remote, &ref_updates, callbacks)?;

    // TODO: add support for partially pushed refs? we could update the view
    // excluding rejected refs, but the transaction would be aborted anyway
    // if we returned an Err.
    for (name, update) in &targets.branch_updates {
        let remote_symbol = RemoteRefSymbol { name, remote };
        let git_ref_name = format!("refs/remotes/{remote}/{name}");
        let new_remote_ref = RemoteRef {
            target: RefTarget::resolved(update.new_target.clone()),
            state: RemoteRefState::Tracking,
        };
        mut_repo.set_git_ref_target(&git_ref_name, new_remote_ref.target.clone());
        mut_repo.set_remote_bookmark(remote_symbol, new_remote_ref);
    }

    Ok(())
}

/// Pushes the specified Git refs without updating the repo view.
pub fn push_updates(
    repo: &dyn Repo,
    git_settings: &GitSettings,
    remote_name: &str,
    updates: &[GitRefUpdate],
    callbacks: RemoteCallbacks<'_>,
) -> Result<(), GitPushError> {
    let mut qualified_remote_refs_expected_locations = HashMap::new();
    let mut refspecs = vec![];
    for update in updates {
        qualified_remote_refs_expected_locations.insert(
            update.qualified_name.as_str(),
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
    if git_settings.subprocess {
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
    } else {
        let git_repo = git_backend.open_git_repo()?;
        let refspecs: Vec<String> = refspecs.iter().map(RefSpec::to_git_format).collect();
        git2_push_refs(
            repo,
            &git_repo,
            remote_name,
            &qualified_remote_refs_expected_locations,
            &refspecs,
            callbacks,
        )
    }
}

fn git2_push_refs(
    repo: &dyn Repo,
    git_repo: &git2::Repository,
    remote_name: &str,
    qualified_remote_refs_expected_locations: &HashMap<&str, Option<&CommitId>>,
    refspecs: &[String],
    callbacks: RemoteCallbacks<'_>,
) -> Result<(), GitPushError> {
    let mut remote = git_repo.find_remote(remote_name).map_err(|err| {
        if is_remote_not_found_err(&err) {
            GitPushError::NoSuchRemote(remote_name.to_string())
        } else {
            GitPushError::InternalGitError(err)
        }
    })?;
    let mut remaining_remote_refs: HashSet<_> = qualified_remote_refs_expected_locations
        .keys()
        .copied()
        .collect();
    let mut failed_push_negotiations = vec![];
    let push_result = {
        let mut push_options = git2::PushOptions::new();
        let mut proxy_options = git2::ProxyOptions::new();
        proxy_options.auto();
        push_options.proxy_options(proxy_options);
        let mut callbacks = callbacks.into_git();
        callbacks.push_negotiation(|updates| {
            for update in updates {
                let dst_refname = update
                    .dst_refname()
                    .expect("Expect reference name to be valid UTF-8");
                let expected_remote_location = *qualified_remote_refs_expected_locations
                    .get(dst_refname)
                    .expect("Push is trying to move a ref it wasn't asked to move");
                let oid_to_maybe_commitid =
                    |oid: git2::Oid| (!oid.is_zero()).then(|| CommitId::from_bytes(oid.as_bytes()));
                let actual_remote_location = oid_to_maybe_commitid(update.src());
                let local_location = oid_to_maybe_commitid(update.dst());

                match allow_push(
                    repo.index(),
                    actual_remote_location.as_ref(),
                    expected_remote_location,
                    local_location.as_ref(),
                ) {
                    Ok(PushAllowReason::NormalMatch) => {}
                    Ok(PushAllowReason::UnexpectedNoop) => {
                        tracing::info!(
                            "The push of {dst_refname} is unexpectedly a no-op, the remote branch \
                             is already at {actual_remote_location:?}. We expected it to be at \
                             {expected_remote_location:?}. We don't consider this an error.",
                        );
                    }
                    Ok(PushAllowReason::ExceptionalFastforward) => {
                        // TODO(ilyagr): We could consider printing a user-facing message at
                        // this point.
                        tracing::info!(
                            "We allow the push of {dst_refname} to {local_location:?}, even \
                             though it is unexpectedly at {actual_remote_location:?} on the \
                             server rather than the expected {expected_remote_location:?}. The \
                             desired location is a descendant of the actual location, and the \
                             actual location is a descendant of the expected location.",
                        );
                    }
                    Err(()) => {
                        // While we show debug info in the message with `--debug`,
                        // there's probably no need to show the detailed commit
                        // locations to the user normally. They should do a `jj git
                        // fetch`, and the resulting branch conflicts should contain
                        // all the information they need.
                        tracing::info!(
                            "Cannot push {dst_refname} to {local_location:?}; it is at \
                             unexpectedly at {actual_remote_location:?} on the server as opposed \
                             to the expected {expected_remote_location:?}",
                        );
                        failed_push_negotiations.push(dst_refname.to_string());
                    }
                }
            }
            if failed_push_negotiations.is_empty() {
                Ok(())
            } else {
                Err(git2::Error::from_str("failed push negotiation"))
            }
        });
        callbacks.push_update_reference(|refname, status| {
            // The status is Some if the ref update was rejected
            if status.is_none() {
                remaining_remote_refs.remove(refname);
            }
            Ok(())
        });
        push_options.remote_callbacks(callbacks);
        remote.push(refspecs, Some(&mut push_options))
    };
    if !failed_push_negotiations.is_empty() {
        // If the push negotiation returned an error, `remote.push` would not
        // have pushed anything and would have returned an error, as expected.
        // However, the error it returns is not necessarily the error we'd
        // expect. It also depends on the exact versions of `libgit2` and
        // `git2.rs`. So, we cannot rely on it containing any useful
        // information. See https://github.com/rust-lang/git2-rs/issues/1042.
        assert!(push_result.is_err());
        failed_push_negotiations.sort();
        Err(GitPushError::RefInUnexpectedLocation(
            failed_push_negotiations,
        ))
    } else {
        push_result?;
        if remaining_remote_refs.is_empty() {
            Ok(())
        } else {
            // TODO: this is probably dead code right now
            // The only way this would happen is if a push fails from some
            // other reason other than a lease failing (which our tests don't cover)
            Err(GitPushError::RefUpdateRejected(
                remaining_remote_refs
                    .iter()
                    .sorted()
                    .map(|name| name.to_string())
                    .collect(),
            ))
        }
    }
}

fn subprocess_push_refs(
    git_repo: &gix::Repository,
    git_ctx: &GitSubprocessContext,
    remote_name: &str,
    qualified_remote_refs_expected_locations: &HashMap<&str, Option<&CommitId>>,
    refspecs: &[RefSpec],
    mut callbacks: RemoteCallbacks<'_>,
) -> Result<(), GitPushError> {
    // check the remote exists
    if git_repo.try_find_remote(remote_name).is_none() {
        return Err(GitPushError::NoSuchRemote(remote_name.to_owned()));
    }

    let mut remaining_remote_refs: HashSet<_> = qualified_remote_refs_expected_locations
        .keys()
        .copied()
        .collect();

    let refs_to_push: Vec<RefToPush> = refspecs
        .iter()
        .map(|full_refspec| RefToPush::new(full_refspec, qualified_remote_refs_expected_locations))
        .collect();

    let (failed_ref_matches, successful_pushes) =
        git_ctx.spawn_push(remote_name, &refs_to_push, &mut callbacks)?;

    for remote_ref in successful_pushes {
        remaining_remote_refs.remove(remote_ref.as_str());
    }

    if !failed_ref_matches.is_empty() {
        let mut refs_in_unexpected_locations = failed_ref_matches;
        refs_in_unexpected_locations.sort();
        Err(GitPushError::RefInUnexpectedLocation(
            refs_in_unexpected_locations,
        ))
    } else if remaining_remote_refs.is_empty() {
        Ok(())
    } else {
        // TODO: this is probably dead code right now
        // The only way this would happen is if a push fails from some
        // other reason other than a lease failing (which our tests don't cover)
        Err(GitPushError::RefUpdateRejected(
            remaining_remote_refs
                .iter()
                .sorted()
                .map(|name| name.to_string())
                .collect(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PushAllowReason {
    NormalMatch,
    ExceptionalFastforward,
    UnexpectedNoop,
}

fn allow_push(
    index: &dyn Index,
    actual_remote_location: Option<&CommitId>,
    expected_remote_location: Option<&CommitId>,
    destination_location: Option<&CommitId>,
) -> Result<PushAllowReason, ()> {
    if actual_remote_location == expected_remote_location {
        return Ok(PushAllowReason::NormalMatch);
    }

    // If the remote ref is in an unexpected location, we still allow some
    // pushes, based on whether `jj git fetch` would result in a conflicted ref.
    //
    // For `merge_ref_targets` to work correctly, `actual_remote_location` must
    // be a commit that we locally know about.
    //
    // This does not lose any generality since for `merge_ref_targets` to
    // resolve to `local_target` below, it is conceptually necessary (but not
    // sufficient) for the destination_location to be either a descendant of
    // actual_remote_location or equal to it. Either way, we would know about that
    // commit locally.
    if !actual_remote_location.is_none_or(|id| index.has_id(id)) {
        return Err(());
    }
    let remote_target = RefTarget::resolved(actual_remote_location.cloned());
    let base_target = RefTarget::resolved(expected_remote_location.cloned());
    // The push destination is the local position of the ref
    let local_target = RefTarget::resolved(destination_location.cloned());
    if refs::merge_ref_targets(index, &remote_target, &base_target, &local_target) == local_target {
        // Fetch would not change the local branch, so the push is OK in spite of
        // the discrepancy with the expected location. We return some debug info and
        // verify some invariants before OKing the push.
        Ok(if actual_remote_location == destination_location {
            // This is the situation of what we call "A - B + A = A"
            // conflicts, see also test_refs.rs and
            // https://github.com/jj-vcs/jj/blob/c9b44f382824301e6c0fdd6f4cbc52bb00c50995/lib/src/merge.rs#L92.
            PushAllowReason::UnexpectedNoop
        } else {
            // Due to our ref merge rules, this case should happen if an only
            // if:
            //
            // 1. This is a fast-forward.
            // 2. The expected location is an ancestor of both the actual location and the
            //    destination (local position).
            PushAllowReason::ExceptionalFastforward
        })
    } else {
        Err(())
    }
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

impl<'a> RemoteCallbacks<'a> {
    fn into_git(mut self) -> git2::RemoteCallbacks<'a> {
        let mut callbacks = git2::RemoteCallbacks::new();
        if let Some(progress_cb) = self.progress {
            callbacks.transfer_progress(move |progress| {
                progress_cb(&Progress {
                    bytes_downloaded: (progress.received_objects() < progress.total_objects())
                        .then(|| progress.received_bytes() as u64),
                    overall: (progress.indexed_objects() + progress.indexed_deltas()) as f32
                        / (progress.total_objects() + progress.total_deltas()) as f32,
                });
                true
            });
        }
        if let Some(sideband_progress_cb) = self.sideband_progress {
            callbacks.sideband_progress(move |data| {
                sideband_progress_cb(data);
                true
            });
        }
        // TODO: We should expose the callbacks to the caller instead -- the library
        // crate shouldn't read environment variables.
        let mut tried_ssh_agent = false;
        let mut ssh_key_paths_to_try: Option<Vec<PathBuf>> = None;
        callbacks.credentials(move |url, username_from_url, allowed_types| {
            let span = tracing::debug_span!("RemoteCallbacks.credentials");
            let _ = span.enter();

            let git_config = git2::Config::open_default();
            let credential_helper = git_config
                .and_then(|conf| git2::Cred::credential_helper(&conf, url, username_from_url));
            if let Ok(creds) = credential_helper {
                tracing::info!("using credential_helper");
                return Ok(creds);
            } else if let Some(username) = username_from_url {
                if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                    // Try to get the SSH key from the agent once. We don't even check if
                    // $SSH_AUTH_SOCK is set because Windows uses another mechanism.
                    if !tried_ssh_agent {
                        tracing::info!(username, "trying ssh_key_from_agent");
                        tried_ssh_agent = true;
                        return git2::Cred::ssh_key_from_agent(username).map_err(|err| {
                            tracing::error!(err = %err);
                            err
                        });
                    }

                    let paths = ssh_key_paths_to_try.get_or_insert_with(|| {
                        if let Some(ref mut cb) = self.get_ssh_keys {
                            let mut paths = cb(username);
                            paths.reverse();
                            paths
                        } else {
                            vec![]
                        }
                    });

                    if let Some(path) = paths.pop() {
                        tracing::info!(username, path = ?path, "trying ssh_key");
                        return git2::Cred::ssh_key(username, None, &path, None).map_err(|err| {
                            tracing::error!(err = %err);
                            err
                        });
                    }
                }
                if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                    if let Some(ref mut cb) = self.get_password {
                        if let Some(pw) = cb(url, username) {
                            tracing::info!(
                                username,
                                "using userpass_plaintext with username from url"
                            );
                            return git2::Cred::userpass_plaintext(username, &pw).map_err(|err| {
                                tracing::error!(err = %err);
                                err
                            });
                        }
                    }
                }
            } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(ref mut cb) = self.get_username_password {
                    if let Some((username, pw)) = cb(url) {
                        tracing::info!(username, "using userpass_plaintext");
                        return git2::Cred::userpass_plaintext(&username, &pw).map_err(|err| {
                            tracing::error!(err = %err);
                            err
                        });
                    }
                }
            }
            tracing::info!("using default");
            git2::Cred::default()
        });
        callbacks
    }
}

#[derive(Clone, Debug)]
pub struct Progress {
    /// `Some` iff data transfer is currently in progress
    pub bytes_downloaded: Option<u64>,
    pub overall: f32,
}
