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
//! Scope: this module covers the local object+ref copy path (`file://` and plain
//! paths) plus an in-process `git://` (anonymous Git-daemon) FETCH path, both
//! backed by `grit-lib`. `http(s)` and `ssh` remotes stay on jj's existing
//! subprocess / `gix` path (see [`crate::git_subprocess`]). The functions here
//! translate between jj's production types ([`RefSpec`], [`GitFetchStatus`] /
//! [`GitRefUpdates`], [`GitPushStats`], [`RefToPush`], `RemoteName`) and the
//! `grit-lib` `transfer` / `fetch` / `gc` APIs, so the orchestration in
//! [`crate::git`] can adopt them without otherwise changing shape.

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
use grit_lib::transport::ConnectOptions;
use grit_lib::transport::GitDaemonTransport;
use grit_lib::transport::Service;
use grit_lib::transport::Transport as _;
use grit_lib::fetch::NoProgress;
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

/// If `remote` is an anonymous `git://` (Git-daemon) remote, return its URL
/// string in the form `grit_lib::transport::parse_git_url` accepts
/// (`git://host[:port]/path`). Returns `None` for every other scheme, which stay
/// on jj's subprocess / `gix` transport path.
///
/// This is the `git://` analogue of [`local_remote_git_dir`]; only the FETCH
/// direction is wired in-process (push over `git://` stays on the subprocess
/// path).
pub(crate) fn git_daemon_remote_url(
    remote: &gix::Remote,
    direction: gix::remote::Direction,
) -> Option<String> {
    let url = remote.url(direction)?;
    if url.scheme != gix::url::Scheme::Git {
        return None;
    }
    // `to_bstring` reproduces the canonical `git://host[:port]/path` form, which
    // `parse_git_url` parses; require valid UTF-8 (Git daemon URLs always are).
    let s = url.to_bstring();
    s.to_str().ok().map(|s| s.to_owned())
}

/// Whether the remote at `remote_git_dir` has any receive hook installed
/// (`pre-receive`, `update`, `post-receive`). The in-process push path does not
/// run hooks, so callers route pushes to such remotes through the subprocess
/// path, which does (and can also reject the push or forward push options).
pub(crate) fn remote_has_receive_hooks(remote_git_dir: &Path) -> bool {
    let hooks = remote_git_dir.join("hooks");
    ["pre-receive", "update", "post-receive"]
        .iter()
        .any(|name| {
            let p = hooks.join(name);
            // A hook is active when the file exists and is executable (Git's
            // `find_hook`). On non-unix, existence is sufficient.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::metadata(&p)
                    .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                p.is_file()
            }
        })
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

/// Translate jj's [`FetchTagsOverride`] into a grit-lib [`TagMode`]. With no
/// explicit override, fall back to the remote's configured tag policy
/// (`remote.<name>.tagOpt`), exactly as the subprocess path lets `git fetch`
/// consult it.
fn tag_mode(fetch_tags: Option<FetchTagsOverride>, configured: gix::remote::fetch::Tags) -> TagMode {
    match fetch_tags {
        Some(FetchTagsOverride::AllTags) => TagMode::All,
        Some(FetchTagsOverride::NoTags) => TagMode::None,
        None => match configured {
            gix::remote::fetch::Tags::All => TagMode::All,
            gix::remote::fetch::Tags::None => TagMode::None,
            // Git's default: follow tags pointing at fetched objects.
            gix::remote::fetch::Tags::Included => TagMode::Following,
        },
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
/// Wired into [`crate::git::GitFetch::fetch`] for local remotes (shallow/`depth`
/// fetches still use the subprocess path). `configured_tags` is the remote's
/// `tagOpt` policy, consulted when no explicit `fetch_tags` override is given.
pub(crate) fn fetch_local(
    local_git_dir: &Path,
    remote_git_dir: &Path,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    fetch_tags: Option<FetchTagsOverride>,
    configured_tags: gix::remote::fetch::Tags,
) -> Result<GitFetchStatus, GitLocalError> {
    if refspecs.is_empty() {
        return Ok(GitFetchStatus::Updates(GitRefUpdates::default()));
    }

    let opts = FetchOptions {
        refspecs: refspecs.iter().map(|r| r.to_git_format()).collect(),
        negative_refspecs: negative_refspecs.iter().map(|r| r.to_git_format()).collect(),
        tags: tag_mode(fetch_tags, configured_tags),
        prune,
        dry_run: false,
    };

    let outcome = grit_lib::transfer::fetch_local(local_git_dir, remote_git_dir, &opts)?;
    fetch_outcome_to_status(outcome)
}

/// Fetch from an anonymous `git://` (Git-daemon) remote entirely in process,
/// over a TCP pkt-line connection, with no `git` subprocess and no `gix`
/// transport.
///
/// This mirrors [`fetch_local`] but drives the wire protocol:
/// [`grit_lib::transport::GitDaemonTransport`] connects and reads the
/// advertisement, then [`grit_lib::fetch::fetch_remote`] runs the want/have
/// negotiation, ingests the pack, and writes the tracking refs. The returned
/// [`FetchOutcome`] has the same shape as the local path, so
/// [`fetch_outcome_to_status`] converts it identically.
///
/// Protocol v2 is requested explicitly (git's default for `git://`); the server
/// silently downgrades to v0/v1 if it lacks v2, and grit-lib handles either.
/// Shallow/`depth` fetches stay on the subprocess path.
pub(crate) fn fetch_git_daemon(
    local_git_dir: &Path,
    remote_url: &str,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    fetch_tags: Option<FetchTagsOverride>,
    configured_tags: gix::remote::fetch::Tags,
) -> Result<GitFetchStatus, GitLocalError> {
    if refspecs.is_empty() {
        return Ok(GitFetchStatus::Updates(GitRefUpdates::default()));
    }

    let opts = FetchOptions {
        refspecs: refspecs.iter().map(|r| r.to_git_format()).collect(),
        negative_refspecs: negative_refspecs.iter().map(|r| r.to_git_format()).collect(),
        tags: tag_mode(fetch_tags, configured_tags),
        prune,
        dry_run: false,
    };

    // Connect to the daemon. Request protocol v2 explicitly (git's own default
    // for `git://` is v2-capable); grit-lib's daemon transport performs the v2
    // handshake and `fetch_remote` runs the v2 `ls-refs` + `command=fetch`
    // negotiation. A server that lacks v2 silently downgrades to v0/v1, which
    // `fetch_remote` also handles, so no explicit fallback is needed here.
    let transport = GitDaemonTransport::new();
    let connect_opts = ConnectOptions {
        protocol_version: 2,
        ..Default::default()
    };
    let mut conn = transport.connect(remote_url, Service::UploadPack, &connect_opts)?;

    let mut progress = NoProgress;
    let outcome =
        grit_lib::fetch::fetch_remote(local_git_dir, conn.as_mut(), &opts, &mut progress)?;
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
/// On a successful push, Git updates the local clone's remote-tracking ref for
/// the pushed branch (`refs/heads/<b>` → `refs/remotes/<remote>/<b>`); this
/// helper mirrors that so jj's import sees the same state as a subprocess push.
/// `--push-option` forwarding to remote hooks is not handled here: callers route
/// pushes that carry push options to the subprocess path.
pub(crate) fn push_local(
    local_git_dir: &Path,
    remote_git_dir: &Path,
    references: &[RefToPush],
    fetch_refspecs: &[String],
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
    let mut tracking_updates: Vec<grit_lib::gc::RefTransactionItem> = Vec::new();
    for result in &outcome.results {
        let name: GitRefNameBuf = result.remote_ref.as_str().into();
        match result.status {
            PushRefStatus::Ok | PushRefStatus::UpToDate => {
                // Update the local clone's remote-tracking ref for a pushed
                // branch, but only when a fetch refspec maps it — exactly as Git
                // does after a push (a remote with a narrow fetch refspec leaves
                // unmapped tracking refs untouched).
                if let Some(tracking) = mapped_tracking_ref(&result.remote_ref, fetch_refspecs) {
                    tracking_updates.push(grit_lib::gc::RefTransactionItem {
                        name: tracking,
                        // Deletion clears the tracking ref; otherwise point it at
                        // the just-pushed object.
                        new_oid: if result.deletion { None } else { result.new_oid },
                        expected_old: None,
                    });
                }
                stats.pushed.push(name);
            }
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

    if !tracking_updates.is_empty() {
        grit_lib::gc::update_refs(local_git_dir, &tracking_updates)?;
    }
    Ok(stats)
}

/// Map a just-pushed ref (e.g. `refs/heads/main`) to the local remote-tracking
/// ref a fetch refspec would place it under (e.g. `refs/remotes/origin/main`),
/// or `None` when no fetch refspec maps it. This mirrors Git updating only the
/// tracking refs covered by the remote's fetch refspecs after a push.
fn mapped_tracking_ref(pushed_ref: &str, fetch_refspecs: &[String]) -> Option<String> {
    for spec in fetch_refspecs {
        let body = spec.strip_prefix('+').unwrap_or(spec);
        let Some((src, dst)) = body.split_once(':') else {
            continue;
        };
        if dst.is_empty() {
            continue;
        }
        if let Some(star) = src.find('*') {
            // Wildcard refspec: match the fixed prefix/suffix and substitute the
            // captured segment into the destination's `*`.
            let (prefix, suffix) = (&src[..star], &src[star + 1..]);
            if pushed_ref.len() >= prefix.len() + suffix.len()
                && pushed_ref.starts_with(prefix)
                && pushed_ref.ends_with(suffix)
            {
                let middle = &pushed_ref[prefix.len()..pushed_ref.len() - suffix.len()];
                if let Some(dstar) = dst.find('*') {
                    return Some(format!("{}{}{}", &dst[..dstar], middle, &dst[dstar + 1..]));
                }
            }
        } else if src == pushed_ref {
            return Some(dst.to_owned());
        }
    }
    None
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
    // force-with-lease: jj always pushes with a lease equal to its view of the
    // remote ref. `Some(oid)` expects that value; `None` expects the ref to be
    // *absent* (jj has no remote-tracking entry for it). The subprocess path
    // expresses these as `--force-with-lease=<ref>:<oid>` and `=<ref>:` (empty,
    // i.e. must not exist) respectively.
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
        expect_absent: r.expected_location.is_none(),
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
