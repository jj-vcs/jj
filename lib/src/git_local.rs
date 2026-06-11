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

//! In-process fetch / push / gc for LOCAL (`file://` and plain path) Git
//! remotes, backed by `grit-lib` instead of a `git` subprocess or `gix`
//! transport.
//!
//! Scope: this module covers ONLY the local object+ref copy path. `git://`,
//! `http(s)`, and `ssh` remotes stay on jj's existing subprocess / `gix` path
//! (see [`crate::git_subprocess`]). The functions here translate between jj's
//! production types ([`RefSpec`], [`GitFetchStatus`] / [`GitRefUpdates`],
//! [`GitPushStats`], [`RefToPush`], [`RemoteName`]) and the `grit-lib`
//! `transfer` / `gc` APIs, so the orchestration in [`crate::git`] can adopt them
//! for local remotes without otherwise changing shape.

use std::path::Path;
use std::path::PathBuf;

use bstr::ByteSlice as _;

use grit_lib::objects::ObjectId as GritObjectId;
use grit_lib::transfer::FetchOptions;
use grit_lib::transfer::FetchOutcome;
use grit_lib::transfer::PushOptions;
use grit_lib::transfer::PushRefSpec;
use grit_lib::transfer::TagMode;
use grit_lib::transfer::UpdateMode;
use grit_lib::push_report::PushRefStatus;
use thiserror::Error;

use crate::git::FetchTagsOverride;
use crate::git::GitPushStats;
use crate::git::NegativeRefSpec;
use crate::git::RefSpec;
use crate::git::RefToPush;
use crate::git_subprocess::GitFetchStatus;
use crate::git_subprocess::GitRefUpdates;
use crate::merge::Diff;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefNameBuf;

/// Error raised while performing an in-process local-remote operation.
#[derive(Debug, Error)]
pub enum GitLocalError {
    /// The underlying grit-lib operation failed.
    #[error("local git operation failed: {0}")]
    Grit(String),
    /// An object id produced by grit-lib could not be represented as a
    /// [`gix::ObjectId`] (e.g. unexpected hash width).
    #[error("invalid object id from local git operation: {0}")]
    InvalidOid(String),
}

impl From<grit_lib::error::Error> for GitLocalError {
    fn from(err: grit_lib::error::Error) -> Self {
        Self::Grit(err.to_string())
    }
}

/// If `remote` is a LOCAL remote (`file://` URL or a bare filesystem path),
/// return the path to its git directory, suitable for the `grit-lib` local
/// transfer APIs. Returns `None` for `git://`, `http(s)`, and `ssh` remotes,
/// which must stay on jj's subprocess / `gix` transport path.
///
/// The resolved path is the remote's git directory: the path itself when it is
/// a bare repo (contains an `objects/` dir), or `<path>/.git` when it is a
/// non-bare working copy.
pub(crate) fn local_remote_git_dir(
    remote: &gix::Remote,
    direction: gix::remote::Direction,
) -> Option<PathBuf> {
    let url = remote.url(direction)?;
    // Only `file://` and scheme-less local paths are in scope.
    if url.scheme != gix::url::Scheme::File {
        return None;
    }
    let raw = url.path.to_str().ok()?;
    let path = PathBuf::from(raw);
    Some(resolve_git_dir(&path))
}

/// Resolve a filesystem repo path to its git directory: `<path>/.git` when that
/// exists (non-bare working copy), otherwise the path itself (bare repo).
fn resolve_git_dir(path: &Path) -> PathBuf {
    let dot_git = path.join(".git");
    if dot_git.is_dir() {
        dot_git
    } else {
        path.to_path_buf()
    }
}

/// Convert a grit-lib [`GritObjectId`] into the `gix::ObjectId` jj consumes.
fn to_gix_oid(oid: &GritObjectId) -> Result<gix::ObjectId, GitLocalError> {
    gix::ObjectId::try_from(oid.as_bytes())
        .map_err(|_| GitLocalError::InvalidOid(oid.to_hex()))
}

/// Convert a `gix::ObjectId` into a grit-lib [`GritObjectId`].
fn to_grit_oid(oid: &gix::oid) -> Result<GritObjectId, GitLocalError> {
    GritObjectId::from_bytes(oid.as_bytes()).map_err(|e| GitLocalError::InvalidOid(e.to_string()))
}

/// The null oid matching the hash width of `sample`, used to fill the side of a
/// [`Diff`] that has no value (newly created or deleted refs), mirroring how the
/// subprocess porcelain parser records null oids.
fn null_gix_oid_like(sample: &gix::oid) -> gix::ObjectId {
    gix::ObjectId::null(sample.kind())
}

/// Translate jj's [`FetchTagsOverride`] plus the default behaviour into a
/// grit-lib [`TagMode`].
fn tag_mode(fetch_tags: Option<FetchTagsOverride>) -> TagMode {
    match fetch_tags {
        Some(FetchTagsOverride::AllTags) => TagMode::All,
        Some(FetchTagsOverride::NoTags) => TagMode::None,
        // jj's default fetch follows tags pointing at fetched objects, matching
        // Git's `--tags`-less default.
        None => TagMode::Following,
    }
}

/// Fetch from a LOCAL remote git directory entirely in process.
///
/// `refspecs` / `negative_refspecs` are jj's parsed refspecs; they are rendered
/// to git-format refspec strings (the exact strings the subprocess path would
/// hand to `git fetch`) and passed to [`grit_lib::transfer::fetch_local`]. The
/// returned [`FetchOutcome`] is translated into a [`GitFetchStatus`] carrying a
/// [`GitRefUpdates`], the same result type the subprocess path produces, so
/// callers in [`crate::git`] do not branch on the transport.
///
/// Adopt-ready but not yet wired into [`crate::git::GitFetch::fetch`]: see the
/// note there. grit-lib's `fetch_local` does not yet honor a remote's configured
/// tag policy for the default (no-override) case, nor classify conflicting tag
/// updates as clean rejections, so local fetch stays on the subprocess path
/// until those land. Branch fetching through this helper is correct.
#[expect(dead_code)]
pub(crate) fn fetch_local(
    local_git_dir: &Path,
    remote_git_dir: &Path,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    fetch_tags: Option<FetchTagsOverride>,
) -> Result<GitFetchStatus, GitLocalError> {
    if refspecs.is_empty() {
        return Ok(GitFetchStatus::Updates(GitRefUpdates::default()));
    }

    let opts = FetchOptions {
        refspecs: refspecs.iter().map(|r| r.to_git_format()).collect(),
        negative_refspecs: negative_refspecs.iter().map(|r| r.to_git_format()).collect(),
        tags: tag_mode(fetch_tags),
        prune,
        dry_run: false,
    };

    let outcome = grit_lib::transfer::fetch_local(local_git_dir, remote_git_dir, &opts)?;
    fetch_outcome_to_status(outcome)
}

/// Translate a grit-lib [`FetchOutcome`] into jj's [`GitFetchStatus`].
fn fetch_outcome_to_status(outcome: FetchOutcome) -> Result<GitFetchStatus, GitLocalError> {
    let mut updates = GitRefUpdates::default();
    for update in &outcome.updates {
        // The local tracking ref name is what jj records; updates with no local
        // destination (empty dst refspecs) are not stored and carry no jj-visible
        // ref change.
        let Some(local_ref) = &update.local_ref else {
            continue;
        };
        let name: GitRefNameBuf = local_ref.as_str().into();

        // Fill each missing side with a null oid of the present side's hash
        // width (both sides null is unexpected; default to SHA-1 null).
        let (old, new) = match (&update.old_oid, &update.new_oid) {
            (Some(o), Some(n)) => (to_gix_oid(o)?, to_gix_oid(n)?),
            (Some(o), None) => {
                let o = to_gix_oid(o)?;
                (o, null_gix_oid_like(&o))
            }
            (None, Some(n)) => {
                let n = to_gix_oid(n)?;
                (null_gix_oid_like(&n), n)
            }
            (None, None) => (
                gix::ObjectId::null(gix::hash::Kind::Sha1),
                gix::ObjectId::null(gix::hash::Kind::Sha1),
            ),
        };
        let diff = Diff::new(old, new);

        match update.mode {
            // Successfully applied updates (new / fast-forward / forced) and
            // pruned refs are recorded as `updated`, matching the subprocess
            // porcelain flags ' ', '+', '-', 't', '*'.
            UpdateMode::New
            | UpdateMode::FastForward
            | UpdateMode::Forced
            | UpdateMode::DeletedMissing => updates.updated.push((name, diff)),
            // Rejections map to `rejected`, matching porcelain flag '!'.
            UpdateMode::NonFastForwardRejected
            | UpdateMode::TagUpdateRejected
            | UpdateMode::SourceObjectNotFound => updates.rejected.push((name, diff)),
            // No jj-visible change.
            UpdateMode::UpToDate
            | UpdateMode::NoChangeNeeded
            | UpdateMode::Unborn => {}
        }
    }
    Ok(GitFetchStatus::Updates(updates))
}

/// Push to a LOCAL remote git directory entirely in process.
///
/// Each [`RefToPush`] carries the jj [`RefSpec`] (source object / destination /
/// force) and the expected-old-id used for compare-and-swap (force-with-lease).
/// These are mapped to grit-lib [`PushRefSpec`]s and the
/// [`grit_lib::transfer::PushOutcome`] is translated back into jj's
/// [`GitPushStats`].
///
/// Adopt-ready but not yet wired into [`crate::git::push_updates`]: see the note
/// there. grit-lib's `push_local` does not yet update the local clone's
/// remote-tracking refs, honor the force-with-lease "up-to-date is OK" ordering,
/// or carry `--push-option` values, so local push stays on the subprocess path
/// until those land.
#[expect(dead_code)]
pub(crate) fn push_local(
    local_git_dir: &Path,
    remote_git_dir: &Path,
    references: &[RefToPush],
) -> Result<GitPushStats, GitLocalError> {
    let mut specs: Vec<PushRefSpec> = Vec::with_capacity(references.len());
    for r in references {
        specs.push(ref_to_push_spec(r)?);
    }

    let outcome = grit_lib::transfer::push_local(
        local_git_dir,
        remote_git_dir,
        &specs,
        &PushOptions::default(),
    )?;

    let mut stats = GitPushStats::default();
    for result in &outcome.results {
        let name: GitRefNameBuf = result.remote_ref.as_str().into();
        match result.status {
            PushRefStatus::Ok | PushRefStatus::UpToDate => stats.pushed.push(name),
            // Client-side rejections (lease / non-fast-forward / needs-force).
            PushRefStatus::RejectStale
            | PushRefStatus::RejectNonFastForward
            | PushRefStatus::RejectNeedsForce
            | PushRefStatus::RejectAlreadyExists
            | PushRefStatus::RejectFetchFirst
            | PushRefStatus::AtomicPushFailed => {
                stats.rejected.push((name, result.message.clone()));
            }
            // Declined by the remote side.
            PushRefStatus::RemoteRejected => {
                stats.remote_rejected.push((name, result.message.clone()));
            }
        }
    }
    Ok(stats)
}

/// Build a grit-lib [`PushRefSpec`] from a jj [`RefToPush`].
fn ref_to_push_spec(r: &RefToPush) -> Result<PushRefSpec, GitLocalError> {
    let spec = r.refspec;
    // A refspec with no source side is a deletion (`:refs/heads/foo`).
    let delete = spec.source().is_none();
    let src = match spec.source() {
        Some(s) => Some(resolve_source_oid(s)?),
        None => None,
    };
    // force-with-lease: the expected current value of the remote ref. `None`
    // (absent expected location) disables the CAS check, as in the subprocess
    // `--force-with-lease=<ref>:` form.
    let expected_old = match r.expected_location {
        Some(oid) => Some(to_grit_oid(oid)?),
        None => None,
    };
    Ok(PushRefSpec {
        src,
        dst: spec.destination().to_owned(),
        force: spec.is_forced(),
        delete,
        expected_old,
    })
}

/// Resolve a refspec source (a hex object id, as jj always uses for push) to a
/// grit-lib object id.
fn resolve_source_oid(source: &str) -> Result<GritObjectId, GitLocalError> {
    GritObjectId::from_hex(source)
        .map_err(|_| GitLocalError::InvalidOid(source.to_owned()))
}

/// The default branch of a LOCAL remote, replacing `git remote show <remote>`'s
/// HEAD-symref line for local remotes.
pub(crate) fn remote_default_branch_local(
    remote_git_dir: &Path,
) -> Result<Option<RefNameBuf>, GitLocalError> {
    let branch = grit_lib::gc::remote_default_branch_local(remote_git_dir)?;
    Ok(branch.map(|b| b.as_str().into()))
}
